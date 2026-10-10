//! Encrypted backup export and import (VAULT-07).
//!
//! The file is a single JSON document that reuses the vault's own envelopes, so it holds no
//! plaintext at all:
//!
//! ```json
//! { "format": "hatoba-backup", "v": 1, "created_at": 1700000000000,
//!   "kdf_salt": "…", "kdf_params": "{…}",
//!   "protected_vault_key": "{envelope}", "recovery_vault_key": "{envelope}",
//!   "items": [ { "id": "…", "envelope": "{envelope}", "deleted": false, "updated_at": 1 } ] }
//! ```
//!
//! Opening it needs the master password (or the recovery code through the same key blobs).

use std::io::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::crypto::{AAD_VAULT_KEY, KdfParams, MasterKeys, decode_salt, unwrap_key};
use crate::error::{Error, Result};
use crate::model::new_id;
use crate::store::{ItemRow, Store, StoreOps, meta};
use crate::vault::{Vault, seal_vault_check};

/// Value of the `format` field.
pub const BACKUP_FORMAT: &str = "hatoba-backup";
/// The only backup version this build reads and writes.
pub const BACKUP_VERSION: u32 = 1;

/// The backup document.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupFile {
    /// Always [`BACKUP_FORMAT`].
    pub format: String,
    /// Always [`BACKUP_VERSION`].
    pub v: u32,
    /// Creation time, Unix ms.
    pub created_at: i64,
    /// Base64 Argon2 salt.
    pub kdf_salt: String,
    /// KDF parameters JSON.
    pub kdf_params: String,
    /// Vault key wrapped by `enc_key`.
    pub protected_vault_key: String,
    /// Vault key wrapped by the recovery key.
    pub recovery_vault_key: String,
    /// Every item, tombstones included.
    pub items: Vec<BackupItem>,
}

/// One item of a backup.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupItem {
    /// Item id.
    pub id: String,
    /// Envelope JSON; absent for tombstones.
    pub envelope: Option<String>,
    /// Tombstone flag.
    pub deleted: bool,
    /// Plaintext `updated_at`.
    pub updated_at: i64,
}

/// Writes a backup of `store` to `path`, atomically (temp file + rename).
pub(crate) fn export(store: &Store, path: &Path, now_ms: i64) -> Result<()> {
    let get = |key: &str| store.get_meta(key)?.ok_or(Error::VaultNotInitialized);
    let file = BackupFile {
        format: BACKUP_FORMAT.to_owned(),
        v: BACKUP_VERSION,
        created_at: now_ms,
        kdf_salt: get(meta::KDF_SALT)?,
        kdf_params: get(meta::KDF_PARAMS)?,
        protected_vault_key: get(meta::PROTECTED_VAULT_KEY)?,
        recovery_vault_key: get(meta::RECOVERY_VAULT_KEY)?,
        items: store
            .item_rows()?
            .into_iter()
            .map(|r| BackupItem {
                id: r.id,
                envelope: r.envelope,
                deleted: r.deleted,
                updated_at: r.updated_at,
            })
            .collect(),
    };
    let json = serde_json::to_vec_pretty(&file)?;

    let mut tmp_name = path
        .file_name()
        .ok_or_else(|| Error::Format("backup path has no file name".into()))?
        .to_owned();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    {
        let mut f = open_private(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

#[cfg(unix)]
fn open_private(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_private(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

/// Reads and structurally validates a backup file.
///
/// # Errors
/// [`Error::Io`]; [`Error::Format`] for something that is not a Hatoba backup;
/// [`Error::UnsupportedVersion`] for a newer backup version.
pub fn read_backup(path: &Path) -> Result<BackupFile> {
    let bytes = std::fs::read(path)?;
    let file: BackupFile =
        serde_json::from_slice(&bytes).map_err(|_| Error::Format("not a Hatoba backup".into()))?;
    if file.format != BACKUP_FORMAT {
        return Err(Error::Format("not a Hatoba backup".into()));
    }
    if file.v != BACKUP_VERSION {
        return Err(Error::UnsupportedVersion(format!("backup v{}", file.v)));
    }
    Ok(file)
}

impl Vault {
    /// Restores a backup into an empty database (P2). The backup's master password is required;
    /// all items are marked unsynced so they upload if sync is enabled later. The vault is
    /// left unlocked and gets a fresh device id.
    ///
    /// # Errors
    /// [`Error::VaultAlreadyInitialized`]; [`Error::WrongPassword`]; format errors from
    /// [`read_backup`].
    pub fn import_backup_into_new_vault(&mut self, path: &Path, password: &str) -> Result<()> {
        if self.is_initialized() {
            return Err(Error::VaultAlreadyInitialized);
        }
        let file = read_backup(path)?;
        let params = KdfParams::from_json(&file.kdf_params)?;
        let salt = decode_salt(&file.kdf_salt)?;
        let keys = MasterKeys::from_password(password, &salt, &params);
        let vault_key = match unwrap_key(&keys.enc_key, AAD_VAULT_KEY, &file.protected_vault_key) {
            Ok(k) => k,
            Err(Error::Decrypt) => return Err(Error::WrongPassword),
            Err(e) => return Err(e),
        };
        let check = seal_vault_check(&vault_key)?;
        self.store.transaction(|tx| {
            tx.set_meta(
                meta::SCHEMA_VERSION,
                &crate::store::SCHEMA_VERSION.to_string(),
            )?;
            tx.set_meta(meta::KDF_SALT, &file.kdf_salt)?;
            tx.set_meta(meta::KDF_PARAMS, &file.kdf_params)?;
            tx.set_meta(meta::PROTECTED_VAULT_KEY, &file.protected_vault_key)?;
            tx.set_meta(meta::RECOVERY_VAULT_KEY, &file.recovery_vault_key)?;
            tx.set_meta(meta::VAULT_CHECK, &check)?;
            tx.set_meta(meta::DEVICE_ID, &new_id())?;
            for item in &file.items {
                tx.upsert_item_row(&ItemRow {
                    id: item.id.clone(),
                    envelope: item.envelope.clone().filter(|_| !item.deleted),
                    revision: 0,
                    deleted: item.deleted,
                    dirty: true,
                    updated_at: item.updated_at,
                })?;
            }
            Ok(())
        })?;
        self.finish_unlock(vault_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::KdfParams;
    use crate::model::{Host, HostAuth, Item};

    const PW: &str = "backup-test-password";

    fn populated() -> (Vault, String, String) {
        let mut vault = Vault::open_in_memory().unwrap();
        vault
            .create_with_params(PW, KdfParams::for_tests())
            .unwrap();
        let host = vault
            .put(
                None,
                Item::Host(Host {
                    name: "backup-host-name".into(),
                    auth: HostAuth::password("backup-pass"),
                    ..Host::default()
                }),
            )
            .unwrap();
        let gone = vault
            .put(
                None,
                Item::Host(Host {
                    name: "to-delete".into(),
                    ..Host::default()
                }),
            )
            .unwrap();
        vault.delete(&gone).unwrap();
        (vault, host, gone)
    }

    #[test]
    fn export_contains_only_ciphertext_and_works_while_locked() {
        let (mut vault, host, gone) = populated();
        vault.lock();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hatoba.backup");
        vault.export_backup(&path).unwrap();
        assert!(!dir.path().join("hatoba.backup.tmp").exists());

        let raw = std::fs::read_to_string(&path).unwrap();
        for needle in [
            "backup-host-name",
            "backup-pass",
            "to-delete",
            PW,
            "\"type\"",
            crate::model::DEFAULT_FONT_FAMILY,
        ] {
            assert!(!raw.contains(needle), "{needle} leaked into the backup");
        }
        let file = read_backup(&path).unwrap();
        assert_eq!((file.format.as_str(), file.v), ("hatoba-backup", 1));
        assert_eq!(file.items.len(), 3); // host, tombstone, settings
        let tombstone = file.items.iter().find(|i| i.id == gone).unwrap();
        assert!(tombstone.deleted && tombstone.envelope.is_none());
        assert!(
            file.items
                .iter()
                .find(|i| i.id == host)
                .unwrap()
                .envelope
                .is_some()
        );
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        for key in [
            "format",
            "v",
            "created_at",
            "kdf_salt",
            "kdf_params",
            "protected_vault_key",
            "recovery_vault_key",
            "items",
        ] {
            assert!(json.get(key).is_some(), "missing {key}");
        }
    }

    #[test]
    fn import_restores_everything_into_a_new_vault() {
        let (vault, host, gone) = populated();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.json");
        vault.export_backup(&path).unwrap();

        let mut fresh = Vault::open_in_memory().unwrap();
        assert!(matches!(
            fresh.import_backup_into_new_vault(&path, "wrong"),
            Err(Error::WrongPassword)
        ));
        assert!(!fresh.status().initialized);
        fresh.import_backup_into_new_vault(&path, PW).unwrap();
        assert!(fresh.is_unlocked());
        assert_eq!(fresh.get(&host).unwrap().display_name(), "backup-host-name");
        assert!(fresh.get(&gone).is_none());
        assert_ne!(fresh.device_id(), vault.device_id());
        assert_eq!(
            fresh.pending_count(),
            3,
            "imported rows are queued for upload"
        );
        // The password and recovery blobs came along.
        fresh.lock();
        fresh.unlock(PW).unwrap();
        assert!(matches!(
            fresh.import_backup_into_new_vault(&path, PW),
            Err(Error::VaultAlreadyInitialized)
        ));
    }

    #[test]
    fn rejects_foreign_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        std::fs::write(&path, b"{\"hello\":1}").unwrap();
        assert!(matches!(read_backup(&path), Err(Error::Format(_))));
        std::fs::write(&path, br#"{"format":"other","v":1,"created_at":0,"kdf_salt":"","kdf_params":"","protected_vault_key":"","recovery_vault_key":"","items":[]}"#).unwrap();
        assert!(matches!(read_backup(&path), Err(Error::Format(_))));
        std::fs::write(&path, br#"{"format":"hatoba-backup","v":9,"created_at":0,"kdf_salt":"","kdf_params":"","protected_vault_key":"","recovery_vault_key":"","items":[]}"#).unwrap();
        assert!(matches!(
            read_backup(&path),
            Err(Error::UnsupportedVersion(_))
        ));
        assert!(matches!(
            read_backup(&dir.path().join("missing")),
            Err(Error::Io(_))
        ));
    }

    #[test]
    fn export_requires_a_vault() {
        let vault = Vault::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            vault.export_backup(&dir.path().join("b")),
            Err(Error::VaultNotInitialized)
        ));
    }
}
