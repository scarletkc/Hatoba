//! Process-wide state managed by Tauri. Secrets only ever live here, in Rust (spec §3.2).

use std::sync::{Arc, Mutex, MutexGuard};

use std::time::Instant;

use hatoba_core::vault::Vault;
use tauri::{AppHandle, Manager};

use crate::ai::AiManager;
use crate::deploy::Deployments;
use crate::error::{AppError, AppResult};
use crate::lock::LockPolicy;
use crate::platform::secrets::KeyringStore;
use crate::ssh::SshManager;
use crate::sync::SyncController;

pub use hatoba_core::sync::SharedVault;

pub struct AppState {
    pub vault: SharedVault,
    pub secrets: Arc<KeyringStore>,
    pub ssh: SshManager,
    pub sync: SyncController,
    /// The in-app deployment in progress (§6.7).
    pub deploy: Deployments,
    /// The AI assistant's running turns and tools (§13.1).
    pub ai: AiManager,
    pub mica: bool,
    pub lock_policy: LockPolicy,
    last_activity: Mutex<Instant>,
}

impl AppState {
    pub fn new(vault: Vault, mica: bool) -> Self {
        Self {
            vault: Arc::new(Mutex::new(vault)),
            secrets: Arc::new(KeyringStore),
            ssh: SshManager::default(),
            sync: SyncController::default(),
            deploy: Deployments::default(),
            ai: AiManager::default(),
            mica,
            lock_policy: LockPolicy::default(),
            last_activity: Mutex::new(Instant::now()),
        }
    }

    /// Locks the vault mutex. Poisoning only happens after a panic while holding it; the vault's
    /// own invariants are transactional, so recovering the guard is safe.
    pub fn vault(&self) -> MutexGuard<'_, Vault> {
        self.vault.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Runs `f` with the vault, requiring it to be unlocked.
    pub fn with_unlocked<R>(&self, f: impl FnOnce(&mut Vault) -> AppResult<R>) -> AppResult<R> {
        let mut vault = self.vault();
        if !vault.is_unlocked() {
            return Err(AppError::locked());
        }
        f(&mut vault)
    }

    pub fn touch_activity(&self) {
        *self.last_activity.lock().unwrap_or_else(|p| p.into_inner()) = Instant::now();
    }

    pub fn idle_for(&self) -> std::time::Duration {
        self.last_activity
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .elapsed()
    }
}

pub fn state(app: &AppHandle) -> tauri::State<'_, AppState> {
    app.state::<AppState>()
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// Runs CPU-heavy work (Argon2id, RSA generation) off the async runtime.
pub async fn blocking<R: Send + 'static>(
    f: impl FnOnce() -> AppResult<R> + Send + 'static,
) -> AppResult<R> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError::internal(format!("background task failed: {e}")))?
}
