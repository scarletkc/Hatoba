//! Snapshots of what the AI assistant's file tools changed, kept so the user can revert a change
//! (spec AI-41, §5.2).
//!
//! A snapshot belongs to one `edit_file` or `write_file` call: its conversation, the response
//! that made the call, and the call's id. Its description and the file's content are opaque
//! strings to this module, each sealed under the vault key with an AAD that names the row and the
//! column. Snapshots stay on this device: they are not items, so they never sync, and they are
//! not part of the conversation's entries, so they never reach the model.
//!
//! A snapshot is stored before its write begins, as [`SnapshotState::Unfinished`], so a write
//! that fails midway, or that the app never sees end, can still be reverted.

use std::collections::HashSet;

use zeroize::Zeroizing;

use crate::crypto::{self, AAD_AI_FILE_SNAPSHOT_PREFIX, seal};
use crate::error::{Error, Result};
use crate::model::Item;
use crate::store::{SnapshotRow, SnapshotState, StoreOps};
use crate::vault::Vault;

/// The most bytes of content the snapshots hold together; storing one drops the content of the
/// oldest until they fit.
pub const AI_SNAPSHOTS_MAX_BYTES: u64 = 32 * 1024 * 1024;

/// Which call a snapshot belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotKey<'a> {
    /// The conversation.
    pub conversation_id: &'a str,
    /// The entry id of the response that made the call.
    pub response_id: &'a str,
    /// The call's id.
    pub tool_call_id: &'a str,
}

/// A snapshot as listed: its call and its description, without the content.
#[derive(Clone, PartialEq, Eq)]
pub struct FileSnapshot {
    /// The entry id of the response that made the call.
    pub response_id: String,
    /// The call's id.
    pub tool_call_id: String,
    /// The description the caller stored, opened.
    pub meta: Zeroizing<String>,
    /// Where the change stands.
    pub state: SnapshotState,
}

impl std::fmt::Debug for FileSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileSnapshot")
            .field("response_id", &self.response_id)
            .field("tool_call_id", &self.tool_call_id)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// One snapshot, opened: its description, and its content while the change can be reverted.
pub struct StoredSnapshot {
    /// The description the caller stored.
    pub meta: Zeroizing<String>,
    /// The file's content the caller stored; `None` once reverted or dropped.
    pub content: Option<Zeroizing<String>>,
    /// Where the change stands.
    pub state: SnapshotState,
}

impl std::fmt::Debug for StoredSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredSnapshot")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

fn aad(part: &str, key: &SnapshotKey<'_>) -> String {
    let ids = serde_json::to_string(&[key.conversation_id, key.response_id, key.tool_call_id])
        .unwrap_or_default();
    format!("{AAD_AI_FILE_SNAPSHOT_PREFIX}{part}/{ids}")
}

fn key_of(row: &SnapshotRow) -> SnapshotKey<'_> {
    SnapshotKey {
        conversation_id: &row.conversation_id,
        response_id: &row.response_id,
        tool_call_id: &row.tool_call_id,
    }
}

fn open(
    vault_key: &[u8; 32],
    part: &str,
    key: &SnapshotKey<'_>,
    sealed: &str,
) -> Option<Zeroizing<String>> {
    let plain = crypto::open_json(vault_key, aad(part, key).as_bytes(), sealed).ok()?;
    String::from_utf8(plain.to_vec()).ok().map(Zeroizing::new)
}

/// Seals every file snapshot again under `new` (the vault joined another vault, flow C). A
/// snapshot `old` cannot open is deleted.
pub(crate) fn reseal_snapshots(tx: &impl StoreOps, old: &[u8; 32], new: &[u8; 32]) -> Result<()> {
    for listed in tx.snapshot_list(None)? {
        let Some(row) = tx.snapshot(
            &listed.conversation_id,
            &listed.response_id,
            &listed.tool_call_id,
        )?
        else {
            continue;
        };
        let key = key_of(&row);
        let reseal = |part: &str, sealed: &str| -> Result<Option<String>> {
            let Some(plain) = open(old, part, &key, sealed) else {
                return Ok(None);
            };
            Ok(Some(
                seal(new, aad(part, &key).as_bytes(), plain.as_bytes())?.to_json(),
            ))
        };
        let meta = reseal("meta", &row.meta)?;
        let content = match &row.content {
            Some(sealed) => reseal("content", sealed)?.map(Some),
            None => Some(None),
        };
        match (meta, content) {
            (Some(meta), Some(content)) => tx.put_snapshot(&SnapshotRow {
                meta,
                content,
                ..row.clone()
            })?,
            _ => tx.delete_snapshot(&row.conversation_id, &row.response_id, &row.tool_call_id)?,
        }
    }
    Ok(())
}

impl Vault {
    /// Stores the snapshot of a call's change before its write begins, as
    /// [`SnapshotState::Unfinished`], replacing one the call has already, then drops the content
    /// of the oldest snapshots that still hold some until all of them hold at most
    /// [`AI_SNAPSHOTS_MAX_BYTES`]. Returns how many it dropped, of any conversation.
    ///
    /// # Errors
    /// [`Error::Locked`]; [`Error::ItemNotFound`] if the conversation does not exist; storage
    /// errors.
    pub fn ai_put_file_snapshot(
        &mut self,
        key: &SnapshotKey<'_>,
        meta: &str,
        content: &str,
    ) -> Result<u64> {
        let unlocked = self.unlocked.as_ref().ok_or(Error::Locked)?;
        if !matches!(self.get(key.conversation_id), Some(Item::AiConversation(_))) {
            return Err(Error::ItemNotFound(key.conversation_id.to_owned()));
        }
        let row = SnapshotRow {
            conversation_id: key.conversation_id.to_owned(),
            response_id: key.response_id.to_owned(),
            tool_call_id: key.tool_call_id.to_owned(),
            meta: seal(
                &unlocked.vault_key,
                aad("meta", key).as_bytes(),
                meta.as_bytes(),
            )?
            .to_json(),
            content: Some(
                seal(
                    &unlocked.vault_key,
                    aad("content", key).as_bytes(),
                    content.as_bytes(),
                )?
                .to_json(),
            ),
            size: content.len() as u64,
            state: SnapshotState::Unfinished,
            created_at: self.clock.now_ms(),
        };
        self.store.transaction(|tx| {
            tx.put_snapshot(&row)?;
            tx.trim_snapshots(AI_SNAPSHOTS_MAX_BYTES)
        })
    }

    /// The snapshots of a conversation, oldest first, with their descriptions opened. One this
    /// vault key cannot open is left out.
    ///
    /// # Errors
    /// [`Error::Locked`]; storage errors.
    pub fn ai_file_snapshots(&self, conversation_id: &str) -> Result<Vec<FileSnapshot>> {
        let unlocked = self.unlocked.as_ref().ok_or(Error::Locked)?;
        Ok(self
            .store
            .snapshot_list(Some(conversation_id))?
            .into_iter()
            .filter_map(|row| {
                let meta = open(&unlocked.vault_key, "meta", &key_of(&row), &row.meta)?;
                Some(FileSnapshot {
                    response_id: row.response_id,
                    tool_call_id: row.tool_call_id,
                    meta,
                    state: row.state,
                })
            })
            .collect())
    }

    /// A call's snapshot, opened. `None` when there is none, or this vault key cannot open it.
    ///
    /// # Errors
    /// [`Error::Locked`]; storage errors.
    pub fn ai_file_snapshot(&self, key: &SnapshotKey<'_>) -> Result<Option<StoredSnapshot>> {
        let unlocked = self.unlocked.as_ref().ok_or(Error::Locked)?;
        let Some(row) =
            self.store
                .snapshot(key.conversation_id, key.response_id, key.tool_call_id)?
        else {
            return Ok(None);
        };
        let Some(meta) = open(&unlocked.vault_key, "meta", key, &row.meta) else {
            return Ok(None);
        };
        let content = match &row.content {
            Some(sealed) => match open(&unlocked.vault_key, "content", key, sealed) {
                Some(content) => Some(content),
                None => return Ok(None),
            },
            None => None,
        };
        Ok(Some(StoredSnapshot {
            meta,
            content,
            state: row.state,
        }))
    }

    /// Records that the write of a call's change ended and wrote the file, with the description
    /// `meta` when it changed (the write resolved the path, say). Returns whether the snapshot
    /// was there and unfinished.
    ///
    /// # Errors
    /// [`Error::Locked`]; storage errors.
    pub fn ai_mark_file_written(
        &mut self,
        key: &SnapshotKey<'_>,
        meta: Option<&str>,
    ) -> Result<bool> {
        let unlocked = self.unlocked.as_ref().ok_or(Error::Locked)?;
        let meta = match meta {
            Some(meta) => Some(
                seal(
                    &unlocked.vault_key,
                    aad("meta", key).as_bytes(),
                    meta.as_bytes(),
                )?
                .to_json(),
            ),
            None => None,
        };
        self.store.set_snapshot_written(
            key.conversation_id,
            key.response_id,
            key.tool_call_id,
            meta.as_deref(),
        )
    }

    /// Deletes a call's snapshot: its write never began.
    ///
    /// # Errors
    /// [`Error::Locked`]; storage errors.
    pub fn ai_delete_file_snapshot(&mut self, key: &SnapshotKey<'_>) -> Result<()> {
        if !self.is_unlocked() {
            return Err(Error::Locked);
        }
        self.store
            .delete_snapshot(key.conversation_id, key.response_id, key.tool_call_id)
    }

    /// Records that a call's change was reverted: the snapshot keeps its description and drops
    /// the content. Returns whether it was there and not reverted yet; one the size limit dropped
    /// while its revert ran is marked too.
    ///
    /// # Errors
    /// [`Error::Locked`]; storage errors.
    pub fn ai_mark_file_reverted(&mut self, key: &SnapshotKey<'_>) -> Result<bool> {
        if !self.is_unlocked() {
            return Err(Error::Locked);
        }
        self.store
            .set_snapshot_reverted(key.conversation_id, key.response_id, key.tool_call_id)
    }

    /// Deletes the snapshots whose conversation or response is gone, as when another device
    /// deleted the conversation or edited a message before the call (AI-26), and those this vault
    /// key cannot open. Returns how many it deleted.
    ///
    /// # Errors
    /// [`Error::Locked`]; storage errors.
    pub fn ai_sweep_file_snapshots(&mut self) -> Result<usize> {
        let unlocked = self.unlocked.as_ref().ok_or(Error::Locked)?;
        let rows = self.store.snapshot_list(None)?;
        if rows.is_empty() {
            return Ok(0);
        }
        let mut responses: HashSet<(&str, &str)> = HashSet::new();
        for (_, item) in self.items() {
            if let Item::AiMessage(m) = item {
                responses.insert((m.conversation_id.as_str(), m.entry_id.as_str()));
            }
        }
        let gone: Vec<&SnapshotRow> = rows
            .iter()
            .filter(|row| {
                !matches!(
                    self.get(&row.conversation_id),
                    Some(Item::AiConversation(_))
                ) || !responses.contains(&(row.conversation_id.as_str(), row.response_id.as_str()))
                    || open(&unlocked.vault_key, "meta", &key_of(row), &row.meta).is_none()
            })
            .collect();
        if gone.is_empty() {
            return Ok(0);
        }
        let count = gone.len();
        self.store.transaction(|tx| {
            for row in gone {
                tx.delete_snapshot(&row.conversation_id, &row.response_id, &row.tool_call_id)?;
            }
            Ok(())
        })?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AiConversation;

    fn vault() -> Vault {
        crate::vault_ai::tests::vault().0
    }

    fn conversation(vault: &mut Vault) -> String {
        vault
            .put(
                None,
                Item::AiConversation(AiConversation {
                    title: "t".into(),
                    host_id: None,
                    quick_target: None,
                    origin: None,
                    local: None,
                    pinned: false,
                    context_start: None,
                    effort: None,
                    created_at: 1,
                    updated_at: 0,
                }),
            )
            .unwrap()
    }

    fn key<'a>(conversation_id: &'a str, response_id: &'a str, call: &'a str) -> SnapshotKey<'a> {
        SnapshotKey {
            conversation_id,
            response_id,
            tool_call_id: call,
        }
    }

    #[test]
    fn snapshots_are_sealed_and_kept_on_the_device() {
        let mut vault = vault();
        let c = conversation(&mut vault);
        let response = vault
            .ai_append_entry(&c, r#"{"role":"assistant"}"#)
            .unwrap();
        let k = key(&c, &response, "call_1");
        let pending = vault.pending_count();
        vault.ai_put_file_snapshot(&k, "META", "CONTENT").unwrap();

        // Nothing readable in the database, and nothing to sync.
        let row = vault
            .store
            .snapshot(&c, &response, "call_1")
            .unwrap()
            .unwrap();
        assert!(!row.meta.contains("META") && !row.content.unwrap().contains("CONTENT"));
        assert_eq!(row.size, 7);
        assert_eq!(vault.pending_count(), pending);

        // Stored before the write begins, so unfinished until the write ends.
        let listed = vault.ai_file_snapshots(&c).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            (
                listed[0].tool_call_id.as_str(),
                listed[0].meta.as_str(),
                listed[0].state
            ),
            ("call_1", "META", SnapshotState::Unfinished)
        );
        let stored = vault.ai_file_snapshot(&k).unwrap().unwrap();
        assert_eq!(
            (
                stored.meta.as_str(),
                stored.content.as_deref().map(String::as_str),
                stored.state
            ),
            ("META", Some("CONTENT"), SnapshotState::Unfinished)
        );

        // Written, with the path as the write resolved it: the new description is sealed too.
        assert!(vault.ai_mark_file_written(&k, Some("RESOLVED")).unwrap());
        assert!(!vault.ai_mark_file_written(&k, None).unwrap());
        let row = vault
            .store
            .snapshot(&c, &response, "call_1")
            .unwrap()
            .unwrap();
        assert!(!row.meta.contains("RESOLVED"));
        let stored = vault.ai_file_snapshot(&k).unwrap().unwrap();
        assert_eq!(
            (stored.meta.as_str(), stored.state),
            ("RESOLVED", SnapshotState::Written)
        );

        // An envelope opens only in its own row.
        let mut moved = vault
            .store
            .snapshot(&c, &response, "call_1")
            .unwrap()
            .unwrap();
        moved.tool_call_id = "call_2".into();
        vault.store.put_snapshot(&moved).unwrap();
        assert!(
            vault
                .ai_file_snapshot(&key(&c, &response, "call_2"))
                .unwrap()
                .is_none()
        );

        // Reverted: the description stays, the content goes.
        assert!(vault.ai_mark_file_reverted(&k).unwrap());
        assert!(!vault.ai_mark_file_reverted(&k).unwrap());
        let stored = vault.ai_file_snapshot(&k).unwrap().unwrap();
        assert_eq!(
            (stored.content.is_none(), stored.state),
            (true, SnapshotState::Reverted)
        );
        assert_eq!(
            vault.ai_file_snapshots(&c).unwrap()[0].state,
            SnapshotState::Reverted
        );

        // An unfinished write can be reverted too; one that never began goes.
        let other = key(&c, &response, "call_3");
        vault.ai_put_file_snapshot(&other, "m", "c").unwrap();
        assert!(vault.ai_mark_file_reverted(&other).unwrap());
        vault.ai_put_file_snapshot(&other, "m", "c").unwrap();
        vault.ai_delete_file_snapshot(&other).unwrap();
        assert!(vault.ai_file_snapshot(&other).unwrap().is_none());

        vault.lock();
        assert!(matches!(vault.ai_file_snapshots(&c), Err(Error::Locked)));
        assert!(matches!(
            vault.ai_put_file_snapshot(&k, "m", "c"),
            Err(Error::Locked)
        ));
        assert!(matches!(
            vault.ai_mark_file_written(&k, None),
            Err(Error::Locked)
        ));
        assert!(matches!(
            vault.ai_delete_file_snapshot(&k),
            Err(Error::Locked)
        ));
    }

    #[test]
    fn the_cap_drops_the_oldest_content_and_keeps_the_rows() {
        let mut vault = vault();
        let c = conversation(&mut vault);
        let response = vault
            .ai_append_entry(&c, r#"{"role":"assistant"}"#)
            .unwrap();
        let half = "x".repeat(usize::try_from(AI_SNAPSHOTS_MAX_BYTES / 2).unwrap());
        // Each put says how many it dropped, so the panel reads other conversations' changes
        // again only then.
        let drops: Vec<u64> = ["a", "b", "c"]
            .into_iter()
            .map(|call| {
                vault
                    .ai_put_file_snapshot(&key(&c, &response, call), call, &half)
                    .unwrap()
            })
            .collect();
        assert_eq!(drops, [0, 0, 1]);
        let states: Vec<(String, SnapshotState)> = vault
            .ai_file_snapshots(&c)
            .unwrap()
            .into_iter()
            .map(|s| (s.meta.to_string(), s.state))
            .collect();
        assert_eq!(
            states,
            [
                ("a".into(), SnapshotState::Dropped),
                ("b".into(), SnapshotState::Unfinished),
                ("c".into(), SnapshotState::Unfinished)
            ]
        );
        let dropped = key(&c, &response, "a");
        assert!(
            vault
                .ai_file_snapshot(&dropped)
                .unwrap()
                .unwrap()
                .content
                .is_none()
        );
        assert!(!vault.ai_mark_file_written(&dropped, None).unwrap());
        // A revert that began before the limit dropped it still put the file back.
        assert!(vault.ai_mark_file_reverted(&dropped).unwrap());
        assert_eq!(
            vault.ai_file_snapshot(&dropped).unwrap().unwrap().state,
            SnapshotState::Reverted
        );
    }

    #[test]
    fn snapshots_go_with_their_conversation_and_response() {
        let mut vault = vault();
        let c = conversation(&mut vault);
        assert!(matches!(
            vault.ai_put_file_snapshot(&key("missing", "r", "x"), "m", "c"),
            Err(Error::ItemNotFound(_))
        ));
        let first = vault
            .ai_append_entry(&c, r#"{"role":"assistant"}"#)
            .unwrap();
        let user = vault.ai_append_entry(&c, r#"{"role":"user"}"#).unwrap();
        let second = vault
            .ai_append_entry(&c, r#"{"role":"assistant"}"#)
            .unwrap();
        vault
            .ai_put_file_snapshot(&key(&c, &first, "a"), "1", "x")
            .unwrap();
        vault
            .ai_put_file_snapshot(&key(&c, &second, "b"), "2", "y")
            .unwrap();

        // An edit of the user's message deletes the snapshots of the responses after it.
        vault.ai_delete_entries_from(&c, &user).unwrap();
        let left: Vec<String> = vault
            .ai_file_snapshots(&c)
            .unwrap()
            .into_iter()
            .map(|s| s.tool_call_id)
            .collect();
        assert_eq!(left, ["a"]);

        // A response deleted elsewhere is swept, and so is an unreadable row.
        let other = conversation(&mut vault);
        let kept = vault
            .ai_append_entry(&other, r#"{"role":"assistant"}"#)
            .unwrap();
        vault
            .ai_put_file_snapshot(&key(&other, &kept, "k"), "k", "k")
            .unwrap();
        vault
            .ai_put_file_snapshot(&key(&other, "gone", "g"), "g", "g")
            .unwrap();
        let mut bad = vault.store.snapshot(&other, &kept, "k").unwrap().unwrap();
        bad.tool_call_id = "bad".into();
        vault.store.put_snapshot(&bad).unwrap();
        assert_eq!(vault.ai_sweep_file_snapshots().unwrap(), 2);
        assert_eq!(vault.ai_file_snapshots(&other).unwrap().len(), 1);
        assert_eq!(vault.ai_sweep_file_snapshots().unwrap(), 0);

        // Deleting the conversation deletes the rest.
        vault.ai_delete_conversation(&c).unwrap();
        assert!(vault.store.snapshot_list(Some(&c)).unwrap().is_empty());
        assert_eq!(vault.ai_file_snapshots(&other).unwrap().len(), 1);
    }
}
