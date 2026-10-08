//! Vault lifecycle (spec §8.1, §4.3): create, unlock, lock, password change, recovery, backup.

use std::sync::Arc;

use hatoba_core::sync::{SyncConfig, flows, save_session};
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

use crate::commands::settings::adopt_legacy_prefs_after_unlock;
use crate::commands::sync::{backend_from_input, persist};
use crate::dto::{LockReason, SyncConfigInput, SyncKind, VaultState, VaultStatus};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::platform::biometric;
use crate::state::{AppState, blocking};
use crate::{lock, sync};

/// Common work after any successful unlock (password, recovery code, biometrics, restore).
fn after_unlock(app: &AppHandle, state: &AppState) {
    // Only when sync is off; otherwise `sync::run_round` does it after pulling.
    if let Err(e) = state.with_unlocked(adopt_legacy_prefs_after_unlock) {
        tracing::warn!(
            "could not move the terminal prefs into the synced settings: {}",
            e.detail
        );
    }
    // Message parts of conversations deleted while this device added to them (§13.7); every
    // sync round does the same after its pull.
    match state.with_unlocked(|v| Ok(v.ai_sweep_orphaned_parts()?)) {
        Ok(0) => {}
        Ok(parts) => tracing::info!(parts, "deleted message parts of deleted conversations"),
        Err(e) => tracing::warn!("could not delete orphaned message parts: {}", e.detail),
    }
    lock::refresh_policy(state);
    state.touch_activity();
    sync::on_unlock(app);
}

#[tauri::command]
#[specta::specta]
pub fn vault_status(state: State<'_, AppState>) -> VaultStatus {
    let vault = state.vault();
    let info = vault.status();
    let sync_kind = match vault.sync_config().ok().flatten() {
        None => SyncKind::None,
        Some(SyncConfig::Worker { .. }) => SyncKind::Worker,
        Some(SyncConfig::D1 { .. }) => SyncKind::D1,
    };
    VaultStatus {
        state: if !info.initialized {
            VaultState::Uninitialized
        } else if info.unlocked {
            VaultState::Unlocked
        } else {
            VaultState::Locked
        },
        failed_attempts: info.failed_attempts,
        retry_at: info.retry_at,
        sync_kind,
        biometric_available: biometric::available(),
        biometric_enabled: info.initialized && biometric::enrolled(state.secrets.as_ref()),
    }
}

/// VAULT-01/02: creates the vault and returns the recovery code (shown exactly once).
#[tauri::command]
#[specta::specta]
pub async fn vault_create(
    app: AppHandle,
    state: State<'_, AppState>,
    password: String,
) -> AppResult<String> {
    if password.chars().count() < 8 {
        return Err(AppError::invalid(
            "password",
            "the master password must have at least 8 characters",
        ));
    }
    let password = Zeroizing::new(password);
    let vault = state.vault.clone();
    let code = blocking(move || {
        Ok(vault
            .lock()
            .map_err(|_| AppError::internal("vault poisoned"))?
            .create(&password)?)
    })
    .await?;
    after_unlock(&app, &state);
    Ok(code.to_string())
}

/// VAULT-03 / SEC-06: wrong passwords are throttled with increasing delays (persisted).
#[tauri::command]
#[specta::specta]
pub async fn vault_unlock(
    app: AppHandle,
    state: State<'_, AppState>,
    password: String,
) -> AppResult<()> {
    let password = Zeroizing::new(password);
    let vault = state.vault.clone();
    blocking(move || {
        Ok(vault
            .lock()
            .map_err(|_| AppError::internal("vault poisoned"))?
            .unlock(&password)?)
    })
    .await?;
    after_unlock(&app, &state);
    Ok(())
}

/// SEC-07 (P1): Windows Hello.
#[tauri::command]
#[specta::specta]
pub async fn vault_unlock_biometric(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let key = biometric::unlock(&app, state.secrets.as_ref()).await?;
    state.vault().unlock_with_vault_key(key)?;
    after_unlock(&app, &state);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn vault_lock(app: AppHandle) {
    lock::lock_vault(&app, LockReason::Manual).await;
}

#[tauri::command]
#[specta::specta]
pub async fn vault_verify_password(
    state: State<'_, AppState>,
    password: String,
) -> AppResult<bool> {
    let password = Zeroizing::new(password);
    let vault = state.vault.clone();
    blocking(move || {
        Ok(vault
            .lock()
            .map_err(|_| AppError::internal("vault poisoned"))?
            .verify_password(&password)?)
    })
    .await
}

/// VAULT-05: re-wraps only the vault key (no item is re-encrypted); with sync, the server copy is
/// updated atomically and other devices' sessions are revoked.
#[tauri::command]
#[specta::specta]
pub async fn vault_change_password(
    app: AppHandle,
    state: State<'_, AppState>,
    current: String,
    next: String,
) -> AppResult<()> {
    if next.chars().count() < 8 {
        return Err(AppError::invalid(
            "next",
            "the master password must have at least 8 characters",
        ));
    }
    match state.sync.backend() {
        Some(backend) => {
            flows::change_password_remote(&state.vault, backend.as_ref(), &current, &next).await?
        }
        None if state.sync_configured() => {
            return Err(AppError::new(
                ErrorCode::SyncOffline,
                "sync must be reachable to change the master password",
            ));
        }
        None => {
            let (cur, new) = (Zeroizing::new(current), Zeroizing::new(next));
            let vault = state.vault.clone();
            blocking(move || {
                vault
                    .lock()
                    .map_err(|_| AppError::internal("vault poisoned"))?
                    .change_password(&cur, &new)?;
                Ok(())
            })
            .await?;
        }
    }
    // Biometric unlock wraps the vault key, which did not change, so it stays valid.
    sync::emit_status(&app);
    Ok(())
}

/// VAULT-06: resets the master password with the recovery code. Works offline when sync is off.
#[tauri::command]
#[specta::specta]
pub async fn vault_recover(
    app: AppHandle,
    state: State<'_, AppState>,
    recovery_code: String,
    new_password: String,
) -> AppResult<()> {
    if new_password.chars().count() < 8 {
        return Err(AppError::invalid(
            "new_password",
            "the master password must have at least 8 characters",
        ));
    }
    let config = state.vault().sync_config()?;
    match config {
        Some(cfg) => {
            let backend = sync::backend_for(&cfg, state.secrets.as_ref())?;
            let session = flows::recover_remote(
                &state.vault,
                backend.as_ref(),
                &recovery_code,
                &new_password,
                sync::device_info(),
            )
            .await?;
            save_session(state.secrets.as_ref(), &session)?;
        }
        None => {
            let (code, pw) = (Zeroizing::new(recovery_code), Zeroizing::new(new_password));
            let vault = state.vault.clone();
            blocking(move || {
                vault
                    .lock()
                    .map_err(|_| AppError::internal("vault poisoned"))?
                    .recover(&code, &pw)?;
                Ok(())
            })
            .await?;
        }
    }
    after_unlock(&app, &state);
    Ok(())
}

/// Flow B (§6.6): first launch on a new device — download the vault from the cloud.
#[tauri::command]
#[specta::specta]
pub async fn vault_restore_from_cloud(
    app: AppHandle,
    state: State<'_, AppState>,
    config: SyncConfigInput,
    password: String,
) -> AppResult<()> {
    let (cfg, backend) = backend_from_input(&config)?;
    let session = flows::restore_from_cloud(
        &state.vault,
        backend.as_ref(),
        &password,
        sync::device_info(),
    )
    .await?;
    persist(&state, &cfg, &config, &session)?;
    state.sync.set_backend(Some(Arc::clone(&backend)));
    after_unlock(&app, &state);
    Ok(())
}

/// VAULT-07: single-file encrypted backup in the same envelope format.
#[tauri::command]
#[specta::specta]
pub fn vault_export_backup(state: State<'_, AppState>, path: String) -> AppResult<()> {
    state.with_unlocked(|v| Ok(v.export_backup(std::path::Path::new(&path))?))
}

/// Issues a new recovery code (the old one stops working everywhere once synced).
#[tauri::command]
#[specta::specta]
pub async fn vault_rotate_recovery(
    state: State<'_, AppState>,
    password: String,
) -> AppResult<String> {
    let code = match state.sync.backend() {
        Some(backend) => {
            flows::rotate_recovery_remote(&state.vault, backend.as_ref(), &password).await?
        }
        None if state.sync_configured() => {
            return Err(AppError::new(
                ErrorCode::SyncOffline,
                "sync must be reachable to create a new recovery code",
            ));
        }
        None => {
            let pw = Zeroizing::new(password);
            let vault = state.vault.clone();
            blocking(move || {
                Ok(vault
                    .lock()
                    .map_err(|_| AppError::internal("vault poisoned"))?
                    .rotate_recovery(&pw)?
                    .0)
            })
            .await?
        }
    };
    Ok(code.to_string())
}

/// SEC-07 (P1): enrol Windows Hello. Requires the master password as a fresh confirmation.
#[tauri::command]
#[specta::specta]
pub async fn biometric_enable(
    app: AppHandle,
    state: State<'_, AppState>,
    password: String,
) -> AppResult<()> {
    if !vault_verify_password(state.clone(), password).await? {
        return Err(AppError::new(
            ErrorCode::WrongPassword,
            "wrong master password",
        ));
    }
    let key = state.with_unlocked(|v| Ok(v.vault_key_copy()?))?;
    biometric::enroll(&app, state.secrets.as_ref(), &key).await
}

#[tauri::command]
#[specta::specta]
pub fn biometric_disable(state: State<'_, AppState>) -> AppResult<()> {
    biometric::remove(state.secrets.as_ref())
}
