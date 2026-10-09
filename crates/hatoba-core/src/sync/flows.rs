//! The user-facing sync flows (spec §6.6) built on [`SyncBackend`] and the vault.
//!
//! Every flow follows the same discipline: lock the shared vault only for short local steps,
//! never across a network `await`, and run Argon2 inside `spawn_blocking`.
//!
//! **Design note: the recovery secret.** Uploading `recovery_auth` at setup needs the recovery
//! secret, which the vault must not keep. The vault therefore stores `recovery_auth` *sealed
//! under the vault key* in the local meta key `recovery_auth_sealed` when the vault is created
//! (or the recovery code is rotated or used). Only an unlocked vault can read it back, which is
//! exactly the moment sync is enabled.
//!
//! **Design note: device names.** `POST /v1/login` wants the device name sealed with the vault
//! key, but on a new device the vault key can only be fetched *after* logging in. The first login
//! of such a flow therefore uses a provisional name sealed with `enc_key`, and a second login
//! (which replaces the first session for the same device id) sends the properly sealed name.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::crypto::{
    AAD_RECOVERY_VAULT_KEY, AAD_VAULT_KEY, KdfParams, Key32, MasterKeys, decode_salt, device_aad,
    open_json, seal, unwrap_key,
};
use crate::error::{Error, Result};
use crate::model::new_id;
use crate::platform::DeviceInfo;
use crate::recovery::RecoveryCode;
use crate::store::{StoreOps, meta};
use crate::sync::backend::{
    DeviceLogin, RecoveryUpdate, Session, SyncBackend, VaultInit, VaultMeta, VaultMetaUpdate,
};
use crate::sync::engine::{SyncOptions, sync_round};
use crate::sync::{SharedVault, SyncConfig, lock_vault};
use crate::vault::{PasswordChange, Vault};

/// Runs CPU-heavy work (Argon2) off the async executor.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| Error::Poisoned)?
}

fn with_vault<T>(vault: &SharedVault, f: impl FnOnce(&mut Vault) -> Result<T>) -> Result<T> {
    let mut guard = lock_vault(vault)?;
    f(&mut guard)
}

/// Seals `{"name":…,"platform":…}` for the login request.
fn seal_device(key: &[u8; 32], device_id: &str, info: &DeviceInfo) -> Result<String> {
    let plain = Zeroizing::new(serde_json::to_vec(info)?);
    Ok(seal(key, device_aad(device_id).as_bytes(), &plain)?.to_json())
}

fn open_device(key: &[u8; 32], device_id: &str, sealed: &str) -> Option<DeviceInfo> {
    let plain = open_json(key, device_aad(device_id).as_bytes(), sealed).ok()?;
    serde_json::from_slice(&plain).ok()
}

fn device_login(key: &[u8; 32], device_id: &str, info: &DeviceInfo) -> Result<DeviceLogin> {
    Ok(DeviceLogin {
        device_id: device_id.to_owned(),
        sealed_name: seal_device(key, device_id, info)?,
    })
}

// ---- flow A: first device enables sync ------------------------------------------------------

/// Flow A: publishes this device's vault to an empty remote and uploads everything.
///
/// 1. `health`: for an empty remote, every local item is re-queued as new (revision 0), then
///    `setup` runs with this vault's metadata, `auth_key` and sealed-then-unsealed
///    `recovery_auth`. A remote that already holds *this* vault (an earlier attempt set it up,
///    then failed or lost the response) is adopted instead, and any other vault yields
///    [`Error::RemoteInitialized`] (the MVP answer to flow C: restore on a new device).
/// 2. `login`, a check that the remote's wrapped vault keys are this vault's, then a full round.
///
/// A call that fails at any point after `setup` can therefore be retried with the same arguments.
/// The vault must be unlocked. `setup_token` is the Worker's `SETUP_TOKEN` (`None` for D1 mode).
/// The caller stores the returned session in the OS credential store and records the backend
/// with `Vault::set_sync_config`.
///
/// # Errors
/// [`Error::Locked`], [`Error::WrongPassword`], [`Error::Throttled`], [`Error::RemoteInitialized`],
/// [`Error::InvalidSetupToken`], [`Error::RecoveryMaterialMissing`], transport errors.
pub async fn enable_sync(
    vault: &SharedVault,
    backend: &dyn SyncBackend,
    password: &str,
    setup_token: Option<&str>,
    device: DeviceInfo,
) -> Result<Session> {
    with_vault(vault, |v| {
        if v.is_unlocked() {
            Ok(())
        } else {
            Err(Error::Locked)
        }
    })?;

    let pw = Zeroizing::new(password.to_owned());
    let shared = Arc::clone(vault);
    let material = blocking(move || lock_vault(&shared)?.sync_setup_material(&pw)).await?;

    if backend.health().await?.initialized {
        // An earlier attempt published this vault, then failed (or lost the response) before
        // sync was configured locally. Its items stay queued, so a normal round finishes it.
        ensure_own_kdf(backend, &material.meta).await?;
    } else {
        // The remote is brand new: everything local is "never synced". Queue it before `setup`
        // so that an attempt interrupted after `setup` leaves the vault ready to upload.
        with_vault(vault, |v| {
            v.store.transaction(|tx| {
                tx.mark_all_dirty_for_upload()?;
                tx.delete_meta(meta::SYNC_CURSOR)
            })
        })?;
        let setup = backend
            .setup(VaultInit {
                schema_version: material.meta.schema_version,
                kdf_salt: material.meta.kdf_salt.clone(),
                kdf_params: material.meta.kdf_params.clone(),
                auth_key: material.auth_key.clone(),
                protected_vault_key: material.meta.protected_vault_key.clone(),
                recovery_vault_key: material.meta.recovery_vault_key.clone(),
                recovery_auth: material.recovery_auth.clone(),
                setup_token: setup_token.map(|t| Zeroizing::new(t.to_owned())),
            })
            .await;
        match setup {
            // Initialised since `health`: by a retried request of ours, or by someone else.
            Err(Error::RemoteInitialized) => ensure_own_kdf(backend, &material.meta).await?,
            other => other?,
        }
    }

    let login = device_login(&material.vault_key, &material.device_id, &device)?;
    let session = backend.login(&material.auth_key, &login).await?;
    // The vault key identity can only be read after login; check it before uploading anything.
    let remote = backend.fetch_vault().await?;
    if remote.protected_vault_key != material.meta.protected_vault_key
        || remote.recovery_vault_key != material.meta.recovery_vault_key
    {
        return Err(Error::RemoteInitialized);
    }

    sync_round(vault, backend, &SyncOptions::default()).await?;
    Ok(session)
}

/// Checks, without authenticating, that an initialised remote can hold this vault: the same
/// salt and KDF parameters. Another vault yields [`Error::RemoteInitialized`] before the password
/// is ever tried against it.
async fn ensure_own_kdf(backend: &dyn SyncBackend, local: &crate::vault::LocalMeta) -> Result<()> {
    let kdf = backend.prelogin().await?;
    if kdf.kdf_salt == local.kdf_salt && kdf.kdf_params == local.kdf_params {
        Ok(())
    } else {
        Err(Error::RemoteInitialized)
    }
}

// ---- authentication shared by flows B and "sign in again" -----------------------------------

/// Result of authenticating against an initialised remote.
struct Authenticated {
    session: Session,
    meta: VaultMeta,
    vault_key: Key32,
}

/// `prelogin` → derive keys → `login` → `fetch_vault` → unwrap the vault key → `login` again with
/// the properly sealed device name. KDF parameters from the server are validated (never weaker
/// than the spec defaults) *before* any key is derived from the password.
async fn authenticate(
    backend: &dyn SyncBackend,
    password: &str,
    device_id: &str,
    device: &DeviceInfo,
) -> Result<Authenticated> {
    let kdf = backend.prelogin().await?;
    let params = KdfParams::from_json(&kdf.kdf_params)?;
    let salt = decode_salt(&kdf.kdf_salt)?;

    let pw = Zeroizing::new(password.to_owned());
    let keys = blocking(move || Ok(MasterKeys::from_password(&pw, &salt, &params))).await?;

    // First login: the vault key is not known yet, so the device name is sealed with enc_key.
    let provisional = device_login(&keys.enc_key, device_id, device)?;
    backend.login(&keys.auth_key, &provisional).await?;

    let meta = backend.fetch_vault().await?;
    if meta.kdf_salt != kdf.kdf_salt || meta.kdf_params != kdf.kdf_params {
        // The password was changed between our two requests; the keys we derived are stale.
        return Err(Error::Server(
            "vault changed during sign-in; try again".into(),
        ));
    }
    if meta.schema_version > crate::store::SCHEMA_VERSION {
        return Err(Error::UnsupportedVersion(format!(
            "remote schema {}",
            meta.schema_version
        )));
    }
    let vault_key = unwrap_key(&keys.enc_key, AAD_VAULT_KEY, &meta.protected_vault_key)?;

    let proper = device_login(&vault_key, device_id, device)?;
    let session = backend.login(&keys.auth_key, &proper).await?;
    Ok(Authenticated {
        session,
        meta,
        vault_key,
    })
}

// ---- flow B: new device joins -------------------------------------------------------------

/// Flow B: sets up this (empty) device from an existing remote vault.
///
/// Derives keys from the remote's salt and the master password, logs in, unwraps the vault key
/// with `enc_key`, and writes the vault metadata locally under a new device id together with
/// `config`, in one transaction. The vault is left unlocked and the local master password becomes
/// the remote one.
///
/// Items are not pulled here: the caller stores the returned session and then runs an ordinary
/// sync round. Once this returns, the device is a configured sync device, so an interrupted
/// first pull resumes from the saved cursor on the next round, also after a restart.
///
/// # Errors
/// [`Error::VaultAlreadyInitialized`], [`Error::RemoteNotInitialized`], [`Error::WrongPassword`],
/// [`Error::WeakKdfParams`], transport errors.
pub async fn restore_from_cloud(
    vault: &SharedVault,
    backend: &dyn SyncBackend,
    password: &str,
    device: DeviceInfo,
    config: &SyncConfig,
) -> Result<Session> {
    with_vault(vault, |v| {
        if v.status().initialized {
            Err(Error::VaultAlreadyInitialized)
        } else {
            Ok(())
        }
    })?;
    let device_id = new_id();
    let auth = authenticate(backend, password, &device_id, &device).await?;
    with_vault(vault, |v| {
        v.install_remote_vault(&auth.meta, auth.vault_key, &device_id, Some(config))
    })?;
    Ok(auth.session)
}

/// Signs in again after the session expired or was revoked ("authentication expired").
///
/// Works whether the vault is locked or not. The remote must hold *this* vault
/// ([`Error::VaultMismatch`] otherwise). If the master password was changed on another device,
/// the new password's metadata is adopted locally, so the local password changes too.
///
/// # Errors
/// [`Error::WrongPassword`], [`Error::VaultMismatch`], [`Error::VaultNotInitialized`], transport errors.
pub async fn sign_in(
    vault: &SharedVault,
    backend: &dyn SyncBackend,
    password: &str,
    device: DeviceInfo,
) -> Result<Session> {
    let device_id = with_vault(vault, |v| {
        if v.status().initialized {
            Ok(v.device_id())
        } else {
            Err(Error::VaultNotInitialized)
        }
    })?;
    let auth = authenticate(backend, password, &device_id, &device).await?;
    with_vault(vault, |v| {
        if !v.verify_vault_key(&auth.vault_key)? {
            return Err(Error::VaultMismatch);
        }
        if v.local_meta()? != local_view(&auth.meta) {
            v.adopt_remote_meta(&auth.meta)?;
        }
        Ok(())
    })?;
    Ok(auth.session)
}

fn local_view(remote: &VaultMeta) -> crate::vault::LocalMeta {
    crate::vault::LocalMeta {
        schema_version: remote.schema_version,
        kdf_salt: remote.kdf_salt.clone(),
        kdf_params: remote.kdf_params.clone(),
        protected_vault_key: remote.protected_vault_key.clone(),
        recovery_vault_key: remote.recovery_vault_key.clone(),
    }
}

// ---- password change and recovery -----------------------------------------------------------

fn meta_update(change: &PasswordChange) -> VaultMetaUpdate {
    VaultMetaUpdate {
        kdf_salt: change.kdf_salt_b64.clone(),
        kdf_params: change.kdf_params_json.clone(),
        auth_key: change.auth_key.clone(),
        protected_vault_key: change.protected_vault_key_json.clone(),
        recovery: None,
    }
}

/// Changes the master password locally *and* on the remote (VAULT-05).
///
/// The remote is updated first, so a failure leaves both sides on the old password. Other devices'
/// sessions are revoked by the server and must sign in again with the new password.
///
/// # Errors
/// [`Error::WrongPassword`], [`Error::Throttled`], [`Error::EmptyPassword`], [`Error::Unauthorized`],
/// transport errors.
pub async fn change_password_remote(
    vault: &SharedVault,
    backend: &dyn SyncBackend,
    current: &str,
    new: &str,
) -> Result<()> {
    let (cur, new) = (
        Zeroizing::new(current.to_owned()),
        Zeroizing::new(new.to_owned()),
    );
    let shared = Arc::clone(vault);
    let change = blocking(move || lock_vault(&shared)?.stage_password_change(&cur, &new)).await?;
    backend.update_vault_meta(meta_update(&change)).await?;
    with_vault(vault, |v| v.commit_password_change(&change))
}

/// Replaces the recovery code locally and on the remote. Returns the new code, to be shown once.
///
/// # Errors
/// As [`change_password_remote`].
pub async fn rotate_recovery_remote(
    vault: &SharedVault,
    backend: &dyn SyncBackend,
    password: &str,
) -> Result<RecoveryCode> {
    let pw = Zeroizing::new(password.to_owned());
    let shared = Arc::clone(vault);
    let (staged, meta) = blocking(move || {
        let guard = lock_vault(&shared)?;
        let staged = guard.stage_recovery_rotation(&pw)?;
        Ok((staged, guard.local_meta()?))
    })
    .await?;
    backend
        .update_vault_meta(VaultMetaUpdate {
            kdf_salt: meta.kdf_salt,
            kdf_params: meta.kdf_params,
            auth_key: staged.auth_key.clone(),
            protected_vault_key: meta.protected_vault_key,
            recovery: Some(RecoveryUpdate {
                recovery_vault_key: staged.change.recovery_vault_key_json.clone(),
                recovery_auth: staged.change.recovery_auth.clone(),
            }),
        })
        .await?;
    with_vault(vault, |v| v.commit_recovery_rotation(&staged))?;
    Ok(staged.code)
}

/// Resets the master password with the recovery code against the remote (VAULT-06).
///
/// Works on a device that already has the vault and on a fresh install (where it also restores
/// the vault, like flow B). Steps: `POST /v1/recover` proves possession of the code and returns
/// the recovery-wrapped vault key plus a restricted session; the new password's keys are derived;
/// `PUT /v1/vault/password` installs them (revoking all sessions); the local vault is updated and
/// unlocked; finally a normal login with the new password and a sync round.
///
/// # Errors
/// [`Error::WrongRecoveryCode`], [`Error::VaultMismatch`], [`Error::EmptyPassword`],
/// [`Error::WeakKdfParams`], transport errors.
pub async fn recover_remote(
    vault: &SharedVault,
    backend: &dyn SyncBackend,
    code: &str,
    new_password: &str,
    device: DeviceInfo,
) -> Result<Session> {
    let code = RecoveryCode::parse(code)?;
    let recovery_key = code.recovery_key();
    let recovery_auth = code.recovery_auth();
    let (device_id, existing) = with_vault(vault, |v| {
        Ok(if v.status().initialized {
            (v.device_id(), true)
        } else {
            (new_id(), false)
        })
    })?;

    // The vault key is unknown until the recovery blob is unwrapped, so seal the name provisionally.
    let provisional = device_login(&recovery_key, &device_id, &device)?;
    let recovered = backend.recover(&recovery_auth, &provisional).await?;

    let params = KdfParams::from_json(&recovered.kdf.kdf_params)?;
    let vault_key = match unwrap_key(
        &recovery_key,
        AAD_RECOVERY_VAULT_KEY,
        &recovered.recovery_vault_key,
    ) {
        Ok(k) => k,
        Err(Error::Decrypt) => return Err(Error::WrongRecoveryCode),
        Err(e) => return Err(e),
    };
    if existing && !with_vault(vault, |v| v.verify_vault_key(&vault_key))? {
        return Err(Error::VaultMismatch);
    }

    let (vk, pw) = (vault_key.clone(), Zeroizing::new(new_password.to_owned()));
    let change = blocking(move || Vault::derive_password_change(&vk, &pw, &params)).await?;
    backend.update_vault_meta(meta_update(&change)).await?;

    with_vault(vault, |v| {
        if existing {
            v.apply_recovered_password(&change, vault_key.clone(), &recovery_auth)
        } else {
            let meta = VaultMeta {
                schema_version: crate::store::SCHEMA_VERSION,
                kdf_salt: change.kdf_salt_b64.clone(),
                kdf_params: change.kdf_params_json.clone(),
                protected_vault_key: change.protected_vault_key_json.clone(),
                recovery_vault_key: recovered.recovery_vault_key.clone(),
                seq: 0,
            };
            v.install_remote_vault(&meta, vault_key.clone(), &device_id, None)
        }
    })?;

    // The server revoked every session, including the restricted one: log in with the new password.
    let login = device_login(&vault_key, &device_id, &device)?;
    let session = backend.login(&change.auth_key, &login).await?;
    sync_round(vault, backend, &SyncOptions::default()).await?;
    Ok(session)
}

// ---- device management ----------------------------------------------------------------------

/// A device as shown in "Settings → Cloud sync → Devices".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceEntry {
    /// Device id.
    pub device_id: String,
    /// Decrypted device name, if it could be read (a device that has not finished signing in
    /// shows `None`).
    pub name: Option<String>,
    /// Decrypted platform label.
    pub platform: Option<String>,
    /// First login, Unix ms.
    pub created_at: i64,
    /// Last request, Unix ms.
    pub last_seen: i64,
    /// Session expiry, Unix ms.
    pub expires_at: i64,
    /// Whether this is the calling device.
    pub current: bool,
}

/// Lists signed-in devices with their names decrypted locally.
///
/// # Errors
/// [`Error::Locked`] (names need the vault key), [`Error::Unauthorized`], transport errors.
pub async fn devices(vault: &SharedVault, backend: &dyn SyncBackend) -> Result<Vec<DeviceEntry>> {
    let key = with_vault(vault, |v| v.vault_key_copy())?;
    let remote = backend.devices().await?;
    Ok(remote
        .into_iter()
        .map(|d| {
            let info = open_device(&key, &d.device_id, &d.device_name);
            DeviceEntry {
                name: info.as_ref().map(|i| i.name.clone()),
                platform: info.map(|i| i.platform),
                device_id: d.device_id,
                created_at: d.created_at,
                last_seen: d.last_seen,
                expires_at: d.expires_at,
                current: d.current,
            }
        })
        .collect())
}

/// Signs a device out everywhere (its sessions are deleted on the server).
///
/// # Errors
/// [`Error::Unauthorized`], transport errors.
pub async fn revoke_device(backend: &dyn SyncBackend, device_id: &str) -> Result<()> {
    backend.revoke_device(device_id).await
}
