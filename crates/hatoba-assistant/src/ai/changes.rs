//! The changes a conversation's file tool calls made (AI-41): the list and the diffs the panel
//! shows, reverting a change, and the snapshots that keep them.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use hatoba_core::sync::SharedVault;
use hatoba_core::{SnapshotKey, SnapshotState, Vault};

use super::entries::find_conversation;
use super::job::live_session;
use super::{AiEnv, AiManager, Inner, files, guard, unlocked};
use crate::dto::{AiFileChange, AiFileChangeState, AiFilePreview, AiFileRevert};
use crate::error::{AppError, AppResult, ErrorCode};

impl AiManager {
    /// The changes of the conversation's `edit_file` and `write_file` calls that this device
    /// keeps (AI-41), for the panel to offer Revert after a call ran.
    pub fn file_changes(
        &self,
        vault: &SharedVault,
        conversation_id: &str,
    ) -> AppResult<Vec<AiFileChange>> {
        let v = unlocked(vault)?;
        find_conversation(&v, conversation_id)?;
        file_changes(&v, conversation_id)
    }

    /// What the call of `key` changed, from the snapshot this device keeps (AI-41), as the panel
    /// diffs it. `None` once the change was reverted or dropped.
    pub fn file_change_diff(
        &self,
        vault: &SharedVault,
        key: SnapshotKey<'_>,
    ) -> AppResult<Option<AiFilePreview>> {
        let v = unlocked(vault)?;
        find_conversation(&v, key.conversation_id)?;
        let Some(stored) = v.ai_file_snapshot(&key)? else {
            return Err(AppError::not_found("file change"));
        };
        let Some(content) = stored.content else {
            return Ok(None);
        };
        let unreadable = |_| AppError::internal("unreadable file snapshot");
        let change = files::Change {
            meta: serde_json::from_str(&stored.meta).map_err(unreadable)?,
            content: serde_json::from_str(&content).map_err(unreadable)?,
        };
        Ok(Some(files::change_preview(&change)))
    }

    /// Returns once no `edit_file` or `write_file` call runs on the SSH session: a write that has
    /// begun runs to its end, within 30 seconds, so a lock closes the session after it (§13.4,
    /// "File tools").
    pub async fn file_writes_ended(&self, session_id: &str) {
        self.0.writes.ended(session_id).await;
    }

    /// `ai_file_revert` (AI-41): reverts the change the call of `key` made, on the conversation's
    /// tab `session_id`, while the conversation is idle. With `restore` (Restore anyway), it
    /// writes over what the diff of the last revert showed. The read is an op of the
    /// conversation, which the lock ends; a write that has begun runs to its end, and a lock that
    /// disconnects SSH sessions waits for it. A revert ends the model's record of the file, and
    /// the next message notes it.
    pub async fn file_revert(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        key: SnapshotKey<'_>,
        session_id: Option<&str>,
        restore: bool,
    ) -> AppResult<AiFileRevert> {
        let conversation_id = key.conversation_id;
        let base_key = (
            conversation_id.to_owned(),
            key.response_id.to_owned(),
            key.tool_call_id.to_owned(),
        );
        let (change, later_writes, op, _running) = {
            let v = unlocked(vault)?;
            find_conversation(&v, conversation_id)?;
            if self.busy(conversation_id) {
                return Err(AppError::invalid(
                    "conversation_id",
                    "The conversation is busy; revert the change once it is idle.",
                ));
            }
            let Some(stored) = v.ai_file_snapshot(&key)? else {
                return Err(AppError::not_found("file change"));
            };
            let content = match (stored.state, stored.content) {
                (SnapshotState::Reverted, _) => {
                    return Err(AppError::invalid(
                        "tool_call_id",
                        "the change was reverted already",
                    ));
                }
                (state, Some(content)) if state.revertible() => content,
                _ => {
                    return Err(AppError::invalid(
                        "tool_call_id",
                        "the change can no longer be reverted: the snapshot limit dropped it",
                    ));
                }
            };
            let unreadable = |_| AppError::internal("unreadable file snapshot");
            let change = files::Change {
                meta: serde_json::from_str(&stored.meta).map_err(unreadable)?,
                content: serde_json::from_str(&content).map_err(unreadable)?,
            };
            let later = later_writes(&v, &key, &change.meta);
            // Counted under the guard a lock takes first, so the lock sees the write.
            let writing = session_id.map(|id| self.0.writes.begin(id));
            // Under the guard a turn takes to start too: none starts until the revert ends.
            let reverting = Reverting::new(&self.0, conversation_id);
            (
                change,
                later,
                self.start_op(conversation_id),
                (writing, reverting),
            )
        };
        let shown = if restore {
            guard(&self.0.restore_bases).get(&base_key).copied()
        } else {
            None
        };
        let Some(files::Revert { mut outcome, found }) =
            files::revert(live_session(env, session_id), &change, shown, op.token()).await
        else {
            return Err(AppError::new(
                ErrorCode::Cancelled,
                "the revert was stopped",
            ));
        };
        match (&mut outcome, found) {
            (AiFileRevert::Reverted { wrote }, _) => {
                guard(&self.0.restore_bases).remove(&base_key);
                // The file changed on the host either way, so the model reads it again.
                self.0
                    .seen
                    .forget_file(conversation_id, &change.meta.server, &change.meta.path);
                // Success is reported only once recorded, so the next message notes it. Reverting
                // again finds the file as it held before the call, and records it then.
                if let Err(e) = unlocked(vault).and_then(|mut v| Ok(v.ai_mark_file_reverted(&key)?))
                {
                    tracing::warn!(
                        conversation_id,
                        code = ?e.code,
                        "could not record a reverted file change"
                    );
                    return Err(e);
                }
                tracing::info!(conversation_id, wrote = *wrote, "AI file change reverted");
            }
            (AiFileRevert::Changed { later, .. }, Some(found)) => {
                *later = later_writes;
                guard(&self.0.restore_bases).insert(base_key, found);
            }
            _ => {}
        }
        Ok(outcome)
    }

    /// Refuses to start a turn while a revert of the conversation runs (AI-41).
    pub(super) fn not_reverting(&self, conversation_id: &str) -> AppResult<()> {
        if guard(&self.0.reverts).contains(conversation_id) {
            return Err(AppError::invalid(
                "conversation_id",
                "A file change of this conversation is being reverted; send the message once \
                 the revert has ended.",
            ));
        }
        Ok(())
    }
}

/// The changes of a conversation's file tool calls this device keeps (AI-41).
pub(super) fn file_changes(v: &Vault, conversation_id: &str) -> AppResult<Vec<AiFileChange>> {
    Ok(v.ai_file_snapshots(conversation_id)?
        .into_iter()
        .filter_map(|s| {
            let meta: files::ChangeMeta = serde_json::from_str(&s.meta).ok()?;
            Some(AiFileChange {
                response_id: s.response_id,
                tool_call_id: s.tool_call_id,
                server: meta.server,
                path: meta.path,
                created: meta.created,
                state: match s.state {
                    SnapshotState::Unfinished => AiFileChangeState::Unfinished,
                    SnapshotState::Written => AiFileChangeState::Written,
                    SnapshotState::Reverted => AiFileChangeState::Reverted,
                    SnapshotState::Dropped => AiFileChangeState::Dropped,
                },
            })
        })
        .collect())
}

/// A revert of a conversation's file change runs (AI-41) until this drops.
pub(super) struct Reverting<'a>(&'a Inner, String);

impl<'a> Reverting<'a> {
    pub(super) fn new(inner: &'a Inner, conversation_id: &str) -> Self {
        guard(&inner.reverts).insert(conversation_id.to_owned());
        Self(inner, conversation_id.to_owned())
    }
}

impl Drop for Reverting<'_> {
    fn drop(&mut self) {
        guard(&self.0.reverts).remove(&self.1);
    }
}

/// Whether a call of the conversation after the one of `key` wrote the same file, and its change
/// can still be reverted (AI-41): reverting it first, newest first, may bring the file back to
/// what this call wrote.
pub(super) fn later_writes(v: &Vault, key: &SnapshotKey<'_>, meta: &files::ChangeMeta) -> bool {
    let Ok(snapshots) = v.ai_file_snapshots(key.conversation_id) else {
        return false;
    };
    snapshots
        .iter()
        .skip_while(|s| s.response_id != key.response_id || s.tool_call_id != key.tool_call_id)
        .skip(1)
        .filter(|s| s.state.revertible())
        .filter_map(|s| serde_json::from_str::<files::ChangeMeta>(&s.meta).ok())
        .any(|later| later.server == meta.server && later.path == meta.path)
}

/// Keeps the snapshot of the change an `edit_file` or `write_file` call makes, in the vault,
/// before its write begins (AI-41).
pub(super) struct KeepSnapshot<'a> {
    vault: &'a SharedVault,
    key: SnapshotKey<'a>,
    /// What the kept snapshot says, until the write resolved the path.
    meta: Mutex<Option<files::ChangeMeta>>,
    /// Keeping it dropped older snapshots over the size limit, of any conversation.
    dropped: AtomicBool,
}

impl<'a> KeepSnapshot<'a> {
    pub(super) fn new(vault: &'a SharedVault, key: SnapshotKey<'a>) -> Self {
        Self {
            vault,
            key,
            meta: Mutex::default(),
            dropped: AtomicBool::new(false),
        }
    }

    pub(super) fn dropped(&self) -> bool {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl files::Keep for KeepSnapshot<'_> {
    fn keep(&self, change: &files::Change) -> Result<(), files::Stop> {
        let mut v = unlocked(self.vault).map_err(|_| files::Stop::Cancelled)?;
        let Some((meta, content)) = change.encode() else {
            return Ok(());
        };
        // Any other failure is logged, and the call writes without a snapshot.
        match v.ai_put_file_snapshot(&self.key, &meta, &content) {
            Ok(dropped) => {
                *guard(&self.meta) = Some(change.meta.clone());
                self.dropped.store(dropped > 0, Ordering::Relaxed);
            }
            Err(hatoba_core::Error::Locked) => return Err(files::Stop::Cancelled),
            Err(e) => tracing::warn!(
                conversation_id = self.key.conversation_id,
                "could not store a file snapshot: {e}"
            ),
        }
        Ok(())
    }

    fn ended(&self, resolved: Option<&str>) {
        let Some(mut meta) = guard(&self.meta).take() else {
            return;
        };
        // Locked meanwhile: the snapshot stays unfinished, and can be reverted as such.
        let Ok(mut v) = unlocked(self.vault) else {
            return;
        };
        let result = match resolved {
            Some(path) if path != meta.path => {
                meta.path = path.to_owned();
                let meta = serde_json::to_string(&meta).ok();
                v.ai_mark_file_written(&self.key, meta.as_deref()).map(drop)
            }
            Some(_) => v.ai_mark_file_written(&self.key, None).map(drop),
            None => v.ai_delete_file_snapshot(&self.key),
        };
        if let Err(e) = result {
            tracing::warn!(
                conversation_id = self.key.conversation_id,
                "could not record the end of a file write: {e}"
            );
        }
    }
}
