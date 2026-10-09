//! Locking: manual (Ctrl+Shift+L), idle timeout and system sleep (SEC-01/02/03).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use tauri::AppHandle;
use tauri_specta::Event;

use crate::dto::{LockReason, VaultLockedEvent};
use crate::state::{AppState, state};
use crate::sync;

/// Copied out of the synced settings while unlocked, because they must be honoured after the
/// vault (and with it the settings item) is locked.
#[derive(Default)]
pub struct LockPolicy {
    auto_lock_minutes: AtomicU32,
    disconnect_on_lock: AtomicBool,
}

pub fn refresh_policy(state: &AppState) {
    let vault = state.vault();
    if vault.is_unlocked() {
        let s = vault.settings();
        state
            .lock_policy
            .auto_lock_minutes
            .store(s.auto_lock_minutes, Ordering::Relaxed);
        state
            .lock_policy
            .disconnect_on_lock
            .store(s.lock_disconnects_sessions, Ordering::Relaxed);
    }
}

/// Runs a sync round, then refreshes the policy from whatever settings it pulled. The refresh
/// also runs when the round fails, because pages applied before the failure stay applied.
pub async fn refresh_after_sync<T>(state: &AppState, round: impl Future<Output = T>) -> T {
    let result = round.await;
    refresh_policy(state);
    result
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

/// Locks the vault: wipes keys and decrypted items from memory (zeroize, SEC-01), stops sync and
/// tells the WebView. Live SSH sessions stay connected but masked unless the user chose otherwise.
pub async fn lock_vault(app: &AppHandle, reason: LockReason) {
    let state = state(app);
    let was_unlocked = {
        let mut vault = state.vault();
        let was = vault.is_unlocked();
        vault.lock();
        was
    };
    sync::stop(app);
    // DEPLOY-07: a deployment's tokens do not outlive the unlocked vault.
    state.deploy.clear();
    if disconnects_on_lock(&state) {
        state.ssh.disconnect_all().await;
    }
    if was_unlocked {
        tracing::info!("vault locked ({reason:?})");
        let _ = VaultLockedEvent { reason }.emit(app);
    }
}

/// Background watchers for idle timeout and system sleep.
pub fn spawn_watchers(app: AppHandle) {
    let idle_app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(15)).await;
            let state = state(&idle_app);
            if idle_lock_due(&state, state.idle_for()) {
                lock_vault(&idle_app, LockReason::Idle).await;
            }
        }
    });
    tauri::async_runtime::spawn(async move {
        let mut sleeps = crate::platform::power::watch_sleep();
        while sleeps.recv().await.is_some() {
            if state(&app).vault().is_unlocked() {
                lock_vault(&app, LockReason::Sleep).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use hatoba_core::model::{Item, SETTINGS_ID};
    use hatoba_core::{Error, KdfParams, Vault};

    use super::{disconnects_on_lock, idle_lock_due, refresh_after_sync, refresh_policy};
    use crate::state::AppState;

    const PW: &str = "correct horse battery staple";

    /// An unlocked vault whose policy was cached as "never lock, keep sessions".
    fn unlocked_with_old_policy() -> AppState {
        let mut vault = Vault::open_in_memory().unwrap();
        vault
            .create_with_params(PW, KdfParams::for_tests())
            .unwrap();
        let mut settings = vault.settings();
        settings.auto_lock_minutes = 0;
        settings.lock_disconnects_sessions = false;
        vault
            .put(Some(SETTINGS_ID), Item::Settings(settings))
            .unwrap();
        let state = AppState::new(vault, false);
        refresh_policy(&state);
        state
    }

    /// What applying a pulled settings item does to the local vault.
    fn apply_remote_settings(state: &AppState) {
        let mut vault = state.vault();
        let mut settings = vault.settings();
        settings.auto_lock_minutes = 1;
        settings.lock_disconnects_sessions = true;
        vault
            .put(Some(SETTINGS_ID), Item::Settings(settings))
            .unwrap();
    }

    const IDLE: Duration = Duration::from_secs(75);

    #[tokio::test]
    async fn pulled_settings_take_effect_after_the_round() {
        let state = unlocked_with_old_policy();
        assert!(!idle_lock_due(&state, IDLE));
        assert!(!disconnects_on_lock(&state));

        refresh_after_sync(&state, async { apply_remote_settings(&state) }).await;

        assert!(idle_lock_due(&state, IDLE));
        assert!(!idle_lock_due(&state, Duration::from_secs(30)));
        assert!(disconnects_on_lock(&state));
    }

    #[tokio::test]
    async fn settings_pulled_before_a_failure_still_take_effect() {
        let state = unlocked_with_old_policy();

        let result = refresh_after_sync(&state, async {
            apply_remote_settings(&state);
            Err::<(), _>(Error::Protocol("pull cursor did not advance".into()))
        })
        .await;

        assert!(result.is_err());
        assert!(idle_lock_due(&state, IDLE));
        assert!(disconnects_on_lock(&state));
    }
}
