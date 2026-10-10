//! Anthropic Messages stream assembly.
//!
//! `raw` is the content block array: each block starts as `content_block_start` sent it and
//! takes its deltas (`text_delta`, `thinking_delta`, `signature_delta`, `input_json_delta`,
//! `citations_delta`; unknown deltas are merged field by field), so thinking signatures,
//! redacted thinking and block types Hatoba does not know come back unchanged on replay.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::openai::merge_field;
use super::sse::SseEvent;
use super::{Assembled, Assembler, StreamEvent};
use crate::entry::{Finish, ToolCall, Usage};
use crate::error::AiError;
use crate::net::message_of;

#[derive(Debug, Default)]
struct Block {
    value: Map<String, Value>,
    /// `input_json_delta` fragments of a tool block, parsed at `content_block_stop`.
    partial_json: Option<String>,
    stopped: bool,
}

#[derive(Debug, Default)]
struct UsageCounts {
    input: u64,
    cache_creation: u64,
    cache_read: u64,
    output: u64,
    seen: bool,
}

impl UsageCounts {
    /// Takes the counts present in a `usage` object (`message_delta` counts are cumulative and
    /// override `message_start`'s).
    fn update(&mut self, usage: &Value) {
        for (key, slot) in [
            ("input_tokens", &mut self.input),
            ("cache_creation_input_tokens", &mut self.cache_creation),
            ("cache_read_input_tokens", &mut self.cache_read),
            ("output_tokens", &mut self.output),
        ] {
            if let Some(n) = usage.get(key).and_then(Value::as_u64) {
                *slot = n;
                self.seen = true;
            }
        }
    }
}

/// Whether text from block `index` needs a blank line before it: there is text already, and a
/// block of another kind came between it and this one.
fn needs_separator(so_far: &str, last: Option<u64>, index: u64) -> bool {
    !so_far.is_empty() && last.is_some_and(|last| last != index && last + 1 != index)
}

/// Maps `stop_reason`.
pub(crate) fn finish_of(reason: Option<&str>, has_calls: bool) -> Finish {
    match reason {
        Some("max_tokens" | "model_context_window_exceeded") => Finish::Length,
        Some("refusal") => Finish::Refused,
        _ if has_calls => Finish::ToolCalls,
        _ => Finish::Stop,
    }
}

/// The HTTP status an `error` event's type stands for.
fn status_of(error_type: &str) -> u16 {
    match error_type {
        "invalid_request_error" => 400,
        "authentication_error" => 401,
        "billing_error" => 402,
        "permission_error" => 403,
        "not_found_error" => 404,
        "request_too_large" => 413,
        "rate_limit_error" => 429,
        "overloaded_error" => 529,
        _ => 500,
    }
}

fn stream_error(event: &Value) -> AiError {
    let error = event.get("error").unwrap_or(event);
    AiError::Http {
        status: status_of(error.get("type").and_then(Value::as_str).unwrap_or("")),
        message: message_of(event).unwrap_or_else(|| "the provider reported an error".into()),
    }
}

/// Assembles one Anthropic Messages response.
#[derive(Debug, Default)]
pub(crate) struct AnthropicStream {
    blocks: BTreeMap<u64, Block>,
    text: String,
    reasoning: String,
    last_text_block: Option<u64>,
    last_reasoning_block: Option<u64>,
    tool_calls: Vec<ToolCall>,
    stop_reason: Option<String>,
    usage: UsageCounts,
    done: bool,
}

impl AnthropicStream {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Appends answer text. Adjacent text blocks (as citations produce) join directly; text
    /// after another kind of block (a tool call, thinking) starts after a blank line.
    fn push_text(&mut self, index: u64, text: &str, emit: &mut (dyn FnMut(StreamEvent) + Send)) {
        if text.is_empty() {
            return;
        }
        if needs_separator(&self.text, self.last_text_block, index) {
            self.text.push_str("\n\n");
            emit(StreamEvent::Text("\n\n".into()));
        }
        self.last_text_block = Some(index);
        self.text.push_str(text);
        emit(StreamEvent::Text(text.to_owned()));
    }

    /// Appends reasoning, like [`Self::push_text`].
    fn push_reasoning(
        &mut self,
        index: u64,
        text: &str,
        emit: &mut (dyn FnMut(StreamEvent) + Send),
    ) {
        if text.is_empty() {
            return;
        }
        if needs_separator(&self.reasoning, self.last_reasoning_block, index) {
            self.reasoning.push_str("\n\n");
            emit(StreamEvent::Reasoning("\n\n".into()));
        }
        self.last_reasoning_block = Some(index);
        self.reasoning.push_str(text);
        emit(StreamEvent::Reasoning(text.to_owned()));
    }

    fn start_block(
        &mut self,
        index: u64,
        block: Map<String, Value>,
        emit: &mut (dyn FnMut(StreamEvent) + Send),
    ) {
        let kind = block.get("type").and_then(Value::as_str).unwrap_or("");
        let initial = match kind {
            "text" => block.get("text"),
            "thinking" => block.get("thinking"),
            _ => None,
        }
        .and_then(Value::as_str)
        .map(str::to_owned);
        let is_text = kind == "text";
        self.blocks.insert(
            index,
            Block {
                value: block,
                ..Block::default()
            },
        );
        if let Some(initial) = initial {
            if is_text {
                self.push_text(index, &initial, emit);
            } else {
                self.push_reasoning(index, &initial, emit);
            }
        }
    }

    fn delta(
        &mut self,
        index: u64,
        delta: &Map<String, Value>,
        emit: &mut (dyn FnMut(StreamEvent) + Send),
    ) {
        let block = self.blocks.entry(index).or_default();
        let str_of = |key: &str| {
            delta
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        match delta.get("type").and_then(Value::as_str).unwrap_or("") {
            "text_delta" => {
                let text = str_of("text");
                merge_field(&mut block.value, "text", &json!(text));
                self.push_text(index, &text, emit);
            }
            "thinking_delta" => {
                let thinking = str_of("thinking");
                merge_field(&mut block.value, "thinking", &json!(thinking));
                self.push_reasoning(index, &thinking, emit);
            }
            "signature_delta" => {
                merge_field(&mut block.value, "signature", &json!(str_of("signature")));
            }
            "input_json_delta" => {
                block
                    .partial_json
                    .get_or_insert_with(String::new)
                    .push_str(&str_of("partial_json"));
            }
            "citations_delta" => {
                if let Some(citation) = delta.get("citation") {
                    merge_field(&mut block.value, "citations", &json!([citation]));
                }
            }
            _ => {
                for (key, value) in delta {
                    if key != "type" {
                        merge_field(&mut block.value, key, value);
                    }
                }
            }
        }
    }

    fn stop_block(&mut self, index: u64, emit: &mut (dyn FnMut(StreamEvent) + Send)) {
        let Some(block) = self.blocks.get_mut(&index) else {
            return;
        };
        if block.stopped {
            return;
        }
        block.stopped = true;
        let mut arguments = None;
        if let Some(partial) = block.partial_json.take() {
            let input = if partial.trim().is_empty() {
                json!({})
            } else {
                match serde_json::from_str::<Value>(&partial) {
                    Ok(value @ Value::Object(_)) => value,
                    // Cut off mid-stream: `raw` needs an object; the text stays in `arguments`.
                    _ => json!({}),
                }
            };
            block.value.insert("input".into(), input);
            if !partial.trim().is_empty() {
                arguments = Some(partial);
            }
        }
        if block.value.get("type").and_then(Value::as_str) != Some("tool_use") {
            return;
        }
        let field = |key: &str| {
            block
                .value
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let call = ToolCall {
            id: field("id"),
            name: field("name"),
            arguments: arguments.unwrap_or_else(|| {
                block
                    .value
                    .get("input")
                    .filter(|v| v.is_object())
                    .map_or_else(|| "{}".to_owned(), Value::to_string)
            }),
        };
        emit(StreamEvent::ToolCall(call.clone()));
        self.tool_calls.push(call);
    }

    fn on_message(
        &mut self,
        message: &Value,
        emit: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<(), AiError> {
        let index = || message.get("index").and_then(Value::as_u64);
        match message.get("type").and_then(Value::as_str).unwrap_or("") {
            "message_start" => {
                if let Some(usage) = message.pointer("/message/usage") {
                    self.usage.update(usage);
                }
            }
            "content_block_start" => {
                let index = index().unwrap_or(self.blocks.len() as u64);
                let block = message
                    .get("content_block")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                self.start_block(index, block, emit);
            }
            "content_block_delta" => {
                if let (Some(index), Some(Value::Object(delta))) = (index(), message.get("delta")) {
                    self.delta(index, delta, emit);
                }
            }
            "content_block_stop" => {
                if let Some(index) = index() {
                    self.stop_block(index, emit);
                }
            }
            "message_delta" => {
                if let Some(reason) = message
                    .pointer("/delta/stop_reason")
                    .and_then(Value::as_str)
                {
                    self.stop_reason = Some(reason.to_owned());
                }
                if let Some(usage) = message.get("usage") {
                    self.usage.update(usage);
                }
            }
            "message_stop" => self.done = true,
            "error" => return Err(stream_error(message)),
            // `ping` and event types added later.
            _ => {}
        }
        Ok(())
    }
}

impl Assembler for AnthropicStream {
    fn on_event(
        &mut self,
        event: &SseEvent,
        emit: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<(), AiError> {
        let data = event.data.trim();
        if data.is_empty() {
            return Ok(());
        }
        let message: Value = serde_json::from_str(data)
            .map_err(|_| AiError::Protocol("a stream event is not valid JSON".into()))?;
        self.on_message(&message, emit)
    }

    /// A whole message from a server that ignored `stream: true`.
    fn on_json(
        &mut self,
        body: Value,
        emit: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<(), AiError> {
        if body.get("type").and_then(Value::as_str) == Some("error") {
            return Err(stream_error(&body));
        }
        if let Some(usage) = body.get("usage") {
            self.usage.update(usage);
        }
        if let Some(blocks) = body.get("content").and_then(Value::as_array) {
            for (index, block) in (0u64..).zip(blocks) {
                let block = block.as_object().cloned().unwrap_or_default();
                self.start_block(index, block, emit);
                self.stop_block(index, emit);
            }
        }
        self.stop_reason = body
            .get("stop_reason")
            .and_then(Value::as_str)
            .map(str::to_owned);
        self.done = true;
        Ok(())
    }

    fn is_done(&self) -> bool {
        self.done
    }

    fn finish(mut self, emit: &mut (dyn FnMut(StreamEvent) + Send)) -> Result<Assembled, AiError> {
        if !self.done && self.stop_reason.is_none() {
            return Err(AiError::Network(
                "the stream ended before the response was complete".into(),
            ));
        }
        let open: Vec<u64> = self
            .blocks
            .iter()
            .filter(|(_, b)| !b.stopped)
            .map(|(i, _)| *i)
            .collect();
        for index in open {
            self.stop_block(index, emit);
        }
        let finish = finish_of(self.stop_reason.as_deref(), !self.tool_calls.is_empty());
        let usage = self.usage.seen.then(|| Usage {
            input_tokens: self.usage.input + self.usage.cache_creation + self.usage.cache_read,
            output_tokens: self.usage.output,
            estimated: false,
            cache_read_tokens: self.usage.cache_read,
            cache_write_tokens: self.usage.cache_creation,
        });
        let raw = Value::Array(
            self.blocks
                .into_values()
                .map(|b| Value::Object(b.value))
                .collect(),
        );
        Ok(Assembled {
            text: self.text,
            reasoning: self.reasoning,
            tool_calls: self.tool_calls,
            finish,
            usage,
            raw,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_reasons_map_to_finish() {
        assert_eq!(finish_of(Some("end_turn"), false), Finish::Stop);
        assert_eq!(finish_of(Some("stop_sequence"), false), Finish::Stop);
        assert_eq!(finish_of(Some("pause_turn"), false), Finish::Stop);
        assert_eq!(finish_of(Some("tool_use"), true), Finish::ToolCalls);
        assert_eq!(finish_of(Some("end_turn"), true), Finish::ToolCalls);
        assert_eq!(finish_of(Some("max_tokens"), true), Finish::Length);
        assert_eq!(
            finish_of(Some("model_context_window_exceeded"), false),
            Finish::Length
        );
        assert_eq!(finish_of(Some("refusal"), false), Finish::Refused);
        assert_eq!(finish_of(None, false), Finish::Stop);
    }

    #[test]
    fn error_types_map_to_statuses() {
        let err = stream_error(&json!({
            "type": "error",
            "error": {"type": "overloaded_error", "message": "Overloaded"}
        }));
        assert_eq!(
            err,
            AiError::Http {
                status: 529,
                message: "Overloaded".into()
            }
        );
        assert_eq!(status_of("rate_limit_error"), 429);
        assert_eq!(status_of("something_new"), 500);
    }
}
