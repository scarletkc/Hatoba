//! Request bodies built from stored entries (spec §13.7, the entry-to-message table).
//!
//! Both protocols share one plan: the entries in order, each assistant entry followed by the
//! results of its calls, so every request is valid even when results were stored out of order,
//! twice, or not at all. Then each protocol renders the plan into its own message format.
//!
//! - An assistant entry is sent as its `raw` message when its provider and model are the
//!   request's (and `raw` has the protocol's shape); otherwise it is rebuilt from `text` and
//!   `tool_calls`, which leaves out whatever only `raw` held (reasoning, signatures).
//! - Chat Completions tool-call arguments are always a JSON object in what is sent: a call cut off
//!   mid-stream (`finish: length`) stores text such as `{"command":"ls`, and servers that parse
//!   assistant tool-call arguments (vLLM does) refuse every later request that carries it. A
//!   `raw` message holding such arguments is not replayed, and a rebuilt call sends `{}` for them
//!   ([`chat_arguments`]).
//! - An assistant entry with blank text and no tool calls (e.g. a response that was only
//!   reasoning, or an empty refusal) is skipped: neither protocol accepts an empty assistant
//!   message, and it carries nothing the model needs.
//! - A call whose result is missing gets a `cancelled` result; a result whose call is not in the
//!   context (e.g. before `context_start`) is dropped.
//! - Tool results carry wording for `rejected` and `cancelled` so the model knows what happened
//!   ([`tool_result_text`]).
//! - **A request that offers no tools has no structured tool blocks.** `ChatRequest::tools` is
//!   empty for Compact (AI-21), yet the history may hold calls and results, and Anthropic
//!   rejects `tool_use` / `tool_result` blocks in a request without `tools` (several Chat
//!   Completions servers do the same with `tool_calls` and `tool` messages). The history is then sent as text: an assistant entry that made calls is rebuilt
//!   from its fields, never from `raw`, as an assistant message with its text followed by one
//!   description per call ([`flattened_assistant_text`]), and each result becomes a `user`
//!   message that names the call it answers ([`flattened_result_text`]). On Anthropic these
//!   results merge with the user content around them like any other user text. Assistant entries
//!   without calls still replay `raw`, unless it holds tool blocks (`tool_use`, `tool_result`
//!   and variants such as `server_tool_use` for Anthropic; `tool_calls` or `function_call` for
//!   Chat Completions), in which case they are rebuilt from `text`. With tools offered nothing
//!   changes.

use std::borrow::Cow;

use serde_json::{Map, Value, json};

use super::ChatRequest;
use crate::entry::{AiEntry, AssistantEntry, EntryBody, ToolCall, ToolStatus, pair_results};
use crate::provider::Protocol;

/// `max_tokens` of an Anthropic request when the model's output limit is unknown (AI-02).
pub(crate) const DEFAULT_MAX_TOKENS: u64 = 16_000;

/// A request body and whether any assistant entry was replayed from `raw`.
pub(crate) struct Built {
    pub body: Value,
    pub used_raw: bool,
}

enum Turn<'a> {
    User(Cow<'a, str>),
    Assistant {
        entry: &'a AssistantEntry,
        raw: bool,
    },
    /// An assistant entry that made tool calls, as plain text (a request without tools).
    AssistantText(String),
    ToolResult {
        id: &'a str,
        status: ToolStatus,
        content: &'a str,
    },
}

fn raw_usable(protocol: Protocol, raw: &Value, tools_offered: bool) -> bool {
    let shaped = match protocol {
        Protocol::ChatCompletions => raw.is_object() && raw_arguments_are_objects(raw),
        Protocol::Anthropic => raw.as_array().is_some_and(|blocks| !blocks.is_empty()),
    };
    shaped && (tools_offered || !raw_has_tool_blocks(protocol, raw))
}

/// Whether every tool call in a Chat Completions `raw` message carries arguments that are a JSON
/// object: the text of one, or an object itself (servers that send the arguments parsed). Blank,
/// cut-off or missing arguments are not, and the message is then rebuilt rather than replayed.
fn raw_arguments_are_objects(raw: &Value) -> bool {
    let is_object = |arguments: Option<&Value>| match arguments {
        Some(Value::Object(_)) => true,
        Some(Value::String(text)) => is_json_object(text),
        _ => false,
    };
    let calls = match raw.get("tool_calls") {
        Some(Value::Array(calls)) => calls
            .iter()
            .all(|call| is_object(call.pointer("/function/arguments"))),
        _ => true,
    };
    let legacy = match raw.get("function_call") {
        Some(function_call @ Value::Object(_)) => is_object(function_call.get("arguments")),
        _ => true,
    };
    calls && legacy
}

/// Whether a stored `raw` message carries tool calls or results in its protocol's own format.
fn raw_has_tool_blocks(protocol: Protocol, raw: &Value) -> bool {
    match protocol {
        Protocol::ChatCompletions => ["tool_calls", "function_call"]
            .into_iter()
            .any(|key| raw.get(key).is_some_and(|value| !value.is_null())),
        Protocol::Anthropic => raw.as_array().is_some_and(|blocks| {
            blocks.iter().any(|block| {
                block
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| {
                        // `tool_use`, `tool_result`, `server_tool_use`, `mcp_tool_use`,
                        // `web_search_tool_result`, …
                        kind == "tool_use"
                            || kind == "tool_result"
                            || kind.ends_with("_tool_use")
                            || kind.ends_with("_tool_result")
                    })
            })
        }),
    }
}

fn plan<'a>(req: &ChatRequest<'a>, protocol: Protocol, allow_raw: bool) -> (Vec<Turn<'a>>, bool) {
    let entries: &'a [AiEntry] = req.entries;
    let pairs = pair_results(entries);
    // Without tools the history carries no tool blocks (see the module docs).
    let tools_offered = !req.tools.is_empty();
    let mut turns = Vec::new();
    let mut used_raw = false;
    for (i, entry) in entries.iter().enumerate() {
        match &entry.body {
            EntryBody::User { text } => turns.push(Turn::User(Cow::Borrowed(text.as_str()))),
            EntryBody::Summary { text } => turns.push(Turn::User(Cow::Owned(summary_text(text)))),
            EntryBody::Tool { .. } => {}
            EntryBody::Assistant(a) => {
                if a.text.trim().is_empty() && a.tool_calls.is_empty() {
                    continue;
                }
                if tools_offered || a.tool_calls.is_empty() {
                    let raw = allow_raw
                        && a.provider_id == req.provider_id
                        && a.model_id == req.model.id
                        && raw_usable(protocol, &a.raw, tools_offered);
                    used_raw |= raw;
                    turns.push(Turn::Assistant { entry: a, raw });
                } else {
                    turns.push(Turn::AssistantText(flattened_assistant_text(a)));
                }
                for (c, call) in a.tool_calls.iter().enumerate() {
                    let (status, content) = match pairs.get(&(i, c)).map(|&pos| &entries[pos].body)
                    {
                        Some(EntryBody::Tool {
                            status, content, ..
                        }) => (*status, content.as_str()),
                        _ => (ToolStatus::Cancelled, ""),
                    };
                    turns.push(if tools_offered {
                        Turn::ToolResult {
                            id: call.id.as_str(),
                            status,
                            content,
                        }
                    } else {
                        Turn::User(Cow::Owned(flattened_result_text(call, status, content)))
                    });
                }
            }
        }
    }
    (turns, used_raw)
}

/// The user message that carries a Compact summary (AI-21).
pub(crate) fn summary_text(summary: &str) -> String {
    format!("Summary of the earlier conversation:\n\n{summary}")
}

/// The text of a tool result as the model receives it.
pub(crate) fn tool_result_text(status: ToolStatus, content: &str) -> Cow<'_, str> {
    match (status, content.is_empty()) {
        (ToolStatus::Ok, true) => Cow::Borrowed("(no output)"),
        (ToolStatus::Error, true) => Cow::Borrowed("The tool failed without a message."),
        (ToolStatus::Ok | ToolStatus::Error, false) => Cow::Borrowed(content),
        (ToolStatus::Rejected, true) => Cow::Borrowed("The user rejected this tool call."),
        (ToolStatus::Rejected, false) => Cow::Owned(format!(
            "The user rejected this tool call. The user's reason: {content}"
        )),
        (ToolStatus::Cancelled, true) => {
            Cow::Borrowed("The tool call was cancelled before it finished.")
        }
        (ToolStatus::Cancelled, false) => Cow::Owned(format!(
            "The tool call was cancelled before it finished.\n{content}"
        )),
    }
}

/// An assistant entry that made tool calls, as the text of a request that offers no tools: the
/// entry's text (when it has any), then one block per call, separated by blank lines:
///
/// ```text
/// [Tool call: run_command (id: call_1)]
/// Arguments: {"command":"ls"}
/// ```
///
/// The arguments are the stored text, trimmed (`{}` when blank), so cut-off arguments stay as
/// the model wrote them. Neither protocol's tool-call structure is used, and `raw` is never read,
/// because it carries `tool_use` blocks or `tool_calls`.
pub(crate) fn flattened_assistant_text(entry: &AssistantEntry) -> String {
    let mut parts = Vec::with_capacity(entry.tool_calls.len() + 1);
    if !entry.text.trim().is_empty() {
        parts.push(entry.text.trim_end().to_owned());
    }
    for call in &entry.tool_calls {
        let arguments = match call.arguments.trim() {
            "" => "{}",
            arguments => arguments,
        };
        parts.push(format!(
            "[Tool call: {} (id: {})]\nArguments: {arguments}",
            call.name, call.id
        ));
    }
    parts.join("\n\n")
}

/// The result of `call` as the `user` text of a request that offers no tools: a header naming the
/// call and the status (`ok`, `error`, `rejected` or `cancelled`), then the text the model
/// receives for the result with tools ([`tool_result_text`], so the wording for rejected,
/// cancelled and empty results is the same):
///
/// ```text
/// [Tool result: run_command (id: call_1), status: rejected]
/// The user rejected this tool call. The user's reason: too risky
/// ```
pub(crate) fn flattened_result_text(call: &ToolCall, status: ToolStatus, content: &str) -> String {
    let status_name = match status {
        ToolStatus::Ok => "ok",
        ToolStatus::Error => "error",
        ToolStatus::Rejected => "rejected",
        ToolStatus::Cancelled => "cancelled",
    };
    format!(
        "[Tool result: {} (id: {}), status: {status_name}]\n{}",
        call.name,
        call.id,
        tool_result_text(status, content)
    )
}

/// Whether `text` is a JSON object.
fn is_json_object(text: &str) -> bool {
    matches!(serde_json::from_str::<Value>(text), Ok(Value::Object(_)))
}

/// Tool arguments as a JSON object (Anthropic's `tool_use.input` must be one); anything else,
/// including arguments cut off mid-stream, becomes `{}`.
fn arguments_object(arguments: &str) -> Value {
    match serde_json::from_str::<Value>(arguments) {
        Ok(value @ Value::Object(_)) => value,
        _ => json!({}),
    }
}

/// The `arguments` text of a rebuilt Chat Completions tool call: the stored text when it is a JSON
/// object, so the model's own wording goes back unchanged, and `{}` for anything else (blank,
/// cut off, not an object).
fn chat_arguments(arguments: &str) -> Cow<'_, str> {
    if is_json_object(arguments) {
        Cow::Borrowed(arguments)
    } else {
        Cow::Owned(arguments_object(arguments).to_string())
    }
}

/// `POST {base}/chat/completions` body. When the request offers no tools there are no `tool_calls`
/// and no `tool` messages in it: calls and results are sent as text (see the module docs).
pub(crate) fn chat_completions(req: &ChatRequest<'_>, stream_options: bool) -> Built {
    let (turns, used_raw) = plan(req, Protocol::ChatCompletions, true);
    let mut messages = Vec::with_capacity(turns.len() + 1);
    if !req.system.is_empty() {
        messages.push(json!({"role": "system", "content": req.system}));
    }
    for turn in turns {
        match turn {
            Turn::User(text) => messages.push(json!({"role": "user", "content": text})),
            Turn::Assistant { entry, raw: true } => {
                let mut message = entry.raw.clone();
                if let Value::Object(map) = &mut message {
                    map.insert("role".into(), json!("assistant"));
                }
                messages.push(message);
            }
            Turn::Assistant { entry, raw: false } => {
                let mut message = Map::new();
                message.insert("role".into(), json!("assistant"));
                message.insert(
                    "content".into(),
                    if entry.text.is_empty() {
                        Value::Null
                    } else {
                        json!(entry.text)
                    },
                );
                if !entry.tool_calls.is_empty() {
                    let calls: Vec<Value> = entry
                        .tool_calls
                        .iter()
                        .map(|call| {
                            json!({
                                "id": call.id,
                                "type": "function",
                                "function": {
                                    "name": call.name,
                                    "arguments": chat_arguments(&call.arguments),
                                },
                            })
                        })
                        .collect();
                    message.insert("tool_calls".into(), Value::Array(calls));
                }
                messages.push(Value::Object(message));
            }
            Turn::AssistantText(text) => {
                messages.push(json!({"role": "assistant", "content": text}));
            }
            Turn::ToolResult {
                id,
                status,
                content,
            } => messages.push(json!({
                "role": "tool",
                "tool_call_id": id,
                "content": tool_result_text(status, content),
            })),
        }
    }
    let mut body = Map::new();
    body.insert("model".into(), json!(req.model.id));
    body.insert("messages".into(), Value::Array(messages));
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    },
                })
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }
    body.insert("stream".into(), json!(true));
    if stream_options {
        body.insert("stream_options".into(), json!({"include_usage": true}));
    }
    Built {
        body: Value::Object(body),
        used_raw,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    User,
    Assistant,
}

/// Appends blocks as a message of `role`, merging into the previous message when it has the
/// same role (Anthropic needs alternating roles).
fn push_blocks(messages: &mut Vec<(Role, Vec<Value>)>, role: Role, blocks: Vec<Value>) {
    if blocks.is_empty() {
        return;
    }
    match messages.last_mut() {
        Some((last, content)) if *last == role => content.extend(blocks),
        _ => messages.push((role, blocks)),
    }
}

/// `POST {base}/v1/messages` body. With `allow_raw` false every assistant entry is rebuilt (the
/// thinking-binding retry). When the request offers no tools there are no `tool_use` and
/// `tool_result` blocks in it, which Anthropic refuses without `tools`: calls and results are sent
/// as text and merge with the neighbouring messages of their role (see the module docs).
pub(crate) fn anthropic(req: &ChatRequest<'_>, allow_raw: bool) -> Built {
    let (turns, used_raw) = plan(req, Protocol::Anthropic, allow_raw);
    let mut messages: Vec<(Role, Vec<Value>)> = Vec::new();
    for turn in turns {
        match turn {
            Turn::User(text) => {
                if !text.trim().is_empty() {
                    push_blocks(
                        &mut messages,
                        Role::User,
                        vec![json!({"type": "text", "text": text})],
                    );
                }
            }
            Turn::Assistant { entry, raw: true } => {
                let blocks = entry.raw.as_array().cloned().unwrap_or_default();
                push_blocks(&mut messages, Role::Assistant, blocks);
            }
            Turn::Assistant { entry, raw: false } => {
                let mut blocks = Vec::new();
                if !entry.text.trim().is_empty() {
                    blocks.push(json!({"type": "text", "text": entry.text}));
                }
                for call in &entry.tool_calls {
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": arguments_object(&call.arguments),
                    }));
                }
                push_blocks(&mut messages, Role::Assistant, blocks);
            }
            Turn::AssistantText(text) => push_blocks(
                &mut messages,
                Role::Assistant,
                vec![json!({"type": "text", "text": text})],
            ),
            Turn::ToolResult {
                id,
                status,
                content,
            } => {
                let mut block = json!({
                    "type": "tool_result",
                    "tool_use_id": id,
                    "content": tool_result_text(status, content),
                });
                if status != ToolStatus::Ok {
                    block["is_error"] = json!(true);
                }
                push_blocks(&mut messages, Role::User, vec![block]);
            }
        }
    }
    let messages: Vec<Value> = messages
        .into_iter()
        .map(|(role, mut blocks)| {
            let role = match role {
                Role::User => {
                    // Tool results must open the user message that follows the tool calls.
                    let (mut results, rest): (Vec<Value>, Vec<Value>) = blocks
                        .into_iter()
                        .partition(|b| b.get("type") == Some(&json!("tool_result")));
                    results.extend(rest);
                    blocks = results;
                    "user"
                }
                Role::Assistant => "assistant",
            };
            json!({"role": role, "content": blocks})
        })
        .collect();

    let mut body = Map::new();
    body.insert("model".into(), json!(req.model.id));
    body.insert(
        "max_tokens".into(),
        json!(req.model.max_output_tokens.unwrap_or(DEFAULT_MAX_TOKENS)),
    );
    body.insert("cache_control".into(), json!({"type": "ephemeral"}));
    if !req.system.is_empty() {
        body.insert("system".into(), json!(req.system));
    }
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }
    body.insert("messages".into(), Value::Array(messages));
    body.insert("stream".into(), json!(true));
    Built {
        body: Value::Object(body),
        used_raw,
    }
}

/// Results that no call in `entries` pairs with (exposed for tests).
#[cfg(test)]
pub(crate) fn orphan_results(entries: &[AiEntry]) -> Vec<usize> {
    use std::collections::HashSet;

    let used: HashSet<usize> = pair_results(entries).into_values().collect();
    entries
        .iter()
        .enumerate()
        .filter(|(i, e)| matches!(e.body, EntryBody::Tool { .. }) && !used.contains(i))
        .map(|(i, _)| i)
        .collect()
}
