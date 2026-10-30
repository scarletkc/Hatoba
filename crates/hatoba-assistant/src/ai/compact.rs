//! Compaction (AI-21, AI-22): `ai_compact`, the automatic compaction before a message and between
//! tool calls, and the token estimates that decide when it is due.

use std::collections::HashMap;

use hatoba_ai::AiError;
use hatoba_ai::chat::{ToolDef, complete};
use hatoba_ai::entry::{AiEntry, AssistantEntry, EntryBody};
use hatoba_ai::provider::ModelSpec;
use hatoba_ai::tools::{self, escape_closing_tag};
use hatoba_core::Vault;
use hatoba_core::model::Item;
use hatoba_core::sync::SharedVault;
use tokio_util::sync::CancellationToken;

use super::entries::{Entries, append, cancel_open_calls, find_conversation};
use super::notes::{mcp_deferred, noted_permission};
use super::places::conversation_memory_place;
use super::request::{next_request, resolve_model, turn_server_id};
use super::side::{COMPACT, SideRequest, log_side_request_usage};
use super::view::{entry_view, error_kind, error_text};
use super::{AiEnv, AiManager, OpGuard, Request, Turn, lock, memory, unlocked};
use crate::dto::{AiEntryView, AiTurnContext, AiTurnEvent};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::mcp::Offer;
use crate::now_ms;

impl AiManager {
    /// `ai_compact` (AI-21): asks the model for a summary of the context with the request a turn
    /// would send next ([`compact_request`]), stores it as a `summary` entry and moves
    /// `context_start` to it. Refused while a turn runs. The conversation counts as compacting
    /// until the summary is stored, which refuses a side question (AI-42). `instructions`: the
    /// user's own for the summary, which `/compact <instructions>` gives (AI-50).
    pub async fn compact(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        conversation_id: &str,
        context: &AiTurnContext,
        instructions: Option<&str>,
    ) -> AppResult<AiEntryView> {
        let mut stored = false;
        let prepared = self.prepare_compact(vault, conversation_id, context, &mut stored);
        if stored {
            env.changed();
        }
        let op = prepared?;
        // AI-30: the MCP tools the next request offers, so the Compact request offers them too.
        let offer = self.0.mcp.offer(vault, context, &op.cancel).await;
        let server_id = turn_server_id(env, context);
        let compact = {
            let v = unlocked(vault)?;
            if op.cancel.is_cancelled() {
                return Err(AppError::new(
                    ErrorCode::Cancelled,
                    "compaction was stopped",
                ));
            }
            compact_request(
                &v,
                conversation_id,
                context,
                server_id.as_deref(),
                &offer,
                &compact_instruction(instructions),
                false,
            )?
        };
        let summary = match self.summarize(conversation_id, compact, &op.cancel).await {
            Ok(summary) => summary,
            Err(e) => {
                tracing::warn!(
                    conversation_id,
                    kind = error_kind(&e),
                    status = ?e.status(),
                    "AI compaction failed"
                );
                return Err(e.into());
            }
        };
        let Some(summary) = summary else {
            return Err(AppError::new(
                ErrorCode::Ai,
                "the model returned no summary",
            ));
        };
        let view = {
            let mut v = unlocked(vault)?;
            if op.cancel.is_cancelled() {
                return Err(AppError::new(
                    ErrorCode::Cancelled,
                    "compaction was stopped",
                ));
            }
            let view = store_summary(&mut v, conversation_id, &summary)?;
            // The summary no longer holds the files the model read (AI-38…40).
            self.0.seen.forget(conversation_id);
            view
        };
        drop(op);
        tracing::info!(conversation_id, "AI conversation compacted");
        env.changed();
        Ok(view)
    }

    /// The checks of `ai_compact` under the vault guard, before any server starts: no turn runs,
    /// the context has something to summarize, and the model exists. Gives the calls without a
    /// result a cancelled one, and registers the compaction so that stop and lock reach it.
    fn prepare_compact(
        &self,
        vault: &SharedVault,
        conversation_id: &str,
        context: &AiTurnContext,
        stored: &mut bool,
    ) -> AppResult<OpGuard<'_>> {
        let mut v = unlocked(vault)?;
        find_conversation(&v, conversation_id)?;
        if self.is_running(conversation_id) {
            return Err(AppError::invalid("conversation_id", "a turn is running"));
        }
        // AI-41: the summary would be built before the revert is recorded.
        self.not_reverting(conversation_id)?;
        let entries = Entries::load(&v, conversation_id)?;
        if !cancel_open_calls(&mut v, conversation_id, &entries, HashMap::new())?.is_empty() {
            *stored = true;
        }
        self.forget_file_bases(conversation_id);
        self.forget_verdicts(conversation_id);
        if !compactable(&context_entries(&v, conversation_id)?) {
            return Err(AppError::invalid("conversation_id", "nothing to compact"));
        }
        resolve_model(&v, &context.provider_id, &context.model_id)
            .map_err(|message| AppError::invalid("model_id", message))?;
        Ok(self.start_compaction(conversation_id))
    }

    /// Asks the model for the summary a Compact request wants (AI-21). An answer that calls tools
    /// or has no text is dropped (none of its calls runs or is stored), and the request goes once
    /// more without tools, with the system prompt that says so. `None` when that answer is no
    /// summary either: an endpoint can still return calls without tools, and their text is then
    /// only what comes before them.
    async fn summarize(
        &self,
        conversation_id: &str,
        compact: SideRequest,
        cancel: &CancellationToken,
    ) -> Result<Option<String>, AiError> {
        let SideRequest {
            mut request,
            plain_system,
        } = compact;
        let http = self.http();
        let response = complete(&http, &request.provider, &request.chat(), cancel).await?;
        log_side_request_usage(conversation_id, COMPACT, false, response.usage);
        if let Some(summary) = summary_of(&response) {
            return Ok(Some(summary));
        }
        tracing::info!(
            conversation_id,
            tool_calls = response.tool_calls.len(),
            "AI compaction answered without a summary; asking again without tools"
        );
        request.tools = Vec::new();
        request.system = plain_system;
        let response = complete(&http, &request.provider, &request.chat(), cancel).await?;
        log_side_request_usage(conversation_id, COMPACT, true, response.usage);
        if !response.tool_calls.is_empty() {
            tracing::info!(
                conversation_id,
                tool_calls = response.tool_calls.len(),
                "AI compaction answered with tool calls again"
            );
        }
        Ok(summary_of(&response))
    }

    /// AI-22, before a new message or a retry: builds the Compact request from the request the
    /// turn would send next, with the MCP tools it would offer (starting their servers, as the
    /// turn would), then compacts like [`Self::auto_compact`]. `defers`: the new message defers
    /// the conversation's MCP tools (AI-45), so its requests, and this one, offer `search_tools`
    /// and `call_mcp_tool`. The conversation counts as compacting from the start, which refuses a
    /// side question (AI-42).
    pub(super) async fn compact_before(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        conversation_id: &str,
        turn: &Turn,
        defers: bool,
    ) -> Result<(), CompactFailure> {
        let _compaction = self.start_compaction(conversation_id);
        let offer = self.0.mcp.offer(vault, &turn.context, &turn.cancel).await;
        let compact = {
            let v = lock(vault);
            if turn.cancel.is_cancelled() || !v.is_unlocked() {
                return Err(CompactFailure::Stopped);
            }
            compact_request(
                &v,
                conversation_id,
                &turn.context,
                turn.server_id.as_deref(),
                &offer,
                COMPACT_INSTRUCTION,
                defers,
            )
            .map_err(|e| CompactFailure::Failed {
                status: None,
                message: e.detail,
            })?
        };
        self.auto_compact(vault, env, conversation_id, turn, compact)
            .await
    }

    /// AI-22: runs a Compact request inside a turn, exactly like `ai_compact`, stores the
    /// summary (moving `context_start` to it) and sends its entry on the turn's channel. The
    /// conversation counts as compacting until the summary is stored (AI-42).
    pub(super) async fn auto_compact(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        conversation_id: &str,
        turn: &Turn,
        compact: SideRequest,
    ) -> Result<(), CompactFailure> {
        let _compaction = self.start_compaction(conversation_id);
        tracing::info!(conversation_id, "AI context nearly full; compacting");
        let summary = match self.summarize(conversation_id, compact, &turn.cancel).await {
            Ok(summary) => summary,
            Err(_) if turn.cancel.is_cancelled() => return Err(CompactFailure::Stopped),
            Err(AiError::Cancelled) => return Err(CompactFailure::Stopped),
            Err(e) => {
                tracing::warn!(
                    conversation_id,
                    kind = error_kind(&e),
                    status = ?e.status(),
                    "AI automatic compaction failed"
                );
                return Err(CompactFailure::Failed {
                    status: e.status(),
                    message: error_text(&e),
                });
            }
        };
        let Some(summary) = summary else {
            tracing::warn!(conversation_id, "AI automatic compaction got no summary");
            return Err(CompactFailure::Failed {
                status: None,
                message: "The context is nearly full, and the model returned no summary \
                          when asked to compact it."
                    .into(),
            });
        };
        {
            let mut v = lock(vault);
            if turn.cancel.is_cancelled() || !v.is_unlocked() {
                return Err(CompactFailure::Stopped);
            }
            match store_summary(&mut v, conversation_id, &summary) {
                Ok(view) => {
                    self.0.seen.forget(conversation_id);
                    turn.emit(AiTurnEvent::Entry { entry: view });
                }
                Err(e) if e.code == ErrorCode::Locked => return Err(CompactFailure::Stopped),
                Err(e) => {
                    return Err(CompactFailure::Failed {
                        status: None,
                        message: e.detail,
                    });
                }
            }
        }
        tracing::info!(conversation_id, "AI conversation compacted automatically");
        env.changed();
        Ok(())
    }
}

/// The final instruction of a Compact request (AI-21). The request offers the conversation's
/// tools, so it first asks for text only. The summary leaves out what the assistant can use:
/// every request states that in its system prompt, and a summary written by the fallback without
/// tools would otherwise tell the requests after it that there are none. It leaves out the memories
/// too (AI-53): the summary is stored with them as they are then, and a copy in the summary would
/// outlive an edit of one.
pub(super) const COMPACT_INSTRUCTION: &str = "Respond with text only. Do not call any tools.\n\n\
     Summarize the conversation so far so that the summary can replace it as the context of the \
     rest of this conversation. Keep everything later work needs: the user's goals and requests, \
     the hosts and systems involved, the commands that were run and their outcomes, files and \
     settings that were changed, decisions made, and open problems and next steps. Leave out what \
     no longer matters, and leave out what the assistant can use, such as its tools, skills and \
     terminal connection: every request states that in its system prompt, and it can change \
     between requests. The memories in a <memory> note are stored with the summary as they are \
     then, so leave out what the note says. Write the summary in the user's language and reply \
     with the summary only.";

/// The instruction of a Compact request between tool calls (AI-22), after which the turn goes on
/// from the summary alone: it asks for the task in progress and its next step as well.
pub(super) const COMPACT_MID_TURN_INSTRUCTION: &str = "Respond with text only. Do not call any tools.\n\n\
     The context is nearly full in the middle of a task. Summarize the conversation so far so \
     that the summary can replace it and the task can go on from it. Keep the user's goals and \
     requests, the hosts and systems involved, the task in progress and the step it has reached, \
     what the latest tool results showed, the commands that were run and their outcomes, files \
     and settings that were changed, decisions made, and the next step. Leave out what no longer \
     matters, and leave out what the assistant can use, such as its tools, skills and terminal \
     connection: every request states that in its system prompt, and it can change between \
     requests. The memories in a <memory> note are stored with the summary as they are then, so \
     leave out what the note says. Write the summary in the user's language and reply with the \
     summary only.";

/// The most output tokens a Compact request asks for (Anthropic's `max_tokens`, AI-21).
const COMPACT_MAX_TOKENS: u64 = 16_000;

/// The most output tokens a Compact request asks for, in hundredths of the context window: what
/// a context just under the AI-22 threshold leaves free, less a hundredth for the instruction and
/// the error of the estimate, so the request stays inside the window.
const COMPACT_OUTPUT_PERCENT: u64 = 9;

/// AI-22: a request whose context would pass this share of the context window is compacted
/// first, in tenths.
const AUTO_COMPACT_TENTHS: u64 = 9;

/// The rough text-to-token ratio of estimates, the same as the panel's meter (AI-20).
const CHARS_PER_TOKEN: usize = 4;

/// Why an automatic compaction did not happen.
pub(super) enum CompactFailure {
    /// The turn was stopped, or the vault locked.
    Stopped,
    /// The request failed or the summary could not be stored.
    Failed {
        status: Option<u16>,
        message: String,
    },
}

/// The instruction of a Compact request (AI-21), with the user's own instructions for the summary
/// when `/compact <instructions>` gave some (AI-50).
pub(super) fn compact_instruction(instructions: Option<&str>) -> String {
    match instructions.map(str::trim).filter(|s| !s.is_empty()) {
        None => COMPACT_INSTRUCTION.to_owned(),
        Some(extra) => format!(
            "{COMPACT_INSTRUCTION}\n\nThe user gave these instructions for the summary, which \
             come first:\n<compact_instructions>\n{}\n</compact_instructions>",
            escape_closing_tag(extra, "compact_instructions")
        ),
    }
}

/// The summary in an answer to a Compact request: its text, when it has some and calls no tools.
fn summary_of(response: &AssistantEntry) -> Option<String> {
    let text = response.text.trim();
    (response.tool_calls.is_empty() && !text.is_empty()).then(|| text.to_owned())
}

/// The Compact request (AI-21) for the conversation's context, built from the next request of a
/// turn with `context` and `server_id` whose MCP tools are `offer`, with less output. `defers`:
/// the message it comes before defers the MCP tools (AI-45), which its stored entries do not say
/// yet.
fn compact_request(
    v: &Vault,
    conversation_id: &str,
    context: &AiTurnContext,
    server_id: Option<&str>,
    offer: &Offer,
    instruction: &str,
    defers: bool,
) -> AppResult<SideRequest> {
    let conversation = find_conversation(v, conversation_id)?;
    let mut entries = Entries::load(v, conversation_id)?;
    let from = entries.context_from(conversation.context_start.as_deref());
    let deferred = defers || mcp_deferred(&entries.list);
    let list = entries.list.split_off(from);
    if !compactable(&list) {
        return Err(AppError::invalid("conversation_id", "nothing to compact"));
    }
    let next = next_request(v, context, server_id, offer, list, deferred)
        .map_err(|message| AppError::invalid("model_id", message))?;
    Ok(SideRequest::compact(v, context, next, instruction))
}

/// The conversation's context: its entries from `context_start` on (AI-21).
fn context_entries(v: &Vault, conversation_id: &str) -> AppResult<Vec<AiEntry>> {
    let conversation = find_conversation(v, conversation_id)?;
    let mut entries = Entries::load(v, conversation_id)?;
    let from = entries.context_from(conversation.context_start.as_deref());
    Ok(entries.list.split_off(from))
}

/// Whether a context has something to summarize: a message or a response.
fn compactable(context: &[AiEntry]) -> bool {
    context
        .iter()
        .any(|e| matches!(e.body, EntryBody::User { .. } | EntryBody::Assistant(_)))
}

/// The output tokens a Compact request asks for: at most 16,000, the model's output limit, and 9%
/// of its context window, so that a context compacted at 90% of the window (AI-22) leaves room
/// for the summary. Anthropic refuses a request whose input and `max_tokens` pass the window, and
/// a normal request asks for the model's whole output limit.
pub(super) fn compact_output_tokens(model: &ModelSpec) -> u64 {
    let mut tokens = COMPACT_MAX_TOKENS;
    if let Some(output) = model.max_output_tokens.filter(|n| *n > 0) {
        tokens = tokens.min(output);
    }
    if let Some(window) = model.context_window.filter(|w| *w > 0) {
        tokens = tokens.min(window.saturating_mul(COMPACT_OUTPUT_PERCENT) / 100);
    }
    tokens.max(1)
}

/// AI-22, before a new message is stored: whether the context plus the message would pass 90% of
/// the model's known context window. Never when the context is empty or ends with a summary (it
/// was just compacted, and no entry has followed it since), or the model cannot be resolved (the
/// turn then says why).
pub(super) fn compaction_before(
    v: &Vault,
    conversation_id: &str,
    context: &AiTurnContext,
    text: &str,
    saved: u64,
) -> AppResult<bool> {
    let Ok((_, model)) = resolve_model(v, &context.provider_id, &context.model_id) else {
        return Ok(false);
    };
    let Some(window) = model.context_window.filter(|w| *w > 0) else {
        return Ok(false);
    };
    let list = context_entries(v, conversation_id)?;
    if !compactable(&list)
        || matches!(
            list.last().map(|e| &e.body),
            Some(EntryBody::Summary { .. })
        )
    {
        return Ok(false);
    }
    let mut tokens = context_tokens(&list).saturating_add(estimate_tokens(text));
    // `saved`: tool definitions the last response's usage counts and the message's requests leave
    // out (AI-45). Without a usage, the estimate counted no tool definitions.
    if list
        .iter()
        .any(|e| matches!(&e.body, EntryBody::Assistant(a) if a.usage.is_some()))
    {
        tokens = tokens.saturating_sub(saved);
    }
    Ok(over_threshold(tokens, window))
}

/// AI-45: the tokens of tool definitions a message that defers the MCP tools takes out of its
/// requests, against the last request, which offered the tools of `last` directly.
pub(super) fn deferred_saving(v: &Vault, context: &AiTurnContext, last: &Offer) -> u64 {
    let Ok((provider, _)) = resolve_model(v, &context.provider_id, &context.model_id) else {
        return 0;
    };
    let size = |defs: &[ToolDef]| estimate_tokens(&serde_json::to_string(defs).unwrap_or_default());
    size(&last.tool_defs(provider.protocol)).saturating_sub(size(&tools::deferred_mcp_tools()))
}

/// Whether a request's tools are those of a conversation whose MCP tools are deferred (AI-45).
pub(super) fn offers_deferred(tools: &[ToolDef]) -> bool {
    tools.iter().any(|t| t.name == tools::CALL_MCP_TOOL)
}

fn over_threshold(tokens: u64, window: u64) -> bool {
    tokens.saturating_mul(10) > window.saturating_mul(AUTO_COMPACT_TENTHS)
}

/// Stores a summary entry and moves `context_start` to it (AI-21). The model sees nothing before
/// a summary, so the summary starts with the `permission_mode` note (AI-16) of the mode the model
/// was told of, which the request puts before the summary's block: the mode stays that one, also
/// for the rest of a turn compacted between tool calls (AI-22). A conversation whose messages never
/// named one gets no note, and its next message names the mode. The `memory` note (AI-53) follows
/// it, with the memories as they are now: the global ones and those of the conversation's host.
fn store_summary(v: &mut Vault, conversation_id: &str, summary: &str) -> AppResult<AiEntryView> {
    let mut conversation = find_conversation(v, conversation_id)?;
    let noted = noted_permission(&Entries::load(v, conversation_id)?.list);
    let place = conversation_memory_place(v, &conversation);
    let text = format!(
        "{}{}{summary}",
        noted.map(tools::permission_mode_block).unwrap_or_default(),
        memory::note(v, place.as_ref())
    );
    let entry = AiEntry::summary(now_ms(), text);
    let entry_id = append(v, conversation_id, &entry)?;
    conversation.context_start = Some(entry_id.clone());
    v.put(Some(conversation_id), Item::AiConversation(conversation))?;
    Ok(entry_view(&entry_id, &entry))
}

/// AI-22, inside a turn: whether the request's context would pass 90% of the model's known
/// context window. Only between tool calls, when the context ends with a tool result: a new
/// message is checked before it is stored ([`compaction_before`]), because a summary stored after
/// it would leave it outside the context. After a compaction the context ends with the summary,
/// so it is never compacted twice in a row.
pub(super) fn compaction_due(request: &Request) -> bool {
    let Some(window) = request.model.context_window.filter(|w| *w > 0) else {
        return false;
    };
    matches!(
        request.entries.last().map(|e| &e.body),
        Some(EntryBody::Tool { .. })
    ) && over_threshold(context_tokens(&request.entries), window)
}

/// The tokens a context uses, the way the panel's meter counts them (AI-20): the last response's
/// input and output tokens, plus about 4 characters a token for every entry stored after it.
fn context_tokens(entries: &[AiEntry]) -> u64 {
    let last_usage = entries
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, e)| match &e.body {
            EntryBody::Assistant(a) => a
                .usage
                .map(|u| (i + 1, u.input_tokens.saturating_add(u.output_tokens))),
            _ => None,
        });
    let (from, base) = last_usage.unwrap_or((0, 0));
    entries[from..]
        .iter()
        .map(|e| estimate_tokens(&entry_text(e)))
        .fold(base, u64::saturating_add)
}

/// The text of an entry the estimate counts (the meter's `entryText`).
fn entry_text(entry: &AiEntry) -> std::borrow::Cow<'_, str> {
    match &entry.body {
        EntryBody::User { text, .. } | EntryBody::Summary { text } => text.as_str().into(),
        EntryBody::Tool { content, .. } => content.as_str().into(),
        EntryBody::Assistant(a) => {
            let mut text = a.text.clone();
            for call in &a.tool_calls {
                text.push_str(&call.name);
                text.push_str(&call.arguments);
            }
            text.into()
        }
    }
}

/// Tokens from text length, rounded up, counting UTF-16 units like the WebView's `length`.
pub(super) fn estimate_tokens(text: &str) -> u64 {
    text.encode_utf16().count().div_ceil(CHARS_PER_TOKEN) as u64
}
