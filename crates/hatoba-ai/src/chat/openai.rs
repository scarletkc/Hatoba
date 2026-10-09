//! Chat Completions stream assembly.
//!
//! `raw` is the assistant message object that the deltas add up to, every field kept: string
//! fields are concatenated across deltas, tool calls are merged by `index`, objects are merged
//! recursively and arrays extended. Identifier fields (`id`, `type`, `name`) that a server
//! repeats in every delta are kept once. The streaming-only `index` of tool calls is removed at
//! the end, and a missing `role`, call `type` or call `id` is filled in, so `raw` is a valid
//! message to replay.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use super::sse::SseEvent;
use super::{Assembled, Assembler, StreamEvent};
use crate::entry::{Finish, ToolCall, Usage};
use crate::error::AiError;
use crate::net::{clean_message, message_of};

/// Keys whose repeated delta values are the same identifier, not fragments to concatenate.
fn is_identifier(key: &str) -> bool {
    matches!(key, "id" | "type" | "name")
}

/// Merges one delta field into `target`.
pub(crate) fn merge_field(target: &mut Map<String, Value>, key: &str, value: &Value) {
    if key == "role" {
        if value.is_string() {
            target.insert(key.to_owned(), value.clone());
        }
        return;
    }
    let Some(slot) = target.get_mut(key) else {
        target.insert(key.to_owned(), value.clone());
        return;
    };
    match (slot, value) {
        (_, Value::Null) => {}
        (Value::String(existing), Value::String(delta)) => {
            // A repeated identifier is the same value again, not a fragment.
            if !(is_identifier(key) && existing == delta) {
                existing.push_str(delta);
            }
        }
        (Value::Object(existing), Value::Object(delta)) => merge_object(existing, delta),
        (Value::Array(existing), Value::Array(delta)) => existing.extend(delta.iter().cloned()),
        (slot, value) => *slot = value.clone(),
    }
}

/// Merges a delta object into `target` (tool calls by `index`).
pub(crate) fn merge_object(target: &mut Map<String, Value>, delta: &Map<String, Value>) {
    for (key, value) in delta {
        match (key.as_str(), value) {
            ("tool_calls", Value::Array(calls)) => merge_tool_calls(target, calls),
            _ => merge_field(target, key, value),
        }
    }
}

/// Merges tool call deltas: by `index`; without one, by `id`; without either, into the last call
/// unless the delta starts a new named call.
fn merge_tool_calls(target: &mut Map<String, Value>, calls: &[Value]) {
    let list = target
        .entry("tool_calls")
        .or_insert_with(|| Value::Array(Vec::new()));
    if !list.is_array() {
        *list = Value::Array(Vec::new());
    }
    let Value::Array(list) = list else {
        return;
    };
    for call in calls {
        let Value::Object(delta) = call else {
            continue;
        };
        let position = if let Some(index) = delta.get("index").and_then(Value::as_u64) {
            list.iter()
                .position(|c| c.get("index").and_then(Value::as_u64) == Some(index))
        } else if let Some(id) = delta
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        {
            list.iter()
                .position(|c| c.get("id").and_then(Value::as_str) == Some(id))
        } else {
            let starts_new = delta
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .is_some_and(|n| !n.is_empty())
                && list
                    .last()
                    .and_then(|c| c.pointer("/function/name"))
                    .and_then(Value::as_str)
                    .is_some_and(|n| !n.is_empty());
            if starts_new {
                None
            } else {
                list.len().checked_sub(1)
            }
        };
        match position.and_then(|p| list.get_mut(p)) {
            Some(Value::Object(existing)) => merge_object(existing, delta),
            _ => list.push(Value::Object(delta.clone())),
        }
    }
}

/// [`AiError::Http`] for an `error` object inside a stream chunk.
fn stream_error(error: &Value) -> AiError {
    let status = ["code", "status"]
        .iter()
        .find_map(|k| error.get(*k).and_then(Value::as_u64))
        .filter(|s| (400..600).contains(s))
        .unwrap_or(500);
    let message = message_of(&json!({ "error": error }))
        .or_else(|| error.as_str().map(str::to_owned))
        .unwrap_or_else(|| "the provider reported an error".into());
    AiError::Http {
        status: u16::try_from(status).unwrap_or(500),
        message,
    }
}

fn first_u64(object: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|k| object.get(*k).and_then(Value::as_u64))
}

/// Maps `finish_reason` (AI-06, §13.7): a response with tool calls is `ToolCalls` unless it was
/// cut off or filtered, whatever reason the server gave.
pub(crate) fn finish_of(reason: Option<&str>, has_calls: bool, refusal: bool) -> Finish {
    match reason {
        Some("length") => Finish::Length,
        Some("content_filter") => Finish::Refused,
        _ if has_calls => Finish::ToolCalls,
        _ if refusal => Finish::Refused,
        _ => Finish::Stop,
    }
}

/// A fresh call id for servers that send none, unique enough that results never pair with the
/// wrong call.
fn synthetic_call_id(position: usize) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("call_{nanos:x}_{position}")
}

/// Assembles one Chat Completions response.
#[derive(Debug, Default)]
pub(crate) struct ChatCompletionsStream {
    message: Map<String, Value>,
    text: String,
    reasoning: String,
    finish_reason: Option<String>,
    usage: Option<Usage>,
    done: bool,
}

impl ChatCompletionsStream {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn on_chunk(
        &mut self,
        chunk: &Value,
        emit: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<(), AiError> {
        if let Some(error) = chunk.get("error").filter(|e| !e.is_null()) {
            return Err(stream_error(error));
        }
        // vLLM-style `{"object": "error", "message": …, "code": …}`.
        if chunk.get("object").and_then(Value::as_str) == Some("error") {
            return Err(stream_error(chunk));
        }
        if let Some(usage) = chunk.get("usage").filter(|u| u.is_object()) {
            self.usage = Some(Usage {
                input_tokens: first_u64(usage, &["prompt_tokens", "input_tokens"]).unwrap_or(0),
                output_tokens: first_u64(usage, &["completion_tokens", "output_tokens"])
                    .unwrap_or(0),
                estimated: false,
            });
        }
        let Some(choices) = chunk.get("choices").and_then(Value::as_array) else {
            return Ok(());
        };
        let choice = choices
            .iter()
            .find(|c| c.get("index").and_then(Value::as_u64).unwrap_or(0) == 0);
        let Some(choice) = choice else {
            return Ok(());
        };
        if let Some(Value::Object(delta)) = choice.get("delta").or_else(|| choice.get("message")) {
            if let Some(text) = delta
                .get("content")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                self.text.push_str(text);
                emit(StreamEvent::Text(text.to_owned()));
            }
            // Servers send `reasoning_content` (DeepSeek, Qwen, vLLM) or `reasoning` (OpenRouter,
            // Ollama, newer vLLM); some send both with the same text.
            let reasoning = ["reasoning_content", "reasoning"].iter().find_map(|k| {
                delta
                    .get(*k)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
            });
            if let Some(reasoning) = reasoning {
                self.reasoning.push_str(reasoning);
                emit(StreamEvent::Reasoning(reasoning.to_owned()));
            }
            merge_object(&mut self.message, delta);
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(reason.to_owned());
        }
        Ok(())
    }
}

impl Assembler for ChatCompletionsStream {
    fn on_event(
        &mut self,
        event: &SseEvent,
        emit: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<(), AiError> {
        let data = event.data.trim();
        if data.is_empty() {
            return Ok(());
        }
        if data == "[DONE]" {
            self.done = true;
            return Ok(());
        }
        let parsed = serde_json::from_str::<Value>(data);
        if event.event.as_deref() == Some("error") {
            return Err(match parsed {
                Ok(value) => stream_error(value.get("error").unwrap_or(&value)),
                Err(_) => AiError::Http {
                    status: 500,
                    message: clean_message(data),
                },
            });
        }
        let chunk =
            parsed.map_err(|_| AiError::Protocol("a stream chunk is not valid JSON".into()))?;
        self.on_chunk(&chunk, emit)
    }

    fn on_json(
        &mut self,
        body: Value,
        emit: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<(), AiError> {
        self.on_chunk(&body, emit)?;
        self.done = true;
        Ok(())
    }

    fn is_done(&self) -> bool {
        self.done
    }

    fn finish(self, emit: &mut (dyn FnMut(StreamEvent) + Send)) -> Result<Assembled, AiError> {
        if !self.done && self.finish_reason.is_none() {
            return Err(AiError::Network(
                "the stream ended before the response was complete".into(),
            ));
        }
        let mut raw = self.message;
        raw.insert("role".into(), json!("assistant"));
        let mut tool_calls = Vec::new();
        if let Some(Value::Array(calls)) = raw.get_mut("tool_calls") {
            for (position, call) in calls.iter_mut().enumerate() {
                let Value::Object(call) = call else {
                    continue;
                };
                call.remove("index");
                let id = match call.get("id").and_then(Value::as_str) {
                    Some(id) if !id.is_empty() => id.to_owned(),
                    _ => {
                        let id = synthetic_call_id(position);
                        call.insert("id".into(), json!(id));
                        id
                    }
                };
                call.entry("type").or_insert_with(|| json!("function"));
                let function = call.get("function");
                let name = function
                    .and_then(|f| f.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let arguments = match function.and_then(|f| f.get("arguments")) {
                    Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
                    Some(value @ Value::Object(_)) => value.to_string(),
                    _ => "{}".to_owned(),
                };
                let call = ToolCall {
                    id,
                    name,
                    arguments,
                };
                emit(StreamEvent::ToolCall(call.clone()));
                tool_calls.push(call);
            }
        }
        let refusal = raw
            .get("refusal")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty());
        let finish = finish_of(
            self.finish_reason.as_deref(),
            !tool_calls.is_empty(),
            refusal,
        );
        Ok(Assembled {
            text: self.text,
            reasoning: self.reasoning,
            tool_calls,
            finish,
            usage: self.usage,
            raw: Value::Object(raw),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(event: Option<&str>, data: &str) -> Result<(), AiError> {
        let mut stream = ChatCompletionsStream::new();
        stream.on_event(
            &SseEvent {
                event: event.map(str::to_owned),
                data: data.to_owned(),
            },
            &mut |_| {},
        )
    }

    #[test]
    fn error_chunks_in_their_various_shapes() {
        let http = |status, message: &str| {
            Err(AiError::Http {
                status,
                message: message.into(),
            })
        };
        assert_eq!(
            feed(None, r#"{"error":{"message":"overloaded","code":529}}"#),
            http(529, "overloaded")
        );
        assert_eq!(
            feed(None, r#"{"error":{"message":"bad","code":"server_error"}}"#),
            http(500, "bad")
        );
        assert_eq!(
            feed(
                None,
                r#"{"object":"error","message":"context too long","code":400}"#
            ),
            http(400, "context too long")
        );
        assert_eq!(
            feed(
                Some("error"),
                r#"{"message":"quota exceeded","status":429}"#
            ),
            http(429, "quota exceeded")
        );
        assert_eq!(
            feed(Some("error"), "upstream\nreset"),
            http(500, "upstream reset")
        );
        assert!(matches!(feed(None, "{not json"), Err(AiError::Protocol(_))));
        assert_eq!(feed(None, "  "), Ok(()));
    }

    fn merged(deltas: &[Value]) -> Value {
        let mut target = Map::new();
        for delta in deltas {
            merge_object(&mut target, delta.as_object().unwrap());
        }
        Value::Object(target)
    }

    #[test]
    fn strings_concatenate_and_identifiers_do_not() {
        let out = merged(&[
            json!({"role": "assistant", "content": "Hel", "refusal": null}),
            json!({"role": "assistant", "content": "lo", "x_vendor": {"sig": "ab"}}),
            json!({"content": null, "x_vendor": {"sig": "cd", "n": 1}}),
        ]);
        assert_eq!(
            out,
            json!({"role": "assistant", "content": "Hello", "refusal": null,
                   "x_vendor": {"sig": "abcd", "n": 1}})
        );
    }

    #[test]
    fn tool_calls_merge_by_index_id_or_position() {
        let by_index = merged(&[
            json!({"tool_calls": [{"index": 0, "id": "a", "type": "function", "function": {"name": "f", "arguments": ""}}]}),
            json!({"tool_calls": [{"index": 1, "id": "b", "type": "function", "function": {"name": "g", "arguments": "{\"y\""}}]}),
            json!({"tool_calls": [{"index": 0, "function": {"arguments": "{\"x\":1}"}}]}),
            json!({"tool_calls": [{"index": 1, "id": "b", "function": {"name": "g", "arguments": ":2}"}}]}),
        ]);
        assert_eq!(
            by_index["tool_calls"],
            json!([
                {"index": 0, "id": "a", "type": "function", "function": {"name": "f", "arguments": "{\"x\":1}"}},
                {"index": 1, "id": "b", "type": "function", "function": {"name": "g", "arguments": "{\"y\":2}"}}
            ])
        );

        let no_index = merged(&[
            json!({"tool_calls": [{"id": "a", "function": {"name": "f", "arguments": "{}"}}]}),
            json!({"tool_calls": [{"id": "b", "function": {"name": "g", "arguments": "{\"q\""}}]}),
            json!({"tool_calls": [{"function": {"arguments": ":1}"}}]}),
            json!({"tool_calls": [{"function": {"name": "h", "arguments": "{}"}}]}),
        ]);
        let calls = no_index["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[1]["function"]["arguments"], "{\"q\":1}");
        assert_eq!(calls[2]["function"]["name"], "h");
    }

    #[test]
    fn identifiers_split_into_fragments_still_join() {
        let out = merged(&[
            json!({"tool_calls": [{"index": 0, "function": {"name": "get_"}}]}),
            json!({"tool_calls": [{"index": 0, "function": {"name": "weather"}}]}),
        ]);
        assert_eq!(out["tool_calls"][0]["function"]["name"], "get_weather");
    }

    #[test]
    fn finish_reasons_map_to_finish() {
        assert_eq!(finish_of(Some("stop"), false, false), Finish::Stop);
        assert_eq!(finish_of(Some("stop"), true, false), Finish::ToolCalls);
        assert_eq!(finish_of(None, true, false), Finish::ToolCalls);
        assert_eq!(
            finish_of(Some("tool_calls"), true, false),
            Finish::ToolCalls
        );
        assert_eq!(
            finish_of(Some("function_call"), true, false),
            Finish::ToolCalls
        );
        assert_eq!(finish_of(Some("length"), true, false), Finish::Length);
        assert_eq!(
            finish_of(Some("content_filter"), false, false),
            Finish::Refused
        );
        assert_eq!(finish_of(Some("stop"), false, true), Finish::Refused);
        assert_eq!(finish_of(Some("eos"), false, false), Finish::Stop);
        assert_eq!(finish_of(Some("tool_calls"), false, false), Finish::Stop);
    }
}
