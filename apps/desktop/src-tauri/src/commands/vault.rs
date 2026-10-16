//! Vault lifecycle (spec §8.1, §4.3): create, unlock, lock, password change, recovery, backup.

use std::sync::Arc;

use hatoba_core::platform::{SecretStore, secret_keys};
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

/// SEC-09: the one requirement a new master password has; a weak one only gets a warning.
const MIN_PASSWORD_CHARS: usize = 8;

/// SEC-11: a PIN has at least this many characters, of any kind.
const MIN_PIN_CHARS: usize = 4;

/// Refuses a new master password shorter than [`MIN_PASSWORD_CHARS`] as invalid `field`.
fn check_new_password(field: &str, password: &str) -> AppResult<()> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(AppError::invalid(
            field,
            format!("the master password must have at least {MIN_PASSWORD_CHARS} characters"),
        ));
    }
    Ok(())
}

/// Refuses a PIN shorter than [`MIN_PIN_CHARS`] as invalid `pin`.
fn check_new_pin(pin: &str) -> AppResult<()> {
    if pin.chars().count() < MIN_PIN_CHARS {
        return Err(AppError::invalid(
            "pin",
            format!("the PIN must have at least {MIN_PIN_CHARS} characters"),
        ));
    }
    Ok(())
}

/// SEC-11: a PIN that works after a restart keeps the vault key it wraps in the OS credential
/// store. Written again after every unlock, since the last wrong PIN removes it there.
fn keep_pin_for_restart(state: &AppState) {
    let wrapped = {
        let vault = state.vault();
        if !vault.pin_status().persist {
            return;
        }
        vault.pin_wrapped_key()
    };
    if let Some(wrapped) = wrapped
        && let Err(e) = state.secrets.set(secret_keys::PIN_VAULT_KEY, &wrapped)
    {
        tracing::warn!("could not keep the PIN for after a restart: {e}");
    }
}

/// Removes the PIN's copy in the OS credential store. A copy left behind is harmless: it is read
/// only while the PIN is kept for after a restart, and setting that writes it again.
fn forget_stored_pin(state: &AppState) {
    if let Err(e) = state.secrets.delete(secret_keys::PIN_VAULT_KEY) {
        tracing::warn!("could not remove the PIN kept for after a restart: {e}");
    }
}

/// Common work after any successful unlock (password, recovery code, biometrics, PIN, restore).
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
    keep_pin_for_restart(state);
    lock::refresh_policy(state);
    state.touch_activity();
    sync::on_unlock(app);
}

#[tauri::command]
#[specta::specta]
pub fn vault_status(state: State<'_, AppState>) -> VaultStatus {
    let vault = state.vault();
    let info = vault.status();
    let pin = vault.pin_status();
    // After a restart, a PIN kept for that unlocks with its copy in the OS credential store. A PIN
    // out of attempts does not, even when that copy is still there.
    let pin_unlock = !pin.exhausted
        && (pin.armed
            || (pin.persist
                && state
                    .secrets
                    .get(secret_keys::PIN_VAULT_KEY)
                    .ok()
                    .flatten()
                    .is_some()));
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
        pin_enabled: pin.enabled,
        pin_persist: pin.persist,
        pin_unlock: info.initialized && pin.enabled && pin_unlock,
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
    check_new_password("password", &password)?;
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

/// SEC-11: unlocks with the PIN. After a restart, a PIN kept for that unwraps its copy in the OS
/// credential store, which the last wrong PIN removes.
#[tauri::command]
#[specta::specta]
pub async fn vault_unlock_pin(
    app: AppHandle,
    state: State<'_, AppState>,
    pin: String,
) -> AppResult<()> {
    let pin = Zeroizing::new(pin);
    let stored = if state.vault().pin_status().persist {
        state.secrets.get(secret_keys::PIN_VAULT_KEY)?
    } else {
        None
    };
    let vault = state.vault.clone();
    let result = blocking(move || {
        Ok(vault
            .lock()
            .map_err(|_| AppError::internal("vault poisoned"))?
            .unlock_with_pin(&pin, stored.as_deref().map(String::as_str))?)
    })
    .await;
    // The copy goes with the last wrong PIN, or later if removing it failed then.
    if let Err(e) = &result
        && (e.code == ErrorCode::PinUnavailable
            || (e.code == ErrorCode::WrongPin && e.attempts_left == Some(0)))
    {
        forget_stored_pin(&state);
    }
    result?;
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
    check_new_password("next", &next)?;
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
    // Biometric and PIN unlock wrap the vault key, which did not change, so they stay valid.
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
    check_new_password("new_password", &new_password)?;
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
        &cfg,
    )
    .await?;
    persist(&state, &cfg, &config, &session)?;
    state.sync.set_backend(Some(Arc::clone(&backend)));
    // The first pull is an ordinary round: if it fails, the sync status shows why and the
    // scheduler resumes from the saved cursor, so the restore itself has succeeded.
    let _ = sync::run_round(&app).await;
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

/// SEC-11: sets or replaces the PIN. Requires the master password as a fresh confirmation, like
/// Windows Hello. A new PIN needs the master password after a restart until [`pin_set_persist`]
/// says otherwise, so setting it needs nothing from the OS credential store.
#[tauri::command]
#[specta::specta]
pub async fn pin_enable(
    state: State<'_, AppState>,
    password: String,
    pin: String,
) -> AppResult<()> {
    check_new_pin(&pin)?;
    if !vault_verify_password(state.clone(), password).await? {
        return Err(AppError::new(
            ErrorCode::WrongPassword,
            "wrong master password",
        ));
    }
    let pin = Zeroizing::new(pin);
    let vault = state.vault.clone();
    blocking(move || {
        vault
            .lock()
            .map_err(|_| AppError::internal("vault poisoned"))?
            .set_pin(&pin, false)?;
        Ok(())
    })
    .await?;
    // A copy kept for an earlier PIN no longer opens.
    forget_stored_pin(&state);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn pin_disable(state: State<'_, AppState>) -> AppResult<()> {
    state.vault().clear_pin()?;
    forget_stored_pin(&state);
    Ok(())
}

/// SEC-11: whether the PIN also works after a restart. On, the vault key it wraps goes to the OS
/// credential store; off, it is removed there.
#[tauri::command]
#[specta::specta]
pub fn pin_set_persist(state: State<'_, AppState>, persist: bool) -> AppResult<()> {
    let wrapped = state.vault().set_pin_persist(persist)?;
    match wrapped {
        Some(wrapped) => {
            if let Err(e) = state.secrets.set(secret_keys::PIN_VAULT_KEY, &wrapped) {
                if let Err(undo) = state.vault().set_pin_persist(false) {
                    tracing::warn!(
                        "could not record that the PIN needs the master password after a restart: {undo}"
                    );
                }
                return Err(e.into());
            }
        }
        None => forget_stored_pin(&state),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_master_password_needs_eight_characters() {
        let err = check_new_password("next", &"x".repeat(MIN_PASSWORD_CHARS - 1)).unwrap_err();
        assert_eq!(
            (err.code, err.field.as_deref()),
            (ErrorCode::InvalidInput, Some("next"))
        );
        assert!(check_new_password("next", &"x".repeat(MIN_PASSWORD_CHARS)).is_ok());
        // Characters, not bytes: eight CJK characters are enough, seven are not.
        assert!(check_new_password("password", &"日".repeat(MIN_PASSWORD_CHARS)).is_ok());
        assert!(check_new_password("password", &"日".repeat(MIN_PASSWORD_CHARS - 1)).is_err());
    }

    #[test]
    fn a_pin_needs_four_characters_of_any_kind() {
        let err = check_new_pin("123").unwrap_err();
        assert_eq!(
            (err.code, err.field.as_deref()),
            (ErrorCode::InvalidInput, Some("pin"))
        );
        assert!(check_new_pin("1234").is_ok());
        assert!(check_new_pin("a$c9").is_ok());
        assert!(check_new_pin("日本語ん").is_ok());
    }
}
