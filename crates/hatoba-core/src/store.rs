//! The local SQLite store (spec §5.2).
//!
//! The database holds only ciphertext (plus a few non-secret counters and the opaque
//! `local_prefs` and `star_prompt` strings). `secure_delete` is on so freed pages do not keep old rows around.
//!
//! All row-level operations live on the [`StoreOps`] trait, implemented by both [`Store`]
//! (auto-commit) and [`StoreTx`] (inside a transaction opened with [`Store::transaction`]).

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::error::{Error, Result};

/// Newest schema this build understands.
pub const SCHEMA_VERSION: u32 = 1;

/// Number of conflict-log rows kept; older ones are pruned on insert.
pub const CONFLICT_LOG_LIMIT: i64 = 500;

/// Keys of the `meta` table.
pub mod meta {
    /// Schema version of the local database (also sent to the Worker on setup).
    pub const SCHEMA_VERSION: &str = "schema_version";
    /// Base64 Argon2 salt.
    pub const KDF_SALT: &str = "kdf_salt";
    /// KDF parameters as JSON.
    pub const KDF_PARAMS: &str = "kdf_params";
    /// Envelope JSON: vault key wrapped by `enc_key`.
    pub const PROTECTED_VAULT_KEY: &str = "protected_vault_key";
    /// Envelope JSON: vault key wrapped by `recovery_key`.
    pub const RECOVERY_VAULT_KEY: &str = "recovery_vault_key";
    /// This device's id (UUIDv7).
    pub const DEVICE_ID: &str = "device_id";
    /// JSON description of the configured sync backend (no secrets).
    pub const SYNC_BACKEND: &str = "sync_backend";
    /// Highest server `seq` pulled so far.
    pub const SYNC_CURSOR: &str = "sync_cursor";
    /// Time of the last completed sync round, Unix ms.
    pub const SYNC_LAST_AT: &str = "sync_last_at";
    /// Consecutive failed unlock attempts (SEC-06).
    pub const UNLOCK_FAILURES: &str = "unlock_failures";
    /// Time of the last failed unlock attempt, Unix ms.
    pub const UNLOCK_LAST_FAILURE_AT: &str = "unlock_last_failure_at";
    /// Opaque plaintext JSON of device-local UI preferences.
    pub const LOCAL_PREFS: &str = "local_prefs";
    /// Opaque plaintext JSON of the device-local star prompt state.
    pub const STAR_PROMPT: &str = "star_prompt";
    /// Envelope JSON: `recovery_auth` sealed under the vault key, so sync setup can upload it.
    pub const RECOVERY_AUTH_SEALED: &str = "recovery_auth_sealed";
    /// Envelope JSON: a constant sealed under the vault key, to verify a candidate vault key.
    pub const VAULT_CHECK: &str = "vault_check";
}

/// One row of the `items` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemRow {
    /// Item id.
    pub id: String,
    /// Envelope JSON; `None` for a tombstone.
    pub envelope: Option<String>,
    /// Server-confirmed revision; 0 if never synced.
    pub revision: u64,
    /// Tombstone flag.
    pub deleted: bool,
    /// Local changes not yet pushed.
    pub dirty: bool,
    /// Plaintext `updated_at`, mirrored from the item (also known to the server).
    pub updated_at: i64,
}

/// A row of the local-only `conflict_log` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictRow {
    /// Autoincrement id.
    pub id: i64,
    /// The item the conflict was about.
    pub item_id: String,
    /// How it was resolved (see `sync::conflict::Resolution`).
    pub resolution: String,
    /// The local version's envelope (still encrypted); `None` if it was a tombstone.
    pub local_envelope: Option<String>,
    /// The remote version's envelope; `None` if it was a tombstone.
    pub remote_envelope: Option<String>,
    /// Whether the local side was a deletion.
    pub local_deleted: bool,
    /// Whether the remote side was a deletion.
    pub remote_deleted: bool,
    /// Local `updated_at`.
    pub local_updated_at: Option<i64>,
    /// Remote `updated_at`.
    pub remote_updated_at: Option<i64>,
    /// When the conflict was resolved, Unix ms.
    pub created_at: i64,
    /// Whether the user has looked at it.
    pub reviewed: bool,
}

/// A conflict to append to the log.
#[derive(Clone, Debug)]
pub struct NewConflict {
    /// The item the conflict was about.
    pub item_id: String,
    /// Resolution name.
    pub resolution: String,
    /// Local envelope (encrypted).
    pub local_envelope: Option<String>,
    /// Remote envelope (encrypted).
    pub remote_envelope: Option<String>,
    /// Local side deleted.
    pub local_deleted: bool,
    /// Remote side deleted.
    pub remote_deleted: bool,
    /// Local `updated_at`.
    pub local_updated_at: Option<i64>,
    /// Remote `updated_at`.
    pub remote_updated_at: Option<i64>,
    /// Resolution time, Unix ms.
    pub created_at: i64,
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn from_i64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

const MIGRATIONS: &[&str] = &[
    // 0 -> 1
    "CREATE TABLE items (
        id         TEXT PRIMARY KEY,
        envelope   TEXT,
        revision   INTEGER NOT NULL DEFAULT 0,
        deleted    INTEGER NOT NULL DEFAULT 0,
        dirty      INTEGER NOT NULL DEFAULT 0,
        updated_at INTEGER NOT NULL
     );
     CREATE INDEX idx_items_dirty ON items(dirty) WHERE dirty = 1;
     CREATE TABLE local_state (
        item_id           TEXT PRIMARY KEY,
        last_connected_at INTEGER
     );
     CREATE TABLE conflict_log (
        id                INTEGER PRIMARY KEY AUTOINCREMENT,
        item_id           TEXT NOT NULL,
        resolution        TEXT NOT NULL,
        local_envelope    TEXT,
        remote_envelope   TEXT,
        local_deleted     INTEGER NOT NULL,
        remote_deleted    INTEGER NOT NULL,
        local_updated_at  INTEGER,
        remote_updated_at INTEGER,
        created_at        INTEGER NOT NULL,
        reviewed          INTEGER NOT NULL DEFAULT 0
     );",
];

const ITEM_COLUMNS: &str = "id, envelope, revision, deleted, dirty, updated_at";

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<ItemRow> {
    Ok(ItemRow {
        id: row.get(0)?,
        envelope: row.get(1)?,
        revision: from_i64(row.get(2)?),
        deleted: row.get::<_, i64>(3)? != 0,
        dirty: row.get::<_, i64>(4)? != 0,
        updated_at: row.get(5)?,
    })
}

const CONFLICT_COLUMNS: &str = "id, item_id, resolution, local_envelope, remote_envelope, local_deleted, \
     remote_deleted, local_updated_at, remote_updated_at, created_at, reviewed";

fn row_to_conflict(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConflictRow> {
    Ok(ConflictRow {
        id: row.get(0)?,
        item_id: row.get(1)?,
        resolution: row.get(2)?,
        local_envelope: row.get(3)?,
        remote_envelope: row.get(4)?,
        local_deleted: row.get::<_, i64>(5)? != 0,
        remote_deleted: row.get::<_, i64>(6)? != 0,
        local_updated_at: row.get(7)?,
        remote_updated_at: row.get(8)?,
        created_at: row.get(9)?,
        reviewed: row.get::<_, i64>(10)? != 0,
    })
}

/// Row-level operations, shared by [`Store`] and [`StoreTx`].
pub trait StoreOps {
    /// The underlying connection.
    fn conn(&self) -> &Connection;

    /// Reads a `meta` value.
    fn get_meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    /// Writes a `meta` value.
    fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Removes a `meta` value.
    fn delete_meta(&self, key: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM meta WHERE key = ?1", [key])?;
        Ok(())
    }

    /// Reads an integer `meta` value, treating absent or unparsable values as `None`.
    fn get_meta_i64(&self, key: &str) -> Result<Option<i64>> {
        Ok(self.get_meta(key)?.and_then(|v| v.parse().ok()))
    }

    /// One item row.
    fn item_row(&self, id: &str) -> Result<Option<ItemRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {ITEM_COLUMNS} FROM items WHERE id = ?1"),
                [id],
                row_to_item,
            )
            .optional()?)
    }

    /// Every item row (including tombstones), ordered by id (UUIDv7 ⇒ creation order).
    fn item_rows(&self) -> Result<Vec<ItemRow>> {
        let mut stmt = self
            .conn()
            .prepare(&format!("SELECT {ITEM_COLUMNS} FROM items ORDER BY id"))?;
        let rows = stmt
            .query_map([], row_to_item)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Rows with unpushed local changes, ordered by id.
    fn dirty_rows(&self) -> Result<Vec<ItemRow>> {
        let mut stmt = self.conn().prepare(&format!(
            "SELECT {ITEM_COLUMNS} FROM items WHERE dirty = 1 ORDER BY id"
        ))?;
        let rows = stmt
            .query_map([], row_to_item)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Number of rows with unpushed local changes.
    fn dirty_count(&self) -> Result<u64> {
        let n: i64 =
            self.conn()
                .query_row("SELECT COUNT(*) FROM items WHERE dirty = 1", [], |r| {
                    r.get(0)
                })?;
        Ok(from_i64(n))
    }

    /// Inserts or replaces a whole row.
    fn upsert_item_row(&self, row: &ItemRow) -> Result<()> {
        self.conn().execute(
            "INSERT INTO items (id, envelope, revision, deleted, dirty, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(id) DO UPDATE SET envelope = excluded.envelope, revision = excluded.revision, \
               deleted = excluded.deleted, dirty = excluded.dirty, updated_at = excluded.updated_at",
            params![row.id, row.envelope, to_i64(row.revision), row.deleted, row.dirty, row.updated_at],
        )?;
        Ok(())
    }

    /// Records that `pushed_envelope` was accepted by the server as `revision`.
    ///
    /// The revision always advances, but the dirty flag is cleared **only if the stored
    /// envelope is still exactly what was pushed**: an edit made while the push was in flight
    /// stays dirty and is sent next round. Returns whether the row is now clean.
    fn mark_pushed(&self, id: &str, revision: u64, pushed_envelope: Option<&str>) -> Result<bool> {
        self.conn().execute(
            "UPDATE items SET revision = ?2, dirty = CASE WHEN envelope IS ?3 THEN 0 ELSE dirty END \
             WHERE id = ?1",
            params![id, to_i64(revision), pushed_envelope],
        )?;
        let dirty: Option<i64> = self
            .conn()
            .query_row("SELECT dirty FROM items WHERE id = ?1", [id], |r| r.get(0))
            .optional()?;
        Ok(dirty == Some(0))
    }

    /// Sets only the revision (used when a local edit wins a conflict and is retried).
    fn set_row_revision(&self, id: &str, revision: u64) -> Result<()> {
        self.conn().execute(
            "UPDATE items SET revision = ?2 WHERE id = ?1",
            params![id, to_i64(revision)],
        )?;
        Ok(())
    }

    /// Marks every row dirty with revision 0, so the next round uploads everything
    /// (used when sync is enabled). Returns the number of rows.
    fn mark_all_dirty_for_upload(&self) -> Result<u64> {
        let n = self
            .conn()
            .execute("UPDATE items SET revision = 0, dirty = 1", [])?;
        Ok(n as u64)
    }

    /// Appends to the conflict log and prunes old entries. Returns the new id.
    fn insert_conflict(&self, c: &NewConflict) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO conflict_log (item_id, resolution, local_envelope, remote_envelope, local_deleted, \
               remote_deleted, local_updated_at, remote_updated_at, created_at, reviewed) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)",
            params![
                c.item_id,
                c.resolution,
                c.local_envelope,
                c.remote_envelope,
                c.local_deleted,
                c.remote_deleted,
                c.local_updated_at,
                c.remote_updated_at,
                c.created_at
            ],
        )?;
        let id = self.conn().last_insert_rowid();
        self.conn().execute(
            "DELETE FROM conflict_log WHERE id <= ?1",
            [id - CONFLICT_LOG_LIMIT],
        )?;
        Ok(id)
    }

    /// Conflict log entries, newest first.
    fn conflicts(&self, unreviewed_only: bool) -> Result<Vec<ConflictRow>> {
        let sql = format!(
            "SELECT {CONFLICT_COLUMNS} FROM conflict_log {} ORDER BY id DESC",
            if unreviewed_only {
                "WHERE reviewed = 0"
            } else {
                ""
            }
        );
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt
            .query_map([], row_to_conflict)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// One conflict log entry.
    fn conflict(&self, id: i64) -> Result<Option<ConflictRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {CONFLICT_COLUMNS} FROM conflict_log WHERE id = ?1"),
                [id],
                row_to_conflict,
            )
            .optional()?)
    }

    /// Number of conflicts the user has not looked at yet.
    fn unreviewed_conflict_count(&self) -> Result<u64> {
        let n: i64 = self.conn().query_row(
            "SELECT COUNT(*) FROM conflict_log WHERE reviewed = 0",
            [],
            |r| r.get(0),
        )?;
        Ok(from_i64(n))
    }

    /// Marks a conflict as reviewed. Returns whether the entry exists.
    fn mark_conflict_reviewed(&self, id: i64) -> Result<bool> {
        Ok(self
            .conn()
            .execute("UPDATE conflict_log SET reviewed = 1 WHERE id = ?1", [id])?
            > 0)
    }

    /// Device-local `last_connected_at` for an item.
    fn last_connected(&self, item_id: &str) -> Result<Option<i64>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT last_connected_at FROM local_state WHERE item_id = ?1",
                [item_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Sets the device-local `last_connected_at` (never synced).
    fn set_last_connected(&self, item_id: &str, at_ms: i64) -> Result<()> {
        self.conn().execute(
            "INSERT INTO local_state (item_id, last_connected_at) VALUES (?1, ?2) \
             ON CONFLICT(item_id) DO UPDATE SET last_connected_at = excluded.last_connected_at",
            params![item_id, at_ms],
        )?;
        Ok(())
    }
}

/// The open database.
pub struct Store {
    conn: Connection,
}

/// A transaction handle; commit happens when the closure given to [`Store::transaction`] returns `Ok`.
pub struct StoreTx<'a> {
    tx: Transaction<'a>,
}

impl StoreOps for Store {
    fn conn(&self) -> &Connection {
        &self.conn
    }
}

impl StoreOps for StoreTx<'_> {
    fn conn(&self) -> &Connection {
        &self.tx
    }
}

impl Store {
    /// Opens (creating if needed) the database at `path` and migrates it to the current schema.
    ///
    /// # Errors
    /// SQLite errors, or [`Error::UnsupportedVersion`] if the file was written by a newer build.
    pub fn open(path: &Path) -> Result<Self> {
        let existed = path.exists();
        let conn = Connection::open(path)?;
        let store = Self::init(conn)?;
        if !existed {
            restrict_permissions(path);
        }
        Ok(store)
    }

    /// An in-memory database (tests).
    ///
    /// # Errors
    /// SQLite errors.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // `journal_mode` answers with a row, so use `query_row` rather than `execute`.
        let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "secure_delete", "ON")?;
        let mut store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        let current = self.get_meta_i64(meta::SCHEMA_VERSION)?.unwrap_or(0);
        if current > i64::from(SCHEMA_VERSION) {
            return Err(Error::UnsupportedVersion(format!(
                "database schema {current}"
            )));
        }
        for (step, sql) in MIGRATIONS
            .iter()
            .enumerate()
            .skip(usize::try_from(current).unwrap_or(0))
        {
            self.transaction(|tx| {
                tx.conn().execute_batch(sql)?;
                tx.set_meta(meta::SCHEMA_VERSION, &(step + 1).to_string())
            })?;
        }
        Ok(())
    }

    /// Runs `f` in one immediate transaction: all of its writes commit together or not at all.
    ///
    /// # Errors
    /// Whatever `f` returns, or SQLite errors from begin/commit.
    pub fn transaction<T>(&mut self, f: impl FnOnce(&StoreTx<'_>) -> Result<T>) -> Result<T> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let wrapped = StoreTx { tx };
        let out = f(&wrapped)?;
        wrapped.tx.commit()?;
        Ok(out)
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        tracing::warn!(error = %e, "could not restrict database file permissions");
    }
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, env: Option<&str>, revision: u64, dirty: bool) -> ItemRow {
        ItemRow {
            id: id.into(),
            envelope: env.map(str::to_owned),
            revision,
            deleted: env.is_none(),
            dirty,
            updated_at: 1,
        }
    }

    #[test]
    fn fresh_database_is_migrated() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(
            store.get_meta(meta::SCHEMA_VERSION).unwrap().as_deref(),
            Some("1")
        );
        assert!(store.item_rows().unwrap().is_empty());
        assert!(store.conflicts(false).unwrap().is_empty());
    }

    #[test]
    fn newer_schema_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.db");
        drop(Store::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "UPDATE meta SET value = '99' WHERE key = 'schema_version'",
            [],
        )
        .unwrap();
        drop(conn);
        assert!(matches!(
            Store::open(&path),
            Err(Error::UnsupportedVersion(_))
        ));
    }

    #[test]
    fn pragmas_are_applied() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("v.db")).unwrap();
        let mode: String = store
            .conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        let secure: i64 = store
            .conn
            .query_row("PRAGMA secure_delete", [], |r| r.get(0))
            .unwrap();
        let fk: i64 = store
            .conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        assert_eq!(secure, 1);
        assert_eq!(fk, 1);
    }

    #[test]
    fn meta_round_trip() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.get_meta("k").unwrap(), None);
        store.set_meta("k", "v1").unwrap();
        store.set_meta("k", "v2").unwrap();
        assert_eq!(store.get_meta("k").unwrap().as_deref(), Some("v2"));
        store.delete_meta("k").unwrap();
        assert_eq!(store.get_meta("k").unwrap(), None);
        store.set_meta("n", "12").unwrap();
        assert_eq!(store.get_meta_i64("n").unwrap(), Some(12));
    }

    #[test]
    fn rows_upsert_and_dirty_listing() {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_item_row(&row("a", Some("{}"), 0, true))
            .unwrap();
        store
            .upsert_item_row(&row("b", Some("{}"), 3, false))
            .unwrap();
        store.upsert_item_row(&row("c", None, 2, true)).unwrap();
        assert_eq!(store.item_rows().unwrap().len(), 3);
        let dirty: Vec<_> = store
            .dirty_rows()
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(dirty, ["a", "c"]);
        assert_eq!(store.dirty_count().unwrap(), 2);
        assert!(store.item_row("c").unwrap().unwrap().deleted);
        assert_eq!(store.item_row("zzz").unwrap(), None);
        // replace
        store
            .upsert_item_row(&row("a", Some("{\"x\":1}"), 4, false))
            .unwrap();
        assert_eq!(store.item_row("a").unwrap().unwrap().revision, 4);
        assert_eq!(store.dirty_count().unwrap(), 1);
    }

    #[test]
    fn mark_pushed_clears_dirty_only_if_unchanged() {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_item_row(&row("a", Some("v1"), 0, true))
            .unwrap();
        // Pushed what is stored: clean, revision advances.
        assert!(store.mark_pushed("a", 1, Some("v1")).unwrap());
        let r = store.item_row("a").unwrap().unwrap();
        assert_eq!((r.revision, r.dirty), (1, false));

        // A local edit lands while a push of "v1" is in flight.
        store
            .upsert_item_row(&row("a", Some("v2"), 1, true))
            .unwrap();
        assert!(!store.mark_pushed("a", 2, Some("v1")).unwrap());
        let r = store.item_row("a").unwrap().unwrap();
        assert_eq!(
            (r.revision, r.dirty, r.envelope.as_deref()),
            (2, true, Some("v2"))
        );

        // Same for deletions: tombstone pushed, but the item was edited meanwhile.
        store.upsert_item_row(&row("t", None, 1, true)).unwrap();
        assert!(store.mark_pushed("t", 2, None).unwrap());
        store.upsert_item_row(&row("t", None, 2, true)).unwrap();
        assert!(!store.mark_pushed("t", 3, Some("resurrected")).unwrap());
        // Deleted while an edit was in flight stays dirty too.
        store.upsert_item_row(&row("u", None, 1, true)).unwrap();
        assert!(!store.mark_pushed("u", 2, Some("old")).unwrap());
    }

    #[test]
    fn mark_all_dirty_resets_revisions() {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_item_row(&row("a", Some("x"), 5, false))
            .unwrap();
        store.upsert_item_row(&row("b", None, 7, false)).unwrap();
        assert_eq!(store.mark_all_dirty_for_upload().unwrap(), 2);
        assert!(
            store
                .item_rows()
                .unwrap()
                .iter()
                .all(|r| r.dirty && r.revision == 0)
        );
    }

    #[test]
    fn transaction_is_atomic() {
        let mut store = Store::open_in_memory().unwrap();
        let result: Result<()> = store.transaction(|tx| {
            tx.upsert_item_row(&row("a", Some("x"), 0, true))?;
            tx.set_meta("k", "v")?;
            Err(Error::Poisoned)
        });
        assert!(result.is_err());
        assert!(store.item_rows().unwrap().is_empty());
        assert_eq!(store.get_meta("k").unwrap(), None);

        store
            .transaction(|tx| {
                tx.upsert_item_row(&row("a", Some("x"), 0, true))?;
                tx.set_meta("k", "v")
            })
            .unwrap();
        assert_eq!(store.item_rows().unwrap().len(), 1);
        assert_eq!(store.get_meta("k").unwrap().as_deref(), Some("v"));
    }

    fn conflict(item: &str, at: i64) -> NewConflict {
        NewConflict {
            item_id: item.into(),
            resolution: "remote_wins".into(),
            local_envelope: Some("l".into()),
            remote_envelope: None,
            local_deleted: false,
            remote_deleted: true,
            local_updated_at: Some(1),
            remote_updated_at: Some(2),
            created_at: at,
        }
    }

    #[test]
    fn conflict_log_crud() {
        let store = Store::open_in_memory().unwrap();
        let id1 = store.insert_conflict(&conflict("a", 10)).unwrap();
        let id2 = store.insert_conflict(&conflict("b", 20)).unwrap();
        assert!(id2 > id1);
        let all = store.conflicts(false).unwrap();
        assert_eq!(
            all.iter().map(|c| c.item_id.as_str()).collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert_eq!(store.unreviewed_conflict_count().unwrap(), 2);
        assert!(store.mark_conflict_reviewed(id1).unwrap());
        assert!(!store.mark_conflict_reviewed(9999).unwrap());
        assert_eq!(store.conflicts(true).unwrap().len(), 1);
        let c = store.conflict(id2).unwrap().unwrap();
        assert!(c.remote_deleted && !c.local_deleted && !c.reviewed);
        assert_eq!(c.remote_envelope, None);
    }

    #[test]
    fn conflict_log_is_bounded() {
        let mut store = Store::open_in_memory().unwrap();
        store
            .transaction(|tx| {
                for i in 0..(CONFLICT_LOG_LIMIT + 25) {
                    tx.insert_conflict(&conflict("x", i))?;
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(
            store.conflicts(false).unwrap().len(),
            usize::try_from(CONFLICT_LOG_LIMIT).unwrap()
        );
    }

    #[test]
    fn local_state_is_separate_from_items() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.last_connected("h").unwrap(), None);
        store.set_last_connected("h", 100).unwrap();
        store.set_last_connected("h", 200).unwrap();
        assert_eq!(store.last_connected("h").unwrap(), Some(200));
        assert!(store.item_rows().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn database_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.db");
        drop(Store::open(&path).unwrap());
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
