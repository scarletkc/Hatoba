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

use zeroize::{Zeroize, Zeroizing};

use crate::crypto::{self, Key32, item_aad, seal};
use crate::error::{Error, Result};
use crate::model::{
    Group, Host, Item, KnownHost, PortForward, Proxy, RightClick, SETTINGS_ID, Settings, SshKey,
    TelnetHost,
};
use crate::store::{ItemRow, Store, StoreOps};

pub(crate) use crate::vault_keys::seal_vault_check;
pub use crate::vault_keys::{
    FREE_UNLOCK_ATTEMPTS, MAX_UNLOCK_DELAY_SECS, PasswordChange, RecoveryChange, VaultStatusInfo,
    unlock_delay_ms,
};
pub use crate::vault_sync::LocalMeta;
pub(crate) use crate::vault_sync::SyncCtx;

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

/// Decrypts a row into the form the item map holds. `Ok(None)` for tombstones.
///
/// A conversation message part comes back without its `data` (spec §13.7); opening the
/// conversation reads it again from the store ([`Vault::ai_entries`],
/// [`Vault::ai_entries_from`]).
pub(crate) fn decode_row(key: &[u8; 32], row: &ItemRow) -> Result<Option<Item>> {
    let Some(env) = row.envelope.as_deref().filter(|_| !row.deleted) else {
        return Ok(None);
    };
    let mut item = decode_envelope(key, &row.id, env)?;
    item.strip_message_data();
    Ok(Some(item))
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

/// Monotonic `updated_at` for a local write: `now`, or one millisecond past `previous` when the
/// local clock is at or behind it.
pub(crate) fn next_updated_at(previous: Option<&ItemRow>, now: i64) -> i64 {
    previous.map_or(now, |p| now.max(p.updated_at.saturating_add(1)))
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
    /// The vault key wrapped by the PIN key (SEC-11, `vault_pin`). Kept across [`Vault::lock`]
    /// and never written to the database.
    pub(crate) pin_wrapped: Option<Zeroizing<String>>,
    /// Message parts read back from the store ([`Vault::ai_parts_read`]).
    #[cfg(any(test, feature = "test-util"))]
    pub(crate) ai_parts_read: std::sync::atomic::AtomicUsize,
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
            pin_wrapped: None,
            #[cfg(any(test, feature = "test-util"))]
            ai_parts_read: std::sync::atomic::AtomicUsize::new(0),
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
    /// The item map keeps a message part without its `data`, so it is wiped from `item` here.
    ///
    /// # Errors
    /// [`Error::Locked`]; [`Error::InvalidItem`] for a malformed id or a settings/id mismatch.
    pub fn put(&mut self, id: Option<&str>, mut item: Item) -> Result<String> {
        let id = match (id, &item) {
            (Some(id), _) => id.to_owned(),
            (None, Item::Settings(_)) => SETTINGS_ID.to_owned(),
            (None, _) => crate::model::new_id(),
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

        let updated_at = next_updated_at(previous.as_ref(), now);
        item.set_updated_at(updated_at);
        let envelope = seal_item(&unlocked.vault_key, &id, &item)?;
        let revision = previous.map_or(0, |p| p.revision);
        self.store.upsert_item_row(&ItemRow::live(
            id.clone(),
            envelope.to_json(),
            revision,
            updated_at,
        ))?;
        // A message part keeps only its header in memory.
        item.strip_message_data();
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
        self.store.upsert_item_row(&ItemRow::tombstone(
            id.to_owned(),
            row.revision,
            next_updated_at(Some(&row), now),
        ))?;
        unlocked.items.remove(id);
        Ok(())
    }

    pub(crate) fn collect<T>(&self, pick: impl Fn(&Item) -> Option<&T>) -> Vec<(String, T)>
    where
        T: Clone,
    {
        self.items()
            .filter_map(|(id, item)| pick(item).map(|t| (id.to_owned(), t.clone())))
            .collect()
    }

    #[must_use]
    pub fn hosts(&self) -> Vec<(String, Host)> {
        self.collect(Item::as_host)
    }

    /// All Telnet hosts (TELNET-01).
    #[must_use]
    pub fn telnet_hosts(&self) -> Vec<(String, TelnetHost)> {
        self.collect(Item::as_telnet_host)
    }

    #[must_use]
    pub fn keys(&self) -> Vec<(String, SshKey)> {
        self.collect(Item::as_key)
    }

    #[must_use]
    pub fn groups(&self) -> Vec<(String, Group)> {
        self.collect(Item::as_group)
    }

    #[must_use]
    pub fn known_hosts(&self) -> Vec<(String, KnownHost)> {
        self.collect(Item::as_known_host)
    }

    /// All proxies (SSH-13).
    #[must_use]
    pub fn proxies(&self) -> Vec<(String, Proxy)> {
        self.collect(Item::as_proxy)
    }

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

    /// Fills the terminal behaviour fields that are still unset in the settings with the values
    /// this device kept locally before they were synced, and returns whether it wrote the
    /// settings. A field already set keeps its value, and a default value leaves the field unset.
    ///
    /// Call it only once this device holds the latest settings (after a successful sync round,
    /// or when sync is off). The write gives the whole item a newer `updated_at`, so on a stale
    /// copy it would win over newer edits from other devices (spec §6.4).
    ///
    /// # Errors
    /// [`Error::Locked`]; storage errors.
    pub fn adopt_device_terminal_prefs(
        &mut self,
        right_click: Option<RightClick>,
        confirm_multiline_paste: Option<bool>,
    ) -> Result<bool> {
        if !self.is_unlocked() {
            return Err(Error::Locked);
        }
        // Default settings when none is stored yet; `put` then creates the item.
        let current = self.settings();
        let mut settings = current.clone();
        let t = &mut settings.terminal;
        if t.right_click.is_none()
            && let Some(value) = right_click
        {
            t.set_right_click(value);
        }
        if t.confirm_multiline_paste.is_none()
            && let Some(value) = confirm_multiline_paste
        {
            t.set_confirm_multiline_paste(value);
        }
        if settings == current {
            return Ok(false);
        }
        self.put(Some(SETTINGS_ID), Item::Settings(settings))?;
        Ok(true)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::KdfParams;
    use crate::model::{AiConversation, HostAuth};
    use crate::recovery::RecoveryCode;
    use crate::test_support::{TEST_PASSWORD as PW, host_item as host, unlocked_vault_at};

    fn created() -> (Vault, Arc<ManualClock>, RecoveryCode) {
        unlocked_vault_at(1_000_000, PW)
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
        s.terminal.scrollback = 5_000;
        assert_eq!(vault.put(None, Item::Settings(s)).unwrap(), SETTINGS_ID);
        assert_eq!(vault.settings().terminal.scrollback, 5_000);
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
        for name in ["vault.db-wal", "vault.db"] {
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
            crate::model::DEFAULT_FONT_FAMILY,
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

    #[test]
    fn raw_rows_after_item_writes_preserve_revision_dirty_and_updated_at() {
        let (mut vault, clock, _) = created();
        let t0 = clock.now_ms();

        let mark_synced = |vault: &Vault, id: &str, revision: u64, updated_at: i64| {
            let mut row = vault.store.item_row(id).unwrap().unwrap();
            row.revision = revision;
            row.dirty = false;
            row.updated_at = updated_at;
            vault.store.upsert_item_row(&row).unwrap();
        };

        // 1. Vault::put (insert, then update with clock behind previous.updated_at).
        let put_id = vault.put(None, host("put-host")).unwrap();
        let row = vault.store.item_row(&put_id).unwrap().unwrap();
        assert_eq!(
            (row.revision, row.dirty, row.deleted, row.updated_at),
            (0, true, false, t0)
        );
        assert!(row.envelope.is_some());

        mark_synced(&vault, &put_id, 5, t0 + 500);
        vault.put(Some(&put_id), host("put-host-v2")).unwrap();
        let row = vault.store.item_row(&put_id).unwrap().unwrap();
        assert_eq!(
            (row.revision, row.dirty, row.deleted, row.updated_at),
            (5, true, false, t0 + 501)
        );
        assert!(row.envelope.is_some());

        // 2. Vault::delete (clock behind, then clock ahead).
        vault.delete(&put_id).unwrap();
        let row = vault.store.item_row(&put_id).unwrap().unwrap();
        assert_eq!(
            (row.revision, row.dirty, row.deleted, row.updated_at),
            (5, true, true, t0 + 502)
        );
        assert!(row.envelope.is_none());

        clock.set(t0 + 2_000);
        let del_ahead = vault.put(None, host("del-ahead")).unwrap();
        mark_synced(&vault, &del_ahead, 3, t0 + 1_000);
        vault.delete(&del_ahead).unwrap();
        let row = vault.store.item_row(&del_ahead).unwrap().unwrap();
        assert_eq!(
            (row.revision, row.dirty, row.deleted, row.updated_at),
            (3, true, true, t0 + 2_000)
        );
        assert!(row.envelope.is_none());

        // 3. commit_cascade (puts + deletes, clock behind, and ItemNotFound on missing delete).
        let cascade_put = vault.put(None, host("cascade-put")).unwrap();
        let cascade_del = vault.put(None, host("cascade-del")).unwrap();
        mark_synced(&vault, &cascade_put, 7, t0 + 5_000);
        mark_synced(&vault, &cascade_del, 8, t0 + 6_000);
        vault
            .commit_cascade(
                vec![(cascade_put.clone(), host("cascade-put-v2"))],
                std::slice::from_ref(&cascade_del),
            )
            .unwrap();
        let put_row = vault.store.item_row(&cascade_put).unwrap().unwrap();
        assert_eq!(
            (
                put_row.revision,
                put_row.dirty,
                put_row.deleted,
                put_row.updated_at
            ),
            (7, true, false, t0 + 5_001)
        );
        assert!(put_row.envelope.is_some());
        let del_row = vault.store.item_row(&cascade_del).unwrap().unwrap();
        assert_eq!(
            (
                del_row.revision,
                del_row.dirty,
                del_row.deleted,
                del_row.updated_at
            ),
            (8, true, true, t0 + 6_001)
        );
        assert!(del_row.envelope.is_none());
        assert!(matches!(
            vault.commit_cascade(Vec::new(), &["missing-id".into()]),
            Err(Error::ItemNotFound(_))
        ));

        // 4 & 5. ai_create_conversation + seal_entry.
        clock.set(t0 + 10_000);
        let (conv_id, _entries) = vault
            .ai_create_conversation(
                AiConversation {
                    title: "conv".into(),
                    ..AiConversation::default()
                },
                &[r#"{"role":"user","content":"hi"}"#],
                Some(0),
            )
            .unwrap();
        let conv_row = vault.store.item_row(&conv_id).unwrap().unwrap();
        assert_eq!(
            (
                conv_row.revision,
                conv_row.dirty,
                conv_row.deleted,
                conv_row.updated_at
            ),
            (0, true, false, t0 + 10_000)
        );
        assert!(conv_row.envelope.is_some());
        let msg_ids: Vec<String> = vault
            .items()
            .filter_map(|(id, item)| {
                item.as_ai_message()
                    .filter(|m| m.conversation_id == conv_id)
                    .map(|_| id.to_owned())
            })
            .collect();
        assert_eq!(msg_ids.len(), 1);
        let msg_row = vault.store.item_row(&msg_ids[0]).unwrap().unwrap();
        assert_eq!(
            (
                msg_row.revision,
                msg_row.dirty,
                msg_row.deleted,
                msg_row.updated_at
            ),
            (0, true, false, t0 + 10_000)
        );
        assert!(msg_row.envelope.is_some());

        // 6. vault_ai::tombstone (preserves revision, bumps past updated_at, skips missing).
        mark_synced(&vault, &conv_id, 9, t0 + 12_000);
        mark_synced(&vault, &msg_ids[0], 10, t0 + 12_500);
        vault
            .tombstone(&[conv_id.clone(), msg_ids[0].clone(), "missing-id".into()])
            .unwrap();
        let conv_tomb = vault.store.item_row(&conv_id).unwrap().unwrap();
        assert_eq!(
            (
                conv_tomb.revision,
                conv_tomb.dirty,
                conv_tomb.deleted,
                conv_tomb.updated_at
            ),
            (9, true, true, t0 + 12_001)
        );
        assert!(conv_tomb.envelope.is_none());
        let msg_tomb = vault.store.item_row(&msg_ids[0]).unwrap().unwrap();
        assert_eq!(
            (
                msg_tomb.revision,
                msg_tomb.dirty,
                msg_tomb.deleted,
                msg_tomb.updated_at
            ),
            (10, true, true, t0 + 12_501)
        );
        assert!(msg_tomb.envelope.is_none());
    }
}
