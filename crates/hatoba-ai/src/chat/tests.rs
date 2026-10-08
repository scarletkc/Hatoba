//! Adapter tests: recorded-style Chat Completions and Anthropic streams replayed by a mock
//! server (spec §12, AI assistant), plus request building and stream assembly without HTTP.

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zeroize::Zeroizing;

use super::anthropic::AnthropicStream;
use super::openai::ChatCompletionsStream;
use super::sse::SseParser;
use super::*;
use crate::entry::{EntryBody, ToolStatus};
use crate::provider::{AuthHeader, http_client};
use crate::tools::{ToolSet, builtin_tools};

const KEY: &str = "sk-test-0123456789-secret";
const PROVIDER_ID: &str = "0192aaaa-bbbb-7ccc-8ddd-000000000001";

const OPENAI_TEXT: &str = include_str!("../../tests/fixtures/openai_text.sse");
const OPENAI_NO_USAGE: &str = include_str!("../../tests/fixtures/openai_no_usage.sse");
const OPENAI_LENGTH: &str = include_str!("../../tests/fixtures/openai_length.sse");
const OPENAI_FILTER: &str = include_str!("../../tests/fixtures/openai_content_filter.sse");
const OPENAI_ERROR: &str = include_str!("../../tests/fixtures/openai_midstream_error.sse");
const DEEPSEEK: &str = include_str!("../../tests/fixtures/deepseek_reasoning_tools.sse");
const GEMINI: &str = include_str!("../../tests/fixtures/gemini_tool_call.sse");
const ANTHROPIC_TOOL: &str = include_str!("../../tests/fixtures/anthropic_thinking_tool.sse");
const ANTHROPIC_UNKNOWN: &str = include_str!("../../tests/fixtures/anthropic_redacted_unknown.sse");
const ANTHROPIC_MAX: &str = include_str!("../../tests/fixtures/anthropic_max_tokens.sse");
const ANTHROPIC_REFUSAL: &str = include_str!("../../tests/fixtures/anthropic_refusal.sse");
const ANTHROPIC_ERROR: &str = include_str!("../../tests/fixtures/anthropic_error.sse");
const ANTHROPIC_CUT: &str = include_str!("../../tests/fixtures/anthropic_cut_off.sse");

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

fn provider(server: &MockServer, protocol: Protocol) -> ProviderConfig {
    ProviderConfig {
        protocol,
        base_url: match protocol {
            Protocol::ChatCompletions => format!("{}/v1", server.uri()),
            Protocol::Anthropic => server.uri(),
        },
        api_key: Zeroizing::new(KEY.into()),
        auth_header: AuthHeader::XApiKey,
    }
}

fn sse(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream; charset=utf-8")
        .set_body_string(body.to_owned())
}

fn request<'a>(
    model: &'a ModelSpec,
    entries: &'a [AiEntry],
    tools: &'a [ToolDef],
) -> ChatRequest<'a> {
    ChatRequest {
        provider_id: PROVIDER_ID,
        model,
        system: "SYS",
        tools,
        entries,
    }
}

async fn run(
    provider: &ProviderConfig,
    req: &ChatRequest<'_>,
) -> (Result<AssistantEntry, AiError>, Vec<StreamEvent>) {
    let mut events = Vec::new();
    let result = stream_chat(
        &http_client(),
        provider,
        req,
        &CancellationToken::new(),
        &mut |event| events.push(event),
    )
    .await;
    (result, events)
}

async fn bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

fn text(s: &str) -> StreamEvent {
    StreamEvent::Text(s.into())
}

fn reasoning(s: &str) -> StreamEvent {
    StreamEvent::Reasoning(s.into())
}

fn call(id: &str, name: &str, arguments: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: arguments.into(),
    }
}

fn usage(input_tokens: u64, output_tokens: u64) -> Usage {
    Usage {
        input_tokens,
        output_tokens,
        estimated: false,
    }
}

/// The tools of an attached terminal tab. A request with a history of tool calls and results
/// offers some; with none, the history is sent as text (see the flattening tests).
fn terminal_tools() -> Vec<ToolDef> {
    builtin_tools(&ToolSet {
        terminal: true,
        ..ToolSet::default()
    })
}

/// Feeds a recorded stream through the parser and an assembler in chunks of `size` bytes.
fn assemble<A: Assembler>(
    mut assembler: A,
    stream: &str,
    size: usize,
) -> (Result<Assembled, AiError>, Vec<StreamEvent>) {
    let mut emitted = Vec::new();
    let mut emit = |e| emitted.push(e);
    let mut parser = SseParser::new();
    let mut events = Vec::new();
    let result = (|| {
        for chunk in stream.as_bytes().chunks(size) {
            parser.push(chunk, &mut events)?;
            for event in events.drain(..) {
                if !assembler.is_done() {
                    assembler.on_event(&event, &mut emit)?;
                }
            }
        }
        parser.finish(&mut events);
        for event in events.drain(..) {
            if !assembler.is_done() {
                assembler.on_event(&event, &mut emit)?;
            }
        }
        assembler.finish(&mut emit)
    })();
    (result, emitted)
}

// ---------------------------------------------------------------------------------------------
// Chat Completions
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn chat_completions_streams_text_and_sends_the_expected_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse(OPENAI_TEXT))
        .mount(&server)
        .await;
    let model = ModelSpec::new("gpt-4.1");
    let tools = builtin_tools(&ToolSet {
        terminal: true,
        ..ToolSet::default()
    });
    let entries = vec![AiEntry::user(1, "How full is the disk?")];
    let (result, events) = run(
        &provider(&server, Protocol::ChatCompletions),
        &request(&model, &entries, &tools),
    )
    .await;
    let entry = result.unwrap();
    assert_eq!(
        events,
        vec![
            text("Disk usage"),
            text(" is 42%."),
            StreamEvent::Usage(usage(812, 9))
        ]
    );
    assert_eq!(
        entry,
        AssistantEntry {
            provider_id: PROVIDER_ID.into(),
            model_id: "gpt-4.1".into(),
            text: "Disk usage is 42%.".into(),
            reasoning: None,
            tool_calls: vec![],
            finish: Finish::Stop,
            usage: Some(usage(812, 9)),
            raw: json!({"role": "assistant", "content": "Disk usage is 42%.", "refusal": null}),
        }
    );

    let requests = server.received_requests().await.unwrap();
    let headers = &requests[0].headers;
    assert_eq!(
        headers.get("authorization").unwrap().to_str().unwrap(),
        format!("Bearer {KEY}")
    );
    assert!(headers.get("x-api-key").is_none());
    assert!(headers.get("anthropic-version").is_none());
    assert!(
        headers
            .get("user-agent")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("Hatoba/")
    );
    let body = &bodies(&server).await[0];
    assert_eq!(body["model"], "gpt-4.1");
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"], json!({"include_usage": true}));
    assert_eq!(
        body["messages"],
        json!([
            {"role": "system", "content": "SYS"},
            {"role": "user", "content": "How full is the disk?"}
        ])
    );
    assert_eq!(body["tools"].as_array().unwrap().len(), tools.len());
    assert_eq!(
        body["tools"][0],
        json!({
            "type": "function",
            "function": {
                "name": "read_terminal",
                "description": tools[0].description,
                "parameters": tools[0].input_schema,
            }
        })
    );
    for absent in ["max_tokens", "temperature", "cache_control"] {
        assert!(body.get(absent).is_none(), "{absent} must not be sent");
    }
}

#[tokio::test]
async fn chat_completions_captures_reasoning_and_merges_tool_calls() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(DEEPSEEK))
        .mount(&server)
        .await;
    let model = ModelSpec::new("deepseek-reasoner");
    let entries = vec![AiEntry::user(1, "uptime?")];
    let p = provider(&server, Protocol::ChatCompletions);
    let (result, events) = run(&p, &request(&model, &entries, &[])).await;
    let entry = result.unwrap();
    let uptime = call("call_00_kX1", "run_command", "{\"command\": \"uptime\"}");
    let screen = call("call_01_pQ2", "read_terminal", "{}");
    assert_eq!(
        events,
        vec![
            reasoning("The user wants"),
            reasoning(" the uptime."),
            text("Checking."),
            StreamEvent::ToolCall(uptime.clone()),
            StreamEvent::ToolCall(screen.clone()),
            StreamEvent::Usage(usage(1200, 85)),
        ]
    );
    assert_eq!(
        entry.reasoning.as_deref(),
        Some("The user wants the uptime.")
    );
    assert_eq!(entry.text, "Checking.");
    assert_eq!(entry.tool_calls, vec![uptime, screen]);
    assert_eq!(entry.finish, Finish::ToolCalls);
    assert_eq!(
        entry.raw,
        json!({
            "role": "assistant",
            "content": "Checking.",
            "reasoning_content": "The user wants the uptime.",
            "tool_calls": [
                {"id": "call_00_kX1", "type": "function",
                 "function": {"name": "run_command", "arguments": "{\"command\": \"uptime\"}"}},
                {"id": "call_01_pQ2", "type": "function",
                 "function": {"name": "read_terminal", "arguments": "{}"}}
            ]
        })
    );
    assert!(
        bodies(&server).await[0].get("tools").is_none(),
        "no tools, no field"
    );

    // Replayed to the same model: the raw message, reasoning_content included, goes back as it
    // is, followed by one `tool` message per call.
    let entries = vec![
        AiEntry::user(1, "uptime?"),
        AiEntry::assistant(2, entry.clone()),
        AiEntry::tool(3, "call_00_kX1", ToolStatus::Ok, "up 3 days"),
        AiEntry::tool(4, "call_01_pQ2", ToolStatus::Ok, "$ "),
    ];
    let tools = terminal_tools();
    let (result, _) = run(&p, &request(&model, &entries, &tools)).await;
    result.unwrap();
    let body = &bodies(&server).await[1];
    assert_eq!(body["messages"][2], entry.raw);
    assert_eq!(
        body["messages"][3],
        json!({"role": "tool", "tool_call_id": "call_00_kX1", "content": "up 3 days"})
    );
    assert_eq!(body["messages"][4]["tool_call_id"], "call_01_pQ2");
}

#[tokio::test]
async fn chat_completions_keeps_unknown_fields_and_rebuilds_after_a_switch() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(GEMINI))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse(OPENAI_TEXT))
        .mount(&server)
        .await;
    let gemini = ModelSpec::new("gemini-2.5-pro");
    let p = provider(&server, Protocol::ChatCompletions);
    let first = vec![AiEntry::user(1, "Read the nginx docs")];
    let (result, events) = run(&p, &request(&gemini, &first, &[])).await;
    let entry = result.unwrap();
    // `finish_reason: "stop"` with a tool call still means the model wants the result.
    assert_eq!(entry.finish, Finish::ToolCalls);
    let fetch = call(
        "function-call-17598",
        "fetch_url",
        "{\"url\":\"https://nginx.org/en/docs/\"}",
    );
    assert_eq!(entry.tool_calls, vec![fetch.clone()]);
    assert_eq!(events[0], StreamEvent::ToolCall(fetch));
    assert_eq!(
        entry.raw["tool_calls"][0]["extra_content"]["google"]["thought_signature"],
        "CsQBAdHtim9wF2kJ4n0Qx7bVv1u8Zl3r=="
    );
    assert_eq!(entry.usage, Some(usage(950, 31)));

    let entries = vec![
        AiEntry::user(1, "Read the nginx docs"),
        AiEntry::assistant(2, entry.clone()),
        AiEntry::tool(3, "function-call-17598", ToolStatus::Ok, "# nginx docs"),
    ];
    let tools = terminal_tools();
    // Same provider and model: raw, thought signature included.
    run(&p, &request(&gemini, &entries, &tools))
        .await
        .0
        .unwrap();
    // Another model: rebuilt from text and tool calls; the signature only raw held is gone.
    let other = ModelSpec::new("gpt-4.1");
    run(&p, &request(&other, &entries, &tools)).await.0.unwrap();
    // Same model id from another provider: also rebuilt.
    let mut elsewhere = request(&gemini, &entries, &tools);
    elsewhere.provider_id = "another-provider";
    run(&p, &elsewhere).await.0.unwrap();

    let bodies = bodies(&server).await;
    assert_eq!(bodies[1]["messages"][2], entry.raw);
    let rebuilt = json!({
        "role": "assistant",
        "content": null,
        "tool_calls": [{
            "id": "function-call-17598",
            "type": "function",
            "function": {"name": "fetch_url", "arguments": "{\"url\":\"https://nginx.org/en/docs/\"}"}
        }]
    });
    assert_eq!(bodies[2]["messages"][2], rebuilt);
    assert_eq!(bodies[3]["messages"][2], rebuilt);
    assert_eq!(
        bodies[2]["messages"][3],
        json!({"role": "tool", "tool_call_id": "function-call-17598", "content": "# nginx docs"})
    );
}

#[tokio::test]
async fn chat_completions_retries_without_stream_options_and_estimates_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(
            json!({"stream_options": {"include_usage": true}}),
        ))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {"message": "Unrecognized request argument supplied: stream_options",
                      "type": "invalid_request_error"}
        })))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse(OPENAI_NO_USAGE))
        .mount(&server)
        .await;
    let model = ModelSpec::new("qwen3:8b");
    let entries = vec![AiEntry::user(1, "hi")];
    let (result, events) = run(
        &provider(&server, Protocol::ChatCompletions),
        &request(&model, &entries, &[]),
    )
    .await;
    let entry = result.unwrap();
    let bodies = bodies(&server).await;
    assert_eq!(bodies.len(), 2);
    assert!(bodies[0].get("stream_options").is_some());
    assert!(bodies[1].get("stream_options").is_none());
    assert_eq!(entry.text, "Hello there");
    assert_eq!(
        entry.raw,
        json!({"role": "assistant", "content": "Hello there"})
    );
    let used = entry.usage.unwrap();
    assert!(used.estimated);
    assert_eq!(used.output_tokens, 3, "11 characters ≈ 3 tokens");
    let request_chars = serde_json::to_string(&bodies[1]).unwrap().chars().count() as u64;
    assert_eq!(used.input_tokens, request_chars.div_ceil(4));
    assert_eq!(events.last(), Some(&StreamEvent::Usage(used)));
}

#[tokio::test]
async fn chat_completions_estimates_usage_when_none_is_sent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(OPENAI_NO_USAGE))
        .mount(&server)
        .await;
    let model = ModelSpec::new("local");
    let entries = vec![AiEntry::user(1, "hi")];
    let (result, _) = run(
        &provider(&server, Protocol::ChatCompletions),
        &request(&model, &entries, &[]),
    )
    .await;
    assert!(result.unwrap().usage.unwrap().estimated);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn chat_completions_cut_off_and_declined_responses() {
    for (fixture, finish, expected_usage) in [
        (OPENAI_LENGTH, Finish::Length, usage(100, 4096)),
        (OPENAI_FILTER, Finish::Refused, usage(50, 0)),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(sse(fixture))
            .mount(&server)
            .await;
        let model = ModelSpec::new("gpt-4.1");
        let entries = vec![AiEntry::user(1, "go")];
        let (result, _) = run(
            &provider(&server, Protocol::ChatCompletions),
            &request(&model, &entries, &[]),
        )
        .await;
        let entry = result.unwrap();
        assert_eq!(entry.finish, finish);
        assert_eq!(entry.usage, Some(expected_usage));
    }
}

#[tokio::test]
async fn chat_completions_provider_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": {"message": "Incorrect API key provided.", "type": "invalid_request_error",
                      "code": "invalid_api_key"}
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(503)
                .insert_header("content-type", "text/html")
                .set_body_string("<html><body>Service\n Unavailable</body></html>"),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse(OPENAI_ERROR))
        .mount(&server)
        .await;
    let p = provider(&server, Protocol::ChatCompletions);
    let model = ModelSpec::new("gpt-4.1");
    let entries = vec![AiEntry::user(1, "secret question")];
    let req = request(&model, &entries, &[]);

    let err = run(&p, &req).await.0.unwrap_err();
    assert_eq!(
        err,
        AiError::Http {
            status: 401,
            message: "Incorrect API key provided.".into()
        }
    );
    let err = run(&p, &req).await.0.unwrap_err();
    assert_eq!(err.status(), Some(503));
    let (result, events) = run(&p, &req).await;
    assert_eq!(
        result.unwrap_err(),
        AiError::Http {
            status: 502,
            message: "Upstream provider overloaded".into()
        }
    );
    assert_eq!(events, vec![text("Partial")]);
}

#[tokio::test]
async fn chat_completions_without_a_key_sends_no_authorization() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(OPENAI_TEXT))
        .mount(&server)
        .await;
    let mut p = provider(&server, Protocol::ChatCompletions);
    p.api_key = Zeroizing::new(String::new());
    let model = ModelSpec::new("llama");
    let entries = vec![AiEntry::user(1, "hi")];
    run(&p, &request(&model, &entries, &[])).await.0.unwrap();
    let requests = server.received_requests().await.unwrap();
    assert!(requests[0].headers.get("authorization").is_none());
}

#[tokio::test]
async fn chat_completions_accepts_a_non_streamed_reply() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "x", "object": "chat.completion",
            "choices": [{"index": 0, "finish_reason": "tool_calls", "message": {
                "role": "assistant", "content": "Looking.",
                "tool_calls": [{"id": "c9", "type": "function",
                                "function": {"name": "read_terminal", "arguments": "{}"}}]
            }}],
            "usage": {"prompt_tokens": 5, "completion_tokens": 2}
        })))
        .mount(&server)
        .await;
    let model = ModelSpec::new("m");
    let entries = vec![AiEntry::user(1, "hi")];
    let (result, events) = run(
        &provider(&server, Protocol::ChatCompletions),
        &request(&model, &entries, &[]),
    )
    .await;
    let entry = result.unwrap();
    assert_eq!(entry.finish, Finish::ToolCalls);
    assert_eq!(entry.tool_calls, vec![call("c9", "read_terminal", "{}")]);
    assert_eq!(events.len(), 3);
    assert_eq!(entry.usage, Some(usage(5, 2)));
}

// ---------------------------------------------------------------------------------------------
// Anthropic
// ---------------------------------------------------------------------------------------------

fn thinking_entry() -> AssistantEntry {
    let (assembled, _) = assemble(AnthropicStream::new(), ANTHROPIC_TOOL, 4096);
    let a = assembled.unwrap();
    AssistantEntry {
        provider_id: PROVIDER_ID.into(),
        model_id: "claude-opus-4-8".into(),
        text: a.text,
        reasoning: Some(a.reasoning),
        tool_calls: a.tool_calls,
        finish: a.finish,
        usage: a.usage,
        raw: a.raw,
    }
}

#[tokio::test]
async fn anthropic_streams_thinking_text_and_tool_use() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(sse(ANTHROPIC_TOOL))
        .mount(&server)
        .await;
    let model = ModelSpec::new("claude-opus-4-8");
    let tools = builtin_tools(&ToolSet {
        terminal: true,
        web_search: true,
        read_skill: false,
    });
    let entries = vec![AiEntry::user(1, "Is the disk full?")];
    let (result, events) = run(
        &provider(&server, Protocol::Anthropic),
        &request(&model, &entries, &tools),
    )
    .await;
    let entry = result.unwrap();
    let df = call(
        "toolu_01AbCdEf",
        "run_command",
        "{\"command\": \"df -h\", \"timeout_seconds\": 10}",
    );
    assert_eq!(
        events,
        vec![
            reasoning("The user asks about disk"),
            reasoning(" space; df answers it."),
            text("I'll check the disk"),
            text(" usage."),
            StreamEvent::ToolCall(df.clone()),
            // 21 uncached + 1,530 written to the cache + 4,096 read from it.
            StreamEvent::Usage(usage(5_647, 89)),
        ]
    );
    assert_eq!(entry.text, "I'll check the disk usage.");
    assert_eq!(
        entry.reasoning.as_deref(),
        Some("The user asks about disk space; df answers it.")
    );
    assert_eq!(entry.tool_calls, vec![df]);
    assert_eq!(entry.finish, Finish::ToolCalls);
    assert_eq!(
        entry.raw,
        json!([
            {"type": "thinking", "thinking": "The user asks about disk space; df answers it.",
             "signature": "EqQBCgIYAhIM1gbcDa9GJwZA2b3hGgxDNAhW7dQ=="},
            {"type": "text", "text": "I'll check the disk usage."},
            {"type": "tool_use", "id": "toolu_01AbCdEf", "name": "run_command",
             "input": {"command": "df -h", "timeout_seconds": 10}}
        ])
    );

    let requests = server.received_requests().await.unwrap();
    let headers = &requests[0].headers;
    assert_eq!(headers.get("x-api-key").unwrap().to_str().unwrap(), KEY);
    assert_eq!(
        headers.get("anthropic-version").unwrap().to_str().unwrap(),
        "2023-06-01"
    );
    assert!(headers.get("authorization").is_none());
    assert!(headers.get("anthropic-beta").is_none());
    let body = &bodies(&server).await[0];
    assert_eq!(body["model"], "claude-opus-4-8");
    assert_eq!(body["max_tokens"], 16_000);
    assert_eq!(body["stream"], true);
    assert_eq!(body["cache_control"], json!({"type": "ephemeral"}));
    assert_eq!(body["system"], "SYS");
    assert_eq!(
        body["messages"],
        json!([{"role": "user", "content": [{"type": "text", "text": "Is the disk full?"}]}])
    );
    assert_eq!(
        body["tools"][1],
        json!({
            "name": "run_command",
            "description": tools[1].description,
            "input_schema": tools[1].input_schema,
        })
    );
    for absent in ["thinking", "temperature", "stream_options", "top_p"] {
        assert!(body.get(absent).is_none(), "{absent} must not be sent");
    }
    assert!(!body.to_string().contains("eager_input_streaming"));
}

#[tokio::test]
async fn anthropic_bearer_auth_and_model_output_limit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(ANTHROPIC_REFUSAL))
        .mount(&server)
        .await;
    let mut p = provider(&server, Protocol::Anthropic);
    p.auth_header = AuthHeader::Authorization;
    let model = ModelSpec {
        id: "deepseek-chat".into(),
        context_window: Some(128_000),
        max_output_tokens: Some(8_192),
    };
    let entries = vec![AiEntry::user(1, "hi")];
    let entry = run(&p, &request(&model, &entries, &[])).await.0.unwrap();
    assert_eq!(entry.finish, Finish::Refused);
    assert_eq!(entry.text, "I can't help with that.");
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests[0]
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap(),
        format!("Bearer {KEY}")
    );
    assert!(requests[0].headers.get("x-api-key").is_none());
    let body = &bodies(&server).await[0];
    assert_eq!(body["max_tokens"], 8_192);
    assert!(body.get("tools").is_none());
}

#[tokio::test]
async fn anthropic_replays_raw_to_the_same_model_and_rebuilds_after_a_switch() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(ANTHROPIC_REFUSAL))
        .mount(&server)
        .await;
    let p = provider(&server, Protocol::Anthropic);
    let entry = thinking_entry();
    let entries = vec![
        AiEntry::user(1, "Is the disk full?"),
        AiEntry::assistant(2, entry.clone()),
        AiEntry::tool(3, "toolu_01AbCdEf", ToolStatus::Ok, "/dev/sda1 42%"),
        AiEntry::user(4, "thanks"),
    ];
    let tools = terminal_tools();
    let same = ModelSpec::new("claude-opus-4-8");
    run(&p, &request(&same, &entries, &tools)).await.0.unwrap();
    let other = ModelSpec::new("claude-sonnet-4-6");
    run(&p, &request(&other, &entries, &tools)).await.0.unwrap();

    let bodies = bodies(&server).await;
    let tool_and_text = json!({"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": "toolu_01AbCdEf", "content": "/dev/sda1 42%"},
        {"type": "text", "text": "thanks"}
    ]});
    assert_eq!(
        bodies[0]["messages"],
        json!([
            {"role": "user", "content": [{"type": "text", "text": "Is the disk full?"}]},
            {"role": "assistant", "content": entry.raw},
            tool_and_text
        ])
    );
    assert_eq!(
        bodies[1]["messages"][1],
        json!({"role": "assistant", "content": [
            {"type": "text", "text": "I'll check the disk usage."},
            {"type": "tool_use", "id": "toolu_01AbCdEf", "name": "run_command",
             "input": {"command": "df -h", "timeout_seconds": 10}}
        ]})
    );
    assert_eq!(bodies[1]["messages"][2], tool_and_text);
}

#[tokio::test]
async fn anthropic_retries_once_without_raw_when_thinking_no_longer_binds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "type": "error",
            "error": {"type": "invalid_request_error",
                      "message": "messages.1.content.0: Invalid `signature` in `thinking` block"}
        })))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse(ANTHROPIC_REFUSAL))
        .mount(&server)
        .await;
    let p = provider(&server, Protocol::Anthropic);
    let entries = vec![
        AiEntry::user(1, "Is the disk full?"),
        AiEntry::assistant(2, thinking_entry()),
        AiEntry::tool(3, "toolu_01AbCdEf", ToolStatus::Ok, "42%"),
    ];
    let model = ModelSpec::new("claude-opus-4-8");
    let tools = terminal_tools();
    let entry = run(&p, &request(&model, &entries, &tools)).await.0.unwrap();
    assert_eq!(entry.text, "I can't help with that.");
    let bodies = bodies(&server).await;
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0]["messages"][1]["content"][0]["type"], "thinking");
    let retried = &bodies[1]["messages"][1]["content"];
    assert!(
        retried
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b["type"] != "thinking"),
        "the retry carries no thinking blocks: {retried}"
    );
}

#[tokio::test]
async fn anthropic_does_not_retry_when_nothing_was_replayed_or_the_error_differs() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "type": "error",
            "error": {"type": "invalid_request_error", "message": "thinking block mismatch"}
        })))
        .mount(&server)
        .await;
    let p = provider(&server, Protocol::Anthropic);
    let entries = vec![
        AiEntry::user(1, "Is the disk full?"),
        AiEntry::assistant(2, thinking_entry()),
        AiEntry::tool(3, "toolu_01AbCdEf", ToolStatus::Ok, "42%"),
    ];
    // Another model: nothing is replayed from raw, so a retry would send the same request.
    let tools = terminal_tools();
    let other = ModelSpec::new("claude-haiku-4-5");
    let err = run(&p, &request(&other, &entries, &tools))
        .await
        .0
        .unwrap_err();
    assert_eq!(err.status(), Some(400));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "type": "error",
            "error": {"type": "invalid_request_error", "message": "max_tokens: 999999 > 64000"}
        })))
        .mount(&server)
        .await;
    let p = provider(&server, Protocol::Anthropic);
    let same = ModelSpec::new("claude-opus-4-8");
    let err = run(&p, &request(&same, &entries, &tools))
        .await
        .0
        .unwrap_err();
    assert_eq!(
        err,
        AiError::Http {
            status: 400,
            message: "max_tokens: 999999 > 64000".into()
        }
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn anthropic_keeps_redacted_thinking_and_unknown_blocks() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(ANTHROPIC_UNKNOWN))
        .mount(&server)
        .await;
    let model = ModelSpec::new("claude-sonnet-4-6");
    let entries = vec![AiEntry::user(1, "How do I reload nginx?")];
    let (result, events) = run(
        &provider(&server, Protocol::Anthropic),
        &request(&model, &entries, &[]),
    )
    .await;
    let entry = result.unwrap();
    assert_eq!(
        entry.text,
        "Nginx reloads its configuration with nginx -s reload."
    );
    assert_eq!(entry.reasoning, None);
    assert_eq!(entry.finish, Finish::Stop);
    assert_eq!(entry.usage, Some(usage(10, 40)));
    assert_eq!(
        entry.raw,
        json!([
            {"type": "redacted_thinking",
             "data": "EmwKAhgBEgy3va3pzix/LafPsn4aDFIT2Xlxh0L5L8rLVyIwxtE3rAFBa8cr3qpPkNRj2YfWXGmKDxH4mPnZ5sQ7vB5URj2pabe1"},
            {"type": "future_block", "payload": "ab", "meta": {"v": 1, "w": 2}},
            {"type": "text", "text": "Nginx reloads its configuration", "citations": [
                {"type": "char_location", "cited_text": "nginx -s reload", "document_index": 0,
                 "start_char_index": 0, "end_char_index": 15}
            ]},
            {"type": "text", "text": " with nginx -s reload."}
        ])
    );
    assert_eq!(
        events,
        vec![
            text("Nginx reloads its configuration"),
            text(" with nginx -s reload."),
            StreamEvent::Usage(usage(10, 40))
        ]
    );
}

#[tokio::test]
async fn anthropic_cut_off_declined_and_failed_streams() {
    let server = MockServer::start().await;
    for fixture in [ANTHROPIC_MAX, ANTHROPIC_ERROR, ANTHROPIC_CUT] {
        Mock::given(method("POST"))
            .respond_with(sse(fixture))
            .up_to_n_times(1)
            .mount(&server)
            .await;
    }
    let p = provider(&server, Protocol::Anthropic);
    let model = ModelSpec::new("claude-haiku-4-5");
    let entries = vec![AiEntry::user(1, "list /var")];
    let req = request(&model, &entries, &[]);

    let entry = run(&p, &req).await.0.unwrap();
    assert_eq!(entry.finish, Finish::Length);
    assert_eq!(
        entry.tool_calls,
        vec![call(
            "toolu_cut",
            "run_command",
            "{\"command\": \"ls -la /va"
        )]
    );
    assert_eq!(
        entry.raw[1]["input"],
        json!({}),
        "raw keeps a valid input object"
    );
    assert_eq!(entry.usage, Some(usage(300, 16_000)));

    assert_eq!(
        run(&p, &req).await.0.unwrap_err(),
        AiError::Http {
            status: 529,
            message: "Overloaded".into()
        }
    );
    let (result, events) = run(&p, &req).await;
    assert!(matches!(result, Err(AiError::Network(_))), "{result:?}");
    assert_eq!(events, vec![text("Half an ans")]);
}

// ---------------------------------------------------------------------------------------------
// Both protocols
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn cancellation_while_waiting_for_the_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(OPENAI_TEXT).set_delay(Duration::from_secs(30)))
        .mount(&server)
        .await;
    let p = provider(&server, Protocol::ChatCompletions);
    let model = ModelSpec::new("gpt-4.1");
    let entries = vec![AiEntry::user(1, "hi")];
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        trigger.cancel();
    });
    let started = Instant::now();
    let result = stream_chat(
        &http_client(),
        &p,
        &request(&model, &entries, &[]),
        &cancel,
        &mut |_| {},
    )
    .await;
    assert_eq!(result, Err(AiError::Cancelled));
    assert!(started.elapsed() < Duration::from_secs(5));

    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let result = complete(
        &http_client(),
        &p,
        &request(&model, &entries, &[]),
        &cancelled,
    )
    .await;
    assert_eq!(result, Err(AiError::Cancelled));
}

/// A server that sends the headers and one event, then nothing (the connection stays open).
async fn stalling_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 64 * 1024];
        let _ = socket.read(&mut buf).await;
        let event = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hel\"}}]}\n\n";
        let head = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n{:x}\r\n{event}\r\n",
            event.len()
        );
        socket.write_all(head.as_bytes()).await.unwrap();
        tokio::time::sleep(Duration::from_secs(60)).await;
    });
    format!("http://{addr}/v1")
}

#[tokio::test]
async fn cancellation_in_the_middle_of_a_stream() {
    let p = ProviderConfig {
        protocol: Protocol::ChatCompletions,
        base_url: stalling_server().await,
        api_key: Zeroizing::new(KEY.into()),
        auth_header: AuthHeader::XApiKey,
    };
    let model = ModelSpec::new("m");
    let entries = vec![AiEntry::user(1, "hi")];
    let cancel = CancellationToken::new();
    let on_text = cancel.clone();
    let mut events = Vec::new();
    let started = Instant::now();
    let result = stream_chat(
        &http_client(),
        &p,
        &request(&model, &entries, &[]),
        &cancel,
        &mut |event| {
            events.push(event);
            on_text.cancel();
        },
    )
    .await;
    assert_eq!(result, Err(AiError::Cancelled));
    assert_eq!(events, vec![text("Hel")]);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn complete_returns_the_entry_and_bad_urls_are_refused() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(OPENAI_TEXT))
        .mount(&server)
        .await;
    let p = provider(&server, Protocol::ChatCompletions);
    let model = ModelSpec::new("gpt-4.1");
    let entries = vec![AiEntry::user(1, "Summarize")];
    let entry = complete(
        &http_client(),
        &p,
        &request(&model, &entries, &[]),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(entry.text, "Disk usage is 42%.");

    let public_http = ProviderConfig {
        base_url: "http://8.8.8.8/v1".into(),
        ..p
    };
    let err = run(&public_http, &request(&model, &entries, &[]))
        .await
        .0
        .unwrap_err();
    assert!(matches!(err, AiError::InvalidUrl(_)));
    assert!(!format!("{err} {err:?} {public_http:?}").contains(KEY));
}

// ---------------------------------------------------------------------------------------------
// Assembly and request building without HTTP
// ---------------------------------------------------------------------------------------------

#[test]
fn any_chunking_assembles_the_same_response() {
    for stream in [ANTHROPIC_TOOL, ANTHROPIC_UNKNOWN, ANTHROPIC_MAX] {
        let whole = assemble(AnthropicStream::new(), stream, stream.len());
        for size in [1, 2, 7, 64] {
            assert_eq!(assemble(AnthropicStream::new(), stream, size), whole);
        }
    }
    for stream in [OPENAI_TEXT, DEEPSEEK, GEMINI, OPENAI_NO_USAGE] {
        let whole = assemble(ChatCompletionsStream::new(), stream, stream.len());
        for size in [1, 3, 64] {
            assert_eq!(assemble(ChatCompletionsStream::new(), stream, size), whole);
        }
    }
}

#[test]
fn retry_conditions() {
    let http = |status, message: &str| AiError::Http {
        status,
        message: message.into(),
    };
    assert!(rejects_stream_options(&http(
        400,
        "Unrecognized request argument supplied: stream_options"
    )));
    assert!(rejects_stream_options(&http(
        422,
        "extra field: include_usage"
    )));
    assert!(!rejects_stream_options(&http(500, "stream_options")));
    assert!(!rejects_stream_options(&http(400, "bad model")));
    assert!(!rejects_stream_options(&AiError::Network(
        "stream_options".into()
    )));

    assert!(rejects_replayed_raw(&http(
        400,
        "messages.1.content.0: Invalid `signature` in `thinking` block"
    )));
    assert!(rejects_replayed_raw(&http(
        400,
        "Thinking blocks cannot be modified"
    )));
    assert!(rejects_replayed_raw(&http(
        400,
        "messages.3.content.0: text content blocks must be non-empty"
    )));
    assert!(!rejects_replayed_raw(&http(401, "signature")));
    assert!(!rejects_replayed_raw(&http(400, "max_tokens too large")));
}

fn assistant_with(calls: &[(&str, &str)], text: &str) -> AiEntry {
    AiEntry::assistant(
        0,
        AssistantEntry {
            provider_id: PROVIDER_ID.into(),
            model_id: "old-model".into(),
            text: text.into(),
            tool_calls: calls
                .iter()
                .map(|(id, args)| call(id, "run_command", args))
                .collect(),
            finish: Finish::ToolCalls,
            ..AssistantEntry::default()
        },
    )
}

#[test]
fn consecutive_results_share_one_user_message_with_status_wording() {
    let entries = vec![
        AiEntry::user(0, "clean up"),
        assistant_with(&[("a", "{\"command\":\"rm x\"}"), ("b", "not json")], ""),
        AiEntry::tool(0, "a", ToolStatus::Error, "permission denied"),
        AiEntry::tool(0, "b", ToolStatus::Rejected, "too risky"),
        AiEntry::user(0, "ok"),
    ];
    let model = ModelSpec::new("claude-x");
    let tools = terminal_tools();
    let body = request::anthropic(&request(&model, &entries, &tools), true).body;
    assert_eq!(
        body["messages"],
        json!([
            {"role": "user", "content": [{"type": "text", "text": "clean up"}]},
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "a", "name": "run_command", "input": {"command": "rm x"}},
                {"type": "tool_use", "id": "b", "name": "run_command", "input": {}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "a", "content": "permission denied",
                 "is_error": true},
                {"type": "tool_result", "tool_use_id": "b",
                 "content": "The user rejected this tool call. The user's reason: too risky",
                 "is_error": true},
                {"type": "text", "text": "ok"}
            ]}
        ])
    );
    let cc = request::chat_completions(&request(&model, &entries, &tools), true).body;
    assert_eq!(
        cc["messages"][2]["tool_calls"][1]["function"]["arguments"], "not json",
        "Chat Completions sends the arguments text as stored"
    );
    assert_eq!(
        cc["messages"][4],
        json!({"role": "tool", "tool_call_id": "b",
               "content": "The user rejected this tool call. The user's reason: too risky"})
    );
}

#[test]
fn missing_late_orphaned_and_reused_results_still_make_a_valid_request() {
    let entries = vec![
        AiEntry::tool(0, "zzz", ToolStatus::Ok, "orphan from before the context"),
        AiEntry::user(0, "first"),
        assistant_with(&[("call_0", "{}"), ("call_1", "{}")], "Running."),
        AiEntry::user(0, "typed while it ran"),
        // Stored after the next user message (e.g. merged from another device).
        AiEntry::tool(0, "call_0", ToolStatus::Ok, "one"),
        // call_1 never got a result.
        assistant_with(&[("call_0", "{}")], ""),
        AiEntry::tool(0, "call_0", ToolStatus::Cancelled, ""),
        AiEntry::assistant(0, AssistantEntry::default()),
        AiEntry::summary(0, "We cleaned /tmp."),
    ];
    assert_eq!(request::orphan_results(&entries), vec![0]);
    let model = ModelSpec::new("claude-x");
    let tools = terminal_tools();
    let body = request::anthropic(&request(&model, &entries, &tools), true).body;
    let roles: Vec<&str> = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, ["user", "assistant", "user", "assistant", "user"]);
    assert_eq!(
        body["messages"][2]["content"],
        json!([
            {"type": "tool_result", "tool_use_id": "call_0", "content": "one"},
            {"type": "tool_result", "tool_use_id": "call_1",
             "content": "The tool call was cancelled before it finished.", "is_error": true},
            {"type": "text", "text": "typed while it ran"}
        ])
    );
    assert_eq!(
        body["messages"][4]["content"],
        json!([
            {"type": "tool_result", "tool_use_id": "call_0",
             "content": "The tool call was cancelled before it finished.", "is_error": true},
            {"type": "text", "text": "Summary of the earlier conversation:\n\nWe cleaned /tmp."}
        ])
    );

    let cc = request::chat_completions(&request(&model, &entries, &tools), false).body;
    let roles: Vec<&str> = cc["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(
        roles,
        [
            "system",
            "user",
            "assistant",
            "tool",
            "tool",
            "user",
            "assistant",
            "tool",
            "user"
        ],
        "the empty assistant entry is skipped, the orphan dropped"
    );
    assert_eq!(cc["messages"][3]["content"], "one");
    assert!(cc.get("stream_options").is_none());
}

#[test]
fn request_bodies_are_deterministic() {
    let entries = vec![
        AiEntry::user(0, "a"),
        AiEntry::assistant(2, thinking_entry()),
        AiEntry::tool(3, "toolu_01AbCdEf", ToolStatus::Ok, "42%"),
    ];
    let model = ModelSpec::new("claude-opus-4-8");
    let tools = builtin_tools(&ToolSet {
        terminal: true,
        web_search: true,
        read_skill: true,
    });
    let req = request(&model, &entries, &tools);
    let a = serde_json::to_string(&request::anthropic(&req, true).body).unwrap();
    let b = serde_json::to_string(&request::anthropic(&req, true).body).unwrap();
    assert_eq!(a, b);
    let built = request::anthropic(&req, true);
    assert!(built.used_raw);
    assert!(!request::anthropic(&req, false).used_raw);
    let EntryBody::Assistant(ref replayed) = entries[1].body else {
        unreachable!()
    };
    assert_eq!(built.body["messages"][1]["content"], replayed.raw);
}

#[test]
fn request_debug_shows_no_conversation_content() {
    let entries = vec![AiEntry::user(0, "my password is hunter2")];
    let model = ModelSpec::new("m");
    let mut req = request(&model, &entries, &[]);
    req.system = "secret system text";
    let debug = format!("{req:?}");
    assert!(
        !debug.contains("hunter2") && !debug.contains("secret system"),
        "{debug}"
    );
    assert!(debug.contains("entries: 1"));
}

// ---------------------------------------------------------------------------------------------
// Requests without tools: the history is sent as text (Compact)
// ---------------------------------------------------------------------------------------------

// What the history below becomes as text, for both protocols.
const FLAT_A: &str = "Checking.\n\n\
    [Tool call: run_command (id: c1)]\nArguments: {\"command\":\"df -h\"}\n\n\
    [Tool call: run_command (id: c2)]\nArguments: {\"command\":\"systemctl restart nginx\"}\n\n\
    [Tool call: read_terminal (id: c3)]\nArguments: {}";
const FLAT_B: &str = "[Tool call: read_terminal (id: c4)]\nArguments: {}\n\n\
    [Tool call: run_command (id: c5)]\nArguments: {\"command\":\"ls";
const RESULT_1: &str = "[Tool result: run_command (id: c1), status: ok]\nUse% 42%";
const RESULT_2: &str = "[Tool result: run_command (id: c2), status: rejected]\n\
    The user rejected this tool call. The user's reason: too risky";
const RESULT_3: &str = "[Tool result: read_terminal (id: c3), status: cancelled]\n\
    The tool call was cancelled before it finished.";
const RESULT_4: &str = "[Tool result: read_terminal (id: c4), status: ok]\n(no output)";
const RESULT_5: &str = "[Tool result: run_command (id: c5), status: cancelled]\n\
    The tool call was cancelled before it finished.";

fn assistant_entry(model: &str, text: &str, calls: Vec<ToolCall>, raw: Value) -> AiEntry {
    AiEntry::assistant(
        0,
        AssistantEntry {
            provider_id: PROVIDER_ID.into(),
            model_id: model.into(),
            text: text.into(),
            tool_calls: calls,
            finish: Finish::ToolCalls,
            raw,
            ..AssistantEntry::default()
        },
    )
}

/// A conversation that used tools, as `protocol` stored it for `model` (the request's model, so
/// `raw` is replayed wherever the request may use it):
///
/// 0. user; 1. assistant A with text and three calls (`c3` never got a result); 2-3. results of
///    `c1` (ok) and `c2` (rejected); 4. assistant B, no text, two calls (blank arguments, then
///    cut-off arguments); 5-6. results of `c4` (ok, empty) and `c5` (cancelled); 7. the user asks
///    for a summary; 8. assistant C, text only, with reasoning in `raw`; 9. user.
fn tool_history(protocol: Protocol, model: &str) -> Vec<AiEntry> {
    let a_calls = vec![
        call("c1", "run_command", "{\"command\":\"df -h\"}"),
        call(
            "c2",
            "run_command",
            "{\"command\":\"systemctl restart nginx\"}",
        ),
        call("c3", "read_terminal", "{}"),
    ];
    let b_calls = vec![
        call("c4", "read_terminal", "  "),
        call("c5", "run_command", "{\"command\":\"ls"),
    ];
    let (raw_a, raw_b, raw_c) = match protocol {
        Protocol::Anthropic => (
            json!([
                {"type": "thinking", "thinking": "Plan.", "signature": "sig-a"},
                {"type": "text", "text": "Checking."},
                {"type": "tool_use", "id": "c1", "name": "run_command", "input": {"command": "df -h"}},
                {"type": "tool_use", "id": "c2", "name": "run_command",
                 "input": {"command": "systemctl restart nginx"}},
                {"type": "tool_use", "id": "c3", "name": "read_terminal", "input": {}}
            ]),
            json!([
                {"type": "tool_use", "id": "c4", "name": "read_terminal", "input": {}},
                {"type": "tool_use", "id": "c5", "name": "run_command", "input": {}}
            ]),
            json!([
                {"type": "thinking", "thinking": "Done thinking.", "signature": "sig-c"},
                {"type": "text", "text": "We checked the disk."}
            ]),
        ),
        Protocol::ChatCompletions => {
            let wire = |calls: &[ToolCall]| -> Vec<Value> {
                calls
                    .iter()
                    .map(|c| {
                        json!({"id": c.id, "type": "function",
                               "function": {"name": c.name, "arguments": c.arguments}})
                    })
                    .collect()
            };
            (
                json!({"role": "assistant", "content": "Checking.", "reasoning_content": "Plan.",
                       "tool_calls": wire(&a_calls)}),
                json!({"role": "assistant", "content": null, "tool_calls": wire(&b_calls)}),
                json!({"role": "assistant", "content": "We checked the disk.",
                       "reasoning_content": "Done thinking."}),
            )
        }
    };
    let mut c = assistant_entry(model, "We checked the disk.", vec![], raw_c);
    if let EntryBody::Assistant(a) = &mut c.body {
        a.finish = Finish::Stop;
    }
    vec![
        AiEntry::user(1, "check the disk and restart nginx"),
        assistant_entry(model, "Checking.", a_calls, raw_a),
        AiEntry::tool(3, "c1", ToolStatus::Ok, "Use% 42%"),
        AiEntry::tool(4, "c2", ToolStatus::Rejected, "too risky"),
        assistant_entry(model, "", b_calls, raw_b),
        AiEntry::tool(6, "c4", ToolStatus::Ok, ""),
        AiEntry::tool(7, "c5", ToolStatus::Cancelled, ""),
        AiEntry::user(8, "Summarize the conversation."),
        c,
        AiEntry::user(10, "thanks"),
    ]
}

/// The `raw` of the assistant entry at `index`.
fn raw_of(entries: &[AiEntry], index: usize) -> Value {
    let EntryBody::Assistant(a) = &entries[index].body else {
        panic!("entry {index} is not an assistant entry");
    };
    a.raw.clone()
}

fn anthropic_roles(body: &Value) -> Vec<&str> {
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect()
}

/// Anthropic needs the messages to alternate, to open and to end with the user.
fn assert_alternating(body: &Value) {
    let roles = anthropic_roles(body);
    assert_eq!(roles.first(), Some(&"user"), "{roles:?}");
    assert_eq!(roles.last(), Some(&"user"), "{roles:?}");
    assert!(roles.windows(2).all(|w| w[0] != w[1]), "{roles:?}");
}

/// No `tools` and only text and thinking blocks, which Anthropic accepts without tools.
fn assert_no_anthropic_tool_blocks(body: &Value) {
    assert!(body.get("tools").is_none());
    for message in body["messages"].as_array().unwrap() {
        for block in message["content"].as_array().unwrap() {
            let kind = block["type"].as_str().unwrap();
            assert!(matches!(kind, "text" | "thinking"), "{kind} in {message}");
        }
    }
}

fn assert_no_chat_tool_messages(body: &Value) {
    assert!(body.get("tools").is_none());
    for message in body["messages"].as_array().unwrap() {
        assert_ne!(message["role"], "tool", "{message}");
        for key in ["tool_calls", "function_call", "tool_call_id"] {
            assert!(message.get(key).is_none(), "{key} in {message}");
        }
    }
}

#[test]
fn flattened_texts_name_the_call_and_keep_the_result_wording() {
    let run_command = call("c1", "run_command", "{\"command\":\"ls\"}");
    let result = |status, content| request::flattened_result_text(&run_command, status, content);
    assert_eq!(
        result(ToolStatus::Ok, "a\nb"),
        "[Tool result: run_command (id: c1), status: ok]\na\nb"
    );
    assert_eq!(
        result(ToolStatus::Ok, ""),
        "[Tool result: run_command (id: c1), status: ok]\n(no output)"
    );
    assert_eq!(
        result(ToolStatus::Error, "permission denied"),
        "[Tool result: run_command (id: c1), status: error]\npermission denied"
    );
    assert_eq!(
        result(ToolStatus::Error, ""),
        "[Tool result: run_command (id: c1), status: error]\nThe tool failed without a message."
    );
    assert_eq!(
        result(ToolStatus::Rejected, ""),
        "[Tool result: run_command (id: c1), status: rejected]\nThe user rejected this tool call."
    );
    assert_eq!(
        result(ToolStatus::Rejected, "too risky"),
        "[Tool result: run_command (id: c1), status: rejected]\n\
         The user rejected this tool call. The user's reason: too risky"
    );
    assert_eq!(
        result(ToolStatus::Cancelled, ""),
        "[Tool result: run_command (id: c1), status: cancelled]\n\
         The tool call was cancelled before it finished."
    );
    assert_eq!(
        result(ToolStatus::Cancelled, "stopped"),
        "[Tool result: run_command (id: c1), status: cancelled]\n\
         The tool call was cancelled before it finished.\nstopped"
    );

    let EntryBody::Assistant(with_text) = assistant_with(&[("c1", " {} \n")], "Looking.\n\n").body
    else {
        unreachable!()
    };
    assert_eq!(
        request::flattened_assistant_text(&with_text),
        "Looking.\n\n[Tool call: run_command (id: c1)]\nArguments: {}",
        "text trimmed at the end, arguments trimmed"
    );
    let EntryBody::Assistant(no_text) = assistant_with(&[("c1", "x"), ("c2", "")], " \n").body
    else {
        unreachable!()
    };
    assert_eq!(
        request::flattened_assistant_text(&no_text),
        "[Tool call: run_command (id: c1)]\nArguments: x\n\n\
         [Tool call: run_command (id: c2)]\nArguments: {}",
        "blank text is left out, blank arguments are {{}}"
    );
}

#[test]
fn anthropic_without_tools_sends_calls_and_results_as_text() {
    let model = ModelSpec::new("claude-opus-4-8");
    let entries = tool_history(Protocol::Anthropic, &model.id);
    let built = request::anthropic(&request(&model, &entries, &[]), true);
    let body = &built.body;
    let text = |s: &str| json!({"type": "text", "text": s});
    assert_eq!(
        body["messages"],
        json!([
            {"role": "user", "content": [text("check the disk and restart nginx")]},
            {"role": "assistant", "content": [text(FLAT_A)]},
            {"role": "user", "content": [text(RESULT_1), text(RESULT_2), text(RESULT_3)]},
            {"role": "assistant", "content": [text(FLAT_B)]},
            {"role": "user", "content": [
                text(RESULT_4), text(RESULT_5), text("Summarize the conversation.")
            ]},
            // No tool calls: raw is replayed as before, thinking block included.
            {"role": "assistant", "content": raw_of(&entries, 8)},
            {"role": "user", "content": [text("thanks")]}
        ])
    );
    assert_eq!(body["messages"][5]["content"][0]["type"], "thinking");
    assert!(built.used_raw, "the text-only entry still replays raw");
    assert_no_anthropic_tool_blocks(body);
    assert_alternating(body);
    for absent in ["tools", "thinking", "temperature"] {
        assert!(body.get(absent).is_none(), "{absent} must not be sent");
    }
    assert_eq!(body["system"], "SYS");

    // Deterministic, and the thinking-binding retry (no raw at all) keeps the same text.
    let again = request::anthropic(&request(&model, &entries, &[]), true);
    assert_eq!(body.to_string(), again.body.to_string());
    let rebuilt = request::anthropic(&request(&model, &entries, &[]), false);
    assert!(!rebuilt.used_raw);
    assert_eq!(
        rebuilt.body["messages"][5],
        json!({"role": "assistant", "content": [text("We checked the disk.")]})
    );
    let [kept @ .., _, _] = rebuilt.body["messages"].as_array().unwrap().as_slice() else {
        unreachable!()
    };
    assert_eq!(kept, &body["messages"].as_array().unwrap()[..5]);
    assert_no_anthropic_tool_blocks(&rebuilt.body);
    assert_alternating(&rebuilt.body);
}

#[test]
fn chat_completions_without_tools_sends_calls_and_results_as_text() {
    let model = ModelSpec::new("deepseek-reasoner");
    let entries = tool_history(Protocol::ChatCompletions, &model.id);
    let built = request::chat_completions(&request(&model, &entries, &[]), true);
    let body = &built.body;
    assert_eq!(
        body["messages"],
        json!([
            {"role": "system", "content": "SYS"},
            {"role": "user", "content": "check the disk and restart nginx"},
            {"role": "assistant", "content": FLAT_A},
            {"role": "user", "content": RESULT_1},
            {"role": "user", "content": RESULT_2},
            {"role": "user", "content": RESULT_3},
            {"role": "assistant", "content": FLAT_B},
            {"role": "user", "content": RESULT_4},
            {"role": "user", "content": RESULT_5},
            {"role": "user", "content": "Summarize the conversation."},
            // No tool calls: raw is replayed as before, reasoning_content included.
            raw_of(&entries, 8),
            {"role": "user", "content": "thanks"}
        ])
    );
    assert_eq!(body["messages"][10]["reasoning_content"], "Done thinking.");
    assert!(built.used_raw);
    assert_no_chat_tool_messages(body);
    assert_eq!(body["stream_options"], json!({"include_usage": true}));

    // The retry without `stream_options` builds the same messages.
    let retry = request::chat_completions(&request(&model, &entries, &[]), false);
    assert!(retry.body.get("stream_options").is_none());
    assert_eq!(retry.body["messages"], body["messages"]);
}

#[test]
fn requests_with_tools_keep_structured_tool_blocks() {
    let tools = terminal_tools();
    let model = ModelSpec::new("claude-opus-4-8");
    let entries = tool_history(Protocol::Anthropic, &model.id);
    let built = request::anthropic(&request(&model, &entries, &tools), true);
    assert_eq!(
        built.body["messages"],
        json!([
            {"role": "user", "content": [
                {"type": "text", "text": "check the disk and restart nginx"}
            ]},
            {"role": "assistant", "content": raw_of(&entries, 1)},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "c1", "content": "Use% 42%"},
                {"type": "tool_result", "tool_use_id": "c2", "is_error": true,
                 "content": "The user rejected this tool call. The user's reason: too risky"},
                {"type": "tool_result", "tool_use_id": "c3", "is_error": true,
                 "content": "The tool call was cancelled before it finished."}
            ]},
            {"role": "assistant", "content": raw_of(&entries, 4)},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "c4", "content": "(no output)"},
                {"type": "tool_result", "tool_use_id": "c5", "is_error": true,
                 "content": "The tool call was cancelled before it finished."},
                {"type": "text", "text": "Summarize the conversation."}
            ]},
            {"role": "assistant", "content": raw_of(&entries, 8)},
            {"role": "user", "content": [{"type": "text", "text": "thanks"}]}
        ])
    );
    assert_eq!(built.body["tools"].as_array().unwrap().len(), tools.len());
    assert!(built.used_raw);
    // Rebuilt from fields (another model, or the retry): `tool_use` blocks, not text.
    let other = ModelSpec::new("claude-sonnet-4-6");
    let rebuilt = request::anthropic(&request(&other, &entries, &tools), true);
    assert!(!rebuilt.used_raw);
    assert_eq!(
        rebuilt.body["messages"][1],
        json!({"role": "assistant", "content": [
            {"type": "text", "text": "Checking."},
            {"type": "tool_use", "id": "c1", "name": "run_command", "input": {"command": "df -h"}},
            {"type": "tool_use", "id": "c2", "name": "run_command",
             "input": {"command": "systemctl restart nginx"}},
            {"type": "tool_use", "id": "c3", "name": "read_terminal", "input": {}}
        ]})
    );
    assert_eq!(
        rebuilt.body["messages"][3]["content"][1],
        json!({"type": "tool_use", "id": "c5", "name": "run_command", "input": {}})
    );

    let model = ModelSpec::new("deepseek-reasoner");
    let entries = tool_history(Protocol::ChatCompletions, &model.id);
    let built = request::chat_completions(&request(&model, &entries, &tools), true);
    assert_eq!(
        built.body["messages"],
        json!([
            {"role": "system", "content": "SYS"},
            {"role": "user", "content": "check the disk and restart nginx"},
            raw_of(&entries, 1),
            {"role": "tool", "tool_call_id": "c1", "content": "Use% 42%"},
            {"role": "tool", "tool_call_id": "c2",
             "content": "The user rejected this tool call. The user's reason: too risky"},
            {"role": "tool", "tool_call_id": "c3",
             "content": "The tool call was cancelled before it finished."},
            raw_of(&entries, 4),
            {"role": "tool", "tool_call_id": "c4", "content": "(no output)"},
            {"role": "tool", "tool_call_id": "c5",
             "content": "The tool call was cancelled before it finished."},
            {"role": "user", "content": "Summarize the conversation."},
            raw_of(&entries, 8),
            {"role": "user", "content": "thanks"}
        ])
    );
    assert_eq!(built.body["tools"].as_array().unwrap().len(), tools.len());
    let other = ModelSpec::new("gpt-4.1");
    let rebuilt = request::chat_completions(&request(&other, &entries, &tools), true);
    assert_eq!(
        rebuilt.body["messages"][2]["tool_calls"][2],
        json!({"id": "c3", "type": "function",
               "function": {"name": "read_terminal", "arguments": "{}"}})
    );
    assert_eq!(
        rebuilt.body["messages"][6]["tool_calls"][0]["function"]["arguments"], "{}",
        "blank arguments are sent as {{}} with tools, as before"
    );
}

#[test]
fn raw_holding_tool_blocks_is_not_replayed_without_tools() {
    let tools = terminal_tools();
    let anthropic = ModelSpec::new("claude-opus-4-8");
    for kind in [
        "tool_use",
        "tool_result",
        "server_tool_use",
        "mcp_tool_use",
        "web_search_tool_result",
        "mcp_tool_result",
    ] {
        let raw = json!([
            {"type": "thinking", "thinking": "t", "signature": "s"},
            {"type": kind, "id": "x"},
            {"type": "text", "text": "Searched."}
        ]);
        let entries = vec![
            AiEntry::user(1, "hi"),
            assistant_entry(&anthropic.id, "Searched.", vec![], raw.clone()),
            AiEntry::user(2, "and?"),
        ];
        let built = request::anthropic(&request(&anthropic, &entries, &[]), true);
        assert!(!built.used_raw, "{kind}");
        assert_eq!(
            built.body["messages"][1],
            json!({"role": "assistant", "content": [{"type": "text", "text": "Searched."}]}),
            "{kind}"
        );
        assert_no_anthropic_tool_blocks(&built.body);
        // With tools offered nothing changes: raw goes back as it is.
        let built = request::anthropic(&request(&anthropic, &entries, &tools), true);
        assert!(built.used_raw, "{kind}");
        assert_eq!(built.body["messages"][1]["content"], raw, "{kind}");
    }
    // Blocks that only look unfamiliar are still replayed.
    let raw = json!([
        {"type": "redacted_thinking", "data": "abc"},
        {"type": "future_block", "payload": 1},
        {"type": "text", "text": "Hi."}
    ]);
    let entries = vec![
        AiEntry::user(1, "hi"),
        assistant_entry(&anthropic.id, "Hi.", vec![], raw.clone()),
    ];
    let built = request::anthropic(&request(&anthropic, &entries, &[]), true);
    assert!(built.used_raw);
    assert_eq!(built.body["messages"][1]["content"], raw);

    let chat = ModelSpec::new("deepseek-reasoner");
    for raw in [
        json!({"role": "assistant", "content": "Searched.", "reasoning_content": "t",
               "tool_calls": [{"id": "x", "type": "function",
                               "function": {"name": "f", "arguments": "{}"}}]}),
        json!({"role": "assistant", "content": "Searched.", "reasoning_content": "t",
               "function_call": {"name": "f", "arguments": "{}"}}),
    ] {
        let entries = vec![
            AiEntry::user(1, "hi"),
            assistant_entry(&chat.id, "Searched.", vec![], raw.clone()),
            AiEntry::user(2, "and?"),
        ];
        let built = request::chat_completions(&request(&chat, &entries, &[]), true);
        assert!(!built.used_raw);
        assert_eq!(
            built.body["messages"][2],
            json!({"role": "assistant", "content": "Searched."})
        );
        assert_no_chat_tool_messages(&built.body);
        let built = request::chat_completions(&request(&chat, &entries, &tools), true);
        assert!(built.used_raw);
        assert_eq!(built.body["messages"][2], raw);
    }
    let raw = json!({"role": "assistant", "content": "Hi.", "reasoning_content": "t",
                     "tool_calls": null});
    let entries = vec![
        AiEntry::user(1, "hi"),
        assistant_entry(&chat.id, "Hi.", vec![], raw.clone()),
    ];
    let built = request::chat_completions(&request(&chat, &entries, &[]), true);
    assert!(built.used_raw);
    assert_eq!(built.body["messages"][2], raw);
}

/// A server that behaves like Anthropic: tool blocks without `tools` are a 400.
fn anthropic_tool_blocks_without_tools(req: &wiremock::Request) -> bool {
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    body.get("tools").is_none()
        && body["messages"].as_array().unwrap().iter().any(|m| {
            m["content"].as_array().is_some_and(|blocks| {
                blocks
                    .iter()
                    .any(|b| matches!(b["type"].as_str(), Some("tool_use" | "tool_result")))
            })
        })
}

fn refuse_tool_blocks() -> ResponseTemplate {
    ResponseTemplate::new(400).set_body_json(json!({
        "type": "error",
        "error": {"type": "invalid_request_error",
                  "message": "Requests which include `tool_use` or `tool_result` blocks \
                              must define tools."}
    }))
}

/// A Chat Completions server that refuses `tool_calls` and `tool` messages without `tools`.
fn chat_tool_messages_without_tools(req: &wiremock::Request) -> bool {
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    body.get("tools").is_none()
        && body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["role"] == "tool" || m.get("tool_calls").is_some())
}

#[tokio::test]
async fn anthropic_compact_after_tool_calls_is_accepted_without_tools() {
    let server = MockServer::start().await;
    Mock::given(anthropic_tool_blocks_without_tools)
        .respond_with(refuse_tool_blocks())
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(sse(ANTHROPIC_UNKNOWN))
        .mount(&server)
        .await;
    let p = provider(&server, Protocol::Anthropic);
    let model = ModelSpec::new("claude-opus-4-8");
    let entries = tool_history(Protocol::Anthropic, &model.id);

    // Compact (AI-21): no tools, a history full of them.
    let entry = complete(
        &http_client(),
        &p,
        &request(&model, &entries, &[]),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        entry.text,
        "Nginx reloads its configuration with nginx -s reload."
    );
    // The same history with tools: the structured blocks, which the server accepts then.
    let tools = terminal_tools();
    run(&p, &request(&model, &entries, &tools)).await.0.unwrap();

    let bodies = bodies(&server).await;
    assert_eq!(bodies.len(), 2, "no request was refused");
    assert_no_anthropic_tool_blocks(&bodies[0]);
    assert_alternating(&bodies[0]);
    assert_eq!(bodies[0]["messages"][1]["content"][0]["text"], FLAT_A);
    assert_eq!(bodies[0]["messages"][2]["content"][1]["text"], RESULT_2);
    assert_eq!(bodies[0]["messages"][5]["content"], raw_of(&entries, 8));
    assert_eq!(bodies[1]["tools"].as_array().unwrap().len(), tools.len());
    assert_eq!(bodies[1]["messages"][1]["content"], raw_of(&entries, 1));
    assert_eq!(
        bodies[1]["messages"][2]["content"][0]["type"], "tool_result",
        "with tools the results stay structured"
    );
    assert_alternating(&bodies[1]);
}

#[tokio::test]
async fn chat_completions_without_tools_after_tool_calls_is_accepted() {
    let server = MockServer::start().await;
    Mock::given(chat_tool_messages_without_tools)
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {"message": "tool_calls and tool messages require the tools parameter",
                      "type": "invalid_request_error"}
        })))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse(OPENAI_TEXT))
        .mount(&server)
        .await;
    let p = provider(&server, Protocol::ChatCompletions);
    let model = ModelSpec::new("deepseek-reasoner");
    let entries = tool_history(Protocol::ChatCompletions, &model.id);

    // No tools offered: the history goes as text.
    let entry = run(&p, &request(&model, &entries, &[])).await.0.unwrap();
    assert_eq!(entry.text, "Disk usage is 42%.");
    let tools = terminal_tools();
    run(&p, &request(&model, &entries, &tools)).await.0.unwrap();

    let bodies = bodies(&server).await;
    assert_eq!(bodies.len(), 2, "no request was refused");
    assert_no_chat_tool_messages(&bodies[0]);
    assert_eq!(bodies[0]["messages"][2]["content"], FLAT_A);
    assert_eq!(
        bodies[0]["messages"][5],
        json!({"role": "user", "content": RESULT_3})
    );
    assert_eq!(bodies[0]["messages"][10], raw_of(&entries, 8));
    assert_eq!(bodies[1]["tools"].as_array().unwrap().len(), tools.len());
    assert_eq!(bodies[1]["messages"][2], raw_of(&entries, 1));
    assert_eq!(bodies[1]["messages"][3]["role"], "tool");
}

#[tokio::test]
async fn anthropic_thinking_retry_still_works_without_tools() {
    let server = MockServer::start().await;
    Mock::given(anthropic_tool_blocks_without_tools)
        .respond_with(refuse_tool_blocks())
        .with_priority(1)
        .mount(&server)
        .await;
    // The replayed thinking of the text-only entry no longer binds: one 400, then success.
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "type": "error",
            "error": {"type": "invalid_request_error",
                      "message": "messages.5.content.0: Invalid `signature` in `thinking` block"}
        })))
        .up_to_n_times(1)
        .with_priority(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse(ANTHROPIC_UNKNOWN))
        .mount(&server)
        .await;
    let p = provider(&server, Protocol::Anthropic);
    let model = ModelSpec::new("claude-opus-4-8");
    let entries = tool_history(Protocol::Anthropic, &model.id);

    let entry = run(&p, &request(&model, &entries, &[])).await.0.unwrap();
    assert_eq!(entry.finish, Finish::Stop);
    let bodies = bodies(&server).await;
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0]["messages"][5]["content"][0]["type"], "thinking");
    assert_eq!(
        bodies[1]["messages"][5],
        json!({"role": "assistant", "content": [{"type": "text", "text": "We checked the disk."}]})
    );
    for body in &bodies {
        assert_no_anthropic_tool_blocks(body);
        assert_alternating(body);
        assert_eq!(body["messages"][1]["content"][0]["text"], FLAT_A);
    }
    assert_eq!(
        bodies[0]["messages"].as_array().unwrap()[..5],
        bodies[1]["messages"].as_array().unwrap()[..5],
        "the flattened part is the same in the retry"
    );

    // A 400 about thinking is not retried when no entry replayed raw.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "type": "error",
            "error": {"type": "invalid_request_error", "message": "thinking block mismatch"}
        })))
        .mount(&server)
        .await;
    let p = provider(&server, Protocol::Anthropic);
    let mut flat_only = entries.clone();
    flat_only.truncate(7);
    let err = run(&p, &request(&model, &flat_only, &[]))
        .await
        .0
        .unwrap_err();
    assert_eq!(err.status(), Some(400));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn chat_completions_stream_options_retry_still_works_without_tools() {
    let server = MockServer::start().await;
    Mock::given(chat_tool_messages_without_tools)
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {"message": "tool_calls and tool messages require the tools parameter"}
        })))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(body_partial_json(
        json!({"stream_options": {"include_usage": true}}),
    ))
    .respond_with(ResponseTemplate::new(400).set_body_json(json!({
        "error": {"message": "Unrecognized request argument supplied: stream_options"}
    })))
    .with_priority(2)
    .mount(&server)
    .await;
    Mock::given(method("POST"))
        .respond_with(sse(OPENAI_NO_USAGE))
        .mount(&server)
        .await;
    let p = provider(&server, Protocol::ChatCompletions);
    let model = ModelSpec::new("qwen3:8b");
    let entries = tool_history(Protocol::ChatCompletions, &model.id);

    let entry = run(&p, &request(&model, &entries, &[])).await.0.unwrap();
    assert_eq!(entry.text, "Hello there");
    let bodies = bodies(&server).await;
    assert_eq!(bodies.len(), 2);
    assert!(bodies[0].get("stream_options").is_some());
    assert!(bodies[1].get("stream_options").is_none());
    assert_eq!(bodies[0]["messages"], bodies[1]["messages"]);
    for body in &bodies {
        assert_no_chat_tool_messages(body);
    }
}
