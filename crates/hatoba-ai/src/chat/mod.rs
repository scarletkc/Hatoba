//! The two protocol adapters (spec §13.2): requests built from stored entries, streamed
//! responses turned into the same [`StreamEvent`]s and the same [`AssistantEntry`].
//!
//! - Chat Completions: `POST {base_url}/chat/completions` with `Authorization: Bearer` (none when
//!   the key is empty), `stream: true` and `stream_options.include_usage`. A server that rejects
//!   `stream_options` gets the request again without it, and usage is then estimated.
//! - Anthropic Messages: `POST {base_url}/v1/messages` with `anthropic-version: 2023-06-01`, the
//!   key in `x-api-key` or `Authorization: Bearer`, `max_tokens` from the model (16,000 when
//!   unknown) and top-level `cache_control`. No `thinking`, `temperature`, beta headers or
//!   `eager_input_streaming`, so any Anthropic-compatible vendor accepts the request. When a
//!   request that replayed `raw` is refused with a 400 about thinking, signatures or blocks
//!   (newer Claude models bind signed thinking to the exact prefix), it is sent once more with
//!   every assistant entry rebuilt from its fields.
//! - A request that offers no tools (no terminal tab attached, AI-09; Compact, AI-21) contains no
//!   `tool_use` / `tool_result` blocks, `tool_calls` or `tool` messages, whatever the history
//!   holds: calls and results are sent as text, an assistant entry's calls after its text and
//!   each result as a `user` message that names its call (see `request`). Anthropic refuses
//!   such blocks without `tools`, and so do several Chat Completions servers.
//!
//! Neither adapter logs a request, a response or the key (SEC-04).

mod anthropic;
mod openai;
mod request;
mod sse;

#[cfg(test)]
mod tests;

use futures::StreamExt;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use self::anthropic::AnthropicStream;
use self::openai::ChatCompletionsStream;
use self::sse::{SseEvent, SseParser};
use crate::entry::{AiEntry, AssistantEntry, Finish, ToolCall, Usage};
use crate::error::AiError;
use crate::net;
use crate::provider::{ModelSpec, Protocol, ProviderConfig, validate_base_url};

/// A tool offered to the model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    /// Tool name, e.g. `run_command`.
    pub name: String,
    /// What the tool does and its limits, for the model.
    pub description: String,
    /// JSON Schema of the arguments (an object schema).
    pub input_schema: serde_json::Value,
}

/// One request: the system prompt, the tools and the entries to send. Its `Debug` shows counts,
/// never conversation content (SEC-04).
#[derive(Clone, Copy)]
pub struct ChatRequest<'a> {
    /// The provider item id, compared with [`AssistantEntry::provider_id`] for `raw` replay.
    pub provider_id: &'a str,
    /// The model to call.
    pub model: &'a ModelSpec,
    /// The system prompt ([`system_prompt`](crate::tools::system_prompt)); empty sends none.
    pub system: &'a str,
    /// Tools to offer; empty sends no `tools` field, and then the request has no structured tool
    /// blocks either: the calls and results already in `entries` are sent as plain text, because
    /// providers refuse `tool_use` / `tool_result` blocks and `tool_calls` / `tool` messages
    /// without tools. The desktop shell sends none with no terminal tab attached (AI-09) and for
    /// Compact (AI-21).
    pub tools: &'a [ToolDef],
    /// The conversation from `context_start` on. Calls without a result get a cancelled one in
    /// the request.
    pub entries: &'a [AiEntry],
}

impl std::fmt::Debug for ChatRequest<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatRequest")
            .field("provider_id", &self.provider_id)
            .field("model", &self.model)
            .field("system_chars", &self.system.chars().count())
            .field("tools", &self.tools.len())
            .field("entries", &self.entries.len())
            .finish()
    }
}

/// A streamed piece of a response, the same for both protocols (spec §13.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamEvent {
    /// Answer text to append.
    Text(String),
    /// Reasoning text to append (AI-06).
    Reasoning(String),
    /// A tool call whose arguments are complete.
    ToolCall(ToolCall),
    /// The response's token usage, sent once at the end.
    Usage(Usage),
}

/// What a protocol assembler hands back at the end of a response.
#[derive(Debug, PartialEq)]
pub(crate) struct Assembled {
    pub text: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish: Finish,
    /// Usage the provider reported, if any.
    pub usage: Option<Usage>,
    pub raw: serde_json::Value,
}

/// A protocol's stream state machine, testable without HTTP.
pub(crate) trait Assembler {
    /// One server-sent event.
    fn on_event(
        &mut self,
        event: &SseEvent,
        emit: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<(), AiError>;
    /// A whole JSON response from a server that ignored `stream: true`.
    fn on_json(
        &mut self,
        body: serde_json::Value,
        emit: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<(), AiError>;
    /// The response is complete (`[DONE]`, `message_stop`); the rest of the stream is ignored.
    fn is_done(&self) -> bool;
    /// Ends the response.
    fn finish(self, emit: &mut (dyn FnMut(StreamEvent) + Send)) -> Result<Assembled, AiError>;
}

/// Streams one response and returns the assembled assistant entry: `provider_id` and `model_id`
/// from the request, `raw` with unknown fields kept, `usage` (estimated from text length when the
/// provider reported none) and `finish`.
///
/// Events arrive in order through `on_event`; [`StreamEvent::Usage`] comes last. When `cancel`
/// fires, the connection is dropped and [`AiError::Cancelled`] returned promptly.
pub async fn stream_chat(
    http: &reqwest::Client,
    provider: &ProviderConfig,
    req: &ChatRequest<'_>,
    cancel: &CancellationToken,
    on_event: &mut (dyn FnMut(StreamEvent) + Send),
) -> Result<AssistantEntry, AiError> {
    let base = tokio::select! {
        biased;
        () = cancel.cancelled() => return Err(AiError::Cancelled),
        base = validate_base_url(&provider.base_url) => base?,
    };
    let headers = provider.headers()?;
    match provider.protocol {
        Protocol::ChatCompletions => {
            let url = net::endpoint(&base, "/chat/completions");
            let mut stream_options = true;
            loop {
                let built = request::chat_completions(req, stream_options);
                let (response, request_chars) = post(http, &url, &headers, &built, cancel).await?;
                if !response.status().is_success() {
                    let err = net::http_error(response, cancel).await;
                    if stream_options && rejects_stream_options(&err) {
                        stream_options = false;
                        continue;
                    }
                    return Err(err);
                }
                let assembled =
                    consume(response, ChatCompletionsStream::new(), cancel, on_event).await?;
                return Ok(into_entry(req, assembled, request_chars, on_event));
            }
        }
        Protocol::Anthropic => {
            let url = net::endpoint(&base, "/v1/messages");
            let mut allow_raw = true;
            loop {
                let built = request::anthropic(req, allow_raw);
                let (response, request_chars) = post(http, &url, &headers, &built, cancel).await?;
                if !response.status().is_success() {
                    let err = net::http_error(response, cancel).await;
                    if built.used_raw && rejects_replayed_raw(&err) {
                        allow_raw = false;
                        continue;
                    }
                    return Err(err);
                }
                let assembled = consume(response, AnthropicStream::new(), cancel, on_event).await?;
                return Ok(into_entry(req, assembled, request_chars, on_event));
            }
        }
    }
}

/// [`stream_chat`] without the deltas, for Compact (AI-21). It still streams, so a long summary
/// never hits a provider's limit on non-streaming requests.
pub async fn complete(
    http: &reqwest::Client,
    provider: &ProviderConfig,
    req: &ChatRequest<'_>,
    cancel: &CancellationToken,
) -> Result<AssistantEntry, AiError> {
    stream_chat(http, provider, req, cancel, &mut |_| {}).await
}

/// Whether a Chat Completions error says the server does not accept `stream_options`.
pub(crate) fn rejects_stream_options(err: &AiError) -> bool {
    match err {
        AiError::Http { status, message } if (400..500).contains(status) => {
            let message = message.to_ascii_lowercase();
            message.contains("stream_options")
                || message.contains("include_usage")
                || message.contains("stream options")
        }
        _ => false,
    }
}

/// Whether an Anthropic error is the 400 that newer Claude models return when replayed thinking
/// blocks no longer match the request's prefix (or any other complaint about replayed blocks):
/// the request is then retried once without `raw`.
pub(crate) fn rejects_replayed_raw(err: &AiError) -> bool {
    match err {
        AiError::Http {
            status: 400,
            message,
        } => {
            let message = message.to_ascii_lowercase();
            ["thinking", "signature", "block"]
                .iter()
                .any(|word| message.contains(word))
        }
        _ => false,
    }
}

async fn post(
    http: &reqwest::Client,
    url: &url::Url,
    headers: &HeaderMap,
    built: &request::Built,
    cancel: &CancellationToken,
) -> Result<(reqwest::Response, usize), AiError> {
    let body = serde_json::to_string(&built.body)
        .map_err(|_| AiError::Protocol("the request could not be encoded".into()))?;
    let request_chars = body.chars().count();
    let request = http
        .post(url.clone())
        .headers(headers.clone())
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(body);
    Ok((net::send(request, cancel).await?, request_chars))
}

/// Reads a successful response through an assembler, observing `cancel` between chunks.
async fn consume<A: Assembler>(
    response: reqwest::Response,
    mut assembler: A,
    cancel: &CancellationToken,
    on_event: &mut (dyn FnMut(StreamEvent) + Send),
) -> Result<Assembled, AiError> {
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if content_type.starts_with("application/json") {
        let body = net::read_json(response, cancel).await?;
        assembler.on_json(body, on_event)?;
        return assembler.finish(on_event);
    }
    let mut parser = SseParser::new();
    let mut events = Vec::new();
    let mut stream = response.bytes_stream();
    while !assembler.is_done() {
        let next = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(AiError::Cancelled),
            next = stream.next() => next,
        };
        let ended = match next {
            Some(Ok(chunk)) => {
                parser.push(&chunk, &mut events)?;
                false
            }
            Some(Err(err)) => return Err(net::network_error(&err)),
            None => {
                parser.finish(&mut events);
                true
            }
        };
        for event in events.drain(..) {
            if assembler.is_done() {
                break;
            }
            assembler.on_event(&event, on_event)?;
        }
        if ended {
            break;
        }
    }
    drop(stream);
    assembler.finish(on_event)
}

/// About four characters per token (AI-20), rounded up.
fn estimate_tokens(chars: usize) -> u64 {
    u64::try_from(chars.div_ceil(4)).unwrap_or(u64::MAX)
}

fn into_entry(
    req: &ChatRequest<'_>,
    assembled: Assembled,
    request_chars: usize,
    on_event: &mut (dyn FnMut(StreamEvent) + Send),
) -> AssistantEntry {
    let usage = assembled.usage.unwrap_or_else(|| {
        let output_chars = assembled.text.chars().count()
            + assembled.reasoning.chars().count()
            + assembled
                .tool_calls
                .iter()
                .map(|c| c.name.chars().count() + c.arguments.chars().count())
                .sum::<usize>();
        Usage {
            input_tokens: estimate_tokens(request_chars),
            output_tokens: estimate_tokens(output_chars),
            estimated: true,
        }
    });
    on_event(StreamEvent::Usage(usage));
    AssistantEntry {
        provider_id: req.provider_id.to_owned(),
        model_id: req.model.id.clone(),
        text: assembled.text,
        reasoning: (!assembled.reasoning.is_empty()).then_some(assembled.reasoning),
        tool_calls: assembled.tool_calls,
        finish: assembled.finish,
        usage: Some(usage),
        raw: assembled.raw,
    }
}
