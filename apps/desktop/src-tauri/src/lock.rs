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
    if state.lock_policy.disconnect_on_lock.load(Ordering::Relaxed) {
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
            let minutes = state.lock_policy.auto_lock_minutes.load(Ordering::Relaxed);
            if minutes == 0 || !state.vault().is_unlocked() {
                continue;
            }
            if state.idle_for() >= Duration::from_secs(u64::from(minutes) * 60) {
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
