//! Platform capabilities injected into the core (spec §3.4).
//!
//! `hatoba-core` must stay free of desktop-only dependencies so it can be reused on mobile.
//! Anything OS-specific (keychain, biometrics) is reached through the traits in this module.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{Error, Result};

/// Well-known [`SecretStore`] keys.
pub mod secret_keys {
    /// The sync session token (JSON of [`Session`](crate::sync::Session)).
    pub const SYNC_SESSION: &str = "sync/session";
    /// The Cloudflare API token used by D1 direct mode.
    pub const D1_API_TOKEN: &str = "sync/d1-api-token";
    /// The vault key wrapped for biometric unlock (P1).
    pub const BIOMETRIC_VAULT_KEY: &str = "vault/biometric-key";
    /// The vault key wrapped by the PIN key, kept only while the PIN works after a restart
    /// (SEC-11).
    pub const PIN_VAULT_KEY: &str = "vault/pin-key";
}

/// A secret store backed by the OS credential manager (Windows Credential Manager, macOS
/// Keychain, …). Implemented by the shell; values must never be logged.
pub trait SecretStore: Send + Sync {
    /// Reads a secret.
    ///
    /// # Errors
    /// Backend failures. A missing entry is `Ok(None)`.
    fn get(&self, key: &str) -> Result<Option<Zeroizing<String>>>;

    /// Creates or replaces a secret.
    ///
    /// # Errors
    /// Backend failures.
    fn set(&self, key: &str, value: &str) -> Result<()>;

    /// Removes a secret; removing a missing entry succeeds.
    ///
    /// # Errors
    /// Backend failures.
    fn delete(&self, key: &str) -> Result<()>;
}

/// In-memory [`SecretStore`] for tests and for platforms without a keychain.
#[derive(Default)]
pub struct MemorySecretStore {
    entries: Mutex<HashMap<String, Zeroizing<String>>>,
}

impl MemorySecretStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, key: &str) -> Result<Option<Zeroizing<String>>> {
        let entries = self.entries.lock().map_err(|_| Error::Poisoned)?;
        Ok(entries.get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        let mut entries = self.entries.lock().map_err(|_| Error::Poisoned)?;
        entries.insert(key.to_owned(), Zeroizing::new(value.to_owned()));
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        let mut entries = self.entries.lock().map_err(|_| Error::Poisoned)?;
        entries.remove(key);
        Ok(())
    }
}

/// What this device calls itself in the device list. Sealed with the vault key before it is
/// sent, so the Worker never sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Human-readable device name, e.g. `Alice's laptop`.
    pub name: String,
    /// Platform label, e.g. `windows`, `macos`, `linux`.
    pub platform: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trip() {
        let store = MemorySecretStore::new();
        assert!(store.get("a").unwrap().is_none());
        store.set("a", "one").unwrap();
        store.set("a", "two").unwrap();
        assert_eq!(store.get("a").unwrap().unwrap().as_str(), "two");
        store.delete("a").unwrap();
        store.delete("a").unwrap();
        assert!(store.get("a").unwrap().is_none());
    }

    #[test]
    fn usable_as_trait_object() {
        let store: Box<dyn SecretStore> = Box::new(MemorySecretStore::new());
        store.set(secret_keys::SYNC_SESSION, "{}").unwrap();
        assert!(store.get(secret_keys::SYNC_SESSION).unwrap().is_some());
    }
}
