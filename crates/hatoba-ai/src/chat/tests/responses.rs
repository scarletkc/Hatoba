//! Responses: recorded-style streams replayed by a mock server, request building, and stream
//! assembly without HTTP.

use super::*;
use crate::chat::responses::ResponsesStream;

const TEXT: &str = include_str!("../../../tests/fixtures/responses_text.sse");
const TOOLS: &str = include_str!("../../../tests/fixtures/responses_reasoning_tools.sse");
const INCOMPLETE: &str = include_str!("../../../tests/fixtures/responses_incomplete.sse");
const FAILED: &str = include_str!("../../../tests/fixtures/responses_failed.sse");
const CUT_OFF: &str = include_str!("../../../tests/fixtures/responses_cut_off.sse");

/// An item of `raw` as a stateless request sends it back: without its `id`.
fn without_id(item: &Value) -> Value {
    let mut item = item.clone();
    item.as_object_mut().unwrap().remove("id");
    item
}

/// `raw` items as a stateless request sends them back.
fn replayed(raw: &Value) -> Vec<Value> {
    raw.as_array().unwrap().iter().map(without_id).collect()
}

fn input_types(body: &Value) -> Vec<&str> {
    body["input"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["type"].as_str().unwrap())
        .collect()
}

fn event(name: &str, data: &Value) -> String {
    format!("event: {name}\ndata: {data}\n\n")
}

#[tokio::test]
async fn responses_streams_text_and_sends_the_expected_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(sse(TEXT))
        .mount(&server)
        .await;
    let model = ModelSpec::new("gpt-4.1");
    let tools = terminal_tools();
    let entries = vec![AiEntry::user(1, "How full is the disk?")];
    let (result, events) = run(
        &provider(&server, Protocol::Responses),
        &request(&model, &entries, &tools),
    )
    .await;
    let entry = result.unwrap();
    assert_eq!(
        events,
        vec![
            text("Disk usage"),
            text(" is 42%."),
            StreamEvent::Usage(cached_usage(812, 9, 768, 0))
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
            usage: Some(cached_usage(812, 9, 768, 0)),
            raw: json!([{"id": "msg_7f2", "type": "message", "status": "completed",
                         "content": [{"type": "output_text", "annotations": [], "logprobs": [],
                                      "text": "Disk usage is 42%."}],
                         "role": "assistant"}]),
            terminal: None,
            interrupted: false,
        }
    );

    let requests = server.received_requests().await.unwrap();
    let headers = &requests[0].headers;
    assert_eq!(
        headers.get("authorization").unwrap().to_str().unwrap(),
        format!("Bearer {KEY}")
    );
    assert!(headers.get("x-api-key").is_none());
    let body = &bodies(&server).await[0];
    assert_eq!(body["model"], "gpt-4.1");
    assert_eq!(body["instructions"], "SYS");
    assert_eq!(
        body["input"],
        json!([{"type": "message", "role": "user", "content": "How full is the disk?"}])
    );
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    assert_eq!(body["tools"].as_array().unwrap().len(), tools.len());
    assert_eq!(
        body["tools"][0],
        json!({
            "type": "function",
            "name": "read_terminal",
            "description": tools[0].description,
            "parameters": tools[0].input_schema,
            "strict": false,
        })
    );
    // gpt-4.1 does not reason: no reasoning fields at Default, which it would refuse.
    for absent in [
        "messages",
        "reasoning",
        "include",
        "previous_response_id",
        "max_output_tokens",
        "temperature",
        "stream_options",
        "reasoning_effort",
    ] {
        assert!(body.get(absent).is_none(), "{absent} must not be sent");
    }
}

#[tokio::test]
async fn responses_captures_reasoning_and_tool_calls_and_replays_them_statelessly() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(TOOLS))
        .mount(&server)
        .await;
    let model = ModelSpec::new("gpt-5.1");
    let entries = vec![AiEntry::user(1, "uptime?")];
    let p = provider(&server, Protocol::Responses);
    let (result, events) = run(&p, &request(&model, &entries, &[])).await;
    let entry = result.unwrap();
    let uptime = call("call_kX1", "run_command", "{\"command\":\"uptime\"}");
    let screen = call("call_pQ2", "read_terminal", "{}");
    assert_eq!(
        events,
        vec![
            reasoning("**Checking uptime**"),
            reasoning("\n\n"),
            reasoning("The user wants"),
            reasoning(" the uptime."),
            text("Checking."),
            StreamEvent::ToolCall(uptime.clone()),
            StreamEvent::ToolCall(screen.clone()),
            StreamEvent::Usage(cached_usage(1200, 85, 1152, 0)),
        ]
    );
    assert_eq!(
        entry.reasoning.as_deref(),
        Some("**Checking uptime**\n\nThe user wants the uptime.")
    );
    assert_eq!(entry.text, "Checking.");
    assert_eq!(entry.tool_calls, vec![uptime, screen]);
    assert_eq!(entry.finish, Finish::ToolCalls);
    let raw = entry.raw.as_array().unwrap();
    let kinds: Vec<&str> = raw.iter().map(|i| i["type"].as_str().unwrap()).collect();
    assert_eq!(
        kinds,
        ["reasoning", "message", "function_call", "function_call"]
    );
    assert_eq!(raw[0]["encrypted_content"], "gAAAAABo-enc-0a1");
    assert_eq!(raw[2]["id"], "fc_0a1");
    assert_eq!(raw[2]["arguments"], "{\"command\":\"uptime\"}");

    // GPT-5 reasons at Default: its reasoning is asked for summarized and encrypted.
    let first = &bodies(&server).await[0];
    assert_eq!(first["reasoning"], json!({"summary": "auto"}));
    assert_eq!(first["include"], json!(["reasoning.encrypted_content"]));
    assert!(first.get("tools").is_none(), "no tools, no field");

    // Replayed to the same model: every item without its id, the encrypted reasoning
    // included, then one `function_call_output` per call.
    let entries = vec![
        AiEntry::user(1, "uptime?"),
        AiEntry::assistant(2, entry.clone()),
        AiEntry::tool(3, "call_kX1", ToolStatus::Ok, "up 3 days"),
        AiEntry::tool(4, "call_pQ2", ToolStatus::Ok, "$ "),
    ];
    let tools = terminal_tools();
    let (result, _) = run(&p, &request(&model, &entries, &tools)).await;
    result.unwrap();
    let body = &bodies(&server).await[1];
    let mut expected = vec![json!({"type": "message", "role": "user", "content": "uptime?"})];
    expected.extend(replayed(&entry.raw));
    expected.push(
        json!({"type": "function_call_output", "call_id": "call_kX1",
                         "output": "up 3 days"}),
    );
    expected.push(json!({"type": "function_call_output", "call_id": "call_pQ2", "output": "$ "}));
    assert_eq!(body["input"], Value::Array(expected));
    assert_eq!(body["input"][1]["encrypted_content"], "gAAAAABo-enc-0a1");
    assert!(body.get("previous_response_id").is_none());
    assert_eq!(body["store"], false);
}

#[test]
fn responses_rebuilds_after_a_switch_and_leaves_out_reasoning_it_cannot_send() {
    let tools = terminal_tools();
    let raw = json!([
        // From a request that did not ask for encrypted reasoning: only its id names it.
        {"id": "rs_1", "type": "reasoning", "encrypted_content": null,
         "summary": [{"type": "summary_text", "text": "Plan."}]},
        // gpt-oss on vLLM returns its reasoning as text.
        {"id": "rs_2", "type": "reasoning", "summary": [],
         "content": [{"type": "reasoning_text", "text": "Think."}]},
        {"id": "msg_1", "type": "message", "role": "assistant", "status": "completed",
         "content": [{"type": "output_text", "text": "Checking.", "annotations": []}]},
        {"id": "fc_1", "type": "function_call", "call_id": "c1", "name": "run_command",
         "arguments": "{\"command\":\"df\"}", "status": "completed"}
    ]);
    let entries = vec![
        AiEntry::user(1, "df?"),
        assistant_entry(
            "gpt-oss-120b",
            "Checking.",
            vec![call("c1", "run_command", "{\"command\":\"df\"}")],
            raw.clone(),
        ),
        AiEntry::tool(3, "c1", ToolStatus::Ok, "42%"),
    ];

    let same = ModelSpec::new("gpt-oss-120b");
    let built = request::responses(&request(&same, &entries, &tools), true, true);
    assert!(built.used_raw);
    assert_eq!(
        built.body["input"],
        json!([
            {"type": "message", "role": "user", "content": "df?"},
            without_id(&raw[1]),
            without_id(&raw[2]),
            without_id(&raw[3]),
            {"type": "function_call_output", "call_id": "c1", "output": "42%"}
        ])
    );

    // Another model: rebuilt from the fields, so no reasoning goes along.
    let other = ModelSpec::new("gpt-5-mini");
    let built = request::responses(&request(&other, &entries, &tools), true, true);
    assert!(!built.used_raw);
    assert_eq!(
        built.body["input"],
        json!([
            {"type": "message", "role": "user", "content": "df?"},
            {"type": "message", "role": "assistant", "content": "Checking."},
            {"type": "function_call", "call_id": "c1", "name": "run_command",
             "arguments": "{\"command\":\"df\"}"},
            {"type": "function_call_output", "call_id": "c1", "output": "42%"}
        ])
    );
}

#[test]
fn responses_with_tools_keep_function_calls_and_rebuild_cut_off_arguments() {
    let tools = terminal_tools();
    let model = ModelSpec::new("gpt-5.1");
    let entries = tool_history(Protocol::Responses, &model.id);
    let built = request::responses(&request(&model, &entries, &tools), true, true);
    let output = |id: &str, text: &str| json!({"type": "function_call_output", "call_id": id, "output": text});
    let mut expected = vec![json!({"type": "message", "role": "user",
                                   "content": "check the disk and restart nginx"})];
    expected.extend(replayed(&raw_of(&entries, 1)));
    expected.extend([
        output("c1", "Use% 42%"),
        output(
            "c2",
            "The user rejected this tool call. The user's reason: too risky",
        ),
        output("c3", "The tool call was cancelled before it finished."),
        // Rebuilt, not raw: its arguments are blank and cut off, sent as `{}`.
        json!({"type": "function_call", "call_id": "c4", "name": "read_terminal",
               "arguments": "{}"}),
        json!({"type": "function_call", "call_id": "c5", "name": "run_command",
               "arguments": "{}"}),
        output("c4", "(no output)"),
        output("c5", "The tool call was cancelled before it finished."),
        json!({"type": "message", "role": "user", "content": "Summarize the conversation."}),
    ]);
    expected.extend(replayed(&raw_of(&entries, 8)));
    expected.push(json!({"type": "message", "role": "user", "content": "thanks"}));
    assert_eq!(built.body["input"], Value::Array(expected));
    assert!(built.used_raw);
    assert_eq!(built.body["tools"].as_array().unwrap().len(), tools.len());
}

#[test]
fn responses_without_tools_sends_calls_and_results_as_text() {
    let model = ModelSpec::new("gpt-5.1");
    let entries = tool_history(Protocol::Responses, &model.id);
    let built = request::responses(&request(&model, &entries, &[]), true, true);
    let user = |text: &str| json!({"type": "message", "role": "user", "content": text});
    let assistant = |text: &str| json!({"type": "message", "role": "assistant", "content": text});
    let mut expected = vec![
        user("check the disk and restart nginx"),
        assistant(FLAT_A),
        user(RESULT_1),
        user(RESULT_2),
        user(RESULT_3),
        assistant(FLAT_B),
        user(RESULT_4),
        user(RESULT_5),
        user("Summarize the conversation."),
    ];
    // No tool calls: raw is replayed as before, encrypted reasoning included.
    expected.extend(replayed(&raw_of(&entries, 8)));
    expected.push(user("thanks"));
    assert_eq!(built.body["input"], Value::Array(expected));
    assert!(built.used_raw);
    assert!(built.body.get("tools").is_none());
    assert!(
        input_types(&built.body)
            .iter()
            .all(|kind| matches!(*kind, "message" | "reasoning")),
        "{:?}",
        input_types(&built.body)
    );
}

#[test]
fn raw_of_another_protocol_is_rebuilt() {
    let tools = terminal_tools();
    let model = ModelSpec::new("relay-model");
    // Anthropic content blocks to a provider that now speaks Responses.
    let entries = tool_history(Protocol::Anthropic, &model.id);
    let built = request::responses(&request(&model, &entries, &tools), true, true);
    assert!(!built.used_raw);
    assert!(
        input_types(&built.body)
            .iter()
            .all(|kind| matches!(*kind, "message" | "function_call" | "function_call_output"))
    );
    // Responses items to one that now speaks Anthropic Messages.
    let entries = tool_history(Protocol::Responses, &model.id);
    let built = request::anthropic(&request(&model, &entries, &tools), true, true);
    assert!(!built.used_raw);
    // A Chat Completions message object is no item array either.
    let entries = tool_history(Protocol::ChatCompletions, &model.id);
    let built = request::responses(&request(&model, &entries, &tools), true, true);
    assert!(!built.used_raw);
}

#[test]
fn models_that_reason_by_default() {
    for id in [
        "o1",
        "o3-mini",
        "o4-mini-2025-04-16",
        "gpt-5",
        "gpt-5.1",
        "gpt-5-mini",
        "GPT-6",
        "openai/gpt-5",
        "gpt-5-codex",
        "codex-mini-latest",
        "gpt-oss-120b",
    ] {
        assert!(request::reasons_by_default(&ModelSpec::new(id)), "{id}");
    }
    for id in [
        "gpt-4.1",
        "gpt-4o",
        "gpt-5-chat-latest",
        "gpt-5.1-chat-latest",
        "omni-moderation-latest",
        "grok-4",
        "deepseek-chat",
        "",
    ] {
        assert!(!request::reasons_by_default(&ModelSpec::new(id)), "{id}");
    }
}

#[test]
fn responses_sends_reasoning_within_the_models_levels() {
    let entries = vec![AiEntry::user(1, "hi")];
    let build = |model: &ModelSpec, effort: Option<Effort>, fields: bool, summary: bool| {
        request::responses(
            &with_effort(request(model, &entries, &[]), effort),
            fields,
            summary,
        )
    };
    let plain = ModelSpec::new("gpt-4.1");
    let reasoner = ModelSpec::new("gpt-5.1");
    let encrypted = json!(["reasoning.encrypted_content"]);

    // A model that does not reason gets nothing at Default, and the level when one is chosen.
    let built = build(&plain, None, true, true);
    assert!(built.body.get("reasoning").is_none());
    assert!(built.body.get("include").is_none());
    assert!(!built.sent_effort && !built.sent_summary);
    assert_eq!(built.effort, None);
    let built = build(&plain, Some(Max), true, true);
    assert_eq!(
        built.body["reasoning"],
        json!({"effort": "high", "summary": "auto"}),
        "unknown levels stop at High"
    );
    assert_eq!(built.body["include"], encrypted);
    assert!(built.sent_effort && built.sent_summary);
    assert_eq!(built.effort, Some(High));

    // A model that reasons anyway gets the summary and encrypted reasoning at Default.
    let built = build(&reasoner, None, true, true);
    assert_eq!(built.body["reasoning"], json!({"summary": "auto"}));
    assert_eq!(built.body["include"], encrypted);
    assert!(built.sent_effort && built.sent_summary);
    assert_eq!(built.effort, None);

    // Levels the model list gives.
    let o3 = listed("o3", &[Low, Medium, High, Xhigh], false, None);
    let built = build(&o3, Some(Max), true, true);
    assert_eq!(built.body["reasoning"]["effort"], "xhigh");

    // The retry without summaries keeps the level and the encrypted reasoning.
    let built = build(&reasoner, Some(Low), true, false);
    assert_eq!(built.body["reasoning"], json!({"effort": "low"}));
    assert_eq!(built.body["include"], encrypted);
    assert!(built.sent_effort && !built.sent_summary);
    let built = build(&reasoner, None, true, false);
    assert!(built.body.get("reasoning").is_none());
    assert_eq!(built.body["include"], encrypted);

    // The retry without reasoning fields sends none.
    let built = build(&reasoner, Some(High), false, true);
    assert!(built.body.get("reasoning").is_none());
    assert!(built.body.get("include").is_none());
    assert!(!built.sent_effort && !built.sent_summary);
    assert_eq!(built.effort, None);
}

#[test]
fn responses_retry_conditions() {
    let http = |status, message: &str| AiError::Http {
        status,
        message: message.into(),
    };
    for summary in [
        "Your organization must be verified to generate reasoning summaries.",
        "Unsupported parameter: 'reasoning.summary' is not supported with this model.",
    ] {
        assert!(rejects_summary(&http(400, summary)), "{summary}");
    }
    for reasoning in [
        "Unsupported parameter: 'reasoning.effort' is not supported with this model.",
        "Encrypted content is not supported with this model.",
        "Invalid value: 'reasoning.encrypted_content'.",
        "Unknown parameter: 'reasoning'.",
    ] {
        assert!(rejects_reasoning(&http(400, reasoning)), "{reasoning}");
        assert!(!rejects_summary(&http(400, reasoning)), "{reasoning}");
    }
    assert!(!rejects_reasoning(&http(
        400,
        "Invalid 'input[0].content'."
    )));
    assert!(!rejects_reasoning(&http(500, "reasoning server error")));
    assert!(!rejects_summary(&http(503, "summary unavailable")));
}

#[tokio::test]
async fn responses_retries_without_summary_and_then_without_reasoning() {
    // An unverified organization: summaries are refused, the level stays.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"reasoning": {"summary": "auto"}})))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": {
            "message": "Your organization must be verified to generate reasoning summaries.",
            "type": "invalid_request_error", "param": "reasoning.summary",
            "code": "unsupported_value"}})))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse(TEXT))
        .mount(&server)
        .await;
    let model = ModelSpec::new("gpt-5.1");
    let entries = vec![AiEntry::user(1, "hi")];
    let p = provider(&server, Protocol::Responses);
    let (result, events) = run(&p, &with_effort(request(&model, &entries, &[]), Some(High))).await;
    result.unwrap();
    assert!(!events.contains(&StreamEvent::EffortIgnored));
    let sent = bodies(&server).await;
    assert_eq!(sent.len(), 2);
    assert_eq!(
        sent[0]["reasoning"],
        json!({"effort": "high", "summary": "auto"})
    );
    assert_eq!(sent[1]["reasoning"], json!({"effort": "high"}));
    assert_eq!(sent[1]["include"], json!(["reasoning.encrypted_content"]));

    // A model without reasoning: the fields go, and the panel hears that the level did too.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(
            json!({"include": ["reasoning.encrypted_content"]}),
        ))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": {
            "message": "Encrypted content is not supported with this model.",
            "type": "invalid_request_error", "param": "include", "code": null}})))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse(TEXT))
        .mount(&server)
        .await;
    let model = ModelSpec::new("gpt-4.1");
    let p = provider(&server, Protocol::Responses);
    let (result, events) = run(&p, &with_effort(request(&model, &entries, &[]), Some(Low))).await;
    result.unwrap();
    assert_eq!(events[0], StreamEvent::EffortIgnored);
    let sent = bodies(&server).await;
    assert_eq!(sent.len(), 2);
    assert!(sent[1].get("reasoning").is_none());
    assert!(sent[1].get("include").is_none());

    // Any other 4xx is not retried.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": {
            "message": "Invalid 'input[0].content': string too long.",
            "type": "invalid_request_error"}})))
        .mount(&server)
        .await;
    let p = provider(&server, Protocol::Responses);
    let (result, _) = run(&p, &with_effort(request(&model, &entries, &[]), Some(Low))).await;
    assert!(matches!(result, Err(AiError::Http { status: 400, .. })));
    assert_eq!(bodies(&server).await.len(), 1);
}

#[tokio::test]
async fn responses_cut_off_by_the_output_limit_over_http() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(INCOMPLETE))
        .mount(&server)
        .await;
    let model = ModelSpec::new("o3");
    let entries = vec![AiEntry::user(1, "read the log")];
    let (result, events) = run(
        &provider(&server, Protocol::Responses),
        &request(&model, &entries, &[]),
    )
    .await;
    let entry = result.unwrap();
    assert_eq!(entry.finish, Finish::Length);
    assert_eq!(entry.text, "The log shows");
    assert_eq!(entry.usage, Some(usage(300, 4000)));
    assert_eq!(events.last(), Some(&StreamEvent::Usage(usage(300, 4000))));
}

#[test]
fn responses_filtered_failed_errored_and_interrupted_streams() {
    let (result, _) = assemble(ResponsesStream::new(), INCOMPLETE, 64);
    assert_eq!(result.unwrap().finish, Finish::Length);

    let filtered = event(
        "response.incomplete",
        &json!({"type": "response.incomplete", "response": {
            "status": "incomplete", "incomplete_details": {"reason": "content_filter"},
            "output": [{"type": "message", "role": "assistant",
                        "content": [{"type": "output_text", "text": "Par"}]}]}}),
    );
    let (result, events) = assemble(ResponsesStream::new(), &filtered, 4096);
    let assembled = result.unwrap();
    assert_eq!(assembled.finish, Finish::Refused);
    assert_eq!(events, vec![text("Par")]);

    let refusal = [
        event(
            "response.output_item.added",
            &json!({"type": "response.output_item.added", "output_index": 0,
                    "item": {"type": "message", "role": "assistant", "content": []}}),
        ),
        event(
            "response.refusal.delta",
            &json!({"type": "response.refusal.delta", "output_index": 0, "delta": "I can't"}),
        ),
        event(
            "response.output_item.done",
            &json!({"type": "response.output_item.done", "output_index": 0,
                    "item": {"type": "message", "role": "assistant",
                             "content": [{"type": "refusal", "refusal": "I can't help."}]}}),
        ),
        event(
            "response.completed",
            &json!({"type": "response.completed", "response": {"status": "completed"}}),
        ),
    ]
    .concat();
    let (result, events) = assemble(ResponsesStream::new(), &refusal, 4096);
    let assembled = result.unwrap();
    assert_eq!(assembled.finish, Finish::Refused);
    assert_eq!(assembled.text, "");
    assert!(events.is_empty());

    let (result, events) = assemble(ResponsesStream::new(), FAILED, 64);
    assert_eq!(
        result.unwrap_err(),
        AiError::Http {
            status: 500,
            message: "The server had an error while processing your request.".into()
        }
    );
    assert_eq!(events, vec![text("Partial")]);

    let errored = event(
        "error",
        &json!({"type": "error", "code": "rate_limit_exceeded",
                "message": "Rate limit reached", "param": null, "sequence_number": 2}),
    );
    assert_eq!(
        assemble(ResponsesStream::new(), &errored, 4096)
            .0
            .unwrap_err(),
        AiError::Http {
            status: 429,
            message: "Rate limit reached".into()
        }
    );

    // Only `response.completed` or `response.incomplete` ends a response.
    let (result, events) = assemble(ResponsesStream::new(), CUT_OFF, 64);
    assert!(matches!(result, Err(AiError::Network(_))));
    assert_eq!(events, vec![text("Half an ans")]);
    let (result, _) = assemble(ResponsesStream::new(), "data: [DONE]\n\n", 64);
    assert!(matches!(result, Err(AiError::Network(_))));
    assert!(matches!(
        assemble(ResponsesStream::new(), "data: {nope\n\n", 64).0,
        Err(AiError::Protocol(_))
    ));
}

#[test]
fn any_chunking_assembles_the_same_responses_response() {
    let (whole, whole_events) = assemble(ResponsesStream::new(), TOOLS, TOOLS.len());
    let whole = whole.unwrap();
    for size in [1, 3, 17, 256] {
        let (result, events) = assemble(ResponsesStream::new(), TOOLS, size);
        assert_eq!(result.unwrap(), whole, "chunks of {size}");
        assert_eq!(events, whole_events, "chunks of {size}");
    }
}

#[test]
fn items_without_item_events_come_from_the_terminal_event() {
    let stream = event(
        "response.completed",
        &json!({"type": "response.completed", "response": {
            "status": "completed",
            "output": [
                {"type": "reasoning", "summary": [{"type": "summary_text", "text": "Hmm."}]},
                {"type": "message", "role": "assistant",
                 "content": [{"type": "output_text", "text": "Run it."}]},
                {"type": "function_call", "name": "run_command", "arguments": "{\"command\":\"ls\"}"}
            ],
            "usage": {"input_tokens": 10, "output_tokens": 5}
        }}),
    );
    let (result, events) = assemble(ResponsesStream::new(), &stream, 4096);
    let assembled = result.unwrap();
    assert_eq!(assembled.finish, Finish::ToolCalls);
    assert_eq!(assembled.tool_calls.len(), 1);
    let id = assembled.tool_calls[0].id.clone();
    assert!(id.starts_with("call_"), "a fresh id: {id}");
    assert_eq!(
        assembled.raw[2]["call_id"],
        json!(id),
        "raw pairs with the result"
    );
    assert_eq!(
        events,
        vec![
            reasoning("Hmm."),
            text("Run it."),
            StreamEvent::ToolCall(call(&id, "run_command", "{\"command\":\"ls\"}")),
        ]
    );
    assert_eq!(assembled.usage, Some(usage(10, 5)));
}

#[test]
fn text_deltas_without_item_events_make_a_message_item() {
    let stream = [
        event(
            "response.output_text.delta",
            &json!({"type": "response.output_text.delta", "delta": "Hel"}),
        ),
        event(
            "response.output_text.delta",
            &json!({"type": "response.output_text.delta", "delta": "lo."}),
        ),
        event(
            "response.completed",
            &json!({"type": "response.completed", "response": {"status": "completed"}}),
        ),
    ]
    .concat();
    let (result, events) = assemble(ResponsesStream::new(), &stream, 4096);
    let assembled = result.unwrap();
    assert_eq!(events, vec![text("Hel"), text("lo.")]);
    assert_eq!(
        assembled.raw,
        json!([{"type": "message", "role": "assistant",
                "content": [{"type": "output_text", "text": "Hello.", "annotations": []}]}])
    );
}

#[test]
fn a_call_whose_done_never_came_keeps_its_streamed_arguments() {
    let stream = [
        event(
            "response.output_item.added",
            &json!({"type": "response.output_item.added", "output_index": 0, "item": {
                "id": "fc_1", "type": "function_call", "call_id": "c1", "name": "run_command",
                "arguments": ""}}),
        ),
        event(
            "response.function_call_arguments.delta",
            &json!({"type": "response.function_call_arguments.delta", "item_id": "fc_1",
                    "delta": "{\"command\":\"ls"}),
        ),
        event(
            "response.incomplete",
            &json!({"type": "response.incomplete", "response": {
                "status": "incomplete", "incomplete_details": {"reason": "max_output_tokens"}}}),
        ),
    ]
    .concat();
    let (result, events) = assemble(ResponsesStream::new(), &stream, 4096);
    let assembled = result.unwrap();
    assert_eq!(assembled.finish, Finish::Length);
    let cut = call("c1", "run_command", "{\"command\":\"ls");
    assert_eq!(events, vec![StreamEvent::ToolCall(cut.clone())]);
    assert_eq!(assembled.tool_calls, vec![cut]);
    assert_eq!(assembled.raw[0]["arguments"], "{\"command\":\"ls");
}

#[tokio::test]
async fn responses_accepts_a_non_streamed_reply() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "resp_1", "object": "response", "status": "completed",
            "output": [
                {"id": "msg_1", "type": "message", "role": "assistant",
                 "content": [{"type": "output_text", "text": "Hi there."}]}
            ],
            "usage": {"input_tokens": 20, "output_tokens": 3}
        })))
        .mount(&server)
        .await;
    let model = ModelSpec::new("local-model");
    let entries = vec![AiEntry::user(1, "hi")];
    let (result, events) = run(
        &provider(&server, Protocol::Responses),
        &request(&model, &entries, &[]),
    )
    .await;
    let entry = result.unwrap();
    assert_eq!(entry.text, "Hi there.");
    assert_eq!(entry.finish, Finish::Stop);
    assert_eq!(
        events,
        vec![text("Hi there."), StreamEvent::Usage(usage(20, 3))]
    );

    // A failed response in one piece is an error.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "failed", "output": [],
            "error": {"code": "invalid_prompt", "message": "Flagged."}
        })))
        .mount(&server)
        .await;
    let (result, _) = run(
        &provider(&server, Protocol::Responses),
        &request(&model, &entries, &[]),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        AiError::Http {
            status: 400,
            message: "Flagged.".into()
        }
    );
}
