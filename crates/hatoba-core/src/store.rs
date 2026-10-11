//! The local SQLite store (spec §5.2).
//!
//! The database holds only ciphertext (plus a few non-secret counters, the opaque
//! `local_prefs`, `star_prompt` and `mcp_device_state` strings, and the ids that key the file
//! snapshots). `secure_delete` is on so freed pages do not keep old rows around.
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
    /// The bundled Worker version whose "update available" notice the user dismissed.
    pub const WORKER_UPDATE_DISMISSED: &str = "worker_update_dismissed";
    /// Consecutive failed unlock attempts (SEC-06).
    pub const UNLOCK_FAILURES: &str = "unlock_failures";
    /// Time of the last failed unlock attempt, Unix ms.
    pub const UNLOCK_LAST_FAILURE_AT: &str = "unlock_last_failure_at";
    /// Opaque plaintext JSON of device-local UI preferences.
    pub const LOCAL_PREFS: &str = "local_prefs";
    /// Opaque plaintext JSON of the device-local star prompt state.
    pub const STAR_PROMPT: &str = "star_prompt";
    /// Opaque plaintext JSON of the device-local MCP server state: whether each server is
    /// enabled on this device and what it always allows (spec §13.9). Never holds a secret.
    pub const MCP_DEVICE_STATE: &str = "mcp_device_state";
    /// Envelope JSON: `recovery_auth` sealed under the vault key, so sync setup can upload it.
    pub const RECOVERY_AUTH_SEALED: &str = "recovery_auth_sealed";
    /// Envelope JSON: a constant sealed under the vault key, to verify a candidate vault key.
    pub const VAULT_CHECK: &str = "vault_check";
    /// Envelope JSON: the shell's list of recent quick-connect targets (HOST-12), sealed under
    /// the vault key because it names hosts. Device-local, never synced.
    pub const RECENT_TARGETS: &str = "recent_targets";
    /// Envelope JSON: the shell's AI approvals saved on this device (AI-19), sealed under the
    /// vault key because they name hosts. Device-local, never synced.
    pub const AI_ALLOWANCES: &str = "ai_allowances";
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

/// Device-local columns added after their table shipped, as `(table, column, type)`.
///
/// They are added in place of a migration because `SCHEMA_VERSION` is also the synced vault's
/// schema: a new version would stop devices on older builds from joining the vault, for data
/// that never leaves this device. Older builds name the columns they read and write, so they
/// leave these alone.
const LOCAL_COLUMNS: &[(&str, &str, &str)] = &[
    ("local_state", "os", "TEXT"),
    ("local_state", "show_stats", "INTEGER"),
];

/// Device-local tables added after the schema shipped, created if missing for the same reason as
/// [`LOCAL_COLUMNS`]. Older builds never name them.
///
/// `ai_file_snapshots` keeps what each `edit_file` or `write_file` call changed, so the user can
/// revert it (spec AI-41): `meta` and `content` are envelopes sealed under the vault key, and
/// `content` is NULL once the change was reverted or dropped ([`SnapshotState`]). `size` is the
/// bytes `content` holds in plaintext, which the snapshots' total is capped by.
const LOCAL_TABLES: &[&str] = &["CREATE TABLE IF NOT EXISTS ai_file_snapshots (
        conversation_id TEXT NOT NULL,
        response_id     TEXT NOT NULL,
        tool_call_id    TEXT NOT NULL,
        meta            TEXT NOT NULL,
        content         TEXT,
        size            INTEGER NOT NULL,
        state           TEXT NOT NULL,
        created_at      INTEGER NOT NULL,
        PRIMARY KEY (conversation_id, response_id, tool_call_id)
     )"];

/// Where the change of a file snapshot stands (spec AI-41).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotState {
    /// Stored before the write began: the write runs, or it failed after it may have begun, or
    /// its end was never recorded.
    Unfinished,
    /// The call wrote the file.
    Written,
    /// The user reverted the change; the content is gone.
    Reverted,
    /// The size cap dropped the content, so the change can no longer be reverted.
    Dropped,
}

impl SnapshotState {
    /// The state as the table stores it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unfinished => "unfinished",
            Self::Written => "written",
            Self::Reverted => "reverted",
            Self::Dropped => "dropped",
        }
    }

    /// The content is kept, so the change can be reverted.
    #[must_use]
    pub fn revertible(self) -> bool {
        matches!(self, Self::Unfinished | Self::Written)
    }

    fn parse(text: &str) -> Self {
        match text {
            "unfinished" => Self::Unfinished,
            "written" => Self::Written,
            "reverted" => Self::Reverted,
            // A state a newer build wrote: nothing this one can revert.
            _ => Self::Dropped,
        }
    }
}

/// A row of the device-local `ai_file_snapshots` table (see [`LOCAL_TABLES`]).
#[derive(Clone, PartialEq, Eq)]
pub struct SnapshotRow {
    /// The conversation whose call made the change.
    pub conversation_id: String,
    /// The entry id of the response that made the call.
    pub response_id: String,
    /// The call's id.
    pub tool_call_id: String,
    /// Envelope JSON of the snapshot's description.
    pub meta: String,
    /// Envelope JSON of the file's content before and after; `None` once reverted.
    pub content: Option<String>,
    /// Plaintext bytes of `content`.
    pub size: u64,
    /// Where the change stands.
    pub state: SnapshotState,
    /// When the snapshot was stored, Unix ms.
    pub created_at: i64,
}

impl std::fmt::Debug for SnapshotRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnapshotRow")
            .field("conversation_id", &self.conversation_id)
            .field("response_id", &self.response_id)
            .field("tool_call_id", &self.tool_call_id)
            .field("size", &self.size)
            .field("state", &self.state)
            .field("created_at", &self.created_at)
            .finish_non_exhaustive()
    }
}

const SNAPSHOT_COLUMNS: &str = "conversation_id, response_id, tool_call_id, meta, content, size, \
     state, created_at";

fn row_to_snapshot(row: &rusqlite::Row<'_>) -> rusqlite::Result<SnapshotRow> {
    Ok(SnapshotRow {
        conversation_id: row.get(0)?,
        response_id: row.get(1)?,
        tool_call_id: row.get(2)?,
        meta: row.get(3)?,
        content: row.get(4)?,
        size: from_i64(row.get(5)?),
        state: SnapshotState::parse(&row.get::<_, String>(6)?),
        created_at: row.get(7)?,
    })
}

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

    /// Deletes an item row outright, leaving no tombstone.
    fn delete_item_row(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM items WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Empties the conflict log.
    fn clear_conflicts(&self) -> Result<()> {
        self.conn().execute("DELETE FROM conflict_log", [])?;
        Ok(())
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

    /// Device-local server OS id of a host (HOST-11).
    fn host_os(&self, item_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT os FROM local_state WHERE item_id = ?1",
                [item_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Sets or clears the device-local server OS id of a host (never synced).
    fn set_host_os(&self, item_id: &str, os: Option<&str>) -> Result<()> {
        self.conn().execute(
            "INSERT INTO local_state (item_id, os) VALUES (?1, ?2) \
             ON CONFLICT(item_id) DO UPDATE SET os = excluded.os",
            params![item_id, os],
        )?;
        Ok(())
    }

    /// Whether a host's terminals show the server's resource usage (TERM-12). Device-local.
    fn host_show_stats(&self, item_id: &str) -> Result<bool> {
        Ok(self
            .conn()
            .query_row(
                "SELECT show_stats FROM local_state WHERE item_id = ?1",
                [item_id],
                |r| r.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten()
            .is_some_and(|v| v != 0))
    }

    /// Turns the resource usage of a host's terminals on or off (device-local, never synced).
    fn set_host_show_stats(&self, item_id: &str, on: bool) -> Result<()> {
        self.conn().execute(
            "INSERT INTO local_state (item_id, show_stats) VALUES (?1, ?2) \
             ON CONFLICT(item_id) DO UPDATE SET show_stats = excluded.show_stats",
            params![item_id, i64::from(on)],
        )?;
        Ok(())
    }

    /// Stores a file snapshot (device-local, never synced), replacing one with the same key.
    fn put_snapshot(&self, row: &SnapshotRow) -> Result<()> {
        self.conn().execute(
            &format!(
                "INSERT OR REPLACE INTO ai_file_snapshots ({SNAPSHOT_COLUMNS}) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
            ),
            params![
                row.conversation_id,
                row.response_id,
                row.tool_call_id,
                row.meta,
                row.content,
                to_i64(row.size),
                row.state.as_str(),
                row.created_at
            ],
        )?;
        Ok(())
    }

    /// One file snapshot.
    fn snapshot(
        &self,
        conversation_id: &str,
        response_id: &str,
        tool_call_id: &str,
    ) -> Result<Option<SnapshotRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!(
                    "SELECT {SNAPSHOT_COLUMNS} FROM ai_file_snapshots \
                     WHERE conversation_id = ?1 AND response_id = ?2 AND tool_call_id = ?3"
                ),
                params![conversation_id, response_id, tool_call_id],
                row_to_snapshot,
            )
            .optional()?)
    }

    /// The file snapshots of a conversation, or of every conversation, oldest first; `content`
    /// is left out (`None`), so a list never reads the files.
    fn snapshot_list(&self, conversation_id: Option<&str>) -> Result<Vec<SnapshotRow>> {
        let mut stmt = self.conn().prepare(
            "SELECT conversation_id, response_id, tool_call_id, meta, NULL, size, state, \
             created_at FROM ai_file_snapshots WHERE ?1 IS NULL OR conversation_id = ?1 \
             ORDER BY rowid",
        )?;
        let rows = stmt
            .query_map([conversation_id], row_to_snapshot)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Marks the write of an unfinished snapshot done, replacing its `meta` when one is given.
    /// Returns whether the snapshot existed and was unfinished.
    fn set_snapshot_written(
        &self,
        conversation_id: &str,
        response_id: &str,
        tool_call_id: &str,
        meta: Option<&str>,
    ) -> Result<bool> {
        Ok(self.conn().execute(
            "UPDATE ai_file_snapshots SET state = 'written', meta = COALESCE(?4, meta) \
             WHERE conversation_id = ?1 AND response_id = ?2 AND tool_call_id = ?3 \
             AND state = 'unfinished'",
            params![conversation_id, response_id, tool_call_id, meta],
        )? > 0)
    }

    /// Marks a snapshot's change reverted: its content goes and its size becomes 0. Returns
    /// whether the snapshot existed and was not reverted yet. A dropped one is marked too: a
    /// revert that began before the size limit dropped it still put the file back.
    fn set_snapshot_reverted(
        &self,
        conversation_id: &str,
        response_id: &str,
        tool_call_id: &str,
    ) -> Result<bool> {
        Ok(self.conn().execute(
            "UPDATE ai_file_snapshots SET content = NULL, size = 0, state = 'reverted' \
             WHERE conversation_id = ?1 AND response_id = ?2 AND tool_call_id = ?3 \
             AND state <> 'reverted'",
            params![conversation_id, response_id, tool_call_id],
        )? > 0)
    }

    /// Deletes one file snapshot.
    fn delete_snapshot(
        &self,
        conversation_id: &str,
        response_id: &str,
        tool_call_id: &str,
    ) -> Result<()> {
        self.conn().execute(
            "DELETE FROM ai_file_snapshots \
             WHERE conversation_id = ?1 AND response_id = ?2 AND tool_call_id = ?3",
            params![conversation_id, response_id, tool_call_id],
        )?;
        Ok(())
    }

    /// Deletes a conversation's file snapshots, or only those of responses from `from_response`
    /// on (entry ids order them). Returns how many it deleted.
    fn delete_snapshots(&self, conversation_id: &str, from_response: Option<&str>) -> Result<u64> {
        let n = self.conn().execute(
            "DELETE FROM ai_file_snapshots \
             WHERE conversation_id = ?1 AND (?2 IS NULL OR response_id >= ?2)",
            params![conversation_id, from_response],
        )?;
        Ok(n as u64)
    }

    /// Drops the content of the oldest file snapshots that still hold some until their sizes add
    /// up to at most `max` bytes; their rows stay, as [`SnapshotState::Dropped`]. Returns how many
    /// it dropped.
    fn trim_snapshots(&self, max: u64) -> Result<u64> {
        let mut dropped = 0;
        loop {
            let total: i64 = self.conn().query_row(
                "SELECT COALESCE(SUM(size), 0) FROM ai_file_snapshots",
                [],
                |r| r.get(0),
            )?;
            if from_i64(total) <= max {
                return Ok(dropped);
            }
            let n = self.conn().execute(
                "UPDATE ai_file_snapshots SET content = NULL, size = 0, state = 'dropped' \
                 WHERE rowid = (SELECT rowid FROM ai_file_snapshots \
                 WHERE content IS NOT NULL ORDER BY rowid LIMIT 1)",
                [],
            )?;
            if n == 0 {
                return Ok(dropped);
            }
            dropped += 1;
        }
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
        // Checked inside the write lock, so a second process opening the file waits and then
        // finds the column instead of adding it twice.
        self.transaction(|tx| {
            for (table, column, kind) in LOCAL_COLUMNS {
                let present: bool = tx.conn().query_row(
                    "SELECT EXISTS (SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
                    [table, column],
                    |r| r.get(0),
                )?;
                if !present {
                    tx.conn().execute_batch(&format!(
                        "ALTER TABLE {table} ADD COLUMN {column} {kind}"
                    ))?;
                }
            }
            for sql in LOCAL_TABLES {
                tx.conn().execute_batch(sql)?;
            }
            Ok(())
        })
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
        assert_eq!(store.host_os("h").unwrap(), None);
        store.set_host_os("h", Some("debian")).unwrap();
        assert_eq!(store.host_os("h").unwrap().as_deref(), Some("debian"));
        assert_eq!(store.last_connected("h").unwrap(), Some(200));
        store.set_last_connected("h", 300).unwrap();
        assert_eq!(store.host_os("h").unwrap().as_deref(), Some("debian"));
        store.set_host_os("h", None).unwrap();
        assert_eq!(store.host_os("h").unwrap(), None);
        store.set_host_os("other", Some("ubuntu")).unwrap();
        assert_eq!(store.last_connected("other").unwrap(), None);
        assert!(!store.host_show_stats("h").unwrap());
        store.set_host_show_stats("h", true).unwrap();
        assert!(store.host_show_stats("h").unwrap());
        assert_eq!(store.last_connected("h").unwrap(), Some(300));
        store.set_host_show_stats("h", false).unwrap();
        assert!(!store.host_show_stats("h").unwrap());
        assert!(!store.host_show_stats("other").unwrap());
        assert!(!store.host_show_stats("missing").unwrap());
        assert!(store.item_rows().unwrap().is_empty());
    }

    #[test]
    fn local_columns_are_added_without_a_schema_bump() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.db");
        // A database written by a build from before the `os` column.
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO meta (key, value) VALUES ('schema_version', '1');",
        )
        .unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.execute(
            "INSERT INTO local_state (item_id, last_connected_at) VALUES ('h', 100)",
            [],
        )
        .unwrap();
        drop(conn);

        let store = Store::open(&path).unwrap();
        assert_eq!(store.last_connected("h").unwrap(), Some(100));
        assert_eq!(store.host_os("h").unwrap(), None);
        assert!(!store.host_show_stats("h").unwrap());
        store.set_host_os("h", Some("freebsd")).unwrap();
        store.set_host_show_stats("h", true).unwrap();
        assert_eq!(
            store.get_meta(meta::SCHEMA_VERSION).unwrap().as_deref(),
            Some("1")
        );
        drop(store);
        // Opening again finds the columns and keeps their values.
        let store = Store::open(&path).unwrap();
        assert_eq!(store.host_os("h").unwrap().as_deref(), Some("freebsd"));
        assert!(store.host_show_stats("h").unwrap());
        // The local tables are there too, and opening again keeps their rows.
        store.put_snapshot(&snapshot("c", "r", "t", 5)).unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.snapshot_list(None).unwrap().len(), 1);
    }

    fn snapshot(conversation: &str, response: &str, call: &str, size: u64) -> SnapshotRow {
        SnapshotRow {
            conversation_id: conversation.into(),
            response_id: response.into(),
            tool_call_id: call.into(),
            meta: "m".into(),
            content: Some("c".into()),
            size,
            state: SnapshotState::Written,
            created_at: 1,
        }
    }

    #[test]
    fn file_snapshots_are_capped_oldest_first() {
        let store = Store::open_in_memory().unwrap();
        store.put_snapshot(&snapshot("c1", "r1", "a", 40)).unwrap();
        store.put_snapshot(&snapshot("c2", "r1", "a", 30)).unwrap();
        store.put_snapshot(&snapshot("c1", "r2", "b", 20)).unwrap();
        let keys = |store: &Store| {
            store
                .snapshot_list(None)
                .unwrap()
                .into_iter()
                .map(|r| format!("{}/{}/{}", r.conversation_id, r.response_id, r.tool_call_id))
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(&store), ["c1/r1/a", "c2/r1/a", "c1/r2/b"]);
        // A list leaves the content out; one snapshot has it.
        assert!(
            store.snapshot_list(Some("c1")).unwrap()[0]
                .content
                .is_none()
        );
        assert_eq!(
            store
                .snapshot("c1", "r1", "a")
                .unwrap()
                .unwrap()
                .content
                .as_deref(),
            Some("c")
        );

        // A reverted change keeps its row without content, and is reverted once.
        assert!(store.set_snapshot_reverted("c1", "r1", "a").unwrap());
        assert!(!store.set_snapshot_reverted("c1", "r1", "a").unwrap());
        let reverted = store.snapshot("c1", "r1", "a").unwrap().unwrap();
        assert_eq!(
            (reverted.content, reverted.size, reverted.state),
            (None, 0, SnapshotState::Reverted)
        );
        // Only an unfinished write is marked written, with its new description.
        assert!(
            !store
                .set_snapshot_written("c1", "r2", "b", Some("n"))
                .unwrap()
        );
        store
            .put_snapshot(&SnapshotRow {
                state: SnapshotState::Unfinished,
                ..snapshot("c1", "r2", "b", 20)
            })
            .unwrap();
        assert!(
            store
                .set_snapshot_written("c1", "r2", "b", Some("n"))
                .unwrap()
        );
        let written = store.snapshot("c1", "r2", "b").unwrap().unwrap();
        assert_eq!(
            (written.state, written.meta.as_str()),
            (SnapshotState::Written, "n")
        );

        // Over the cap, the oldest with content are dropped first, keeping their rows; reverted
        // ones are left alone.
        let states = |store: &Store| {
            store
                .snapshot_list(None)
                .unwrap()
                .into_iter()
                .map(|r| (r.state, r.size))
                .collect::<Vec<_>>()
        };
        assert_eq!(store.trim_snapshots(25).unwrap(), 1);
        assert_eq!(keys(&store), ["c1/r1/a", "c2/r1/a", "c1/r2/b"]);
        assert_eq!(
            states(&store),
            [
                (SnapshotState::Reverted, 0),
                (SnapshotState::Dropped, 0),
                (SnapshotState::Written, 20)
            ]
        );
        assert!(
            store
                .snapshot("c2", "r1", "a")
                .unwrap()
                .unwrap()
                .content
                .is_none()
        );
        // A revert that began before the limit dropped it still put the file back.
        assert!(store.set_snapshot_reverted("c2", "r1", "a").unwrap());
        assert!(!store.set_snapshot_reverted("c2", "r1", "a").unwrap());
        assert_eq!(store.trim_snapshots(0).unwrap(), 1);
        assert_eq!(store.trim_snapshots(0).unwrap(), 0);
        store.delete_snapshot("c2", "r1", "a").unwrap();
        store.delete_snapshot("c1", "r2", "b").unwrap();
        assert_eq!(keys(&store), ["c1/r1/a"]);

        // Deleting a conversation's, or those from a response on.
        store.put_snapshot(&snapshot("c1", "r3", "c", 1)).unwrap();
        store.put_snapshot(&snapshot("c2", "r1", "a", 1)).unwrap();
        assert_eq!(store.delete_snapshots("c1", Some("r2")).unwrap(), 1);
        assert_eq!(keys(&store), ["c1/r1/a", "c2/r1/a"]);
        store.delete_snapshot("c2", "r1", "a").unwrap();
        assert_eq!(store.delete_snapshots("c1", None).unwrap(), 1);
        assert!(keys(&store).is_empty());
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
