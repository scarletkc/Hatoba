//! A conversation's stored entries as the runtime reads and writes them, and its title (AI-23).

use std::collections::HashMap;

use hatoba_ai::entry::{AiEntry, EntryBody, ToolStatus, fix_up_missing_results};
use hatoba_core::Vault;
use hatoba_core::model::{AiConversation, Item};
use zeroize::Zeroizing;

use super::blocks::leading_blocks;
use super::view::entry_view;
use crate::dto::AiEntryView;
use crate::error::{AppError, AppResult};
use crate::now_ms;

/// A conversation's stored entries, decoded, oldest first. Unreadable entries are left out.
pub(super) struct Entries {
    pub(super) ids: Vec<String>,
    pub(super) list: Vec<AiEntry>,
}

impl Entries {
    pub(super) fn load(v: &Vault, conversation_id: &str) -> AppResult<Self> {
        Ok(Self::decode(
            conversation_id,
            v.ai_entries(conversation_id)?,
        ))
    }

    /// The entries from `entry_id` on, that entry included if it is stored. Earlier entries are
    /// not read from the store.
    pub(super) fn load_from(v: &Vault, conversation_id: &str, entry_id: &str) -> AppResult<Self> {
        Ok(Self::decode(
            conversation_id,
            v.ai_entries_from(conversation_id, entry_id)?,
        ))
    }

    /// The entries from `response` on, or all of them before it is known: the checks of an open
    /// call read only the conversation's newest response and what follows it.
    pub(super) fn load_open(
        v: &Vault,
        conversation_id: &str,
        response: Option<&str>,
    ) -> AppResult<Self> {
        match response {
            Some(id) => Self::load_from(v, conversation_id, id),
            None => Self::load(v, conversation_id),
        }
    }

    /// The id of the newest response.
    pub(super) fn newest_response(&self) -> Option<String> {
        let at = self
            .list
            .iter()
            .rposition(|e| matches!(e.body, EntryBody::Assistant(_)))?;
        Some(self.ids[at].clone())
    }

    fn decode(conversation_id: &str, stored: Vec<hatoba_core::StoredEntry>) -> Self {
        let mut entries = Self {
            ids: Vec::with_capacity(stored.len()),
            list: Vec::with_capacity(stored.len()),
        };
        for hatoba_core::StoredEntry { entry_id, json } in stored {
            match serde_json::from_str::<AiEntry>(&json) {
                Ok(entry) => {
                    entries.ids.push(entry_id);
                    entries.list.push(entry);
                }
                Err(_) => tracing::warn!(
                    conversation_id,
                    entry_id = %entry_id,
                    "unreadable conversation entry skipped"
                ),
            }
        }
        entries
    }

    /// Index of the first entry of the context (AI-21): at or after `context_start`, which
    /// entry ids order.
    pub(super) fn context_from(&self, context_start: Option<&str>) -> usize {
        context_start.map_or(0, |start| {
            self.ids.partition_point(|id| id.as_str() < start)
        })
    }

    pub(super) fn views(&self) -> Vec<AiEntryView> {
        self.ids
            .iter()
            .zip(&self.list)
            .map(|(id, entry)| entry_view(id, entry))
            .collect()
    }

    /// The results stored after the newest response for its calls `call_ids`, oldest first.
    pub(super) fn newest_results(&self, call_ids: &[String]) -> Vec<AiEntryView> {
        let after = self
            .list
            .iter()
            .rposition(|e| matches!(e.body, EntryBody::Assistant(_)))
            .map_or(0, |i| i + 1);
        self.ids[after..]
            .iter()
            .zip(&self.list[after..])
            .filter(|(_, e)| {
                matches!(&e.body, EntryBody::Tool { tool_call_id, .. } if call_ids.contains(tool_call_id))
            })
            .map(|(id, entry)| entry_view(id, entry))
            .collect()
    }
}

/// Stores a result for every stored call that has none (§13.1), a cancelled one unless `known`
/// has it, and returns them.
/// `known`: the results of calls that ran when the turn stopped: what a tool that had ended
/// returned, or a cancelled result that says what the call may have done (a write that may have
/// changed the file, a command that may still run).
pub(super) fn cancel_open_calls(
    v: &mut Vault,
    conversation_id: &str,
    entries: &Entries,
    mut known: HashMap<String, (ToolStatus, String)>,
) -> AppResult<Vec<AiEntryView>> {
    let now = now_ms();
    fix_up_missing_results(&entries.list)
        .into_iter()
        .map(|call_id| {
            let entry = match known.remove(&call_id) {
                Some((status, content)) => AiEntry::tool(now, call_id, status, content),
                None => AiEntry::cancelled_result(now, call_id),
            };
            let entry_id = append(v, conversation_id, &entry)?;
            Ok(entry_view(&entry_id, &entry))
        })
        .collect()
}

pub(super) fn append(v: &mut Vault, conversation_id: &str, entry: &AiEntry) -> AppResult<String> {
    let json = Zeroizing::new(
        serde_json::to_string(entry)
            .map_err(|_| AppError::internal("could not encode a conversation entry"))?,
    );
    Ok(v.ai_append_entry(conversation_id, &json)?)
}

pub fn find_conversation(v: &Vault, id: &str) -> AppResult<AiConversation> {
    v.get(id)
        .and_then(Item::as_ai_conversation)
        .cloned()
        .ok_or_else(|| AppError::not_found("conversation"))
}

/// AI-23: the title starts as the first line of the first message, cut to 60 characters. The
/// blocks the message starts with (spec §13.3) are skipped, so the title is the first non-empty
/// line the user typed, or the first attachment's name when nothing was typed (the panel sends
/// nothing without typed text, so that is only a fallback). Hatoba's notes never title it.
pub(super) fn title_of(text: &str) -> String {
    let (blocks, typed) = leading_blocks(text);
    let line = typed
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            blocks
                .iter()
                .find(|b| !b.kind.note)
                .map(|b| (b.kind.title)(&b.attrs))
        })
        .unwrap_or_default();
    line.chars().take(60).collect()
}
