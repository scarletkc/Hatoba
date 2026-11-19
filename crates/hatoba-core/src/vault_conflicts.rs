//! Sync conflict log queries and loser restoration on [`Vault`].

use crate::crypto::{self, item_aad};
use crate::error::{Error, Result};
use crate::model::Item;
use crate::store::{ConflictRow, StoreOps};
use crate::sync::conflict::{ConflictEntry, Resolution};
use crate::vault::Vault;

impl Vault {
    /// Conflicts resolved automatically, newest first, with both versions decrypted (P1 UI).
    /// A message part is returned without its `data`.
    ///
    /// # Errors
    /// [`Error::Locked`], storage errors.
    pub fn conflicts(&self, unreviewed_only: bool) -> Result<Vec<ConflictEntry>> {
        let unlocked = self.unlocked.as_ref().ok_or(Error::Locked)?;
        Ok(self
            .store
            .conflicts(unreviewed_only)?
            .into_iter()
            .filter_map(|row| conflict_entry(&unlocked.vault_key, row, false))
            .collect())
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
        let unlocked = self.unlocked.as_ref().ok_or(Error::Locked)?;
        // Only here is a message part's `data` kept: restoring has to write it back.
        let entry = self
            .store
            .conflict(id)?
            .and_then(|row| conflict_entry(&unlocked.vault_key, row, true))
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

/// Decrypts both sides of a logged conflict. `None` for a row whose resolution this build does
/// not know. A side that cannot be read is `None` in the entry.
fn conflict_entry(
    key: &[u8; 32],
    row: ConflictRow,
    keep_message_data: bool,
) -> Option<ConflictEntry> {
    let resolution = Resolution::parse(&row.resolution)?;
    let decode = |envelope: &Option<String>| -> Option<Item> {
        let env = envelope.as_deref()?;
        let plain = crypto::open_json(key, item_aad(&row.item_id).as_bytes(), env).ok()?;
        let mut item = Item::from_plaintext(&plain).ok()?;
        if !keep_message_data {
            item.strip_message_data();
        }
        Some(item)
    };
    Some(ConflictEntry {
        id: row.id,
        local: decode(&row.local_envelope),
        remote: decode(&row.remote_envelope),
        item_id: row.item_id,
        resolution,
        local_deleted: row.local_deleted,
        remote_deleted: row.remote_deleted,
        local_updated_at: row.local_updated_at,
        remote_updated_at: row.remote_updated_at,
        created_at: row.created_at,
        reviewed: row.reviewed,
    })
}
