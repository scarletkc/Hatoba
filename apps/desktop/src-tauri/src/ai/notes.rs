//! The notes Hatoba puts before a new message (spec §13.3): of a stopped reply, a move, the
//! terminal, the model, the permission mode, the memories (AI-53), reverts and the screen, and the
//! notes of the MCP tools (AI-30, AI-45).

use hatoba_ai::entry::{AiEntry, EntryBody};
use hatoba_ai::mcp::{change, deferred};
use hatoba_ai::tools::{self, PermissionMode};
use hatoba_core::model::{AiConversation, Item};
use hatoba_core::{SnapshotState, Vault};

use super::blocks::{
    FILE_REVERT, INTERRUPTED, MCP_CHANGE, MCP_TOOLS, MEMORY, MODEL_CHANGE, PERMISSION_MODE,
    TERMINAL_CHANGE, TERMINAL_STATE, attribute_text, has_note, leading_block, leading_blocks,
    note_attr, note_block,
};
use super::compact::estimate_tokens;
use super::entries::Entries;
use super::places::{
    conversation_memory_place, host_name, local_name, memory_place, moved_from, moves_to,
    tab_local, tab_target, target_name,
};
use super::request::{builtin_tools, resolve_model};
use super::{files, memory};
use crate::dto::{AiPermissionMode, AiTurnContext};
use crate::mcp::Offer;

/// AI-45: whether a conversation's MCP tools are deferred, which they are from the first message
/// that listed them in an `mcp_tools` note. `entries` is every entry of the conversation, so a
/// compaction leaves the choice as it was.
pub(super) fn mcp_deferred(entries: &[AiEntry]) -> bool {
    entries.iter().any(|e| match &e.body {
        EntryBody::User { text, .. } => has_note(text, MCP_TOOLS),
        _ => false,
    })
}

/// AI-45: the `mcp_tools` note the conversation's next message starts with, when its requests
/// offer the MCP tools of `offer`: none while its MCP tools are offered directly, which they stay
/// until their definitions come to a tenth of the model's context window or the requests would
/// offer more than 128 tools, built-in ones included; then the list of those
/// tools, unless the newest note in the context (`entries` from `from` on) lists them already. No
/// note in the context counts as an empty list, so an empty list after a compaction is not noted.
pub(super) fn mcp_note(
    v: &Vault,
    context: &AiTurnContext,
    entries: &[AiEntry],
    from: usize,
    offer: &Offer,
) -> String {
    let Ok((provider, model)) = resolve_model(v, &context.provider_id, &context.model_id) else {
        return String::new();
    };
    let is_deferred = mcp_deferred(entries) || {
        let defs = offer.tool_defs(provider.protocol);
        !defs.is_empty()
            && deferred::defer_due(
                estimate_tokens(&serde_json::to_string(&defs).unwrap_or_default()),
                model.context_window,
                builtin_tools(v, context).len() + defs.len(),
            )
    };
    if !is_deferred {
        return String::new();
    }
    let note = deferred::list_block(&offer.listed(), deferred::list_budget(model.context_window));
    let seen = entries[from.min(entries.len())..]
        .iter()
        .rev()
        .find_map(|e| match &e.body {
            EntryBody::User { text, .. } => note_block(text, MCP_TOOLS),
            _ => None,
        })
        .map_or_else(|| deferred::list_block(&[], u64::MAX), str::to_owned);
    if note.trim_end() == seen.trim_end() {
        String::new()
    } else {
        note
    }
}

/// AI-30: the `mcp_change` note the conversation's next message starts with, when its requests
/// offer the MCP tools of `offer` directly and those come from other servers than the model was
/// last told of ([`mcp_servers_before`]). None before a conversation's first reply, once its MCP
/// tools are deferred, where the `mcp_tools` note says what changed (AI-45), and for the message
/// that defers them, whose `mcp_tools` note is `listed`. `entries` is every entry before the
/// message.
pub(super) fn mcp_change_note(entries: &[AiEntry], offer: &Offer, listed: &str) -> String {
    if !listed.is_empty() || mcp_deferred(entries) {
        return String::new();
    }
    mcp_servers_before(entries).map_or_else(String::new, |before| {
        change::block(&before, &offer.server_names())
    })
}

/// AI-30: the servers whose MCP tools the model was last told of in `entries`, a conversation's
/// entries oldest first: those the newest reply's request offered, as the `mcp_change` notes
/// after it changed them. None before the first reply, and when that reply is from before
/// replies recorded them. A summary leaves them as they were, as it leaves the terminal tools.
fn mcp_servers_before(entries: &[AiEntry]) -> Option<Vec<String>> {
    let mut notes = Vec::new();
    for entry in entries.iter().rev() {
        match &entry.body {
            EntryBody::Assistant(a) => {
                let before = a.mcp_servers.as_ref()?;
                return Some(change::servers_after(before, notes.into_iter().rev()));
            }
            EntryBody::User { text, .. } => notes.extend(note_block(text, MCP_CHANGE)),
            EntryBody::Tool { .. } | EntryBody::Summary { .. } => {}
        }
    }
    None
}

/// AI-53: `notes` with the `memory` note of the host or computer `place` a message is on exactly
/// when no note since the context's start showed its memories. [`notes_before`] decides so
/// already, but a compaction before the message (AI-22) changes what the context shows: its
/// summary shows the memories of where the conversation was, which can make the message's note
/// a repeat, or leave out the host the message moves to. The note goes after the
/// `permission_mode` note and before the `file_revert` and `terminal_state` notes (§13.3).
pub(super) fn with_place_memories(
    v: &Vault,
    earlier: &[AiEntry],
    notes: &str,
    place: Option<&memory::Place>,
) -> String {
    let Some(place) = place else {
        return notes.to_owned();
    };
    let shown = memories_shown(earlier, &place.name);
    match note_block(notes, MEMORY) {
        Some(block) if shown => return notes.replacen(block, "", 1),
        Some(_) => return notes.to_owned(),
        None if shown => return notes.to_owned(),
        None => {}
    }
    let mut at = 0;
    let mut rest = notes;
    while let Some((block, after)) = leading_block(rest) {
        if [FILE_REVERT, TERMINAL_STATE].contains(&block.kind.tag) {
            break;
        }
        at = notes.len() - after.len();
        rest = after;
    }
    format!(
        "{}{}{}",
        &notes[..at],
        memory::place_note(v, place),
        &notes[at..]
    )
}

/// AI-53: whether a `memory` note since the context's start (AI-21) showed the memories of the
/// host or computer named `name`, which it names as its host. `earlier` is the conversation's
/// entries, oldest first; the model sees nothing before a summary, so the summary's note is the
/// last one that counts. Notes stored before [`tools::MEMORY_NOTE_HOST`] name it in `host`.
fn memories_shown(earlier: &[AiEntry], name: &str) -> bool {
    let shows = |text: &str| {
        note_attr(text, MEMORY, tools::MEMORY_NOTE_HOST)
            .or_else(|| note_attr(text, MEMORY, "host"))
            .is_some_and(|host| host == name)
    };
    for entry in earlier.iter().rev() {
        match &entry.body {
            EntryBody::User { text, .. } => {
                if shows(text) {
                    return true;
                }
            }
            EntryBody::Summary { text } => return shows(text),
            EntryBody::Assistant(_) | EntryBody::Tool { .. } => {}
        }
    }
    false
}

/// The notes the conversation's next message would start with if it went with `context` (spec
/// §13.3), as [`AiManager::open_turn`] works them out for a message that edits nothing. A side
/// question of an idle conversation (AI-42) starts with them, unstored, so that its request reads
/// the earlier entries the way the next message's will. `earlier` is every entry of the
/// conversation.
pub(super) fn next_message_notes(
    v: &Vault,
    conversation_id: &str,
    conversation: &AiConversation,
    earlier: &Entries,
    context: &AiTurnContext,
) -> String {
    let target = tab_target(context);
    let local = tab_local(context);
    let host_id = context.host_id.as_deref();
    let (came_from, host_now, place) =
        if moves_to(conversation, host_id, target.as_ref(), local.as_deref()) {
            let came_from = moved_from(v, conversation, host_id, target.as_ref());
            let host_now = host_id
                .and_then(|id| host_name(v, id))
                .or_else(|| target.as_ref().map(target_name))
                .or_else(|| local.as_deref().map(local_name));
            let place = memory_place(v, host_id, target.as_ref(), local.as_deref());
            (came_from, host_now, place)
        } else {
            let host_now = conversation
                .host_id
                .as_deref()
                .and_then(|id| host_name(v, id))
                .or_else(|| conversation.quick_target.as_ref().map(target_name))
                .or_else(|| conversation.local.as_deref().map(local_name));
            (None, host_now, conversation_memory_place(v, conversation))
        };
    let reverts = pending_reverts(v, conversation_id, &earlier.ids, &earlier.list);
    notes_before(
        v,
        &earlier.list,
        came_from.as_deref(),
        (host_now.as_deref(), place.as_ref()),
        context,
        &reverts,
    )
}

/// How a stored model is named in a `model_change` note: its display name and ID, or the ID
/// alone when its provider or the model is gone.
fn stored_model_label(v: &Vault, provider_id: &str, model_id: &str) -> String {
    let name = v
        .get(provider_id)
        .and_then(Item::as_ai_provider)
        .and_then(|p| p.models.iter().find(|m| m.id == model_id))
        .map_or("", |m| m.name.as_str());
    tools::model_label(model_id, name)
}

/// The notes Hatoba puts before a new message (spec §13.3), in this order:
///
/// - `interrupted` (§13.1) when the newest reply in `earlier` is one the user stopped while it
///   streamed. A stop that an earlier message noted already, which got no reply, is not noted
///   again, and neither is one before a summary, which replaced it.
/// - `host_change` (AI-09) when the entries before the message came from another host than the
///   conversation's host now: `came_from` names it (empty when it no longer exists) or the
///   quick-connect target (HOST-12) they came from, and is `None` when the conversation did not
///   move. `host_now` is the display name of the host (or the target) the conversation is on now,
///   and `place` the host or computer whose host memories it shows ([`memory_place`]).
/// - `terminal_change` (AI-09) when the message's requests offer the terminal tools and, as far as
///   `earlier` tells the model, there was none before, or the other way round
///   ([`terminal_before`]).
/// - `model_change` (AI-05) when the message goes to another model ID than the newest reply in
///   `earlier` (the entries before the message) came from. A switch an earlier message noted
///   already, which got no reply (a failed request, say), is not noted again.
/// - `permission_mode` (AI-16) before a conversation's first message, and when the conversation's
///   mode on this device is not the one the model was last told of ([`noted_permission`]).
/// - `memory` (AI-53) before a conversation's first message, while memory is on and there are
///   memories ([`memory::note`]); a summary starts with one too. A later message on a host or
///   computer whose memories no note since the context's start showed ([`memories_shown`]) starts
///   with one of those ([`memory::place_note`]), when it has any.
/// - `file_revert` (AI-41) for each of `reverts`, from [`pending_reverts`].
/// - `terminal_state` (AI-46), the screen of the attached terminal ([`screen_note`]), also before
///   a conversation's first message.
pub(super) fn notes_before(
    v: &Vault,
    earlier: &[AiEntry],
    came_from: Option<&str>,
    (host_now, place): (Option<&str>, Option<&memory::Place>),
    context: &AiTurnContext,
    reverts: &[(String, files::ChangeMeta)],
) -> String {
    let mut notes = String::new();
    let mode = permission_mode(context.permission_mode);
    let permission = if noted_permission(earlier) == Some(mode) {
        String::new()
    } else {
        tools::permission_mode_block(mode)
    };
    // A message with nothing before it has no earlier screens or replies to tell apart, but it can
    // name its mode, show the memories and show the screen now.
    if earlier.is_empty() {
        return permission + &memory::note(v, place) + &screen_note(context, host_now);
    }
    for entry in earlier.iter().rev() {
        match &entry.body {
            EntryBody::Assistant(a) => {
                if a.interrupted {
                    notes.push_str(&tools::interrupted_block());
                }
                break;
            }
            EntryBody::User { text, .. } if has_note(text, INTERRUPTED) => break,
            EntryBody::Summary { .. } => break,
            EntryBody::User { .. } | EntryBody::Tool { .. } => {}
        }
    }
    if let (Some(from), Some(to)) = (came_from, host_now)
        && from != to
    {
        notes.push_str(&tools::host_change_block(from, to));
    }
    if terminal_before(earlier) == Some(!context.tab) {
        notes.push_str(&tools::terminal_change_block(context.tab));
    }
    let to = stored_model_label(v, &context.provider_id, &context.model_id);
    for entry in earlier.iter().rev() {
        match &entry.body {
            EntryBody::Assistant(a) => {
                if a.model_id != context.model_id {
                    let from = stored_model_label(v, &a.provider_id, &a.model_id);
                    notes.push_str(&tools::model_change_block(&from, &to));
                }
                break;
            }
            EntryBody::User { text, .. } => {
                if note_attr(text, MODEL_CHANGE, "to").is_some_and(|noted| noted == to) {
                    break;
                }
            }
            EntryBody::Tool { .. } | EntryBody::Summary { .. } => {}
        }
    }
    notes.push_str(&permission);
    if let Some(place) = place
        && !memories_shown(earlier, &place.name)
    {
        notes.push_str(&memory::place_note(v, place));
    }
    for (tool_call_id, meta) in reverts {
        notes.push_str(&tools::file_revert_block(
            &meta.path,
            tool_call_id,
            &meta.tool,
            meta.created,
        ));
    }
    notes.push_str(&screen_note(context, host_now));
    notes
}

/// A conversation's permission mode on this device (AI-16) as a `permission_mode` note names it.
/// The system prompt names no mode of its own, since the default is the user's setting, so the
/// conversation's first message names it, whichever it is.
fn permission_mode(mode: AiPermissionMode) -> PermissionMode {
    match mode {
        AiPermissionMode::Manual => PermissionMode::Manual,
        AiPermissionMode::Auto => PermissionMode::Auto,
        AiPermissionMode::Bypass => PermissionMode::Bypass,
    }
}

/// AI-16: the permission mode the model was last told of in `entries`, a conversation's entries
/// oldest first: the newest `permission_mode` note, or the one the newest summary starts with.
/// None when the model was told of none: before the first message, or in a conversation from
/// before every first message named its mode. The model sees nothing before a summary, so one
/// without a note tells of none either.
pub(super) fn noted_permission(entries: &[AiEntry]) -> Option<PermissionMode> {
    for entry in entries.iter().rev() {
        match &entry.body {
            EntryBody::User { text, .. } => {
                match note_attr(text, PERMISSION_MODE, "mode").as_deref() {
                    Some("bypass") => return Some(PermissionMode::Bypass),
                    Some("auto") => return Some(PermissionMode::Auto),
                    Some(_) => return Some(PermissionMode::Manual),
                    None => {}
                }
            }
            EntryBody::Summary { text } => return tools::split_permission_note(text).0,
            EntryBody::Assistant(_) | EntryBody::Tool { .. } => {}
        }
    }
    None
}

/// AI-46: the `terminal_state` note of a message whose `context` has the screen of a connected
/// tab, on the host `host` (its display name or quick-connect target). The panel sends the screen
/// while **Send Terminal Screen** is on for the conversation. Empty otherwise, and for a blank
/// screen.
fn screen_note(context: &AiTurnContext, host: Option<&str>) -> String {
    match &context.screen {
        Some(screen) if context.tab => {
            tools::terminal_state_block(host.unwrap_or_default(), screen.alternate, &screen.text)
        }
        _ => String::new(),
    }
}

/// The calls whose `file_revert` notes (AI-41) a user entry starts with.
pub(super) fn noted_reverts(text: &str) -> Vec<String> {
    leading_blocks(text)
        .0
        .iter()
        .take_while(|b| b.kind.note)
        .filter(|b| b.kind.tag == FILE_REVERT)
        .filter_map(|b| b.attrs.iter().find(|(n, _)| *n == "tool_call_id"))
        .map(|(_, value)| attribute_text(value))
        .collect()
}

/// The reverts (AI-41) a new message notes: of the calls in `earlier` (the entries before it, with
/// their `ids`) whose change this device reverted, those that no user entry after the call notes
/// yet, oldest call first, each with its call id and what its snapshot says.
pub(super) fn pending_reverts(
    v: &Vault,
    conversation_id: &str,
    ids: &[String],
    earlier: &[AiEntry],
) -> Vec<(String, files::ChangeMeta)> {
    let snapshots = match v.ai_file_snapshots(conversation_id) {
        Ok(snapshots) => snapshots,
        Err(e) => {
            tracing::warn!(conversation_id, "could not read the file snapshots: {e}");
            return Vec::new();
        }
    };
    let mut reverts = Vec::new();
    for snapshot in snapshots
        .iter()
        .filter(|s| s.state == SnapshotState::Reverted)
    {
        let Some(at) = ids.iter().position(|id| *id == snapshot.response_id) else {
            continue;
        };
        let Some(EntryBody::Assistant(response)) = earlier.get(at).map(|e| &e.body) else {
            continue;
        };
        let Some(order) = response
            .tool_calls
            .iter()
            .position(|c| c.id == snapshot.tool_call_id)
        else {
            continue;
        };
        // Responses may reuse a call id: a note is about the call of the newest response before it.
        let noted = earlier
            .iter()
            .enumerate()
            .skip(at + 1)
            .any(|(j, e)| match &e.body {
                EntryBody::User { text, .. } => {
                    noted_reverts(text).contains(&snapshot.tool_call_id)
                        && response_with_call(&earlier[..j], &snapshot.tool_call_id) == Some(at)
                }
                _ => false,
            });
        let meta = serde_json::from_str::<files::ChangeMeta>(&snapshot.meta);
        if let (false, Ok(meta)) = (noted, meta) {
            reverts.push(((at, order), snapshot.tool_call_id.clone(), meta));
        }
    }
    reverts.sort_by_key(|(place, ..)| *place);
    reverts
        .into_iter()
        .map(|(_, id, meta)| (id, meta))
        .collect()
}

/// The index of the newest response in `entries` that has a call with the id `tool_call_id`.
pub(super) fn response_with_call(entries: &[AiEntry], tool_call_id: &str) -> Option<usize> {
    entries.iter().rposition(|e| match &e.body {
        EntryBody::Assistant(a) => a.tool_calls.iter().any(|c| c.id == tool_call_id),
        _ => false,
    })
}

/// Whether a terminal was attached as far as `earlier` (the entries before a new message) tells
/// the model (AI-09): what the newest reply's request offered, or what a `terminal_change` note
/// says on a newer message that got no reply. `None` when the newest reply is from before replies
/// recorded it, or when there is no reply.
fn terminal_before(earlier: &[AiEntry]) -> Option<bool> {
    for entry in earlier.iter().rev() {
        match &entry.body {
            EntryBody::Assistant(a) => return a.terminal,
            EntryBody::User { text, .. } => {
                if let Some(to) = note_attr(text, TERMINAL_CHANGE, "to") {
                    return Some(to == "attached");
                }
            }
            EntryBody::Tool { .. } | EntryBody::Summary { .. } => {}
        }
    }
    None
}
