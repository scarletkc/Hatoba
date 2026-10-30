//! Conversation operations behind the `ai_*` commands: a conversation with its entries, deleting
//! it, and forking it (AI-43).

use hatoba_ai::entry::{AiEntry, EntryBody, pair_results};
use hatoba_core::model::AiConversation;
use hatoba_core::sync::SharedVault;

use super::changes::file_changes;
use super::entries::{Entries, find_conversation};
use super::places::fork_place;
use super::view::conversation_view;
use super::{AiEnv, AiManager, MAX_TITLE_CHARS, guard, unlocked};
use crate::dto::{AiConversationDetail, AiConversationView, AiTurnEndReason};
use crate::error::{AppError, AppResult};
use crate::now_ms;

impl AiManager {
    /// A conversation with its entries (raw provider messages left out).
    pub fn conversation_detail(
        &self,
        vault: &SharedVault,
        conversation_id: &str,
    ) -> AppResult<AiConversationDetail> {
        let v = unlocked(vault)?;
        let conversation = find_conversation(&v, conversation_id)?;
        let entries = Entries::load(&v, conversation_id)?;
        Ok(AiConversationDetail {
            conversation: conversation_view(&v, conversation_id, &conversation),
            entries: entries.views(),
            running: self.is_running(conversation_id),
            // A conversation stays readable when its snapshots cannot be read; the edit's question
            // reads them again through `ai_file_changes`, which reports the error.
            file_changes: file_changes(&v, conversation_id).unwrap_or_else(|e| {
                tracing::warn!(conversation_id, code = ?e.code, "could not read the file snapshots");
                Vec::new()
            }),
        })
    }

    /// Deletes a conversation and its entries, stopping its turn first.
    pub fn delete_conversation(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        conversation_id: &str,
    ) -> AppResult<()> {
        {
            let mut v = unlocked(vault)?;
            find_conversation(&v, conversation_id)?;
            if let Some(turn) = self.cancel(conversation_id) {
                turn.end(AiTurnEndReason::Stopped);
            }
            let waiting = guard(&self.0.stopping).remove(conversation_id);
            if let Some(turn) = waiting.and_then(|waiting| waiting.halted.turn) {
                turn.end(AiTurnEndReason::Stopped);
            }
            self.stop_sides(conversation_id);
            v.ai_delete_conversation(conversation_id)?;
            self.0.seen.forget(conversation_id);
            self.forget_file_bases(conversation_id);
            self.forget_verdicts(conversation_id);
        }
        tracing::info!(conversation_id, "AI conversation deleted");
        env.changed();
        Ok(())
    }

    /// `ai_conversation_fork` (AI-43): copies the conversation up to the entry `entry_id`, or all
    /// of it without one, into a new conversation titled `title` ([`fork_copies`]), and returns
    /// the new conversation. The original is not touched. Refused while a turn of the
    /// conversation runs, waiting for approval included, or while it is compacted.
    ///
    /// Decrypts every entry of the conversation and encrypts every copy, so run it off the async
    /// runtime. The vault guard is held to read the entries and to write the fork, and released
    /// while the entries are decoded and the copies chosen.
    pub fn fork_conversation(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        conversation_id: &str,
        entry_id: Option<&str>,
        title: &str,
    ) -> AppResult<AiConversationView> {
        let title: String = title.trim().chars().take(MAX_TITLE_CHARS).collect();
        if title.is_empty() {
            return Err(AppError::invalid("title", "the title is empty"));
        }
        let (original, stored, hosts) = {
            let v = unlocked(vault)?;
            let original = find_conversation(&v, conversation_id)?;
            if self.busy(conversation_id) {
                return Err(AppError::invalid(
                    "conversation_id",
                    "The conversation can be forked once its turn or compaction has ended.",
                ));
            }
            let hosts: Vec<(String, String)> = v
                .hosts()
                .into_iter()
                .map(|(id, h)| (id, h.name))
                .chain(v.telnet_hosts().into_iter().map(|(id, h)| (id, h.name)))
                .collect();
            (original, v.ai_entries(conversation_id)?, hosts)
        };
        // The stored JSON is copied as it is, raw messages included; the decoded entries decide
        // what is copied. Unreadable entries are left out, as from every request.
        let mut ids = Vec::new();
        let mut jsons = Vec::new();
        let mut entries = Vec::new();
        for hatoba_core::StoredEntry { entry_id, json } in stored {
            match serde_json::from_str::<AiEntry>(&json) {
                Ok(entry) => {
                    ids.push(entry_id);
                    jsons.push(json);
                    entries.push(entry);
                }
                Err(_) => tracing::warn!(
                    conversation_id,
                    entry_id = %entry_id,
                    "unreadable conversation entry not forked"
                ),
            }
        }
        let at = match entry_id {
            Some(entry_id) => Some(
                ids.iter()
                    .position(|id| id == entry_id)
                    .ok_or_else(|| AppError::not_found("message"))?,
            ),
            None => entries.len().checked_sub(1),
        };
        let Some(at) = at else {
            return Err(AppError::invalid(
                "conversation_id",
                "the conversation has no messages",
            ));
        };
        let copies = fork_copies(&entries, at, entry_id.is_none()).ok_or_else(|| {
            AppError::invalid(
                "entry_id",
                "A conversation can be forked from one of your messages or from a finished reply.",
            )
        })?;
        // Indices up to `at` are copied in place, so they are also their copies' indices.
        let context_start = original.context_start.as_deref().and_then(|start| {
            let from = ids.partition_point(|id| id.as_str() < start);
            if from <= at {
                Some(from)
            } else {
                entries[..=at]
                    .iter()
                    .rposition(|e| matches!(e.body, EntryBody::Summary { .. }))
            }
        });
        let (place, origin) = fork_place(&entries[at + 1..], &original, &hosts);
        let fork = AiConversation {
            title,
            host_id: place.host_id,
            quick_target: place.quick_target,
            local: place.local,
            origin,
            pinned: false,
            context_start: None,
            effort: original.effort,
            created_at: now_ms(),
            updated_at: 0,
        };
        let copied: Vec<&str> = copies.iter().map(|&i| jsons[i].as_str()).collect();
        let view = {
            let mut v = unlocked(vault)?;
            let (id, _) = v.ai_create_conversation(fork, &copied, context_start)?;
            tracing::info!(
                conversation_id,
                fork = %id,
                entries = copied.len(),
                "AI conversation forked"
            );
            conversation_view(&v, &id, &find_conversation(&v, &id)?)
        };
        env.changed();
        Ok(view)
    }
}

/// AI-43: the entries a fork copies, by index, in order: every entry up to `at`, `at` included,
/// and each tool result stored after it that answers a call of a copied assistant entry
/// ([`pair_results`]), so no copied call loses its result; `whole` copies every entry. `None`
/// when `at` is no fork point: a user entry, or an assistant entry whose every call has a result.
fn fork_copies(entries: &[AiEntry], at: usize, whole: bool) -> Option<Vec<usize>> {
    if whole {
        return Some((0..entries.len()).collect());
    }
    let pairs = pair_results(entries);
    match &entries.get(at)?.body {
        EntryBody::User { .. } => {}
        EntryBody::Assistant(a)
            if (0..a.tool_calls.len()).all(|c| pairs.contains_key(&(at, c))) => {}
        _ => return None,
    }
    let mut answers: Vec<usize> = pairs
        .iter()
        .filter(|&(&(call, _), &result)| call <= at && result > at)
        .map(|(_, &result)| result)
        .collect();
    answers.sort_unstable();
    Some((0..=at).chain(answers).collect())
}
