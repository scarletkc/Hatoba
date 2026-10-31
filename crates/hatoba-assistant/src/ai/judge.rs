//! The judge of Auto mode (AI-47): asking it about a call, what it reads, and its verdicts as the
//! panel shows them.

use std::collections::BTreeSet;

use hatoba_ai::entry::{AiEntry, AutoDecision, AutoVerdict, EntryBody, ToolStatus, pair_results};
use hatoba_ai::judge::{self, JudgeCall, JudgeFailure, JudgeState, Verdict};
use hatoba_ai::mcp::deferred;
use hatoba_ai::tools;
use hatoba_core::Vault;
use hatoba_core::model::{
    AiConversation, AiModelRef as CoreModelRef, MAX_CUSTOM_INSTRUCTIONS_BYTES,
    MAX_HOST_AI_NOTES_BYTES,
};
use hatoba_core::sync::SharedVault;
use serde_json::{Map, Value};

use super::blocks::leading_blocks;
use super::calls::{calls_of, open_call_at};
use super::entries::{Entries, find_conversation};
use super::job::proxy_call;
use super::places::{host_name, local_name, saved_host, target_name};
use super::request::{bytes_prefix, resolve_model};
use super::{AiManager, MCP_PREFIX, guard, unlocked};
use crate::dto::{AiAutoVerdict, AiJudgeAnswer, AiJudgeDecision, AiJudgeSampleOutcome, AiModelRef};
use crate::error::{AppError, AppResult};
use crate::mcp::McpManager;

impl AiManager {
    /// `ai_judge` (AI-47): asks the judge of Auto mode about the open call `tool_call_id`, one
    /// that manual approval would ask about, and returns its verdict: run the call without the
    /// approval card, or ask. The judge reads what the user said and what the calls do
    /// ([`judge_state`]); `screen_tail` is the screen's last lines, which a `send_input` call
    /// shows it. Without a judge in `Settings.ai`, or with one whose provider or model is gone,
    /// the verdict asks as `unavailable` and nothing is sent; so does one cleared, replaced or
    /// gone while it was asked, whose verdict no longer counts. The request is an op of the
    /// conversation, so a stop or the lock ends it, and the verdict asks as `cancelled`. At most
    /// [`JUDGE_REQUESTS`] requests run at once across conversations; a call waits for room, and a
    /// stop or the lock ends the wait the same way.
    ///
    /// The verdict is kept, in place of an earlier one, until the call's result is stored, which
    /// carries it when Auto decided the call ([`Self::store_result`]). A call that got its result
    /// while the judge was asked keeps none.
    pub async fn judge(
        &self,
        vault: &SharedVault,
        conversation_id: &str,
        tool_call_id: &str,
        screen_tail: Option<&str>,
    ) -> AppResult<AiAutoVerdict> {
        let (chosen, asked) = {
            let v = unlocked(vault)?;
            let conversation = find_conversation(&v, conversation_id)?;
            let entries = Entries::load(&v, conversation_id)?;
            let (at, index) = open_call_at(&entries, tool_call_id)?;
            let chosen = v.settings().ai.judge_model;
            let asked = chosen
                .as_ref()
                .and_then(|m| resolve_model(&v, &m.provider_id, &m.model_id).ok())
                .map(|(provider, model)| {
                    let state = judge_state(
                        &v,
                        &self.0.mcp,
                        conversation_id,
                        &conversation,
                        &entries.list,
                        (at, index),
                        screen_tail,
                    );
                    (provider, model, state, self.start_op(conversation_id))
                });
            // No judge names no provider or model.
            (chosen.unwrap_or_default(), asked)
        };
        let (verdict, op) = match asked {
            None => (Verdict::failed(JudgeFailure::Unavailable), None),
            Some((provider, model, state, op)) => {
                let room = tokio::select! {
                    biased;
                    () = op.token().cancelled() => None,
                    room = self.0.judge_slots.0.acquire() => room.ok(),
                };
                let verdict = match room {
                    Some(_room) => {
                        judge::judge(&self.http(), &provider, &model, state, op.token()).await
                    }
                    // Stopped while it waited; the slots are never closed.
                    None => Verdict::failed(JudgeFailure::Cancelled),
                };
                (verdict, Some(op))
            }
        };
        let (verdict, auto) = {
            let v = unlocked(vault)?;
            find_conversation(&v, conversation_id)?;
            // The judge the user chose now, which may have changed while it was asked.
            let current =
                v.settings().ai.judge_model.filter(|m| {
                    *m == chosen && resolve_model(&v, &m.provider_id, &m.model_id).is_ok()
                });
            let verdict = if current.is_some() {
                verdict
            } else {
                Verdict::failed(JudgeFailure::Unavailable)
            };
            let auto = verdict.to_auto(&chosen.provider_id, &chosen.model_id);
            // Kept only while the call waits: not after its result, a stop or the lock.
            if open_call_at(&Entries::load(&v, conversation_id)?, tool_call_id).is_ok() {
                guard(&self.0.pending_verdicts).insert(
                    (conversation_id.to_owned(), tool_call_id.to_owned()),
                    auto.clone(),
                );
            }
            (verdict, auto)
        };
        drop(op);
        tracing::info!(
            conversation_id,
            decision = ?verdict.decision,
            failure = verdict.failure.map(JudgeFailure::as_str),
            "AI call judged"
        );
        Ok(auto_view(&auto))
    }

    /// `ai_judge_test` (AI-47): **Test** beside the judge in Settings → AI. Judges the sample
    /// calls of [`judge::samples`] with `judge`, the model the pane shows before it is saved, or
    /// with the saved judge when that is `None`. The model must be listed by its provider, of any
    /// protocol. The requests are a background op, so the lock ends them and the outcomes ask as
    /// `cancelled`. Only how many came out as expected and the kinds of failure are logged.
    pub async fn judge_test(
        &self,
        vault: &SharedVault,
        judge: Option<AiModelRef>,
    ) -> AppResult<Vec<AiJudgeSampleOutcome>> {
        let (chosen, provider, model, op) = {
            let v = unlocked(vault)?;
            let chosen = judge
                .map(|m| CoreModelRef {
                    provider_id: m.provider_id,
                    model_id: m.model_id,
                })
                .or_else(|| v.settings().ai.judge_model)
                .ok_or_else(|| AppError::invalid("judge_model", "no judge model is chosen"))?;
            let (provider, model) = resolve_model(&v, &chosen.provider_id, &chosen.model_id)
                .map_err(|e| AppError::invalid("judge_model", e))?;
            (chosen, provider, model, self.background_op())
        };
        let outcomes = judge::test_judge(&self.http(), &provider, &model, op.token()).await;
        drop(op);
        let as_expected = outcomes
            .iter()
            .filter(|o| o.verdict.failure.is_none() && o.verdict.decision == o.expected)
            .count();
        let failures: BTreeSet<&str> = outcomes
            .iter()
            .filter_map(|o| o.verdict.failure.map(JudgeFailure::as_str))
            .collect();
        tracing::info!(
            samples = outcomes.len(),
            as_expected,
            ?failures,
            "AI judge tested"
        );
        Ok(outcomes
            .into_iter()
            .map(|o| AiJudgeSampleOutcome {
                name: o.name,
                expected: decision_view(o.expected),
                verdict: auto_view(&o.verdict.to_auto(&chosen.provider_id, &chosen.model_id)),
            })
            .collect())
    }
}

/// Most lines of the screen that a `send_input` call shows the judge (AI-47). The panel sends
/// the screen's last lines (`SCREEN_TAIL_LINES` in `features/ai/terminalText.ts`); this only
/// bounds what it sends, so a larger screen from the WebView never reaches the judge.
const MAX_SCREEN_TAIL_LINES: usize = 10;
/// Most characters of them.
const MAX_SCREEN_TAIL_CHARS: usize = 2_000;

/// AI-47: what the judge reads about the call `index` of the response `at` in `entries`, every
/// entry of the conversation, as spec §13.5 lists it, never what the model wrote or a tool
/// returned:
///
/// - `host`: the conversation's host, as the system prompt names it (§13.1): its display name,
///   or its quick-connect target (HOST-12).
/// - `user_messages`: every message the user sent, those before a compaction included, as
///   [`typed_text`] gives it; messages left empty are left out. Summaries are the model's.
/// - `user_rules`: the custom instructions (AI-36) and the host's AI notes (AI-37), each cut to
///   its limit as the system prompt cuts it, under a line that says which they are.
/// - `earlier_calls`: the calls before it, oldest first, those of its own response included, as
///   [`shown_call`] gives them. A call the user rejected adds the reason they typed
///   (`rejection`): the user's own words, which no user message carries.
/// - `call`: the call itself. An MCP tool, called directly or through `call_mcp_tool` (AI-45),
///   adds its server's name and the tool's description and annotations; `send_input` adds
///   `screen_tail` ([`screen_tail`]).
pub(super) fn judge_state(
    v: &Vault,
    mcp: &McpManager,
    conversation_id: &str,
    conversation: &AiConversation,
    entries: &[AiEntry],
    (at, index): (usize, usize),
    screen: Option<&str>,
) -> JudgeState {
    let host = conversation
        .host_id
        .as_deref()
        .and_then(|id| host_name(v, id))
        .or_else(|| conversation.quick_target.as_ref().map(target_name))
        .or_else(|| conversation.local.as_deref().map(local_name));
    let user_messages = entries[..at]
        .iter()
        .filter_map(|e| match &e.body {
            EntryBody::User { text, .. } => Some(typed_text(text)),
            _ => None,
        })
        .filter(|text| !text.is_empty())
        .collect();
    let results = pair_results(entries);
    let rejection = |response: usize, call: usize| match results
        .get(&(response, call))
        .map(|&result| &entries[result].body)
    {
        Some(EntryBody::Tool {
            status: ToolStatus::Rejected,
            content,
            ..
        }) => Some(content.clone()),
        _ => None,
    };
    let calls = calls_of(&entries[at]);
    let earlier_calls = (0..at)
        .flat_map(|response| (0..calls_of(&entries[response]).len()).map(move |c| (response, c)))
        .chain((0..index).map(|c| (at, c)))
        .map(|(response, c)| {
            let call = &calls_of(&entries[response])[c];
            JudgeCall {
                rejection: rejection(response, c),
                ..shown_call(&call.name, &call.arguments)
            }
        })
        .collect();
    let call = &calls[index];
    let mut judged = shown_call(&call.name, &call.arguments);
    let offered = if call.name.starts_with(MCP_PREFIX) {
        mcp.offered(conversation_id, &call.name)
    } else if call.name == tools::CALL_MCP_TOOL {
        proxy_call(v, mcp, conversation_id, &call.arguments)
            .ok()
            .map(|(tool, arguments)| {
                judged = JudgeCall::new(tool.name.clone(), Value::Object(arguments));
                tool
            })
    } else {
        None
    };
    if let Some(tool) = offered {
        judged.server = Some(tool.server_name);
        judged.description = Some(tool.tool.description).filter(|d| !d.trim().is_empty());
        judged.annotations = annotations_of(&tool.tool.annotations);
    }
    if call.name == tools::SEND_INPUT {
        judged.screen_tail = screen.and_then(screen_tail);
    }
    JudgeState {
        host,
        user_rules: judge_rules(v, conversation),
        user_messages,
        earlier_calls,
        call: judged,
    }
}

/// AI-47: a user message as the judge reads it: the text the user typed, without the notes
/// Hatoba adds (spec §13.3), and each attachment as a placeholder that names its kind and size,
/// such as `[file app.log, 40 lines]`, on a line of its own before the text. Empty when the
/// message has neither, as one of notes alone.
pub(super) fn typed_text(text: &str) -> String {
    let (blocks, typed) = leading_blocks(text);
    let placeholders: Vec<String> = blocks
        .iter()
        .filter(|b| !b.kind.note)
        .map(|b| (b.kind.placeholder)(&b.attrs))
        .collect();
    let typed = typed.trim();
    match (placeholders.is_empty(), typed.is_empty()) {
        (true, _) => typed.to_owned(),
        (false, true) => placeholders.join("\n"),
        (false, false) => format!("{}\n\n{typed}", placeholders.join("\n")),
    }
}

/// AI-47: a call as the judge reads it, its arguments parsed (or kept as text when they are not
/// JSON). A `call_mcp_tool` call (AI-45) is the MCP tool it names with that tool's arguments, as
/// the panel shows it; one that names no MCP tool is shown as it is.
fn shown_call(name: &str, arguments: &str) -> JudgeCall {
    if name == tools::CALL_MCP_TOOL
        && let Ok(proxy) = deferred::parse_call(arguments)
        && proxy.tool.starts_with(MCP_PREFIX)
    {
        return JudgeCall::new(proxy.tool, Value::Object(proxy.arguments));
    }
    JudgeCall::parse(name, arguments)
}

/// An MCP tool's annotations as its server named them (AI-30), for the judge; `None` without
/// any.
fn annotations_of(a: &hatoba_ai::mcp::ToolAnnotations) -> Option<Value> {
    let mut hints = Map::new();
    if let Some(title) = &a.title {
        hints.insert("title".into(), Value::from(title.as_str()));
    }
    for (name, hint) in [
        ("readOnlyHint", a.read_only_hint),
        ("destructiveHint", a.destructive_hint),
        ("idempotentHint", a.idempotent_hint),
        ("openWorldHint", a.open_world_hint),
    ] {
        if let Some(hint) = hint {
            hints.insert(name.into(), Value::Bool(hint));
        }
    }
    (!hints.is_empty()).then_some(Value::Object(hints))
}

/// AI-47: the screen's last lines as a `send_input` call shows them to the judge: control
/// characters removed, the blank lines at the end left out, then at most the last
/// [`MAX_SCREEN_TAIL_LINES`] lines and [`MAX_SCREEN_TAIL_CHARS`] characters. `None` when nothing
/// is left.
pub(super) fn screen_tail(screen: &str) -> Option<String> {
    let cleaned: String = screen
        .chars()
        .filter(|c| *c == '\n' || !c.is_control())
        .collect();
    let lines: Vec<&str> = cleaned.lines().map(str::trim_end).collect();
    let end = lines.iter().rposition(|l| !l.is_empty())? + 1;
    let tail = lines[end.saturating_sub(MAX_SCREEN_TAIL_LINES)..end].join("\n");
    let surplus = tail.chars().count().saturating_sub(MAX_SCREEN_TAIL_CHARS);
    Some(tail.chars().skip(surplus).collect())
}

/// AI-47: the custom instructions (AI-36) and the AI notes of the conversation's host (AI-37) as
/// the judge reads them, each cut to its limit as the system prompt cuts it and under a line that
/// says which they are. Empty when both are.
fn judge_rules(v: &Vault, conversation: &AiConversation) -> String {
    let settings = v.settings();
    let instructions = bytes_prefix(
        &settings.ai.custom_instructions,
        MAX_CUSTOM_INSTRUCTIONS_BYTES,
    );
    let host = conversation
        .host_id
        .as_deref()
        .and_then(|id| saved_host(v, id));
    let notes = host.map_or("", |h| bytes_prefix(h.ai_notes, MAX_HOST_AI_NOTES_BYTES));
    [
        ("Custom instructions:", instructions),
        ("Notes for this host:", notes),
    ]
    .into_iter()
    .filter(|(_, text)| !text.trim().is_empty())
    .map(|(label, text)| format!("{label}\n{}", text.trim()))
    .collect::<Vec<_>>()
    .join("\n\n")
}

/// AI-47: a stored verdict of the judge for the panel, its answers in the order the judge asks
/// its questions ([`question_order`]), then any others by id.
pub(super) fn auto_view(verdict: &AutoVerdict) -> AiAutoVerdict {
    let order = question_order();
    let mut answers: Vec<(&String, &f64)> = verdict.answers.iter().collect();
    // Stable, so answers to questions this version does not ask stay in id order.
    answers.sort_by_key(|(id, _)| order.iter().position(|q| q == id).unwrap_or(order.len()));
    AiAutoVerdict {
        provider_id: verdict.provider_id.clone(),
        model_id: verdict.model_id.clone(),
        decision: decision_view(verdict.decision),
        answers: answers
            .into_iter()
            .map(|(question, probability)| AiJudgeAnswer {
                question: question.clone(),
                probability: *probability,
            })
            .collect(),
        concerns: verdict.concerns.clone(),
        failure: verdict.failure.clone(),
    }
}

/// A decision of the judge, for the panel (AI-47).
fn decision_view(decision: AutoDecision) -> AiJudgeDecision {
    match decision {
        AutoDecision::Run => AiJudgeDecision::Run,
        AutoDecision::Ask => AiJudgeDecision::Ask,
    }
}

/// The ids of the judge's questions in the order of its table (spec §13.5). Every tool's questions
/// ([`judge::question_ids`]) keep this order, so the answers of a stored verdict, which does not
/// name its call's tool, show in the order they were asked.
fn question_order() -> Vec<&'static str> {
    let mut order: Vec<&'static str> = Vec::new();
    for tool in [tools::RUN_COMMAND, tools::EDIT_FILE, tools::FETCH_URL] {
        for id in judge::question_ids(tool) {
            if !order.contains(&id) {
                order.push(id);
            }
        }
    }
    order
}
