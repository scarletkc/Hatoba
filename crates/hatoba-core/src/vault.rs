//! The vault: master password, key hierarchy, lock state and the decrypted item map.
//!
//! A [`Vault`] owns the [`Store`] (ciphertext on disk) and, while unlocked, the vault key and a
//! decrypted copy of every live item (SEC-01). [`Vault::lock`] drops and wipes both.
//!
//! The vault is a plain synchronous object; the shell wraps it in
//! `Arc<std::sync::Mutex<Vault>>` ([`SharedVault`](crate::sync::SharedVault)). Methods that run
//! Argon2 (`create`, `unlock`, `verify_password`, `change_password`, …) take about a second:
//! call them from `spawn_blocking`.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use serde::Serialize;
use zeroize::{Zeroize, Zeroizing};

use crate::crypto::{
    self, AAD_RECOVERY_AUTH, AAD_RECOVERY_VAULT_KEY, AAD_VAULT_CHECK, AAD_VAULT_KEY, KdfParams,
    Key32, MasterKeys, b64_encode, decode_salt, item_aad, random_key, random_salt, seal,
    unwrap_key, wrap_key,
};
use crate::error::{Error, Result};
use crate::model::{
    Group, Host, Item, KnownHost, PortForward, SETTINGS_ID, Settings, SshKey, new_id,
};
use crate::recovery::RecoveryCode;
use crate::store::{ItemRow, Store, StoreOps, meta};
use crate::sync::SyncConfig;
use crate::sync::backend::VaultMeta;
use crate::sync::conflict::{ConflictEntry, Resolution};

const VAULT_CHECK_PLAINTEXT: &[u8] = b"hatoba vault check v1";
/// Consecutive wrong passwords allowed before delays start (SEC-06).
pub const FREE_UNLOCK_ATTEMPTS: u32 = 3;
/// Upper bound of the unlock delay.
pub const MAX_UNLOCK_DELAY_SECS: i64 = 300;

// ---- clock --------------------------------------------------------------------------------

/// Source of wall-clock time, injectable so throttling and timestamps are testable.
pub trait Clock: Send + Sync {
    /// Milliseconds since the Unix epoch.
    fn now_ms(&self) -> i64;
}

/// The real clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }
}

/// A clock you move by hand (tests).
#[derive(Debug, Default)]
pub struct ManualClock(AtomicI64);

impl ManualClock {
    /// A clock showing `ms`.
    #[must_use]
    pub fn new(ms: i64) -> Self {
        Self(AtomicI64::new(ms))
    }

    /// Jumps to `ms`.
    pub fn set(&self, ms: i64) {
        self.0.store(ms, Ordering::SeqCst);
    }

    /// Moves forward by `ms`.
    pub fn advance(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Delay in milliseconds imposed after `consecutive_failures` wrong passwords (SEC-06):
/// none for failures 1–3, then `min(2^(n-4), 300)` seconds from the 4th on.
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

/// The non-secret vault metadata kept in `meta`, as uploaded on sync setup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalMeta {
    /// Schema version.
    pub schema_version: u32,
    /// Base64 salt.
    pub kdf_salt: String,
    /// KDF parameters JSON.
    pub kdf_params: String,
    /// Wrapped vault key (password).
    pub protected_vault_key: String,
    /// Wrapped vault key (recovery code).
    pub recovery_vault_key: String,
}

// ---- internal state -----------------------------------------------------------------------

/// Everything that exists only while unlocked.
pub(crate) struct Unlocked {
    pub(crate) vault_key: Key32,
    pub(crate) items: BTreeMap<String, Item>,
}

impl Drop for Unlocked {
    fn drop(&mut self) {
        for item in self.items.values_mut() {
            item.zeroize();
        }
        self.items.clear();
        // `vault_key` is a `Zeroizing` and wipes itself.
    }
}

/// Borrowed pieces of an unlocked vault, handed to the sync engine.
pub(crate) struct SyncCtx<'a> {
    pub(crate) store: &'a mut Store,
    pub(crate) key: &'a [u8; 32],
    pub(crate) items: &'a mut BTreeMap<String, Item>,
    pub(crate) now: i64,
}

impl SyncCtx<'_> {
    /// Re-reads one row and brings the decrypted cache in line with it. Rows that are
    /// tombstones leave the cache; rows that cannot be decrypted are logged (id only) and
    /// skipped, which is reported as `false`.
    pub(crate) fn refresh(&mut self, id: &str) -> Result<bool> {
        match self.store.item_row(id)? {
            Some(row) => match decode_row(self.key, &row) {
                Ok(Some(item)) => {
                    self.items.insert(id.to_owned(), item);
                }
                Ok(None) => {
                    self.items.remove(id);
                }
                Err(_) => {
                    tracing::warn!(item_id = id, "item could not be decrypted; skipping");
                    self.items.remove(id);
                    return Ok(false);
                }
            },
            None => {
                self.items.remove(id);
            }
        }
        Ok(true)
    }
}

/// Decrypts a row. `Ok(None)` for tombstones.
pub(crate) fn decode_row(key: &[u8; 32], row: &ItemRow) -> Result<Option<Item>> {
    let Some(env) = row.envelope.as_deref().filter(|_| !row.deleted) else {
        return Ok(None);
    };
    decode_envelope(key, &row.id, env).map(Some)
}

/// Decrypts one item envelope (JSON text) for the item with the given id.
pub(crate) fn decode_envelope(key: &[u8; 32], id: &str, envelope_json: &str) -> Result<Item> {
    let plain = crypto::open_json(key, item_aad(id).as_bytes(), envelope_json)?;
    Item::from_plaintext(&plain)
}

/// Seals an item for storage and sync.
pub(crate) fn seal_item(key: &[u8; 32], id: &str, item: &Item) -> Result<crypto::Envelope> {
    let plain = item.to_plaintext()?;
    seal(key, item_aad(id).as_bytes(), &plain)
}

/// Item ids are what the Worker accepts: `[A-Za-z0-9_-]{1,64}`.
fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

// ---- the vault ----------------------------------------------------------------------------

/// The local vault. See the [module docs](self).
pub struct Vault {
    pub(crate) store: Store,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) unlocked: Option<Unlocked>,
}

impl Vault {
    /// Opens (creating if necessary) the vault database at `db_path`. The vault starts locked.
    ///
    /// # Errors
    /// SQLite errors, or [`Error::UnsupportedVersion`] for a database from a newer build.
    pub fn open(db_path: &Path) -> Result<Self> {
        Ok(Self::from_store(Store::open(db_path)?))
    }

    /// An in-memory vault (tests).
    ///
    /// # Errors
    /// SQLite errors.
    pub fn open_in_memory() -> Result<Self> {
        Ok(Self::from_store(Store::open_in_memory()?))
    }

    fn from_store(store: Store) -> Self {
        Self {
            store,
            clock: Arc::new(SystemClock),
            unlocked: None,
        }
    }

    /// Replaces the clock (tests).
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Replaces the clock in place (tests).
    pub fn set_clock(&mut self, clock: Arc<dyn Clock>) {
        self.clock = clock;
    }

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

    /// This device's id (empty before the vault exists).
    #[must_use]
    pub fn device_id(&self) -> String {
        self.store
            .get_meta(meta::DEVICE_ID)
            .ok()
            .flatten()
            .unwrap_or_default()
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

        let device_id = new_id();
        let now = self.clock.now_ms();
        self.store.transaction(|tx| {
            tx.set_meta(
                meta::SCHEMA_VERSION,
                &crate::store::SCHEMA_VERSION.to_string(),
            )?;
            tx.set_meta(meta::KDF_SALT, &b64_encode(&salt))?;
            tx.set_meta(meta::KDF_PARAMS, &params.to_json())?;
            tx.set_meta(meta::PROTECTED_VAULT_KEY, &protected)?;
            tx.set_meta(meta::RECOVERY_VAULT_KEY, &recovery_wrapped)?;
            tx.set_meta(meta::RECOVERY_AUTH_SEALED, &recovery_auth_sealed)?;
            tx.set_meta(meta::VAULT_CHECK, &check)?;
            tx.set_meta(meta::DEVICE_ID, &device_id)?;
            tx.upsert_item_row(&ItemRow {
                id: SETTINGS_ID.to_owned(),
                envelope: Some(settings_envelope.to_json()),
                revision: 0,
                deleted: false,
                dirty: true,
                updated_at: now,
            })
        })?;
        self.finish_unlock(vault_key)?;
        Ok(recovery)
    }

    // ---- unlocking ----

    fn load_kdf(&self) -> Result<([u8; crypto::SALT_LEN], KdfParams)> {
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
    fn unwrap_with_password(&self, password: &str) -> Result<(MasterKeys, Key32)> {
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

    /// Locks: wipes the vault key and every decrypted item.
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

    // ---- items ----

    /// All live items as `(id, item)`, in id (≈ creation) order. Empty while locked.
    pub fn items(&self) -> impl Iterator<Item = (&str, &Item)> {
        self.unlocked
            .iter()
            .flat_map(|u| u.items.iter().map(|(id, item)| (id.as_str(), item)))
    }

    /// One item. `None` if missing, deleted or locked.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Item> {
        self.unlocked.as_ref()?.items.get(id)
    }

    /// Inserts or updates an item and returns its id. `updated_at` is set to now (and always
    /// strictly newer than the version it replaces), the item is sealed and marked dirty.
    /// `id = None` allocates a fresh UUIDv7 (the settings item always uses [`SETTINGS_ID`]).
    ///
    /// # Errors
    /// [`Error::Locked`]; [`Error::InvalidItem`] for a malformed id or a settings/id mismatch.
    pub fn put(&mut self, id: Option<&str>, mut item: Item) -> Result<String> {
        let id = match (id, &item) {
            (Some(id), _) => id.to_owned(),
            (None, Item::Settings(_)) => SETTINGS_ID.to_owned(),
            (None, _) => new_id(),
        };
        if !valid_id(&id) {
            return Err(Error::InvalidItem("malformed id".into()));
        }
        if matches!(item, Item::Settings(_)) != (id == SETTINGS_ID) {
            return Err(Error::InvalidItem(
                "the settings item must use the fixed settings id".into(),
            ));
        }
        let previous = self.store.item_row(&id)?;
        let now = self.clock.now_ms();
        let unlocked = self.unlocked.as_mut().ok_or(Error::Locked)?;

        let updated_at = previous
            .as_ref()
            .map_or(now, |p| now.max(p.updated_at.saturating_add(1)));
        item.set_updated_at(updated_at);
        let envelope = seal_item(&unlocked.vault_key, &id, &item)?;
        self.store.upsert_item_row(&ItemRow {
            id: id.clone(),
            envelope: Some(envelope.to_json()),
            revision: previous.map_or(0, |p| p.revision),
            deleted: false,
            dirty: true,
            updated_at,
        })?;
        unlocked.items.insert(id.clone(), item);
        Ok(id)
    }

    /// Deletes an item, leaving a tombstone (`envelope = NULL, deleted = 1`) that syncs.
    ///
    /// # Errors
    /// [`Error::Locked`]; [`Error::ItemNotFound`]; [`Error::InvalidItem`] for the settings item.
    pub fn delete(&mut self, id: &str) -> Result<()> {
        if id == SETTINGS_ID {
            return Err(Error::InvalidItem(
                "the settings item cannot be deleted".into(),
            ));
        }
        let now = self.clock.now_ms();
        let unlocked = self.unlocked.as_mut().ok_or(Error::Locked)?;
        let row = self
            .store
            .item_row(id)?
            .filter(|r| !r.deleted)
            .ok_or_else(|| Error::ItemNotFound(id.to_owned()))?;
        self.store.upsert_item_row(&ItemRow {
            id: id.to_owned(),
            envelope: None,
            revision: row.revision,
            deleted: true,
            dirty: true,
            updated_at: now.max(row.updated_at.saturating_add(1)),
        })?;
        unlocked.items.remove(id);
        Ok(())
    }

    fn collect<T>(&self, pick: impl Fn(&Item) -> Option<&T>) -> Vec<(String, T)>
    where
        T: Clone,
    {
        self.items()
            .filter_map(|(id, item)| pick(item).map(|t| (id.to_owned(), t.clone())))
            .collect()
    }

    /// All hosts.
    #[must_use]
    pub fn hosts(&self) -> Vec<(String, Host)> {
        self.collect(Item::as_host)
    }

    /// All keys.
    #[must_use]
    pub fn keys(&self) -> Vec<(String, SshKey)> {
        self.collect(Item::as_key)
    }

    /// All groups.
    #[must_use]
    pub fn groups(&self) -> Vec<(String, Group)> {
        self.collect(Item::as_group)
    }

    /// All known hosts.
    #[must_use]
    pub fn known_hosts(&self) -> Vec<(String, KnownHost)> {
        self.collect(Item::as_known_host)
    }

    /// All port forwards.
    #[must_use]
    pub fn forwards(&self) -> Vec<(String, PortForward)> {
        self.collect(Item::as_forward)
    }

    /// The settings (defaults while locked or if none is stored yet).
    #[must_use]
    pub fn settings(&self) -> Settings {
        self.get(SETTINGS_ID)
            .and_then(Item::as_settings)
            .cloned()
            .unwrap_or_default()
    }

    // ---- device-local data ----

    /// Records when a host was last connected to. Device-local; never synced.
    ///
    /// # Errors
    /// Storage errors.
    pub fn set_last_connected(&mut self, host_id: &str, at_ms: i64) -> Result<()> {
        self.store.set_last_connected(host_id, at_ms)
    }

    /// When a host was last connected to on this device.
    #[must_use]
    pub fn last_connected(&self, host_id: &str) -> Option<i64> {
        self.store.last_connected(host_id).ok().flatten()
    }

    /// The opaque device-local UI preferences (plaintext JSON owned by the shell). Works while locked.
    ///
    /// # Errors
    /// Storage errors.
    pub fn local_prefs(&self) -> Result<Option<String>> {
        self.store.get_meta(meta::LOCAL_PREFS)
    }

    /// Stores the device-local UI preferences. Works while locked. Must not contain secrets.
    ///
    /// # Errors
    /// Storage errors.
    pub fn set_local_prefs(&mut self, json: &str) -> Result<()> {
        self.store.set_meta(meta::LOCAL_PREFS, json)
    }

    // ---- backup ----

    /// Writes an encrypted backup (VAULT-07). Only envelopes and key blobs are written; the
    /// file is useless without the password or recovery code. Works while locked.
    ///
    /// # Errors
    /// [`Error::VaultNotInitialized`], I/O and storage errors.
    pub fn export_backup(&self, path: &Path) -> Result<()> {
        crate::backup::export(&self.store, path, self.clock.now_ms())
    }

    // ---- sync-facing state ----

    /// The configured sync backend (no secrets), if any.
    ///
    /// # Errors
    /// Storage errors; [`Error::Format`] if the stored value is unreadable.
    pub fn sync_config(&self) -> Result<Option<SyncConfig>> {
        match self.store.get_meta(meta::SYNC_BACKEND)? {
            Some(json) => Ok(Some(
                serde_json::from_str(&json).map_err(|_| Error::Format("sync_backend".into()))?,
            )),
            None => Ok(None),
        }
    }

    /// Records (or clears) the sync backend description. Clearing also resets the pull cursor.
    ///
    /// # Errors
    /// Storage errors.
    pub fn set_sync_config(&mut self, config: Option<&SyncConfig>) -> Result<()> {
        match config {
            Some(c) => self
                .store
                .set_meta(meta::SYNC_BACKEND, &serde_json::to_string(c)?),
            None => self.store.transaction(|tx| {
                tx.delete_meta(meta::SYNC_BACKEND)?;
                tx.delete_meta(meta::SYNC_CURSOR)?;
                tx.delete_meta(meta::SYNC_LAST_AT)
            }),
        }
    }

    /// Highest server sequence number pulled so far.
    #[must_use]
    pub fn sync_cursor(&self) -> u64 {
        let n = self
            .store
            .get_meta_i64(meta::SYNC_CURSOR)
            .ok()
            .flatten()
            .unwrap_or(0);
        u64::try_from(n).unwrap_or(0)
    }

    /// When the last sync round completed (Unix ms).
    #[must_use]
    pub fn sync_last_at(&self) -> Option<i64> {
        self.store.get_meta_i64(meta::SYNC_LAST_AT).ok().flatten()
    }

    /// Number of items with local changes not yet pushed.
    #[must_use]
    pub fn pending_count(&self) -> u64 {
        self.store.dirty_count().unwrap_or(0)
    }

    /// The non-secret vault metadata (for sync setup).
    ///
    /// # Errors
    /// [`Error::VaultNotInitialized`] and storage errors.
    pub fn local_meta(&self) -> Result<LocalMeta> {
        let get = |key: &str| self.store.get_meta(key)?.ok_or(Error::VaultNotInitialized);
        Ok(LocalMeta {
            schema_version: u32::try_from(
                self.store.get_meta_i64(meta::SCHEMA_VERSION)?.unwrap_or(1),
            )
            .unwrap_or(1),
            kdf_salt: get(meta::KDF_SALT)?,
            kdf_params: get(meta::KDF_PARAMS)?,
            protected_vault_key: get(meta::PROTECTED_VAULT_KEY)?,
            recovery_vault_key: get(meta::RECOVERY_VAULT_KEY)?,
        })
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

    /// Verifies the password and gathers what `POST /v1/setup` needs, including the
    /// `recovery_auth` that was sealed under the vault key at creation.
    pub(crate) fn sync_setup_material(&self, password: &str) -> Result<SetupMaterial> {
        let (keys, vault_key) = self.unwrap_with_password(password)?;
        let sealed = self
            .store
            .get_meta(meta::RECOVERY_AUTH_SEALED)?
            .ok_or(Error::RecoveryMaterialMissing)?;
        let recovery_auth = unwrap_key(&vault_key, AAD_RECOVERY_AUTH, &sealed)?;
        Ok(SetupMaterial {
            meta: self.local_meta()?,
            auth_key: keys.auth_key,
            recovery_auth,
            vault_key,
            device_id: self.device_id(),
        })
    }

    /// Splits the vault into the pieces the sync engine needs.
    pub(crate) fn sync_ctx(&mut self) -> Result<SyncCtx<'_>> {
        let now = self.clock.now_ms();
        let Self {
            store, unlocked, ..
        } = self;
        let unlocked = unlocked.as_mut().ok_or(Error::Locked)?;
        Ok(SyncCtx {
            store,
            key: &unlocked.vault_key,
            items: &mut unlocked.items,
            now,
        })
    }

    /// Installs the metadata of a remote vault into an empty local database and unlocks with
    /// the already-unwrapped vault key (flow B). Existing items are not touched.
    pub(crate) fn install_remote_vault(
        &mut self,
        remote: &VaultMeta,
        vault_key: Key32,
        device_id: &str,
    ) -> Result<()> {
        if self.is_initialized() {
            return Err(Error::VaultAlreadyInitialized);
        }
        let check = seal_vault_check(&vault_key)?;
        self.store.transaction(|tx| {
            tx.set_meta(
                meta::SCHEMA_VERSION,
                &crate::store::SCHEMA_VERSION
                    .max(remote.schema_version)
                    .to_string(),
            )?;
            tx.set_meta(meta::KDF_SALT, &remote.kdf_salt)?;
            tx.set_meta(meta::KDF_PARAMS, &remote.kdf_params)?;
            tx.set_meta(meta::PROTECTED_VAULT_KEY, &remote.protected_vault_key)?;
            tx.set_meta(meta::RECOVERY_VAULT_KEY, &remote.recovery_vault_key)?;
            tx.set_meta(meta::VAULT_CHECK, &check)?;
            tx.set_meta(meta::DEVICE_ID, device_id)?;
            tx.delete_meta(meta::SYNC_CURSOR)
        })?;
        self.finish_unlock(vault_key)
    }

    /// Adopts the password-related metadata of the remote vault after verifying it wraps the
    /// same vault key (the password was changed on another device).
    pub(crate) fn adopt_remote_meta(&mut self, remote: &VaultMeta) -> Result<()> {
        self.store.transaction(|tx| {
            tx.set_meta(meta::KDF_SALT, &remote.kdf_salt)?;
            tx.set_meta(meta::KDF_PARAMS, &remote.kdf_params)?;
            tx.set_meta(meta::PROTECTED_VAULT_KEY, &remote.protected_vault_key)?;
            tx.set_meta(meta::RECOVERY_VAULT_KEY, &remote.recovery_vault_key)?;
            // The recovery code may have been rotated elsewhere; our sealed copy is then stale.
            tx.delete_meta(meta::RECOVERY_AUTH_SEALED)
        })
    }

    // ---- conflict log ----

    /// Conflicts resolved automatically, newest first, with both versions decrypted (P1 UI).
    ///
    /// # Errors
    /// [`Error::Locked`], storage errors.
    pub fn conflicts(&self, unreviewed_only: bool) -> Result<Vec<ConflictEntry>> {
        let unlocked = self.unlocked.as_ref().ok_or(Error::Locked)?;
        let decode = |envelope: &Option<String>, item_id: &str| -> Option<Item> {
            let env = envelope.as_deref()?;
            let plain =
                crypto::open_json(&unlocked.vault_key, item_aad(item_id).as_bytes(), env).ok()?;
            Item::from_plaintext(&plain).ok()
        };
        let mut out = Vec::new();
        for row in self.store.conflicts(unreviewed_only)? {
            let Some(resolution) = Resolution::parse(&row.resolution) else {
                continue;
            };
            out.push(ConflictEntry {
                id: row.id,
                local: decode(&row.local_envelope, &row.item_id),
                remote: decode(&row.remote_envelope, &row.item_id),
                item_id: row.item_id,
                resolution,
                local_deleted: row.local_deleted,
                remote_deleted: row.remote_deleted,
                local_updated_at: row.local_updated_at,
                remote_updated_at: row.remote_updated_at,
                created_at: row.created_at,
                reviewed: row.reviewed,
            });
        }
        Ok(out)
    }

    /// Number of conflicts the user has not reviewed.
    #[must_use]
    pub fn unreviewed_conflict_count(&self) -> u64 {
        self.store.unreviewed_conflict_count().unwrap_or(0)
    }

    /// Marks a conflict as seen.
    ///
    /// # Errors
    /// Storage errors.
    pub fn mark_conflict_reviewed(&mut self, id: i64) -> Result<()> {
        self.store.mark_conflict_reviewed(id)?;
        Ok(())
    }

    /// Re-applies the version that lost a conflict as a fresh local edit (so it syncs and wins).
    /// Returns the affected item id and marks the conflict reviewed.
    ///
    /// # Errors
    /// [`Error::Locked`]; [`Error::ItemNotFound`] for an unknown conflict; [`Error::Decrypt`]
    /// if the losing version can no longer be read.
    pub fn restore_conflict_loser(&mut self, id: i64) -> Result<String> {
        let entries = self.conflicts(false)?;
        let entry = entries
            .into_iter()
            .find(|e| e.id == id)
            .ok_or_else(|| Error::ItemNotFound(format!("conflict {id}")))?;
        let (loser, loser_deleted) = if entry.resolution.local_won() {
            (entry.remote, entry.remote_deleted)
        } else {
            (entry.local, entry.local_deleted)
        };
        if loser_deleted {
            // The losing side was a deletion: restoring it means deleting again.
            if self.get(&entry.item_id).is_some() {
                self.delete(&entry.item_id)?;
            }
        } else {
            let item = loser.ok_or(Error::Decrypt)?;
            self.put(Some(&entry.item_id), item)?;
        }
        self.store.mark_conflict_reviewed(id)?;
        Ok(entry.item_id)
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

/// Everything sync setup needs from the local vault, derived from the password.
pub(crate) struct SetupMaterial {
    pub(crate) meta: LocalMeta,
    pub(crate) auth_key: Key32,
    pub(crate) recovery_auth: Key32,
    pub(crate) vault_key: Key32,
    pub(crate) device_id: String,
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
    use super::*;
    use crate::model::HostAuth;

    const PW: &str = "correct horse battery staple";

    fn test_vault() -> (Vault, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock::new(1_000_000));
        let vault = Vault::open_in_memory().unwrap().with_clock(clock.clone());
        (vault, clock)
    }

    fn created() -> (Vault, Arc<ManualClock>, RecoveryCode) {
        let (mut vault, clock) = test_vault();
        let code = vault
            .create_with_params(PW, KdfParams::for_tests())
            .unwrap();
        (vault, clock, code)
    }

    fn host(name: &str) -> Item {
        Item::Host(Host {
            name: name.into(),
            address: format!("{name}.example.org"),
            ..Host::default()
        })
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
            [
                0, 0, 0, 0, 1, 2, 4, 8, 16, 32, 64, 128, 256, 300, 300, 300, 300
            ]
        );
        assert_eq!(unlock_delay_ms(u32::MAX), 300_000);
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
    fn delay_is_capped_at_five_minutes() {
        let (mut vault, clock, _code) = created();
        vault.lock();
        for _ in 0..20 {
            clock.advance(400_000);
            let _ = vault.unlock("bad");
        }
        let status = vault.status();
        assert_eq!(status.failed_attempts, 20);
        assert_eq!(status.retry_at, Some(clock.now_ms() + 300_000));
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
    fn items_crud_and_typed_accessors() {
        let (mut vault, clock, _code) = created();
        clock.advance(10);
        let h = vault.put(None, host("web")).unwrap();
        let g = vault
            .put(
                None,
                Item::Group(Group {
                    name: "prod".into(),
                    ..Group::default()
                }),
            )
            .unwrap();
        let k = vault
            .put(
                None,
                Item::Key(SshKey {
                    name: "laptop".into(),
                    ..SshKey::default()
                }),
            )
            .unwrap();
        vault
            .put(
                None,
                Item::KnownHost(KnownHost {
                    host: "web".into(),
                    port: 22,
                    ..KnownHost::default()
                }),
            )
            .unwrap();
        vault
            .put(
                None,
                Item::Forward(PortForward {
                    host_id: h.clone(),
                    ..PortForward::default()
                }),
            )
            .unwrap();

        assert_eq!(vault.hosts().len(), 1);
        assert_eq!(vault.groups()[0].0, g);
        assert_eq!(vault.keys()[0].0, k);
        assert_eq!(vault.known_hosts().len(), 1);
        assert_eq!(vault.forwards().len(), 1);
        assert_eq!(vault.get(&h).unwrap().updated_at(), clock.now_ms());
        assert_eq!(vault.pending_count(), 6); // + settings

        // Update keeps the id, bumps updated_at, keeps revision.
        clock.advance(10);
        let before = vault.get(&h).unwrap().updated_at();
        let mut edited = vault.get(&h).unwrap().clone();
        if let Item::Host(host) = &mut edited {
            host.auth = HostAuth::password("pw");
        }
        assert_eq!(vault.put(Some(&h), edited).unwrap(), h);
        assert!(vault.get(&h).unwrap().updated_at() > before);

        // Delete leaves a tombstone.
        vault.delete(&h).unwrap();
        assert!(vault.get(&h).is_none());
        let row = vault.store.item_row(&h).unwrap().unwrap();
        assert!(row.deleted && row.envelope.is_none() && row.dirty);
        assert!(matches!(vault.delete(&h), Err(Error::ItemNotFound(_))));
        assert!(matches!(
            vault.delete("missing"),
            Err(Error::ItemNotFound(_))
        ));
        assert!(matches!(
            vault.delete(SETTINGS_ID),
            Err(Error::InvalidItem(_))
        ));
    }

    #[test]
    fn updated_at_is_strictly_increasing_even_if_the_clock_stalls() {
        let (mut vault, clock, _code) = created();
        let id = vault.put(None, host("a")).unwrap();
        let first = vault.get(&id).unwrap().updated_at();
        let item = vault.get(&id).unwrap().clone();
        vault.put(Some(&id), item.clone()).unwrap();
        let second = vault.get(&id).unwrap().updated_at();
        assert_eq!(second, first + 1);
        clock.set(0); // clock jumps backwards
        vault.put(Some(&id), item).unwrap();
        assert_eq!(vault.get(&id).unwrap().updated_at(), second + 1);
    }

    #[test]
    fn settings_have_a_fixed_id() {
        let (mut vault, _clock, _code) = created();
        assert!(matches!(
            vault.put(Some("other"), Item::Settings(Settings::default())),
            Err(Error::InvalidItem(_))
        ));
        assert!(matches!(
            vault.put(Some(SETTINGS_ID), host("h")),
            Err(Error::InvalidItem(_))
        ));
        assert!(matches!(
            vault.put(Some("bad id!"), host("h")),
            Err(Error::InvalidItem(_))
        ));
        let mut s = vault.settings();
        s.auto_lock_minutes = 5;
        assert_eq!(vault.put(None, Item::Settings(s)).unwrap(), SETTINGS_ID);
        assert_eq!(vault.settings().auto_lock_minutes, 5);
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
    fn local_state_and_prefs_work_while_locked() {
        let (mut vault, _clock, _code) = created();
        let id = vault.put(None, host("h")).unwrap();
        vault.set_last_connected(&id, 123).unwrap();
        vault.lock();
        assert_eq!(vault.last_connected(&id), Some(123));
        assert_eq!(vault.local_prefs().unwrap(), None);
        vault.set_local_prefs(r#"{"window":{"w":1200}}"#).unwrap();
        assert_eq!(
            vault.local_prefs().unwrap().as_deref(),
            Some(r#"{"window":{"w":1200}}"#)
        );
        // last_connected is not an item and never dirties anything.
        assert_eq!(vault.pending_count(), 2);
    }

    #[test]
    fn sync_config_round_trip() {
        let (mut vault, _clock, _code) = created();
        assert_eq!(vault.sync_config().unwrap(), None);
        let cfg = SyncConfig::Worker {
            url: "https://sync.example.workers.dev".into(),
            deployment: None,
        };
        vault.set_sync_config(Some(&cfg)).unwrap();
        vault.store.set_meta(meta::SYNC_CURSOR, "42").unwrap();
        assert_eq!(vault.sync_config().unwrap(), Some(cfg));
        assert_eq!(vault.sync_cursor(), 42);
        vault.set_sync_config(None).unwrap();
        assert_eq!(vault.sync_config().unwrap(), None);
        assert_eq!(vault.sync_cursor(), 0);
    }

    // ---- M2 acceptance: no plaintext on disk ----

    #[test]
    fn database_files_contain_no_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        let password = "hunter2-very-secret";
        let host_name = "prod-api-tokyo-secret";
        let key_body = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\n-----END OPENSSH PRIVATE KEY-----";
        let host_password = "ssh-login-password-xyz";

        let mut vault = Vault::open(&path).unwrap();
        vault
            .create_with_params(password, KdfParams::for_tests())
            .unwrap();
        vault
            .put(
                None,
                Item::Host(Host {
                    name: host_name.into(),
                    address: "203.0.113.77".into(),
                    username: "deploy-user-abc".into(),
                    auth: HostAuth::password(host_password),
                    tags: vec!["tag-confidential".into()],
                    ..Host::default()
                }),
            )
            .unwrap();
        vault
            .put(
                None,
                Item::Key(SshKey {
                    name: "my-private-key-name".into(),
                    private_key: Zeroizing::new(key_body.into()),
                    passphrase: Some(Zeroizing::new("key-passphrase-qq".into())),
                    ..SshKey::default()
                }),
            )
            .unwrap();
        let host_id = vault.hosts()[0].0.clone();
        vault.delete(&host_id).unwrap();
        let wal_seen_before_close = std::fs::metadata(dir.path().join("vault.db-wal")).is_ok();
        // Read the raw files while the connection is still open so the -wal contents are included.
        let mut blobs = Vec::new();
        for name in ["vault.db", "vault.db-wal", "vault.db-shm"] {
            if let Ok(bytes) = std::fs::read(dir.path().join(name)) {
                blobs.push((name, bytes));
            }
        }
        drop(vault);
        // And again after a clean close (checkpointed).
        for name in ["vault.db", "vault.db-wal"] {
            if let Ok(bytes) = std::fs::read(dir.path().join(name)) {
                blobs.push((name, bytes));
            }
        }
        assert!(blobs.iter().any(|(n, b)| *n == "vault.db" && !b.is_empty()));
        assert!(
            wal_seen_before_close,
            "test must actually exercise the WAL file"
        );

        let needles = [
            password,
            host_name,
            "203.0.113.77",
            "deploy-user-abc",
            host_password,
            "tag-confidential",
            "my-private-key-name",
            "BEGIN OPENSSH PRIVATE KEY",
            "b3BlbnNzaC1rZXktdjE",
            "key-passphrase-qq",
            r#""type":"host""#,
            r#""type":"key""#,
            r#""type":"settings""#,
            "Cascadia Mono",
        ];
        for (name, bytes) in &blobs {
            for needle in needles {
                assert!(
                    !bytes.windows(needle.len()).any(|w| w == needle.as_bytes()),
                    "plaintext {needle:?} found in {name}"
                );
            }
        }
    }
}
