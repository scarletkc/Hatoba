//! Locking: manual (Ctrl+Shift+L), idle timeout and system sleep (SEC-01/02/03).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use tauri::AppHandle;
use tauri_specta::Event;

use crate::dto::{LocalPrefs, LockReason, VaultLockedEvent};
use crate::state::{AppState, state};
use crate::sync;

/// The device's lock settings (SEC-02/03), copied out of the device-local prefs so that each
/// decision reads them without the vault's mutex.
#[derive(Default)]
pub struct LockPolicy {
    auto_lock_minutes: AtomicU32,
    disconnect_on_lock: AtomicBool,
    lock_on_sleep: AtomicBool,
}

/// Reads the policy from the prefs, which are readable while locked. Runs after unlock and
/// whenever the prefs are saved.
pub fn refresh_policy(state: &AppState) {
    let stored = state.vault().local_prefs().ok().flatten();
    let prefs = stored
        .map(|json| LocalPrefs::from_stored(&json))
        .unwrap_or_default();
    let policy = &state.lock_policy;
    policy
        .auto_lock_minutes
        .store(prefs.auto_lock_minutes, Ordering::Relaxed);
    policy
        .disconnect_on_lock
        .store(prefs.lock_disconnects_sessions, Ordering::Relaxed);
    policy
        .lock_on_sleep
        .store(prefs.lock_on_sleep, Ordering::Relaxed);
}

/// How long the user has been away: the system's idle time where the platform reports it
/// (`platform::idle`), never more than the time since the last activity in Hatoba's window,
/// which is all there is elsewhere.
fn idle_time(state: &AppState) -> Duration {
    let window = state.idle_for();
    crate::platform::idle::system_idle().map_or(window, |system| system.min(window))
}

/// Whether the vault has been idle past the timeout. A policy of 0 minutes never locks.
fn idle_lock_due(state: &AppState, idle: Duration) -> bool {
    let minutes = state.lock_policy.auto_lock_minutes.load(Ordering::Relaxed);
    minutes != 0
        && state.vault().is_unlocked()
        && idle >= Duration::from_secs(u64::from(minutes) * 60)
}

fn disconnects_on_lock(state: &AppState) -> bool {
    state.lock_policy.disconnect_on_lock.load(Ordering::Relaxed)
}

fn locks_on_sleep(state: &AppState) -> bool {
    state.lock_policy.lock_on_sleep.load(Ordering::Relaxed)
}

/// Locks the vault: wipes keys and decrypted items from memory (zeroize, SEC-01), stops sync and
/// tells the WebView. Live SSH sessions stay connected but masked unless the user chose otherwise.
/// Every AI turn and tool stops first, so the assistant never acts behind the lock screen (§13.1),
/// and every MCP server stops (AI-32).
///
/// What needs no waiting comes first: the lock screen, sync and the deployment tokens. Stopping
/// the MCP servers can take seconds (a slow `stdio` server gets a grace period to exit), so it
/// comes after them, alongside closing the SSH sessions.
pub async fn lock_vault(app: &AppHandle, reason: LockReason) {
    let state = state(app);
    let was_unlocked = {
        let mut vault = state.vault();
        let was = vault.is_unlocked();
        // Under the same guard: calls without a result get their cancelled result while the
        // vault can still store it, and nothing writes between the stop and the lock.
        state.ai.stop_all(&mut vault);
        // SSH-16: the hosts' connections stay reusable after the unlock.
        if was {
            state.connections.note_current(&vault);
        }
        vault.lock();
        was
    };
    if was_unlocked {
        tracing::info!("vault locked ({reason:?})");
        let _ = VaultLockedEvent { reason }.emit(app);
    }
    sync::stop(app);
    // DEPLOY-07: a deployment's tokens do not outlive the unlocked vault.
    state.deploy.clear();
    // HOST-14: the dashboard reads nothing behind the lock screen, and its connections close.
    // The hashes of credentials it compares start over, so none outlives the unlocked vault.
    state.dashboard.stop_all();
    crate::dashboard::forget_secret_hashes();
    // HOST-15: what other programs run through the `hatoba` command stops, and its connections
    // close.
    state.cli.lock(app);
    let disconnect = async {
        if disconnects_on_lock(&state) {
            // The sessions open now: one opened after an unlock in the meantime stays.
            let sessions = state.ssh.take_all();
            // SSH-16: and the hosts' connections, which close once the tabs on them did.
            let connections = state.connections.take_all();
            let locals = state.local.take_all();
            let ssh = futures::future::join_all(sessions.into_iter().map(|(id, live)| {
                let ai = &state.ai;
                async move {
                    // A file write the assistant began on it runs to its end first.
                    ai.file_writes_ended(&id).await;
                    live.close().await;
                }
            }));
            let local = futures::future::join_all(locals.into_iter().map(|(id, live)| {
                let ai = &state.ai;
                async move {
                    ai.file_writes_ended(&id).await;
                    live.close();
                }
            }));
            futures::join!(ssh, local);
            futures::future::join_all(connections.iter().map(|c| c.close())).await;
            crate::connection::changed(app);
        }
    };
    // AI-32: every MCP server stops, and `http` sessions close, with the vault.
    futures::join!(state.mcp.stop_all(&state.vault), disconnect);
}

/// Background watchers for idle timeout and system sleep.
pub fn spawn_watchers(app: AppHandle) {
    let idle_app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(15)).await;
            let state = state(&idle_app);
            if idle_lock_due(&state, idle_time(&state)) {
                lock_vault(&idle_app, LockReason::Idle).await;
            }
        }
    });
    tauri::async_runtime::spawn(async move {
        let mut sleeps = crate::platform::power::watch_sleep();
        while sleeps.recv().await.is_some() {
            let state = state(&app);
            if locks_on_sleep(&state) && state.vault().is_unlocked() {
                lock_vault(&app, LockReason::Sleep).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use hatoba_core::{KdfParams, Vault};
    use serde_json::json;

    use super::{disconnects_on_lock, idle_lock_due, locks_on_sleep, refresh_policy};
    use crate::state::AppState;

    const PW: &str = "correct horse battery staple";
    const IDLE: Duration = Duration::from_secs(75);

    fn unlocked() -> AppState {
        let mut vault = Vault::open_in_memory().unwrap();
        vault
            .create_with_params(PW, KdfParams::for_tests())
            .unwrap();
        AppState::new(vault, false)
    }

    #[test]
    fn the_policy_follows_the_device_prefs() {
        let state = unlocked();
        refresh_policy(&state);
        // The defaults: 15 minutes, lock on sleep, keep the sessions.
        assert!(!idle_lock_due(&state, IDLE));
        assert!(idle_lock_due(&state, Duration::from_secs(15 * 60)));
        assert!(locks_on_sleep(&state) && !disconnects_on_lock(&state));

        let prefs = json!({
            "auto_lock_minutes": 1,
            "lock_on_sleep": false,
            "lock_disconnects_sessions": true,
        });
        state.vault().set_local_prefs(&prefs.to_string()).unwrap();
        refresh_policy(&state);
        assert!(idle_lock_due(&state, IDLE));
        assert!(!idle_lock_due(&state, Duration::from_secs(30)));
        assert!(!locks_on_sleep(&state) && disconnects_on_lock(&state));

        state
            .vault()
            .set_local_prefs(r#"{"auto_lock_minutes":0}"#)
            .unwrap();
        refresh_policy(&state);
        assert!(!idle_lock_due(&state, Duration::from_secs(24 * 60 * 60)));
    }
}
