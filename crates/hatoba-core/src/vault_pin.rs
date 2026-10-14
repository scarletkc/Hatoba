//! PIN unlock (SEC-11): a short device-local secret that unlocks the vault again after a lock,
//! so the master password is needed only at the first unlock after the app starts.
//!
//! The PIN goes through Argon2id with the vault's KDF parameters and its own salt, and HKDF turns
//! the result into the PIN key. Two values come from it:
//!
//! - the `pin` meta value keeps the salt, the parameters and the PIN key sealed under the vault
//!   key. Every unlock (password, recovery code, biometrics, PIN) opens it and wraps the vault key
//!   with the PIN key again, so the PIN needs no Argon2 run after a password unlock;
//! - the vault key wrapped by the PIN key is held in memory across [`Vault::lock`], and never
//!   written to the database: a short PIN is cheap to guess offline. The shell may hand it to the
//!   OS credential store, when the user lets the PIN work after a restart, and passes it back to
//!   [`Vault::unlock_with_pin`].
//!
//! After [`MAX_PIN_ATTEMPTS`] wrong PINs in a row the wrapped key is dropped and only the master
//! password (or another full unlock) brings the PIN back.

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::crypto::{
    self, AAD_PIN_KEY, AAD_PIN_VAULT_KEY, KdfParams, Key32, b64_encode, decode_salt, hkdf_sha256,
    random_salt, unwrap_key, wrap_key,
};
use crate::error::{Error, Result};
use crate::store::{StoreOps, meta};
use crate::vault::Vault;

/// Wrong PINs in a row before the PIN stops working until a master password unlock.
pub const MAX_PIN_ATTEMPTS: u32 = 5;

const HKDF_INFO: &str = "hatoba/pin/v1";

/// The PIN unlock on this device, as the shell shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PinStatus {
    /// A PIN is set on this device.
    pub enabled: bool,
    /// The PIN also works after the app restarts: the shell keeps the wrapped vault key in the
    /// OS credential store.
    pub persist: bool,
    /// The wrapped vault key is in memory, so the PIN can unlock without the OS store.
    pub armed: bool,
    /// The PIN ran out of attempts: it unlocks nothing until a full unlock, even with a copy
    /// still in the OS store.
    pub exhausted: bool,
}

/// The `pin` meta value. Holds no secret in the clear: the PIN key is sealed under the vault key.
#[derive(Serialize, Deserialize)]
struct PinRecord {
    /// Base64 Argon2 salt of this PIN.
    salt: String,
    /// The KDF parameters the PIN was derived with.
    kdf_params: KdfParams,
    /// Envelope JSON: the PIN key sealed under the vault key.
    key: String,
    persist: bool,
    /// Wrong PINs in a row; a full unlock resets it.
    failures: u32,
}

fn pin_key(pin: &str, salt: &[u8; crypto::SALT_LEN], params: &KdfParams) -> Key32 {
    let master = crypto::derive_master_key(pin, salt, params);
    hkdf_sha256(&*master, HKDF_INFO)
}

impl Vault {
    fn pin_record(&self) -> Result<Option<PinRecord>> {
        self.store
            .get_meta(meta::PIN)?
            .map(|json| serde_json::from_str(&json).map_err(Error::from))
            .transpose()
    }

    fn save_pin_record(&self, record: &PinRecord) -> Result<()> {
        self.store
            .set_meta(meta::PIN, &serde_json::to_string(record)?)
    }

    /// The PIN unlock on this device. Works while locked.
    #[must_use]
    pub fn pin_status(&self) -> PinStatus {
        match self.pin_record() {
            Ok(Some(record)) => PinStatus {
                enabled: true,
                persist: record.persist,
                armed: self.pin_wrapped.is_some(),
                exhausted: record.failures >= MAX_PIN_ATTEMPTS,
            },
            _ => PinStatus::default(),
        }
    }

    /// Sets or replaces the PIN and returns the vault key wrapped by it, for the shell to keep
    /// in the OS credential store when `persist` is on. Runs Argon2: call it off the async
    /// runtime.
    ///
    /// # Errors
    /// [`Error::Locked`], [`Error::EmptyPassword`] for an empty PIN, storage errors.
    pub fn set_pin(&mut self, pin: &str, persist: bool) -> Result<Zeroizing<String>> {
        let vault_key = self.vault_key_copy()?;
        if pin.is_empty() {
            return Err(Error::EmptyPassword);
        }
        let salt = random_salt()?;
        let (_, kdf_params) = self.load_kdf()?;
        let key = pin_key(pin, &salt, &kdf_params);
        let record = PinRecord {
            salt: b64_encode(&salt),
            kdf_params,
            key: wrap_key(&vault_key, AAD_PIN_KEY, &key)?,
            persist,
            failures: 0,
        };
        let wrapped = Zeroizing::new(wrap_key(&key, AAD_PIN_VAULT_KEY, &vault_key)?);
        self.save_pin_record(&record)?;
        self.pin_wrapped = Some(wrapped.clone());
        Ok(wrapped)
    }

    /// Turns the PIN off. Works while locked.
    ///
    /// # Errors
    /// Storage errors.
    pub fn clear_pin(&mut self) -> Result<()> {
        self.pin_wrapped = None;
        self.store.delete_meta(meta::PIN)
    }

    /// Records whether the PIN works after a restart. Turning it on returns the wrapped vault key
    /// for the shell to keep in the OS credential store.
    ///
    /// # Errors
    /// [`Error::PinUnavailable`] without a PIN, [`Error::Locked`] when turning it on while locked,
    /// storage errors.
    pub fn set_pin_persist(&mut self, persist: bool) -> Result<Option<Zeroizing<String>>> {
        let mut record = self.pin_record()?.ok_or(Error::PinUnavailable)?;
        let wrapped = if persist {
            if !self.is_unlocked() {
                return Err(Error::Locked);
            }
            Some(self.pin_wrapped.clone().ok_or(Error::PinUnavailable)?)
        } else {
            None
        };
        record.persist = persist;
        self.save_pin_record(&record)?;
        Ok(wrapped)
    }

    /// The vault key wrapped by the PIN key, while the PIN can unlock from memory.
    #[must_use]
    pub fn pin_wrapped_key(&self) -> Option<Zeroizing<String>> {
        self.pin_wrapped.clone()
    }

    /// Unlocks with the PIN. `stored` is the wrapped vault key the shell kept in the OS
    /// credential store, used when none is in memory (after a restart). Runs Argon2: call it off
    /// the async runtime.
    ///
    /// # Errors
    /// [`Error::WrongPin`] with the attempts left; [`Error::PinUnavailable`] when no PIN unlock
    /// is possible (the last wrong PIN also drops the wrapped key from memory, and the shell
    /// should drop its stored copy); [`Error::WrongVaultKey`], storage errors.
    pub fn unlock_with_pin(&mut self, pin: &str, stored: Option<&str>) -> Result<()> {
        let mut record = self.pin_record()?.ok_or(Error::PinUnavailable)?;
        if record.failures >= MAX_PIN_ATTEMPTS {
            self.pin_wrapped = None;
            return Err(Error::PinUnavailable);
        }
        let wrapped = match (&self.pin_wrapped, stored) {
            (Some(held), _) => held.clone(),
            (None, Some(stored)) if record.persist => Zeroizing::new(stored.to_owned()),
            _ => return Err(Error::PinUnavailable),
        };
        // Counted before Argon2 runs, so an attempt cut short still counts.
        record.failures += 1;
        self.save_pin_record(&record)?;
        let key = pin_key(pin, &decode_salt(&record.salt)?, &record.kdf_params);
        match unwrap_key(&key, AAD_PIN_VAULT_KEY, &wrapped) {
            // A full unlock resets the failures and wraps the key again.
            Ok(vault_key) => self.unlock_with_vault_key(vault_key),
            Err(Error::Decrypt) => {
                let attempts_left = MAX_PIN_ATTEMPTS.saturating_sub(record.failures);
                if attempts_left == 0 {
                    self.pin_wrapped = None;
                }
                Err(Error::WrongPin { attempts_left })
            }
            Err(e) => Err(e),
        }
    }

    /// After any unlock: wraps `vault_key` with the PIN key again and resets the failures. A PIN
    /// whose key this vault key cannot open (another vault's) is turned off. Never fails the
    /// unlock: problems are logged.
    pub(crate) fn arm_pin(&mut self, vault_key: &Key32) {
        self.pin_wrapped = None;
        let record = match self.pin_record() {
            Ok(Some(record)) => record,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!("could not read the PIN unlock: {e}");
                return;
            }
        };
        let Ok(key) = unwrap_key(vault_key, AAD_PIN_KEY, &record.key) else {
            tracing::warn!("the PIN unlock does not belong to this vault; turning it off");
            if let Err(e) = self.store.delete_meta(meta::PIN) {
                tracing::warn!("could not turn the PIN unlock off: {e}");
            }
            return;
        };
        match wrap_key(&key, AAD_PIN_VAULT_KEY, vault_key) {
            Ok(wrapped) => self.pin_wrapped = Some(Zeroizing::new(wrapped)),
            Err(e) => tracing::warn!("could not prepare the PIN unlock: {e}"),
        }
        if record.failures != 0 {
            let record = PinRecord {
                failures: 0,
                ..record
            };
            if let Err(e) = self.save_pin_record(&record) {
                tracing::warn!("could not reset the PIN failures: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::random_key;
    use crate::vault::ManualClock;
    use std::sync::Arc;

    const PW: &str = "correct horse battery staple";

    fn foreign_key() -> Key32 {
        random_key().expect("rng")
    }

    fn unlocked() -> Vault {
        let mut vault = Vault::open_in_memory()
            .unwrap()
            .with_clock(Arc::new(ManualClock::new(1_700_000_000_000)));
        vault
            .create_with_params(PW, KdfParams::for_tests())
            .unwrap();
        vault
    }

    #[test]
    fn a_pin_unlocks_after_a_lock_until_the_app_restarts() {
        let mut vault = unlocked();
        assert_eq!(vault.pin_status(), PinStatus::default());
        vault.set_pin("2468", false).unwrap();
        vault.lock();
        assert_eq!(
            vault.pin_status(),
            PinStatus {
                enabled: true,
                persist: false,
                armed: true,
                exhausted: false,
            }
        );
        vault.unlock_with_pin("2468", None).unwrap();
        assert!(vault.is_unlocked());

        // A restart: the database stays, memory does not.
        vault.lock();
        vault.pin_wrapped = None;
        assert!(!vault.pin_status().armed);
        assert!(matches!(
            vault.unlock_with_pin("2468", None),
            Err(Error::PinUnavailable)
        ));
        // A stored copy is not used while the PIN is memory-only.
        let stored = {
            let mut other = unlocked();
            other.set_pin("2468", true).unwrap()
        };
        assert!(matches!(
            vault.unlock_with_pin("2468", Some(&stored)),
            Err(Error::PinUnavailable)
        ));
        // The master password arms it again.
        vault.unlock(PW).unwrap();
        vault.lock();
        vault.unlock_with_pin("2468", None).unwrap();
    }

    #[test]
    fn a_persisted_pin_unlocks_after_a_restart_with_the_stored_key() {
        let mut vault = unlocked();
        let stored = vault.set_pin("pin-with-letters", true).unwrap();
        vault.lock();
        vault.pin_wrapped = None;
        vault
            .unlock_with_pin("pin-with-letters", Some(&stored))
            .unwrap();
        assert!(vault.is_unlocked() && vault.pin_status().armed);
    }

    #[test]
    fn five_wrong_pins_turn_it_off_until_a_password_unlock() {
        let mut vault = unlocked();
        let stored = vault.set_pin("2468", true).unwrap();
        vault.lock();
        for left in (1..MAX_PIN_ATTEMPTS).rev() {
            assert!(matches!(
                vault.unlock_with_pin("1357", Some(&stored)),
                Err(Error::WrongPin { attempts_left }) if attempts_left == left
            ));
        }
        assert!(matches!(
            vault.unlock_with_pin("1357", Some(&stored)),
            Err(Error::WrongPin { attempts_left: 0 })
        ));
        assert!(!vault.pin_status().armed && vault.pin_status().exhausted);
        // Neither the right PIN nor the stored key gets in now.
        assert!(matches!(
            vault.unlock_with_pin("2468", Some(&stored)),
            Err(Error::PinUnavailable)
        ));
        assert!(!vault.is_unlocked());

        vault.unlock(PW).unwrap();
        assert!(!vault.pin_status().exhausted);
        vault.lock();
        vault.unlock_with_pin("2468", None).unwrap();
    }

    #[test]
    fn a_right_pin_resets_the_failures() {
        let mut vault = unlocked();
        vault.set_pin("2468", false).unwrap();
        vault.lock();
        for _ in 0..MAX_PIN_ATTEMPTS - 1 {
            assert!(vault.unlock_with_pin("0000", None).is_err());
        }
        vault.unlock_with_pin("2468", None).unwrap();
        vault.lock();
        assert!(matches!(
            vault.unlock_with_pin("0000", None),
            Err(Error::WrongPin { attempts_left }) if attempts_left == MAX_PIN_ATTEMPTS - 1
        ));
    }

    #[test]
    fn the_failures_survive_a_restart() {
        let mut vault = unlocked();
        let stored = vault.set_pin("2468", true).unwrap();
        vault.lock();
        for _ in 0..MAX_PIN_ATTEMPTS - 1 {
            assert!(vault.unlock_with_pin("0000", Some(&stored)).is_err());
        }
        vault.pin_wrapped = None;
        assert!(matches!(
            vault.unlock_with_pin("0000", Some(&stored)),
            Err(Error::WrongPin { attempts_left: 0 })
        ));
    }

    #[test]
    fn clearing_and_persist_switch() {
        let mut vault = unlocked();
        assert!(matches!(
            vault.set_pin_persist(true),
            Err(Error::PinUnavailable)
        ));
        vault.set_pin("2468", false).unwrap();
        let wrapped = vault.set_pin_persist(true).unwrap().unwrap();
        assert_eq!(*wrapped, *vault.pin_wrapped_key().unwrap());
        assert!(vault.pin_status().persist);
        assert!(vault.set_pin_persist(false).unwrap().is_none());
        assert!(!vault.pin_status().persist);

        vault.lock();
        // Turning it on needs the vault key in memory; turning the PIN off does not.
        assert!(matches!(vault.set_pin_persist(true), Err(Error::Locked)));
        vault.clear_pin().unwrap();
        assert_eq!(vault.pin_status(), PinStatus::default());
        assert!(matches!(
            vault.unlock_with_pin("2468", None),
            Err(Error::PinUnavailable)
        ));
    }

    #[test]
    fn setting_a_pin_needs_the_vault_unlocked_and_a_pin() {
        let mut vault = unlocked();
        assert!(matches!(
            vault.set_pin("", false),
            Err(Error::EmptyPassword)
        ));
        vault.lock();
        assert!(matches!(vault.set_pin("2468", false), Err(Error::Locked)));
    }

    #[test]
    fn a_pin_of_another_vault_is_turned_off() {
        let mut vault = unlocked();
        vault.set_pin("2468", false).unwrap();
        let record = vault.pin_record().unwrap().unwrap();
        let foreign = PinRecord {
            key: wrap_key(&foreign_key(), AAD_PIN_KEY, &foreign_key()).unwrap(),
            ..record
        };
        vault.save_pin_record(&foreign).unwrap();
        vault.lock();
        vault.unlock(PW).unwrap();
        assert_eq!(vault.pin_status(), PinStatus::default());
    }

    #[test]
    fn the_database_never_holds_the_wrapped_vault_key() {
        let mut vault = unlocked();
        let wrapped = vault.set_pin("2468", true).unwrap();
        let stored = vault.store.get_meta(meta::PIN).unwrap().unwrap();
        let wrapped_ct = crypto::Envelope::from_json(&wrapped).unwrap().c;
        assert!(!stored.contains(&wrapped_ct));
        assert!(!stored.contains("2468"));
    }
}
