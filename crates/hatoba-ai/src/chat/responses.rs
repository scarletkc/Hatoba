//! OpenAI Responses stream assembly.
//!
//! The stream is typed events: `response.output_item.added` opens an item at its `output_index`,
//! deltas fill it (`response.output_text.delta`, `response.reasoning_summary_text.delta`,
//! `response.reasoning_text.delta`, `response.function_call_arguments.delta`), and
//! `response.output_item.done` closes it with its final value. A reasoning item that opens sends
//! [`StreamEvent::Thinking`], since its summary may never come. Only `response.completed` and
//! `response.incomplete` end a response; `response.failed` and `error` are errors, and a stream
//! that ends before either was interrupted.
//!
//! `raw` is the array of output items in output order, each as `response.output_item.done` sent
//! it, every field kept: a reasoning item carries `encrypted_content` when the request asked for
//! it. An item whose `done` never came is kept as `added` sent it, with the text and arguments
//! that arrived. A server that sends no item events has its items read from the terminal event's
//! `response.output`, and a whole response from a server that ignored `stream: true` likewise.
//! A function call without a `call_id` gets a fresh one, written into `raw` too.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::openai::synthetic_call_id;
use super::sse::SseEvent;
use super::{Assembled, Assembler, StreamEvent};
use crate::entry::{Finish, ToolCall, Usage};
use crate::error::AiError;
use crate::net::{clean_message, message_of};

/// Which part of an item reasoning text came from: a summary part, or a part of the raw
/// reasoning that open-weight models return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReasoningPart {
    Summary(u64),
    Content(u64),
}

#[derive(Debug, Default)]
struct Item {
    value: Map<String, Value>,
    /// Arguments from `response.function_call_arguments.*`, for a call whose `done` never came.
    arguments: String,
    /// Answer text from deltas, for a message whose `done` never came.
    text: String,
    /// Some of its text or reasoning arrived as deltas, so its `done` adds none.
    streamed_text: bool,
    streamed_reasoning: bool,
    /// `response.output_item.done` arrived.
    done: bool,
    /// Its tool call was emitted.
    emitted: bool,
}

impl Item {
    fn kind(&self) -> &str {
        self.value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
    }
}

fn str_at<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// The HTTP status an error `code` stands for.
fn status_of(code: &str) -> u16 {
    if let Some(status) = crate::chatgpt::status_of(code) {
        return status;
    }
    match code {
        "rate_limit_exceeded" | "insufficient_quota" => 429,
        "server_overloaded" | "overloaded" | "service_unavailable" => 503,
        "context_length_exceeded" => 400,
        code if code.starts_with("invalid_") => 400,
        _ => 500,
    }
}

/// [`AiError::Http`] for an `error` object: an `error` event's own fields, or `response.error`.
pub(crate) fn error_of(error: &Value) -> AiError {
    let error = error
        .get("error")
        .filter(|e| e.is_object())
        .unwrap_or(error);
    let status = ["status", "code"]
        .iter()
        .find_map(|k| error.get(*k).and_then(Value::as_u64))
        .filter(|s| (400..600).contains(s))
        .and_then(|s| u16::try_from(s).ok())
        .unwrap_or_else(|| status_of(str_at(error, "code")));
    let message = crate::chatgpt::explain(
        str_at(error, "code"),
        error.get("param").and_then(Value::as_str),
    )
    .or_else(|| message_of(&json!({ "error": error })))
    .unwrap_or_else(|| "the provider reported an error".into());
    AiError::Http { status, message }
}

/// Token usage of a `response.usage` object. `input_tokens` includes the cached ones.
fn usage_of(usage: &Value) -> Option<Usage> {
    let count = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| usage.get(*k).and_then(Value::as_u64))
    };
    let input = count(&["input_tokens", "prompt_tokens"]);
    let output = count(&["output_tokens", "completion_tokens"]);
    if input.is_none() && output.is_none() {
        return None;
    }
    Some(Usage {
        input_tokens: input.unwrap_or(0),
        output_tokens: output.unwrap_or(0),
        estimated: false,
        cache_read_tokens: usage
            .pointer("/input_tokens_details/cached_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_write_tokens: 0,
    })
}

/// Maps how a response ended: an incomplete one was cut off (`max_output_tokens`) or filtered
/// (`content_filter`); otherwise one with tool calls is `ToolCalls` and one with a refusal is
/// `Refused`.
pub(crate) fn finish_of(incomplete: Option<&str>, has_calls: bool, refused: bool) -> Finish {
    match incomplete {
        Some("content_filter") => Finish::Refused,
        Some(_) => Finish::Length,
        None if has_calls => Finish::ToolCalls,
        None if refused => Finish::Refused,
        None => Finish::Stop,
    }
}

/// Assembles one Responses response.
#[derive(Debug, Default)]
pub(crate) struct ResponsesStream {
    items: BTreeMap<u64, Item>,
    text: String,
    reasoning: String,
    last_text_item: Option<u64>,
    last_reasoning: Option<(u64, ReasoningPart)>,
    tool_calls: Vec<ToolCall>,
    refused: bool,
    /// `Some(reason)` once the response ended incomplete (`""` without a reason).
    incomplete: Option<String>,
    usage: Option<Usage>,
    done: bool,
}

impl ResponsesStream {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The output index an event refers to: its `output_index`, else the item its `item_id`
    /// names, else the newest item.
    fn index_of(&self, message: &Value) -> u64 {
        if let Some(index) = message.get("output_index").and_then(Value::as_u64) {
            return index;
        }
        let id = str_at(message, "item_id");
        let named = (!id.is_empty())
            .then(|| {
                self.items
                    .iter()
                    .find(|(_, item)| item.value.get("id").and_then(Value::as_str) == Some(id))
                    .map(|(index, _)| *index)
            })
            .flatten();
        named
            .or_else(|| self.items.keys().next_back().copied())
            .unwrap_or(0)
    }

    /// Appends answer text. Text of one message joins directly; text of another item starts
    /// after a blank line.
    fn push_text(&mut self, index: u64, text: &str, emit: &mut (dyn FnMut(StreamEvent) + Send)) {
        if text.is_empty() {
            return;
        }
        if !self.text.is_empty() && self.last_text_item != Some(index) {
            self.text.push_str("\n\n");
            emit(StreamEvent::Text("\n\n".into()));
        }
        self.last_text_item = Some(index);
        self.text.push_str(text);
        emit(StreamEvent::Text(text.to_owned()));
    }

    /// Appends reasoning. Each summary part, like each reasoning item, starts after a blank line.
    fn push_reasoning(
        &mut self,
        index: u64,
        part: ReasoningPart,
        text: &str,
        emit: &mut (dyn FnMut(StreamEvent) + Send),
    ) {
        if text.is_empty() {
            return;
        }
        if !self.reasoning.is_empty() && self.last_reasoning != Some((index, part)) {
            self.reasoning.push_str("\n\n");
            emit(StreamEvent::Reasoning("\n\n".into()));
        }
        self.last_reasoning = Some((index, part));
        self.reasoning.push_str(text);
        emit(StreamEvent::Reasoning(text.to_owned()));
    }

    /// An item's final value: the text and reasoning it holds that no delta brought, then its
    /// tool call.
    fn item_done(
        &mut self,
        index: u64,
        value: Map<String, Value>,
        emit: &mut (dyn FnMut(StreamEvent) + Send),
    ) {
        let item = self.items.entry(index).or_default();
        let (streamed_text, streamed_reasoning) = (item.streamed_text, item.streamed_reasoning);
        item.value = value;
        item.done = true;
        let value = Value::Object(item.value.clone());
        match str_at(&value, "type") {
            "message" => {
                for part in value
                    .get("content")
                    .and_then(Value::as_array)
                    .map_or(&[][..], Vec::as_slice)
                {
                    match str_at(part, "type") {
                        "output_text" if !streamed_text => {
                            self.push_text(index, str_at(part, "text"), emit);
                        }
                        "refusal" => self.refused = true,
                        _ => {}
                    }
                }
            }
            "reasoning" if !streamed_reasoning => {
                let parts = |key: &str| {
                    value
                        .get(key)
                        .and_then(Value::as_array)
                        .map_or(&[][..], Vec::as_slice)
                };
                for (i, part) in (0u64..).zip(parts("summary")) {
                    self.push_reasoning(
                        index,
                        ReasoningPart::Summary(i),
                        str_at(part, "text"),
                        emit,
                    );
                }
                for (i, part) in (0u64..).zip(parts("content")) {
                    self.push_reasoning(
                        index,
                        ReasoningPart::Content(i),
                        str_at(part, "text"),
                        emit,
                    );
                }
            }
            "function_call" => self.emit_call(index, emit),
            _ => {}
        }
    }

    /// Emits the tool call of a `function_call` item once, its `call_id` filled in when missing.
    fn emit_call(&mut self, index: u64, emit: &mut (dyn FnMut(StreamEvent) + Send)) {
        let position = self.tool_calls.len();
        let Some(item) = self.items.get_mut(&index) else {
            return;
        };
        if item.emitted || item.kind() != "function_call" {
            return;
        }
        item.emitted = true;
        let value = &mut item.value;
        let arguments = match value.get("arguments") {
            Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
            Some(object @ Value::Object(_)) => object.to_string(),
            _ if !item.arguments.trim().is_empty() => {
                value.insert("arguments".into(), json!(item.arguments));
                item.arguments.clone()
            }
            _ => "{}".to_owned(),
        };
        let id = match value.get("call_id").and_then(Value::as_str) {
            Some(id) if !id.is_empty() => id.to_owned(),
            _ => {
                let id = synthetic_call_id(position);
                value.insert("call_id".into(), json!(id));
                id
            }
        };
        // A call of a namespaced tool (the ChatGPT route, AI-52) names the tool by its own name
        // and the namespace apart; a name that comes qualified loses the namespace.
        let name = value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let name = match value.get("namespace").and_then(Value::as_str) {
            Some(namespace) if !namespace.is_empty() => name
                .strip_prefix(namespace)
                .and_then(|rest| rest.strip_prefix('.'))
                .unwrap_or(name),
            _ => name,
        };
        let call = ToolCall {
            id,
            name: name.to_owned(),
            arguments,
        };
        emit(StreamEvent::ToolCall(call.clone()));
        self.tool_calls.push(call);
    }

    /// The terminal event's response: its usage, whether it is incomplete, and its output when no
    /// item event brought any.
    fn end(
        &mut self,
        response: Option<&Value>,
        incomplete: bool,
        emit: &mut (dyn FnMut(StreamEvent) + Send),
    ) {
        let response = response.unwrap_or(&Value::Null);
        if let Some(usage) = response.get("usage").and_then(usage_of) {
            self.usage = Some(usage);
        }
        if incomplete || str_at(response, "status") == "incomplete" {
            self.incomplete = Some(
                response
                    .pointer("/incomplete_details/reason")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            );
        }
        if self.items.is_empty() {
            let output = response.get("output").and_then(Value::as_array);
            for (index, item) in (0u64..).zip(output.map_or(&[][..], Vec::as_slice)) {
                if let Value::Object(item) = item {
                    self.item_done(index, item.clone(), emit);
                }
            }
        }
        self.done = true;
    }

    fn on_message(
        &mut self,
        message: &Value,
        emit: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<(), AiError> {
        let delta = || str_at(message, "delta");
        match str_at(message, "type") {
            "response.output_item.added" => {
                if let Some(Value::Object(value)) = message.get("item") {
                    // The model thinks, whether or not a summary comes of it.
                    if value.get("type").and_then(Value::as_str) == Some("reasoning") {
                        emit(StreamEvent::Thinking);
                    }
                    let index = message
                        .get("output_index")
                        .and_then(Value::as_u64)
                        .unwrap_or_else(|| self.items.keys().next_back().map_or(0, |i| i + 1));
                    let item = self.items.entry(index).or_default();
                    if !item.done {
                        item.value = value.clone();
                    }
                }
            }
            "response.output_item.done" => {
                if let Some(Value::Object(value)) = message.get("item") {
                    let index = self.index_of(message);
                    self.item_done(index, value.clone(), emit);
                }
            }
            "response.output_text.delta" => {
                let index = self.index_of(message);
                let item = self.items.entry(index).or_default();
                item.streamed_text = true;
                item.text.push_str(delta());
                self.push_text(index, delta(), emit);
            }
            "response.refusal.delta" => self.refused = true,
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                let index = self.index_of(message);
                self.items.entry(index).or_default().streamed_reasoning = true;
                let part = if str_at(message, "type") == "response.reasoning_text.delta" {
                    ReasoningPart::Content(
                        message
                            .get("content_index")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                    )
                } else {
                    ReasoningPart::Summary(
                        message
                            .get("summary_index")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                    )
                };
                self.push_reasoning(index, part, delta(), emit);
            }
            "response.function_call_arguments.delta" => {
                let index = self.index_of(message);
                self.items
                    .entry(index)
                    .or_default()
                    .arguments
                    .push_str(delta());
            }
            "response.function_call_arguments.done" => {
                if let Some(arguments) = message.get("arguments").and_then(Value::as_str) {
                    let index = self.index_of(message);
                    arguments.clone_into(&mut self.items.entry(index).or_default().arguments);
                }
            }
            "response.completed" => self.end(message.get("response"), false, emit),
            "response.incomplete" => self.end(message.get("response"), true, emit),
            "response.failed" => {
                let error = message
                    .pointer("/response/error")
                    .filter(|e| e.is_object())
                    .unwrap_or(&Value::Null);
                return Err(if error.is_null() {
                    AiError::Http {
                        status: 500,
                        message: "the response failed".into(),
                    }
                } else {
                    error_of(error)
                });
            }
            "error" => return Err(error_of(message)),
            // `response.created`, `response.in_progress`, content and summary part events,
            // annotations, and event types added later.
            _ => {}
        }
        Ok(())
    }
}

impl Assembler for ResponsesStream {
    fn on_event(
        &mut self,
        event: &SseEvent,
        emit: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<(), AiError> {
        let data = event.data.trim();
        if data.is_empty() || data == "[DONE]" {
            return Ok(());
        }
        let parsed = serde_json::from_str::<Value>(data);
        let Ok(mut message) = parsed else {
            if event.event.as_deref() == Some("error") {
                return Err(AiError::Http {
                    status: 500,
                    message: clean_message(data),
                });
            }
            return Err(AiError::Protocol("a stream event is not valid JSON".into()));
        };
        // The event name stands in for a `type` the data leaves out.
        if message.get("type").is_none()
            && let (Some(name), Value::Object(map)) = (&event.event, &mut message)
        {
            map.insert("type".into(), json!(name));
        }
        self.on_message(&message, emit)
    }

    /// A whole response from a server that ignored `stream: true`.
    fn on_json(
        &mut self,
        body: Value,
        emit: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<(), AiError> {
        let error = body.get("error").filter(|e| e.is_object());
        if str_at(&body, "status") == "failed" || (error.is_some() && body.get("output").is_none())
        {
            return Err(error.map_or_else(
                || AiError::Http {
                    status: 500,
                    message: "the response failed".into(),
                },
                error_of,
            ));
        }
        self.end(Some(&body), false, emit);
        Ok(())
    }

    fn is_done(&self) -> bool {
        self.done
    }

    fn finish(mut self, emit: &mut (dyn FnMut(StreamEvent) + Send)) -> Result<Assembled, AiError> {
        if !self.done {
            return Err(AiError::Network(
                "the stream ended before the response was complete".into(),
            ));
        }
        let open: Vec<u64> = self
            .items
            .iter()
            .filter(|(_, item)| !item.emitted && item.kind() == "function_call")
            .map(|(index, _)| *index)
            .collect();
        for index in open {
            self.emit_call(index, emit);
        }
        let finish = finish_of(
            self.incomplete.as_deref(),
            !self.tool_calls.is_empty(),
            self.refused,
        );
        let raw = Value::Array(
            self.items
                .into_values()
                .map(|mut item| {
                    // A message whose `done` never came, or whose text came without an
                    // `added` (some servers send only the deltas).
                    if !item.done
                        && matches!(item.kind(), "message" | "")
                        && !item.text.is_empty()
                        && item
                            .value
                            .get("content")
                            .and_then(Value::as_array)
                            .is_none_or(Vec::is_empty)
                    {
                        item.value.insert("type".into(), json!("message"));
                        item.value
                            .entry("role")
                            .or_insert_with(|| json!("assistant"));
                        item.value.insert(
                            "content".into(),
                            json!([{"type": "output_text", "text": item.text, "annotations": []}]),
                        );
                    }
                    Value::Object(item.value)
                })
                .collect(),
        );
        Ok(Assembled {
            text: self.text,
            reasoning: self.reasoning,
            tool_calls: self.tool_calls,
            finish,
            usage: self.usage,
            raw,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_reasons_calls_and_refusals_map_to_finish() {
        assert_eq!(finish_of(None, false, false), Finish::Stop);
        assert_eq!(finish_of(None, true, false), Finish::ToolCalls);
        assert_eq!(finish_of(None, false, true), Finish::Refused);
        assert_eq!(
            finish_of(Some("max_output_tokens"), true, false),
            Finish::Length
        );
        assert_eq!(finish_of(Some(""), false, false), Finish::Length);
        assert_eq!(
            finish_of(Some("content_filter"), true, false),
            Finish::Refused
        );
    }

    #[test]
    fn error_codes_map_to_statuses() {
        let err = |value: Value| error_of(&value);
        assert_eq!(
            err(json!({"code": "rate_limit_exceeded", "message": "Slow down"})),
            AiError::Http {
                status: 429,
                message: "Slow down".into()
            }
        );
        assert_eq!(
            err(json!({"type": "error", "error": {"code": "invalid_prompt", "message": "No"}})),
            AiError::Http {
                status: 400,
                message: "No".into()
            }
        );
        assert_eq!(
            err(json!({"code": "server_error", "message": "Boom"})),
            AiError::Http {
                status: 500,
                message: "Boom".into()
            }
        );
        assert_eq!(
            err(json!({"status": 529, "message": "x"})).status(),
            Some(529)
        );
        assert_eq!(status_of("something_new"), 500);
    }
}
