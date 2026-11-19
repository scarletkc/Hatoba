//! Master password, vault creation, unlock, throttling and recovery on [`Vault`].

use std::collections::BTreeMap;

use serde::Serialize;
use zeroize::Zeroizing;

use crate::crypto::{
    self, AAD_RECOVERY_AUTH, AAD_RECOVERY_VAULT_KEY, AAD_VAULT_CHECK, AAD_VAULT_KEY, KdfParams,
    Key32, MasterKeys, b64_encode, decode_salt, random_key, random_salt, seal, unwrap_key,
    wrap_key,
};
use crate::error::{Error, Result};
use crate::model::{Item, SETTINGS_ID, Settings, new_id};
use crate::recovery::RecoveryCode;
use crate::store::{ItemRow, KeyMeta, StoreOps, meta};
use crate::vault::{Unlocked, Vault, decode_row, seal_item};

const VAULT_CHECK_PLAINTEXT: &[u8] = b"hatoba vault check v1";

/// Consecutive wrong passwords allowed before delays start (SEC-06).
pub const FREE_UNLOCK_ATTEMPTS: u32 = 3;
/// Upper bound of the unlock delay.
pub const MAX_UNLOCK_DELAY_SECS: i64 = 60;

/// Delay in milliseconds imposed after `consecutive_failures` wrong passwords (SEC-06):
/// none for failures 1–3, then `min(2^(n-4), 60)` seconds from the 4th on.
#[must_use]
pub fn unlock_delay_ms(consecutive_failures: u32) -> i64 {
    if consecutive_failures <= FREE_UNLOCK_ATTEMPTS {
        return 0;
    }
    let exponent = (consecutive_failures - FREE_UNLOCK_ATTEMPTS - 1).min(16);
    (1_i64 << exponent).min(MAX_UNLOCK_DELAY_SECS) * 1000
}

// ---- public data types --------------------------------------------------------------------

/// What the lock screen needs to know.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VaultStatusInfo {
    /// A vault exists in this database.
    pub initialized: bool,
    /// The vault key is in memory.
    pub unlocked: bool,
    /// Consecutive wrong passwords so far.
    pub failed_attempts: u32,
    /// While throttled: the Unix-ms time at which the next attempt is allowed.
    pub retry_at: Option<i64>,
}

/// New key material after a password change; the sync layer uploads it via
/// `PUT /v1/vault/password`.
pub struct PasswordChange {
    /// Base64 of the new Argon2 salt.
    pub kdf_salt_b64: String,
    /// JSON of the KDF parameters.
    pub kdf_params_json: String,
    /// The new `auth_key` (the Worker stores its SHA-256).
    pub auth_key: Key32,
    /// Envelope JSON of the vault key wrapped by the new `enc_key`.
    pub protected_vault_key_json: String,
}

impl std::fmt::Debug for PasswordChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PasswordChange(<redacted>)")
    }
}

/// New recovery material after rotating the recovery code.
pub struct RecoveryChange {
    /// Envelope JSON of the vault key wrapped by the new `recovery_key`.
    pub recovery_vault_key_json: String,
    /// The new `recovery_auth`.
    pub recovery_auth: Key32,
}

impl std::fmt::Debug for RecoveryChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecoveryChange(<redacted>)")
    }
}

/// A recovery rotation computed but not yet applied.
pub(crate) struct StagedRecovery {
    pub(crate) code: RecoveryCode,
    pub(crate) change: RecoveryChange,
    pub(crate) sealed_auth: String,
    /// The current `auth_key`, needed because the password endpoint requires it alongside the
    /// rotated recovery material.
    pub(crate) auth_key: Key32,
}

impl Vault {
    // ---- status ----

    pub(crate) fn is_initialized(&self) -> bool {
        matches!(self.store.get_meta(meta::PROTECTED_VAULT_KEY), Ok(Some(_)))
    }

    fn failed_attempts(&self) -> u32 {
        let n = self
            .store
            .get_meta_i64(meta::UNLOCK_FAILURES)
            .ok()
            .flatten()
            .unwrap_or(0);
        u32::try_from(n.max(0)).unwrap_or(u32::MAX)
    }

    /// The time before which unlock attempts are refused, if a delay is in force.
    fn throttle_until(&self) -> Option<i64> {
        let delay = unlock_delay_ms(self.failed_attempts());
        if delay == 0 {
            return None;
        }
        let last = self
            .store
            .get_meta_i64(meta::UNLOCK_LAST_FAILURE_AT)
            .ok()
            .flatten()?;
        Some(last.saturating_add(delay))
    }

    /// Lock-screen state.
    #[must_use]
    pub fn status(&self) -> VaultStatusInfo {
        let now = self.clock.now_ms();
        VaultStatusInfo {
            initialized: self.is_initialized(),
            unlocked: self.unlocked.is_some(),
            failed_attempts: self.failed_attempts(),
            retry_at: self.throttle_until().filter(|&t| t > now),
        }
    }

    /// Whether the vault key is in memory.
    #[must_use]
    pub fn is_unlocked(&self) -> bool {
        self.unlocked.is_some()
    }

    /// This device's id (empty before the vault exists, unless a restore from the cloud was
    /// attempted).
    #[must_use]
    pub fn device_id(&self) -> String {
        self.store
            .get_meta(meta::DEVICE_ID)
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    /// The device id a restore from the cloud (flow B) signs in with, stored by the first attempt.
    /// An attempt that fails after signing in leaves a session on the server; a retry signs in as
    /// the same device, which replaces that session instead of listing another device.
    pub(crate) fn restore_device_id(&mut self) -> Result<String> {
        if let Some(id) = self.store.get_meta(meta::DEVICE_ID)? {
            return Ok(id);
        }
        let id = new_id();
        self.store.set_meta(meta::DEVICE_ID, &id)?;
        Ok(id)
    }

    // ---- creation ----

    /// Creates the vault with the spec's Argon2id parameters and leaves it unlocked.
    /// Returns the recovery code, which is shown to the user exactly once.
    ///
    /// # Errors
    /// [`Error::VaultAlreadyInitialized`], [`Error::EmptyPassword`], storage errors.
    pub fn create(&mut self, password: &str) -> Result<RecoveryCode> {
        self.create_with_params(password, KdfParams::default())
    }

    /// As [`create`](Self::create) with explicit KDF parameters (tests use
    /// `KdfParams::for_tests()` under the `test-util` feature).
    ///
    /// # Errors
    /// As [`create`](Self::create).
    pub fn create_with_params(
        &mut self,
        password: &str,
        params: KdfParams,
    ) -> Result<RecoveryCode> {
        if self.is_initialized() {
            return Err(Error::VaultAlreadyInitialized);
        }
        if password.is_empty() {
            return Err(Error::EmptyPassword);
        }
        let salt = random_salt()?;
        let keys = MasterKeys::from_password(password, &salt, &params);
        let vault_key = random_key()?;
        let recovery = RecoveryCode::generate()?;

        let protected = wrap_key(&keys.enc_key, AAD_VAULT_KEY, &vault_key)?;
        let recovery_wrapped =
            wrap_key(&recovery.recovery_key(), AAD_RECOVERY_VAULT_KEY, &vault_key)?;
        let recovery_auth_sealed = seal_recovery_auth(&vault_key, &recovery.recovery_auth())?;
        let check = seal_vault_check(&vault_key)?;
        let settings_item = Item::Settings(Settings {
            updated_at: self.clock.now_ms(),
            ..Settings::default()
        });
        let settings_envelope = seal_item(&vault_key, SETTINGS_ID, &settings_item)?;

        let key_meta = KeyMeta {
            kdf_salt: b64_encode(&salt),
            kdf_params: params.to_json(),
            protected_vault_key: protected,
            recovery_vault_key: recovery_wrapped,
        };
        let device_id = new_id();
        let now = self.clock.now_ms();
        self.store.transaction(|tx| {
            tx.set_meta(
                meta::SCHEMA_VERSION,
                &crate::store::SCHEMA_VERSION.to_string(),
            )?;
            key_meta.write(tx)?;
            tx.set_meta(meta::RECOVERY_AUTH_SEALED, &recovery_auth_sealed)?;
            tx.set_meta(meta::VAULT_CHECK, &check)?;
            tx.set_meta(meta::DEVICE_ID, &device_id)?;
            tx.upsert_item_row(&ItemRow::live(
                SETTINGS_ID.to_owned(),
                settings_envelope.to_json(),
                0,
                now,
            ))
        })?;
        self.finish_unlock(vault_key)?;
        Ok(recovery)
    }

    // ---- unlocking ----

    pub(crate) fn load_kdf(&self) -> Result<([u8; crypto::SALT_LEN], KdfParams)> {
        let salt = self
            .store
            .get_meta(meta::KDF_SALT)?
            .ok_or(Error::VaultNotInitialized)?;
        let params = self
            .store
            .get_meta(meta::KDF_PARAMS)?
            .ok_or(Error::VaultNotInitialized)?;
        // `from_json` enforces the minimum cost, so a tampered database cannot weaken the KDF.
        Ok((decode_salt(&salt)?, KdfParams::from_json(&params)?))
    }

    fn check_throttle(&self) -> Result<()> {
        match self.throttle_until() {
            Some(retry_at) if self.clock.now_ms() < retry_at => Err(Error::Throttled { retry_at }),
            _ => Ok(()),
        }
    }

    fn record_failure(&self) {
        let n = self.failed_attempts().saturating_add(1);
        let now = self.clock.now_ms();
        if let Err(e) = self
            .store
            .set_meta(meta::UNLOCK_LAST_FAILURE_AT, &now.to_string())
            .and_then(|()| self.store.set_meta(meta::UNLOCK_FAILURES, &n.to_string()))
        {
            tracing::error!(error = %e, "could not persist unlock failure counter");
        }
    }

    fn reset_failures(&self) {
        if self.failed_attempts() == 0
            && self
                .store
                .get_meta(meta::UNLOCK_LAST_FAILURE_AT)
                .ok()
                .flatten()
                .is_none()
        {
            return;
        }
        let result = self
            .store
            .delete_meta(meta::UNLOCK_FAILURES)
            .and_then(|()| self.store.delete_meta(meta::UNLOCK_LAST_FAILURE_AT));
        if let Err(e) = result {
            tracing::error!(error = %e, "could not reset unlock failure counter");
        }
    }

    /// Checks the password (honouring and updating the throttle) and returns the derived keys
    /// together with the unwrapped vault key.
    pub(crate) fn unwrap_with_password(&self, password: &str) -> Result<(MasterKeys, Key32)> {
        if !self.is_initialized() {
            return Err(Error::VaultNotInitialized);
        }
        self.check_throttle()?;
        let (salt, params) = self.load_kdf()?;
        let keys = MasterKeys::from_password(password, &salt, &params);
        let protected = self
            .store
            .get_meta(meta::PROTECTED_VAULT_KEY)?
            .ok_or(Error::VaultNotInitialized)?;
        match unwrap_key(&keys.enc_key, AAD_VAULT_KEY, &protected) {
            Ok(vault_key) => {
                self.reset_failures();
                Ok((keys, vault_key))
            }
            Err(Error::Decrypt) => {
                self.record_failure();
                Err(Error::WrongPassword)
            }
            Err(e) => Err(e),
        }
    }

    /// Unlocks with the master password.
    ///
    /// # Errors
    /// [`Error::WrongPassword`]; [`Error::Throttled`] (without running Argon2) after repeated
    /// failures; [`Error::VaultNotInitialized`].
    pub fn unlock(&mut self, password: &str) -> Result<()> {
        let (_keys, vault_key) = self.unwrap_with_password(password)?;
        self.finish_unlock(vault_key)
    }

    /// Unlocks with a vault key held by the OS (biometric unlock, P1). The key is verified
    /// against a check value, so a stale or foreign key is rejected.
    ///
    /// # Errors
    /// [`Error::WrongVaultKey`], [`Error::VaultNotInitialized`].
    pub fn unlock_with_vault_key(&mut self, key: Key32) -> Result<()> {
        if !self.is_initialized() {
            return Err(Error::VaultNotInitialized);
        }
        if !self.verify_vault_key(&key)? {
            return Err(Error::WrongVaultKey);
        }
        self.finish_unlock(key)
    }

    /// Whether `key` is this vault's key. Uses the check value; vaults without one fall back
    /// to decrypting stored items.
    pub(crate) fn verify_vault_key(&self, key: &[u8; 32]) -> Result<bool> {
        if let Some(check) = self.store.get_meta(meta::VAULT_CHECK)? {
            return Ok(open_vault_check(key, &check));
        }
        for row in self.store.item_rows()? {
            if row.deleted {
                continue;
            }
            if decode_row(key, &row).is_ok() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn finish_unlock(&mut self, vault_key: Key32) -> Result<()> {
        if self.store.get_meta(meta::VAULT_CHECK)?.is_none() {
            self.store
                .set_meta(meta::VAULT_CHECK, &seal_vault_check(&vault_key)?)?;
        }
        let items = self.load_items(&vault_key)?;
        self.arm_pin(&vault_key);
        // Replacing `Some(old)` drops it, which wipes it.
        self.unlocked = Some(Unlocked { vault_key, items });
        Ok(())
    }

    fn load_items(&self, key: &[u8; 32]) -> Result<BTreeMap<String, Item>> {
        let mut items = BTreeMap::new();
        for row in self.store.item_rows()? {
            match decode_row(key, &row) {
                Ok(Some(item)) => {
                    items.insert(row.id, item);
                }
                Ok(None) => {}
                // Id only: never the error detail, which could hint at contents.
                Err(_) => {
                    tracing::warn!(item_id = %row.id, "item could not be decrypted; skipping")
                }
            }
        }
        Ok(items)
    }

    /// A copy of the vault key for biometric enrolment. The caller must wrap it for the OS
    /// credential store and drop it immediately.
    ///
    /// # Errors
    /// [`Error::Locked`].
    pub fn vault_key_copy(&self) -> Result<Key32> {
        let unlocked = self.unlocked.as_ref().ok_or(Error::Locked)?;
        Ok(Zeroizing::new(*unlocked.vault_key))
    }

    /// Locks: wipes the vault key and every decrypted item. The vault key wrapped by the PIN
    /// stays, so the PIN can unlock again (SEC-11).
    pub fn lock(&mut self) {
        self.unlocked = None;
    }

    // ---- password operations ----

    /// Checks a password. Wrong guesses count towards the throttle, exactly as for `unlock`.
    ///
    /// # Errors
    /// [`Error::Throttled`], [`Error::VaultNotInitialized`].
    pub fn verify_password(&self, password: &str) -> Result<bool> {
        match self.unwrap_with_password(password) {
            Ok(_) => Ok(true),
            Err(Error::WrongPassword) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Re-derives `enc_key` / `auth_key` from the password (used by sync setup and login).
    /// Counts as a password check.
    ///
    /// # Errors
    /// [`Error::WrongPassword`], [`Error::Throttled`], [`Error::VaultNotInitialized`].
    pub fn master_keys(&self, password: &str) -> Result<MasterKeys> {
        self.unwrap_with_password(password).map(|(keys, _)| keys)
    }

    /// Computes the key material for a new password without touching the database.
    pub(crate) fn derive_password_change(
        vault_key: &[u8; 32],
        new_password: &str,
        params: &KdfParams,
    ) -> Result<PasswordChange> {
        if new_password.is_empty() {
            return Err(Error::EmptyPassword);
        }
        let salt = random_salt()?;
        let keys = MasterKeys::from_password(new_password, &salt, params);
        Ok(PasswordChange {
            kdf_salt_b64: b64_encode(&salt),
            kdf_params_json: params.to_json(),
            protected_vault_key_json: wrap_key(&keys.enc_key, AAD_VAULT_KEY, vault_key)?,
            auth_key: keys.auth_key,
        })
    }

    /// Verifies `current` and prepares a password change without applying it. Used by the
    /// remote flow, which must update the server first.
    pub(crate) fn stage_password_change(&self, current: &str, new: &str) -> Result<PasswordChange> {
        let (_keys, vault_key) = self.unwrap_with_password(current)?;
        let (_salt, params) = self.load_kdf()?;
        Self::derive_password_change(&vault_key, new, &params)
    }

    /// Writes a staged password change (re-wrapped vault key, new salt) to `meta`.
    pub(crate) fn commit_password_change(&mut self, change: &PasswordChange) -> Result<()> {
        self.store.transaction(|tx| {
            tx.set_meta(meta::KDF_SALT, &change.kdf_salt_b64)?;
            tx.set_meta(meta::KDF_PARAMS, &change.kdf_params_json)?;
            tx.set_meta(meta::PROTECTED_VAULT_KEY, &change.protected_vault_key_json)
        })
    }

    /// Changes the master password: new salt, re-wrapped vault key, nothing else. Items are not
    /// re-encrypted. Works whether or not the vault is unlocked and does not change the lock state.
    ///
    /// # Errors
    /// [`Error::WrongPassword`], [`Error::Throttled`], [`Error::EmptyPassword`].
    pub fn change_password(&mut self, current: &str, new: &str) -> Result<PasswordChange> {
        let change = self.stage_password_change(current, new)?;
        self.commit_password_change(&change)?;
        Ok(change)
    }

    /// Resets the master password with the recovery code (VAULT-06). Fully offline: the code
    /// unwraps `recovery_vault_key`. Leaves the vault unlocked and the throttle cleared.
    ///
    /// # Errors
    /// [`Error::WrongRecoveryCode`] for a malformed code or one that does not belong to this vault.
    pub fn recover(&mut self, code: &str, new_password: &str) -> Result<PasswordChange> {
        if !self.is_initialized() {
            return Err(Error::VaultNotInitialized);
        }
        let code = RecoveryCode::parse(code)?;
        let wrapped = self
            .store
            .get_meta(meta::RECOVERY_VAULT_KEY)?
            .ok_or(Error::VaultNotInitialized)?;
        let vault_key = match unwrap_key(&code.recovery_key(), AAD_RECOVERY_VAULT_KEY, &wrapped) {
            Ok(k) => k,
            Err(Error::Decrypt) => return Err(Error::WrongRecoveryCode),
            Err(e) => return Err(e),
        };
        let (_salt, params) = self.load_kdf()?;
        let change = Self::derive_password_change(&vault_key, new_password, &params)?;
        self.apply_recovered_password(&change, vault_key, &code.recovery_auth())?;
        Ok(change)
    }

    /// Persists a password reset performed with the recovery code, clears the unlock throttle
    /// and unlocks with the recovered vault key.
    pub(crate) fn apply_recovered_password(
        &mut self,
        change: &PasswordChange,
        vault_key: Key32,
        recovery_auth: &[u8; 32],
    ) -> Result<()> {
        let sealed = seal_recovery_auth(&vault_key, recovery_auth)?;
        self.store.transaction(|tx| {
            tx.set_meta(meta::KDF_SALT, &change.kdf_salt_b64)?;
            tx.set_meta(meta::KDF_PARAMS, &change.kdf_params_json)?;
            tx.set_meta(meta::PROTECTED_VAULT_KEY, &change.protected_vault_key_json)?;
            tx.set_meta(meta::RECOVERY_AUTH_SEALED, &sealed)?;
            tx.delete_meta(meta::UNLOCK_FAILURES)?;
            tx.delete_meta(meta::UNLOCK_LAST_FAILURE_AT)
        })?;
        self.finish_unlock(vault_key)
    }

    /// Prepares a new recovery code without applying it.
    pub(crate) fn stage_recovery_rotation(&self, password: &str) -> Result<StagedRecovery> {
        let (keys, vault_key) = self.unwrap_with_password(password)?;
        let code = RecoveryCode::generate()?;
        let recovery_vault_key_json =
            wrap_key(&code.recovery_key(), AAD_RECOVERY_VAULT_KEY, &vault_key)?;
        let recovery_auth = code.recovery_auth();
        let sealed_auth = seal_recovery_auth(&vault_key, &recovery_auth)?;
        Ok(StagedRecovery {
            code,
            change: RecoveryChange {
                recovery_vault_key_json,
                recovery_auth,
            },
            sealed_auth,
            auth_key: keys.auth_key,
        })
    }

    /// Writes a staged recovery rotation.
    pub(crate) fn commit_recovery_rotation(&mut self, staged: &StagedRecovery) -> Result<()> {
        self.store.transaction(|tx| {
            tx.set_meta(
                meta::RECOVERY_VAULT_KEY,
                &staged.change.recovery_vault_key_json,
            )?;
            tx.set_meta(meta::RECOVERY_AUTH_SEALED, &staged.sealed_auth)
        })
    }

    /// Replaces the recovery code (after the old one was used or lost). The old code stops working.
    ///
    /// # Errors
    /// [`Error::WrongPassword`], [`Error::Throttled`].
    pub fn rotate_recovery(&mut self, password: &str) -> Result<(RecoveryCode, RecoveryChange)> {
        let staged = self.stage_recovery_rotation(password)?;
        self.commit_recovery_rotation(&staged)?;
        Ok((staged.code, staged.change))
    }

    /// The `recovery_auth` stored sealed under the vault key at creation, so sync setup can
    /// upload it later without the recovery code.
    ///
    /// # Errors
    /// [`Error::Locked`]; [`Error::RecoveryMaterialMissing`] if this vault was restored from
    /// the cloud and never had one.
    #[cfg(test)]
    pub(crate) fn recovery_auth(&self) -> Result<Key32> {
        let unlocked = self.unlocked.as_ref().ok_or(Error::Locked)?;
        let sealed = self
            .store
            .get_meta(meta::RECOVERY_AUTH_SEALED)?
            .ok_or(Error::RecoveryMaterialMissing)?;
        unwrap_key(&unlocked.vault_key, AAD_RECOVERY_AUTH, &sealed)
    }
}

// ---- small sealed constants ---------------------------------------------------------------

fn seal_recovery_auth(vault_key: &[u8; 32], recovery_auth: &[u8; 32]) -> Result<String> {
    wrap_key(vault_key, AAD_RECOVERY_AUTH, recovery_auth)
}

pub(crate) fn seal_vault_check(vault_key: &[u8; 32]) -> Result<String> {
    Ok(seal(vault_key, AAD_VAULT_CHECK.as_bytes(), VAULT_CHECK_PLAINTEXT)?.to_json())
}

fn open_vault_check(vault_key: &[u8; 32], json: &str) -> bool {
    crypto::open_json(vault_key, AAD_VAULT_CHECK.as_bytes(), json)
        .is_ok_and(|p| p.as_slice() == VAULT_CHECK_PLAINTEXT)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::crypto::item_aad;
    use crate::model::SshKey;
    use crate::test_support::{
        TEST_PASSWORD as PW, host_item as host, unlocked_vault_at, vault_on,
    };
    use crate::vault::{Clock, ManualClock};

    fn test_vault() -> (Vault, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock::new(1_000_000));
        (vault_on(&clock), clock)
    }

    fn created() -> (Vault, Arc<ManualClock>, RecoveryCode) {
        unlocked_vault_at(1_000_000, PW)
    }

    #[test]
    fn create_lock_unlock() {
        let (mut vault, _clock, _code) = created();
        assert!(vault.is_unlocked());
        let status = vault.status();
        assert!(
            status.initialized
                && status.unlocked
                && status.failed_attempts == 0
                && status.retry_at.is_none()
        );
        assert!(!vault.device_id().is_empty());
        // The default settings item exists.
        assert_eq!(
            vault.settings(),
            Settings {
                updated_at: 1_000_000,
                ..Settings::default()
            }
        );
        assert_eq!(vault.items().count(), 1);

        let id = vault.put(None, host("alpha")).unwrap();
        vault.lock();
        assert!(!vault.is_unlocked());
        assert_eq!(vault.items().count(), 0);
        assert!(vault.get(&id).is_none());
        assert!(matches!(vault.put(None, host("x")), Err(Error::Locked)));
        assert!(matches!(vault.delete(&id), Err(Error::Locked)));
        assert!(matches!(vault.vault_key_copy(), Err(Error::Locked)));
        assert!(matches!(vault.conflicts(false), Err(Error::Locked)));

        vault.unlock(PW).unwrap();
        assert_eq!(vault.get(&id).unwrap().display_name(), "alpha");
    }

    #[test]
    fn create_twice_is_refused_and_empty_password_rejected() {
        let (mut vault, _clock, _code) = created();
        assert!(matches!(
            vault.create_with_params(PW, KdfParams::for_tests()),
            Err(Error::VaultAlreadyInitialized)
        ));
        let (mut fresh, _c) = test_vault();
        assert!(matches!(
            fresh.create_with_params("", KdfParams::for_tests()),
            Err(Error::EmptyPassword)
        ));
        assert!(!fresh.status().initialized);
        assert!(matches!(fresh.unlock(PW), Err(Error::VaultNotInitialized)));
    }

    #[test]
    fn create_uses_the_spec_kdf_parameters() {
        let (mut vault, _clock) = test_vault();
        let code = vault.create(PW).unwrap();
        assert_eq!(
            vault.store.get_meta(meta::KDF_PARAMS).unwrap().unwrap(),
            r#"{"alg":"argon2id","version":19,"m_kib":65536,"t":3,"p":4}"#
        );
        vault.lock();
        vault.unlock(PW).unwrap();
        assert_eq!(code.to_string().len(), 39);
    }

    #[test]
    fn wrong_password_is_rejected_and_counted() {
        let (mut vault, _clock, _code) = created();
        vault.lock();
        assert!(matches!(vault.unlock("nope"), Err(Error::WrongPassword)));
        assert!(!vault.is_unlocked());
        assert_eq!(vault.status().failed_attempts, 1);
        vault.unlock(PW).unwrap();
        assert_eq!(vault.status().failed_attempts, 0);
    }

    #[test]
    fn throttle_delay_schedule() {
        let secs: Vec<i64> = (0..=16).map(|n| unlock_delay_ms(n) / 1000).collect();
        assert_eq!(
            secs,
            [0, 0, 0, 0, 1, 2, 4, 8, 16, 32, 60, 60, 60, 60, 60, 60, 60]
        );
        assert_eq!(unlock_delay_ms(u32::MAX), 60_000);
    }

    #[test]
    fn throttling_blocks_without_running_argon2_and_recovers_with_time() {
        let (mut vault, clock, _code) = created();
        vault.lock();
        // Failures 1..=3 are free.
        for _ in 0..3 {
            assert!(matches!(vault.unlock("bad"), Err(Error::WrongPassword)));
            assert!(vault.status().retry_at.is_none());
        }
        // 4th failure: 1 s delay from now.
        let t0 = clock.now_ms();
        assert!(matches!(vault.unlock("bad"), Err(Error::WrongPassword)));
        assert_eq!(vault.status().retry_at, Some(t0 + 1000));

        // While throttled even the right password is refused, and the counter does not move.
        clock.advance(999);
        assert!(
            matches!(vault.unlock(PW), Err(Error::Throttled { retry_at }) if retry_at == t0 + 1000)
        );
        assert!(matches!(
            vault.verify_password(PW),
            Err(Error::Throttled { .. })
        ));
        assert!(matches!(
            vault.change_password(PW, "x"),
            Err(Error::Throttled { .. })
        ));
        assert_eq!(vault.status().failed_attempts, 4);

        // Delay over: attempts are allowed again; 5th failure doubles the delay.
        clock.advance(1);
        let t1 = clock.now_ms();
        assert!(matches!(vault.unlock("bad"), Err(Error::WrongPassword)));
        assert_eq!(vault.status().retry_at, Some(t1 + 2000));
        clock.advance(2000);
        assert!(matches!(vault.unlock("bad"), Err(Error::WrongPassword)));
        assert_eq!(vault.status().retry_at, Some(clock.now_ms() + 4000));

        // A correct password after the delay resets everything.
        clock.advance(4000);
        vault.unlock(PW).unwrap();
        let status = vault.status();
        assert_eq!((status.failed_attempts, status.retry_at), (0, None));
    }

    #[test]
    fn delay_is_capped_at_a_minute() {
        let (mut vault, clock, _code) = created();
        vault.lock();
        for _ in 0..20 {
            clock.advance(400_000);
            let _ = vault.unlock("bad");
        }
        let status = vault.status();
        assert_eq!(status.failed_attempts, 20);
        assert_eq!(status.retry_at, Some(clock.now_ms() + 60_000));
    }

    #[test]
    fn throttle_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        let clock = Arc::new(ManualClock::new(5_000_000));
        {
            let mut vault = Vault::open(&path).unwrap().with_clock(clock.clone());
            vault
                .create_with_params(PW, KdfParams::for_tests())
                .unwrap();
            vault.lock();
            for _ in 0..4 {
                let _ = vault.unlock("bad");
            }
        }
        let mut reopened = Vault::open(&path).unwrap().with_clock(clock.clone());
        let status = reopened.status();
        assert_eq!(status.failed_attempts, 4);
        assert_eq!(status.retry_at, Some(5_000_000 + 1000));
        assert!(!status.unlocked);
        assert!(matches!(reopened.unlock(PW), Err(Error::Throttled { .. })));
        clock.advance(1000);
        reopened.unlock(PW).unwrap();
    }

    #[test]
    fn verify_password_counts_failures_and_master_keys_match() {
        let (vault, _clock, _code) = created();
        assert!(vault.verify_password(PW).unwrap());
        assert!(!vault.verify_password("bad").unwrap());
        assert_eq!(vault.status().failed_attempts, 1);
        assert!(matches!(
            vault.master_keys("bad"),
            Err(Error::WrongPassword)
        ));
        assert_eq!(vault.status().failed_attempts, 2);
        let keys = vault.master_keys(PW).unwrap();
        assert_eq!(vault.status().failed_attempts, 0);
        let (salt, params) = vault.load_kdf().unwrap();
        let expected = MasterKeys::from_password(PW, &salt, &params);
        assert_eq!(*keys.auth_key, *expected.auth_key);
    }

    #[test]
    fn change_password_keeps_items_readable() {
        let (mut vault, _clock, _code) = created();
        let id = vault.put(None, host("kept")).unwrap();
        let envelope_before = vault.store.item_row(&id).unwrap().unwrap().envelope;
        let salt_before = vault.store.get_meta(meta::KDF_SALT).unwrap();

        let change = vault.change_password(PW, "a brand new password").unwrap();
        assert_ne!(vault.store.get_meta(meta::KDF_SALT).unwrap(), salt_before);
        assert_eq!(
            change.kdf_salt_b64,
            vault.store.get_meta(meta::KDF_SALT).unwrap().unwrap()
        );
        // No item was re-encrypted.
        assert_eq!(
            vault.store.item_row(&id).unwrap().unwrap().envelope,
            envelope_before
        );
        assert!(
            vault.is_unlocked(),
            "changing the password does not change the lock state"
        );

        vault.lock();
        assert!(
            matches!(vault.unlock(PW), Err(Error::WrongPassword)),
            "old password must stop working"
        );
        vault.unlock("a brand new password").unwrap();
        assert_eq!(vault.get(&id).unwrap().display_name(), "kept");

        // The returned auth_key is what the new password derives.
        let keys = vault.master_keys("a brand new password").unwrap();
        assert_eq!(*keys.auth_key, *change.auth_key);
        assert!(matches!(
            vault.change_password("wrong", "x"),
            Err(Error::WrongPassword)
        ));
        assert!(matches!(
            vault.change_password("a brand new password", ""),
            Err(Error::EmptyPassword)
        ));
    }

    #[test]
    fn change_password_works_while_locked() {
        let (mut vault, _clock, _code) = created();
        vault.lock();
        vault.change_password(PW, "second").unwrap();
        assert!(!vault.is_unlocked());
        vault.unlock("second").unwrap();
    }

    #[test]
    fn recover_with_code_resets_the_password() {
        let (mut vault, clock, code) = created();
        let id = vault.put(None, host("survivor")).unwrap();
        vault.lock();
        for _ in 0..5 {
            let _ = vault.unlock("forgotten");
        }
        assert!(vault.status().retry_at.is_some());

        // A wrong or malformed code changes nothing.
        let other = RecoveryCode::generate().unwrap();
        assert!(matches!(
            vault.recover(&other.to_string(), "new-pw"),
            Err(Error::WrongRecoveryCode)
        ));
        assert!(matches!(
            vault.recover("not a code", "new-pw"),
            Err(Error::WrongRecoveryCode)
        ));
        assert!(!vault.is_unlocked());

        // Case/format-insensitive code works even while throttled, and unlocks the vault.
        let typed = code.to_string().to_lowercase().replace('-', " ");
        let change = vault.recover(&typed, "new-pw").unwrap();
        assert!(vault.is_unlocked());
        assert_eq!(vault.get(&id).unwrap().display_name(), "survivor");
        let status = vault.status();
        assert_eq!((status.failed_attempts, status.retry_at), (0, None));
        assert_eq!(
            *change.auth_key,
            *vault.master_keys("new-pw").unwrap().auth_key
        );

        clock.advance(1);
        vault.lock();
        assert!(matches!(vault.unlock(PW), Err(Error::WrongPassword)));
        vault.unlock("new-pw").unwrap();
        // The recovery code itself keeps working (it was not consumed).
        vault.recover(&code.to_string(), "newer-pw").unwrap();
    }

    #[test]
    fn rotate_recovery_replaces_the_code() {
        let (mut vault, _clock, old_code) = created();
        assert!(matches!(
            vault.rotate_recovery("bad"),
            Err(Error::WrongPassword)
        ));
        let (new_code, change) = vault.rotate_recovery(PW).unwrap();
        assert_ne!(new_code, old_code);
        assert_eq!(*change.recovery_auth, *new_code.recovery_auth());
        assert!(matches!(
            vault.recover(&old_code.to_string(), "x"),
            Err(Error::WrongRecoveryCode)
        ));
        vault.recover(&new_code.to_string(), "x").unwrap();
        // The sealed recovery_auth follows the rotation (sync setup relies on it).
        assert_eq!(*vault.recovery_auth().unwrap(), *new_code.recovery_auth());
    }

    #[test]
    fn recovery_auth_is_kept_sealed_for_sync_setup() {
        let (mut vault, _clock, code) = created();
        assert_eq!(*vault.recovery_auth().unwrap(), *code.recovery_auth());
        let sealed = vault
            .store
            .get_meta(meta::RECOVERY_AUTH_SEALED)
            .unwrap()
            .unwrap();
        assert!(!sealed.contains(&crypto::hex_encode(&*code.recovery_auth())));
        vault.lock();
        assert!(matches!(vault.recovery_auth(), Err(Error::Locked)));
    }

    #[test]
    fn unlock_with_vault_key_verifies_the_key() {
        let (mut vault, _clock, _code) = created();
        let id = vault.put(None, host("bio")).unwrap();
        let key = vault.vault_key_copy().unwrap();
        vault.lock();
        assert!(matches!(
            vault.unlock_with_vault_key(random_key().unwrap()),
            Err(Error::WrongVaultKey)
        ));
        assert!(!vault.is_unlocked());
        vault.unlock_with_vault_key(key).unwrap();
        assert!(vault.get(&id).is_some());
    }

    #[test]
    fn lock_wipes_decrypted_state() {
        let (mut vault, _clock, _code) = created();
        vault
            .put(
                None,
                Item::Key(SshKey {
                    private_key: Zeroizing::new("SECRET".into()),
                    ..SshKey::default()
                }),
            )
            .unwrap();
        assert!(vault.unlocked.as_ref().is_some_and(|u| !u.items.is_empty()));
        vault.lock();
        assert!(vault.unlocked.is_none(), "key and item map are dropped");
        assert_eq!(vault.items().count(), 0);
        assert_eq!(vault.hosts().len() + vault.keys().len(), 0);
        assert_eq!(vault.settings(), Settings::default());
        assert!(matches!(
            vault.unlock_with_vault_key(random_key().unwrap()),
            Err(Error::WrongVaultKey)
        ));
    }

    #[test]
    fn undecryptable_rows_are_skipped_on_unlock() {
        let (mut vault, _clock, _code) = created();
        let id = vault.put(None, host("fine")).unwrap();
        vault
            .store
            .upsert_item_row(&ItemRow {
                id: "garbage".into(),
                envelope: Some(
                    r#"{"v":1,"n":"AAAAAAAAAAAAAAAA","c":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}"#
                        .into(),
                ),
                revision: 1,
                deleted: false,
                dirty: false,
                updated_at: 1,
            })
            .unwrap();
        vault.lock();
        vault.unlock(PW).unwrap();
        assert!(vault.get(&id).is_some());
        assert!(vault.get("garbage").is_none());
    }

    #[test]
    fn items_of_a_type_this_version_lacks_are_skipped_and_kept() {
        let (mut vault, _clock, _code) = created();
        let id = new_id();
        let key = &vault.unlocked.as_ref().unwrap().vault_key;
        let plain = br#"{"type":"serial_host","name":"x"}"#;
        let envelope = seal(key, item_aad(&id).as_bytes(), plain)
            .unwrap()
            .to_json();
        vault
            .store
            .upsert_item_row(&ItemRow {
                id: id.clone(),
                envelope: Some(envelope.clone()),
                revision: 1,
                deleted: false,
                dirty: false,
                updated_at: 1,
            })
            .unwrap();
        vault.lock();
        vault.unlock(PW).unwrap();
        assert!(vault.get(&id).is_none());
        let row = vault.store.item_row(&id).unwrap().unwrap();
        assert_eq!(row.envelope.as_deref(), Some(envelope.as_str()));
        assert!(!row.deleted && !row.dirty);
    }
}
