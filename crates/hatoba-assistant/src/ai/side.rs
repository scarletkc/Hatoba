//! The requests beside the conversation's turns ([`SideRequest`]), which Compact (AI-21) and side
//! questions (AI-42) send, and side questions themselves.

use hatoba_ai::AiError;
use hatoba_ai::chat::{StreamEvent, stream_chat};
use hatoba_ai::entry::{
    AiEntry, AssistantEntry, EntryBody, ToolStatus, Usage, fix_up_missing_results,
};
use hatoba_ai::tools::{PromptTools, escape_closing_tag};
use hatoba_core::Vault;
use hatoba_core::sync::SharedVault;
use tokio_util::sync::CancellationToken;

use super::blocks::leading_blocks;
use super::compact::compact_output_tokens;
use super::entries::{Entries, find_conversation};
use super::notes::{mcp_change_note, mcp_deferred, next_message_notes};
use super::request::{next_request, prompt, turn_server_id};
use super::view::{error_kind, finish_view};
use super::{AiEnv, AiManager, Request, unlocked};
use crate::dto::{AiSideAnswer, AiSideEvent, AiSideQuestionInput, AiTurnContext};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::mcp::Offer;
use crate::now_ms;

impl AiManager {
    /// `ai_side_question` (AI-42): asks the conversation's model `input.text` with the request
    /// the conversation would send next ([`side_request`]), streams the answer to `on_event`, and
    /// returns it. Stores nothing and leaves a running turn as it is: the turn's stop does not
    /// reach it, while [`Self::side_stop`], a newer side question of the conversation (even one
    /// that is refused), deleting the conversation and the lock do. Refused while the
    /// conversation is compacting.
    pub async fn side_question(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        input: AiSideQuestionInput,
        on_event: &(dyn Fn(AiSideEvent) + Send + Sync),
    ) -> AppResult<AiSideAnswer> {
        let AiSideQuestionInput {
            conversation_id,
            question_id,
            text,
            context: next_context,
            approval_call_id,
        } = input;
        let stopped = || AppError::new(ErrorCode::Cancelled, "the side question was stopped");
        let next_server_id = turn_server_id(env, &next_context);
        // The running turn's next request, whose prefix the provider has cached, or the next
        // message's when no turn runs, read under one vault guard: a turn starts and gives way to
        // a new message under it, so its context goes with the entries read here.
        let (side, request) = {
            let v = unlocked(vault)?;
            let side = self.start_side(&conversation_id, question_id);
            find_conversation(&v, &conversation_id)?;
            if self.is_compacting(&conversation_id) {
                return Err(AppError::invalid(
                    "conversation_id",
                    "the conversation is being compacted",
                ));
            }
            let turn = self.running(&conversation_id);
            let (context, server_id, timing) = match &turn {
                Some(turn) => (
                    &turn.context,
                    turn.server_id.as_deref(),
                    SideTiming::DuringTurn {
                        approval: approval_call_id.as_deref(),
                    },
                ),
                None => (&next_context, next_server_id.as_deref(), SideTiming::Idle),
            };
            // AI-30: the MCP tools that request offers, of the servers that run now; none starts
            // for a side question.
            let offer = self.0.mcp.running_offer(&v, context);
            let request = side_request(
                &v,
                &conversation_id,
                context,
                server_id,
                &offer,
                timing,
                &text,
            )?;
            (side, request)
        };
        let cancel = side.token().clone();
        let response = match self
            .side_answer(&conversation_id, request, &cancel, on_event)
            .await
        {
            Ok(response) => response,
            Err(_) if cancel.is_cancelled() => return Err(stopped()),
            Err(e) => {
                tracing::warn!(
                    conversation_id = %conversation_id,
                    kind = error_kind(&e),
                    status = ?e.status(),
                    "AI side question failed"
                );
                return Err(e.into());
            }
        };
        if cancel.is_cancelled() {
            return Err(stopped());
        }
        drop(side);
        // An endpoint can still return calls without tools; none of them runs.
        if !response.tool_calls.is_empty() {
            tracing::info!(
                conversation_id = %conversation_id,
                tool_calls = response.tool_calls.len(),
                "AI side question answered with tool calls again"
            );
            return Err(AppError::new(
                ErrorCode::Ai,
                "the model answered with tool calls instead of text",
            ));
        }
        tracing::info!(conversation_id = %conversation_id, "AI side question answered");
        Ok(AiSideAnswer {
            text: response.text,
            finish: finish_view(response.finish),
        })
    }

    /// Streams the answer to a side question (AI-42). An answer that calls tools is dropped (none
    /// of its calls runs), `restarted` tells the panel to discard what streamed, and the question
    /// goes once more without tools, with the system prompt that says so. Returns the last answer.
    async fn side_answer(
        &self,
        conversation_id: &str,
        request: SideRequest,
        cancel: &CancellationToken,
        on_event: &(dyn Fn(AiSideEvent) + Send + Sync),
    ) -> Result<AssistantEntry, AiError> {
        let SideRequest {
            mut request,
            plain_system,
        } = request;
        let http = self.http();
        // Nothing reaches the panel once the side question is stopped.
        let send = |event: AiSideEvent| {
            if !cancel.is_cancelled() {
                on_event(event);
            }
        };
        let mut forward = |event: StreamEvent| match event {
            StreamEvent::Text(delta) => send(AiSideEvent::Text { delta }),
            StreamEvent::Reasoning(delta) => send(AiSideEvent::Reasoning { delta }),
            // The side panel shows only that the model thinks, which any reasoning event says.
            StreamEvent::Thinking => send(AiSideEvent::Reasoning {
                delta: String::new(),
            }),
            _ => {}
        };
        let response = stream_chat(
            &http,
            &request.provider,
            &request.chat(),
            cancel,
            &mut forward,
        )
        .await?;
        log_side_request_usage(conversation_id, SIDE_QUESTION, false, response.usage);
        if response.tool_calls.is_empty() {
            return Ok(response);
        }
        tracing::info!(
            conversation_id,
            tool_calls = response.tool_calls.len(),
            "AI side question answered with tool calls; asking again without tools"
        );
        send(AiSideEvent::Restarted);
        request.tools = Vec::new();
        request.system = plain_system;
        let response = stream_chat(
            &http,
            &request.provider,
            &request.chat(),
            cancel,
            &mut forward,
        )
        .await?;
        log_side_request_usage(conversation_id, SIDE_QUESTION, true, response.usage);
        Ok(response)
    }
}

/// What comes before a side question (AI-42), in the last user message of its request. The
/// request offers the conversation's tools, so it first asks for text only, then for an answer
/// that leaves the conversation as it is: a turn may be running, and nothing keeps the answer.
pub(super) const SIDE_QUESTION_INSTRUCTION: &str = "Respond with text only. Do not call any tools.\n\n\
     The user asks the side question below while this conversation goes on. Answer it from the \
     conversation so far, briefly and in the user's language. The answer is shown to the user \
     once and is not kept in the conversation, and any task in progress goes on without it, so \
     do not carry on with the task or say that you will do something.";

/// The result a side question's request gives the call whose approval card waits (AI-42).
pub(super) const SIDE_AWAITING_APPROVAL: &str =
    "No result yet: this call is waiting for the user's approval and has not run.";

/// The result a side question's request gives each other call of the running turn without one
/// (AI-42).
pub(super) const SIDE_PENDING: &str =
    "No result yet: this call is still running, or has not started.";

/// A request beside the conversation's turns, whose answer is no reply of the conversation: a
/// Compact request (AI-21) or a side question's (AI-42). It is the conversation's next request, as
/// [`next_request`] builds it for a turn, with a last user message after it. It offers the same
/// tools with the same system prompt and replays the same entries, so the provider reads its
/// prefix from the cache the conversation's requests wrote. A Compact request asks for less
/// output ([`compact_output_tokens`]), which is not part of that prefix; a side question asks for
/// as much as the next request. `plain_system` is the system prompt of the fallback without tools
/// ([`AiManager::summarize`], [`AiManager::side_answer`]).
pub(super) struct SideRequest {
    pub(super) request: Request,
    pub(super) plain_system: String,
}

impl SideRequest {
    /// `next`, the request a turn with `context` sends next, with `message` appended to it.
    /// `plain` says what the fallback without tools asks for.
    fn new(
        v: &Vault,
        context: &AiTurnContext,
        mut next: Request,
        message: impl Into<String>,
        plain: PromptTools,
    ) -> Self {
        next.entries.push(AiEntry::user(now_ms(), message));
        let (plain_system, _) = prompt(v, context, Some(plain), None);
        Self {
            request: next,
            plain_system,
        }
    }

    /// A Compact request (AI-21, AI-22): `next` with `instruction` appended, asking for less
    /// output, so that the summary fits beside a nearly full context.
    pub(super) fn compact(
        v: &Vault,
        context: &AiTurnContext,
        mut next: Request,
        instruction: &str,
    ) -> Self {
        next.model.max_output_tokens = Some(compact_output_tokens(&next.model));
        Self::new(v, context, next, instruction, PromptTools::None)
    }
}

/// When a side question is asked (AI-42), which decides what its request adds for this request
/// only.
#[derive(Clone, Copy)]
enum SideTiming<'a> {
    /// No turn runs: the request is the next message's, so the question starts with the notes
    /// that message would start with (spec §13.3), and calls without a result go as cancelled.
    Idle,
    /// A turn runs, whose stored messages carry their notes ([`side_results`]).
    DuringTurn { approval: Option<&'a str> },
}

/// A side question's request (AI-42), built from the next request of a turn with `context` and
/// `server_id` whose MCP tools are `offer`, like a Compact request. Its last user message is laid
/// out like a message's: what `timing` adds (the notes), the attachment blocks the input area put
/// before the question, then the instruction and the typed question. Nothing of it is stored.
fn side_request(
    v: &Vault,
    conversation_id: &str,
    context: &AiTurnContext,
    server_id: Option<&str>,
    offer: &Offer,
    timing: SideTiming<'_>,
    question: &str,
) -> AppResult<SideRequest> {
    let (attachments, typed) = side_parts(question);
    let typed = typed.trim();
    if typed.is_empty() {
        return Err(AppError::invalid("text", "The question is empty."));
    }
    let conversation = find_conversation(v, conversation_id)?;
    let mut entries = Entries::load(v, conversation_id)?;
    let from = entries.context_from(conversation.context_start.as_deref());
    let notes = match timing {
        SideTiming::Idle => {
            next_message_notes(v, conversation_id, &conversation, &entries, context)
                + &mcp_change_note(&entries.list, offer, "")
        }
        SideTiming::DuringTurn { .. } => String::new(),
    };
    let deferred = mcp_deferred(&entries.list);
    let mut list = entries.list.split_off(from);
    if let SideTiming::DuringTurn { approval } = timing {
        side_results(&mut list, approval);
    }
    let next = next_request(v, context, server_id, offer, list, deferred)
        .map_err(|message| AppError::invalid("model_id", message))?;
    let attachments = attachments.trim_end_matches('\n');
    let attachments = if attachments.is_empty() {
        String::new()
    } else {
        format!("{attachments}\n\n")
    };
    let message = format!(
        "{notes}{attachments}{SIDE_QUESTION_INSTRUCTION}\n\n<side_question>\n{}\n</side_question>",
        escape_closing_tag(typed, "side_question")
    );
    Ok(SideRequest::new(
        v,
        context,
        next,
        message,
        PromptTools::SideQuestion,
    ))
}

/// A side question's text (AI-42) split into the attachment blocks the input area put before it
/// (spec §13.3) and the typed question. Text that starts with a note is all typed, since only
/// Hatoba writes notes.
pub(super) fn side_parts(text: &str) -> (&str, &str) {
    let (blocks, typed) = leading_blocks(text);
    if blocks.iter().any(|b| b.kind.note) {
        return ("", text);
    }
    (&text[..text.len() - typed.len()], typed)
}

/// The results a side question's request (AI-42) gives the calls of the running turn's `context`
/// that have none, for that request only. Only the calls of the turn's newest response, which the
/// context ends with but for results, get a result that says the call waits for the user's
/// approval (the call `approval`, whose card waits) or has not finished. Any other call without a
/// result (one left by a crash, or merged from another device) goes as cancelled, as in the
/// turn's own requests.
fn side_results(context: &mut Vec<AiEntry>, approval: Option<&str>) {
    let newest = context
        .iter()
        .rposition(|e| !matches!(e.body, EntryBody::Tool { .. }))
        .filter(|&i| matches!(context[i].body, EntryBody::Assistant(_)));
    let open_now = newest.map_or(0, |i| fix_up_missing_results(&context[i..]).len());
    let missing = fix_up_missing_results(context);
    let older = missing.len().saturating_sub(open_now);
    let now = now_ms();
    // In conversation order, so each result answers the call it is meant for (`pair_results`).
    for (n, call_id) in missing.into_iter().enumerate() {
        let entry = if n < older {
            AiEntry::cancelled_result(now, call_id)
        } else if approval == Some(call_id.as_str()) {
            AiEntry::tool(now, call_id, ToolStatus::Ok, SIDE_AWAITING_APPROVAL)
        } else {
            AiEntry::tool(now, call_id, ToolStatus::Ok, SIDE_PENDING)
        };
        context.push(entry);
    }
}

/// What [`log_side_request_usage`] logs a [`SideRequest`] as: Compact (AI-21) or a side question
/// (AI-42).
pub(super) const COMPACT: &str = "compact";
const SIDE_QUESTION: &str = "side_question";

/// Logs the token counts of a response to a [`SideRequest`], never its text (SEC-04), so the share
/// the prompt cache served can be measured.
pub(super) fn log_side_request_usage(
    conversation_id: &str,
    request: &str,
    fallback: bool,
    usage: Option<Usage>,
) {
    let usage = usage.unwrap_or_default();
    tracing::info!(
        conversation_id,
        request,
        fallback,
        input_tokens = usage.input_tokens,
        cache_read_tokens = usage.cache_read_tokens,
        cache_write_tokens = usage.cache_write_tokens,
        output_tokens = usage.output_tokens,
        estimated = usage.estimated,
        "AI side request response"
    );
}
