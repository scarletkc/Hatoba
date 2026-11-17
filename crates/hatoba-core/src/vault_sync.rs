//! Sync-facing state and remote vault setup / adoption / joining on [`Vault`].

use std::collections::BTreeMap;

use crate::crypto::{self, AAD_RECOVERY_AUTH, Key32, item_aad, seal, unwrap_key};
use crate::error::{Error, Result};
use crate::model::{Item, SETTINGS_ID};
use crate::store::{ItemRow, KeyMeta, Store, StoreOps, meta};
use crate::sync::SyncConfig;
use crate::sync::backend::{RemoteItem, VaultMeta};
use crate::sync::engine::remote_row;
use crate::vault::{Vault, decode_envelope, decode_row, seal_item};
use crate::vault_keys::seal_vault_check;
use crate::vault_local::SealedLocal;

/// The non-secret vault metadata kept in `meta`, as uploaded on sync setup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalMeta {
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

/// Borrowed pieces of an unlocked vault, handed to the sync engine.
pub(crate) struct SyncCtx<'a> {
    pub(crate) store: &'a mut Store,
    pub(crate) key: &'a [u8; 32],
    pub(crate) items: &'a mut BTreeMap<String, Item>,
    pub(crate) now: i64,
}

impl SyncCtx<'_> {
    /// Re-reads one row and brings the decrypted cache in line with it (a message part without
    /// its `data`, see [`decode_row`]). Rows that are tombstones leave the cache; rows that
    /// cannot be decrypted are logged (id only) and skipped, which is reported as `false`.
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

/// Everything sync setup needs from the local vault, derived from the password.
pub(crate) struct SetupMaterial {
    pub(crate) meta: LocalMeta,
    pub(crate) auth_key: Key32,
    pub(crate) recovery_auth: Key32,
    pub(crate) vault_key: Key32,
    pub(crate) device_id: String,
}

/// Whether `remote` is a live settings item that `vault_key` opens.
fn opens_as_settings(vault_key: &[u8; 32], remote: &RemoteItem) -> bool {
    remote
        .envelope
        .as_deref()
        .filter(|_| !remote.deleted)
        .is_some_and(|env| {
            matches!(
                decode_envelope(vault_key, SETTINGS_ID, env),
                Ok(Item::Settings(_))
            )
        })
}

/// This device's settings for a cloud vault that has none yet (flow C), dated at the epoch:
/// older than the settings of any of the cloud vault's own devices, which therefore win over them
/// whenever they arrive (§6.4).
fn fallback_settings_row(vault_key: &[u8; 32], settings: &Item) -> Result<ItemRow> {
    let mut item = settings.clone();
    item.set_updated_at(0);
    Ok(ItemRow {
        id: SETTINGS_ID.to_owned(),
        envelope: Some(seal_item(vault_key, SETTINGS_ID, &item)?.to_json()),
        revision: 0,
        deleted: false,
        dirty: true,
        updated_at: 0,
    })
}

/// Drops this device's sealed recovery secret unless `recovery_vault_key`, the remote's
/// recovery wrapper about to be stored, is the one stored now: only the same recovery code keeps
/// the secret valid. A code rotated elsewhere, or another vault, whose wrapper never matches,
/// makes it stale.
fn drop_stale_recovery_auth(tx: &impl StoreOps, recovery_vault_key: &str) -> Result<()> {
    if tx.get_meta(meta::RECOVERY_VAULT_KEY)?.as_deref() != Some(recovery_vault_key) {
        tx.delete_meta(meta::RECOVERY_AUTH_SEALED)?;
    }
    Ok(())
}

fn write_remote_key_meta(tx: &impl StoreOps, remote: &VaultMeta) -> Result<()> {
    KeyMeta::write_fields(
        tx,
        &remote.kdf_salt,
        &remote.kdf_params,
        &remote.protected_vault_key,
        &remote.recovery_vault_key,
    )
}

impl Vault {
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
        let keys = KeyMeta::read(&self.store)?;
        Ok(LocalMeta {
            schema_version: u32::try_from(
                self.store.get_meta_i64(meta::SCHEMA_VERSION)?.unwrap_or(1),
            )
            .unwrap_or(1),
            kdf_salt: keys.kdf_salt,
            kdf_params: keys.kdf_params,
            protected_vault_key: keys.protected_vault_key,
            recovery_vault_key: keys.recovery_vault_key,
        })
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
    /// the already-unwrapped vault key (flow B). A sync backend is recorded in the same
    /// transaction, so the vault never exists without the config to resume syncing, and so is
    /// `cloud_settings`, the remote's settings item as read just before, stored as the first pull
    /// would store it. An edit made before that pull then builds on the remote's settings
    /// instead of the defaults, which would replace them everywhere (§6.4).
    /// Other existing items are not touched.
    pub(crate) fn install_remote_vault(
        &mut self,
        remote: &VaultMeta,
        vault_key: Key32,
        device_id: &str,
        config: Option<&SyncConfig>,
        cloud_settings: Option<&RemoteItem>,
    ) -> Result<()> {
        if self.is_initialized() {
            return Err(Error::VaultAlreadyInitialized);
        }
        let check = seal_vault_check(&vault_key)?;
        let config = config.map(serde_json::to_string).transpose()?;
        self.store.transaction(|tx| {
            tx.set_meta(
                meta::SCHEMA_VERSION,
                &crate::store::SCHEMA_VERSION
                    .max(remote.schema_version)
                    .to_string(),
            )?;
            write_remote_key_meta(tx, remote)?;
            tx.set_meta(meta::VAULT_CHECK, &check)?;
            tx.set_meta(meta::DEVICE_ID, device_id)?;
            if let Some(config) = &config {
                tx.set_meta(meta::SYNC_BACKEND, config)?;
            }
            if let Some(remote) = cloud_settings {
                tx.upsert_item_row(&remote_row(remote))?;
            }
            tx.delete_meta(meta::SYNC_CURSOR)
        })?;
        self.finish_unlock(vault_key)
    }

    /// Adopts the password-related metadata of the remote vault after verifying it wraps the
    /// same vault key (the password was changed on another device).
    pub(crate) fn adopt_remote_meta(&mut self, remote: &VaultMeta) -> Result<()> {
        self.store.transaction(|tx| {
            drop_stale_recovery_auth(tx, &remote.recovery_vault_key)?;
            write_remote_key_meta(tx, remote)
        })
    }

    /// Flow C: makes this unlocked vault part of an initialised remote vault, whose metadata and
    /// unwrapped key the caller authenticated for. The local master password becomes the
    /// remote's, `config` is recorded, and the pull starts over, all in one transaction.
    ///
    /// When the remote is this same vault (a device that synced before), every item is queued
    /// again so the next round reconciles both sides. Otherwise each live item is sealed under
    /// the remote vault key and queued as new, except the settings: `cloud_settings`, the
    /// remote's settings item as read just before, replaces the local one. When the remote has
    /// none that its key opens (none yet, or a damaged one), the local one is queued dated at the
    /// epoch, older than any settings of the remote's own devices, which therefore replace it
    /// whenever they arrive (an edit made here after the join is dated as usual). Tombstones,
    /// items that cannot be decrypted, and the conflict log are dropped, every [`SealedLocal`],
    /// file snapshot and command history is sealed again, and the PIN unlock is turned off.
    /// Returns whether the vault key changed, so the caller can drop what still holds the old one
    /// (biometric unlock, the PIN's copy in the OS credential store).
    ///
    /// Once the transaction has committed the join has happened: if reloading the items then
    /// fails, the vault is left locked (never on the old key) and this still returns `Ok`.
    pub(crate) fn join_remote_vault(
        &mut self,
        remote: &VaultMeta,
        vault_key: Key32,
        config: &SyncConfig,
        cloud_settings: Option<&RemoteItem>,
    ) -> Result<bool> {
        let old = self.vault_key_copy()?;
        let rekey = !self.verify_vault_key(&vault_key)?;
        let config = serde_json::to_string(config)?;
        // Device-local values sealed under the vault key, sealed again (`None`: dropped).
        let mut local = Vec::new();
        if rekey {
            for which in SealedLocal::ALL {
                let resealed = match self.store.get_meta(which.meta_key())? {
                    Some(sealed) => crypto::open_json(&old, which.aad(), &sealed)
                        .ok()
                        .map(|plain| seal(&vault_key, which.aad(), &plain))
                        .transpose()?
                        .map(|env| env.to_json()),
                    None => None,
                };
                local.push((which.meta_key(), resealed));
            }
        }
        // A cloud settings item the cloud key cannot open (a damaged one) could not win a conflict
        // either, so it counts as none.
        let settings = match cloud_settings.filter(|remote| opens_as_settings(&vault_key, remote)) {
            Some(remote) if rekey => Some(remote_row(remote)),
            None if rekey => self
                .get(SETTINGS_ID)
                .map(|item| fallback_settings_row(&vault_key, item))
                .transpose()?,
            _ => None,
        };
        let check = seal_vault_check(&vault_key)?;
        self.store.transaction(|tx| {
            if rekey {
                for row in tx.item_rows()? {
                    if row.id == SETTINGS_ID && settings.is_some() {
                        continue;
                    }
                    let plain = row
                        .envelope
                        .as_deref()
                        .filter(|_| !row.deleted)
                        .and_then(|env| {
                            crypto::open_json(&old, item_aad(&row.id).as_bytes(), env).ok()
                        });
                    let Some(plain) = plain else {
                        tx.delete_item_row(&row.id)?;
                        continue;
                    };
                    let envelope = seal(&vault_key, item_aad(&row.id).as_bytes(), &plain)?;
                    tx.upsert_item_row(&ItemRow {
                        envelope: Some(envelope.to_json()),
                        revision: 0,
                        dirty: true,
                        ..row
                    })?;
                }
                if let Some(row) = &settings {
                    tx.upsert_item_row(row)?;
                }
                tx.clear_conflicts()?;
                crate::vault_snapshots::reseal_snapshots(tx, &old, &vault_key)?;
                crate::vault_history::reseal_command_history(tx, &old, &vault_key)?;
                // The PIN wraps the old key, like the biometric unlock, and is turned off with it.
                tx.delete_meta(meta::PIN)?;
                for (key, resealed) in &local {
                    match resealed {
                        Some(sealed) => tx.set_meta(key, sealed)?,
                        None => tx.delete_meta(key)?,
                    }
                }
            } else {
                tx.mark_all_dirty_for_upload()?;
            }
            drop_stale_recovery_auth(tx, &remote.recovery_vault_key)?;
            tx.set_meta(
                meta::SCHEMA_VERSION,
                &crate::store::SCHEMA_VERSION
                    .max(remote.schema_version)
                    .to_string(),
            )?;
            write_remote_key_meta(tx, remote)?;
            tx.set_meta(meta::VAULT_CHECK, &check)?;
            tx.delete_meta(meta::UNLOCK_FAILURES)?;
            tx.delete_meta(meta::UNLOCK_LAST_FAILURE_AT)?;
            tx.set_meta(meta::SYNC_BACKEND, &config)?;
            tx.delete_meta(meta::SYNC_CURSOR)?;
            tx.delete_meta(meta::SYNC_LAST_AT)
        })?;
        // The join has committed: the old key and cache must not stay in use, or new edits would
        // be sealed under a key the vault no longer opens. Unlocking again with the remote's
        // password reloads them. The PIN's copy of the old key goes even if that reload fails.
        if rekey {
            self.pin_wrapped = None;
        }
        if let Err(e) = self.finish_unlock(vault_key) {
            tracing::warn!("could not reload the vault after joining: {e}");
            self.lock();
        }
        Ok(rekey)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::unlocked_vault;

    #[test]
    fn sync_config_round_trip() {
        let (mut vault, _clock) = unlocked_vault();
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
}
