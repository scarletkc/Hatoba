//! Deferred MCP tools (AI-44) in turns against mock Chat Completions and Anthropic Messages
//! providers: the threshold, `search_tools` and `call_mcp_tool` with their approvals, the
//! `mcp_tools` notes, servers that go away, and that the tool definitions and the system prompt
//! stay byte for byte the same from request to request.

use super::*;
use crate::commands::mcp::{proxy_info, set_always_allow};
use crate::mcp::tests::{hanging_server, put_server, set_enabled, stdio_server};
use hatoba_core::model::{McpServer, McpTransport};

/// Tools of the `big` mock server, each with a long description, so that their definitions
/// come to more than a tenth of [`WINDOW`] while the list of their names stays within 2% of it.
const BIG_TOOLS: usize = 40;
/// The context window of the deferred tests' model.
const WINDOW: u64 = 30_000;

fn mcp_reply(id: &Value, result: Value) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_json(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// A Streamable HTTP MCP server with [`BIG_TOOLS`] tools, `tool_00` and on.
async fn big_mock() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/mcp"))
        .respond_with(|request: &Request| {
            match request.method.as_str() {
                "GET" => return ResponseTemplate::new(405),
                "DELETE" => return ResponseTemplate::new(200),
                _ => {}
            }
            let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            let Some(id) = body.get("id").cloned() else {
                return ResponseTemplate::new(202);
            };
            match body["method"].as_str().unwrap_or_default() {
                "initialize" => mcp_reply(
                    &id,
                    json!({
                        "protocolVersion": "2025-03-26",
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "big", "version": "1.0.0"}
                    }),
                )
                .insert_header("mcp-session-id", "big-1"),
                "tools/list" => {
                    let tools: Vec<Value> = (0..BIG_TOOLS)
                        .map(|n| {
                            json!({
                                "name": format!("tool_{n:02}"),
                                "description": format!(
                                    "Padding tool {n:02}. {}",
                                    "It stands for one of the many tools a large server lists. "
                                        .repeat(5)
                                ),
                                "inputSchema": {"type": "object",
                                                "properties": {"value": {"type": "string"}},
                                                "required": ["value"]}
                            })
                        })
                        .collect();
                    mcp_reply(&id, json!({"tools": tools}))
                }
                "tools/call" => mcp_reply(
                    &id,
                    json!({"content": [{"type": "text", "text": format!(
                        "big {}", body["params"]["name"].as_str().unwrap_or("?"))}]}),
                ),
                _ => mcp_reply(&id, json!({})),
            }
        })
        .mount(&server)
        .await;
    server
}

fn big_server(mock: &MockServer) -> McpServer {
    McpServer {
        name: "big".into(),
        transport: McpTransport::Http {
            url: format!("{}/mcp", mock.uri()),
            headers: Default::default(),
        },
        always_ask: false,
        updated_at: 0,
    }
}

/// The two servers of the deferred tests: `files` (the stdio mock, 5 tools, `notify` adds one)
/// and `big` (HTTP, on by default). Returns their ids and the `big` mock, which must live on.
async fn servers(f: &Fixture) -> (String, String, MockServer) {
    let mock = big_mock().await;
    let files = put_server(&f.vault, stdio_server("files"));
    set_enabled(&f.vault, &files, true);
    let big = put_server(&f.vault, big_server(&mock));
    (files, big, mock)
}

/// The text a user entry was stored with.
fn user_text(entry: &AiEntry) -> &str {
    match &entry.body {
        EntryBody::User { text, .. } => text,
        other => panic!("expected a user entry, got {other:?}"),
    }
}

/// The user entries of a conversation.
fn user_texts(f: &Fixture, conversation_id: &str) -> Vec<String> {
    f.entries(conversation_id)
        .iter()
        .filter(|e| matches!(e.body, EntryBody::User { .. }))
        .map(|e| user_text(e).to_owned())
        .collect()
}

fn call_proxy(id: &str, tool: &str, arguments: Value) -> (String, String, Value) {
    (
        id.to_owned(),
        "call_mcp_tool".to_owned(),
        json!({"tool": tool, "arguments": arguments}),
    )
}

/// A Chat Completions response that calls tools: `(id, name, arguments)`.
fn calls_of(list: &[(String, String, Value)]) -> ResponseTemplate {
    let list: Vec<(&str, &str, Value)> = list
        .iter()
        .map(|(id, name, args)| (id.as_str(), name.as_str(), args.clone()))
        .collect();
    calls(&list)
}

impl Sink {
    /// Waits for the `n`th response that asked for tool results.
    async fn tool_calls_done(&self, n: usize) {
        self.until("done with tool calls", |events| {
            events
                .iter()
                .filter(|e| {
                    **e == AiTurnEvent::Done {
                        finish: AiFinish::ToolCalls,
                    }
                })
                .count()
                >= n
        })
        .await;
    }
}

/// What every request of a deferred conversation sends before its messages: the tools and the
/// system prompt, serialized.
fn prefix(body: &Value) -> (String, String) {
    (
        serde_json::to_string(&body["tools"]).unwrap(),
        match body.get("system") {
            Some(system) => serde_json::to_string(system).unwrap(),
            None => system_of(body),
        },
    )
}

const BUILTIN: [&str; 8] = [
    "read_terminal",
    "run_command",
    "send_input",
    "read_file",
    "edit_file",
    "write_file",
    "fetch_url",
    "read_skill",
];

fn deferred_tools() -> Vec<String> {
    BUILTIN
        .iter()
        .chain(&["search_tools", "call_mcp_tool"])
        .map(|s| (*s).to_owned())
        .collect()
}

/// Below the threshold the MCP tools go to the provider directly, as before AI-44, and no message
/// lists them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_tools_below_the_threshold_are_offered_directly() {
    let f = Fixture::new(vec![answer("One."), answer("Two.")]).await;
    f.set_context_window(1_000_000);
    let (_files, _big, _mock) = servers(&f).await;
    let (conv, _) = f.exchange(None, "hello", f.context()).await;
    f.exchange(Some(&conv), "again", f.context()).await;

    let requests = f.requests().await;
    for request in &requests {
        let names = tool_names(request);
        assert!(names.contains(&"mcp__files__echo".to_owned()), "{names:?}");
        assert!(names.contains(&"mcp__big__tool_00".to_owned()));
        assert!(
            !names
                .iter()
                .any(|n| n == "search_tools" || n == "call_mcp_tool")
        );
    }
    assert_eq!(user_texts(&f, &conv), ["hello", "again"]);
    f.mcp.shutdown().await;
}

/// AI-44 on Chat Completions: past a tenth of the window the first message lists the tools, the
/// requests offer `search_tools` and `call_mcp_tool` with the same tools and system prompt before
/// and after a search and a call, a search finds by keyword and by name, a call runs the tool it
/// names under that tool's approvals, arguments are checked against its schema, a tool added by
/// `tools/list_changed` is found at once, and the next message lists the new list.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deferred_mcp_tools_are_searched_and_called_through_fixed_tools() {
    let f = Fixture::new(vec![
        calls(&[("c1", "search_tools", json!({"query": "echo text"}))]),
        calls_of(&[
            call_proxy("c2", "mcp__files__echo", json!({"text": "hi"})),
            (
                "c3".into(),
                "call_mcp_tool".into(),
                json!({"tool": "mcp__files__echo", "arguments": "{\"text\": \"as a string\"}"}),
            ),
            call_proxy("c4", "mcp__files__echo", json!({"text": 5})),
            call_proxy("c5", "mcp__nope__x", json!({})),
            call_proxy("c6", "mcp__files__notify", json!({})),
            (
                "c7".into(),
                "search_tools".into(),
                json!({"query": "mcp__files__added", "max_results": 1}),
            ),
            call_proxy("c8", "mcp__files__added", json!({})),
        ]),
        answer("Done."),
        answer("Second."),
        answer("Third."),
    ])
    .await;
    f.set_context_window(WINDOW);
    let (files, _big, _mock) = servers(&f).await;

    let (started, sink) = f.send(None, "use the tools").await;
    let conv = started.conversation.id;
    // The first message lists the tools: `big` first, servers sorted by name.
    let first = user_texts(&f, &conv).remove(0);
    assert!(
        first.starts_with("<mcp_tools servers=\"2\" tools=\"45\">\n"),
        "{first}"
    );
    assert!(first.contains("- big (40 tools): mcp__big__tool_00, mcp__big__tool_01,"));
    assert!(first.contains(
        "- files (5 tools): mcp__files__echo, mcp__files__fail, mcp__files__notify, \
         mcp__files__pid, mcp__files__env\n</mcp_tools>\n\nuse the tools"
    ));

    sink.tool_calls_done(1).await;
    let c1 = f.run(&conv, "c1", None, None).await.unwrap();
    let (_, status, found) = view_content(&c1);
    assert_eq!(status, AiToolStatus::Ok);
    assert!(found.contains("## mcp__files__echo\nServer: files\nDescription: Echoes text\nInput schema: {\"properties\":{\"text\":{\"type\":\"string\"}},\"type\":\"object\"}"), "{found}");

    sink.tool_calls_done(2).await;
    // AI-31, AI-19: the card's tool is the one the call names, with its Always allow.
    set_always_allow(&mut lock(&f.vault), &files, Some("echo"), true).unwrap();
    let info = |arguments: Value| {
        proxy_info(
            &lock(&f.vault),
            &f.mcp,
            &conv,
            &serde_json::to_string(&arguments).unwrap(),
        )
    };
    let echo = info(json!({"tool": "mcp__files__echo", "arguments": {"text": "hi"}}));
    assert!(echo.runs);
    let tool = echo.tool.unwrap();
    assert_eq!(
        (
            tool.server_id.as_str(),
            tool.server_name.as_str(),
            tool.tool.tool.as_str()
        ),
        (files.as_str(), "files", "echo")
    );
    assert!(tool.tool.always_allow);
    // A call that cannot run gets its error without asking.
    let mismatch = info(json!({"tool": "mcp__files__echo", "arguments": {"text": 5}}));
    assert!(!mismatch.runs && mismatch.tool.is_some());
    let unknown = info(json!({"tool": "mcp__nope__x"}));
    assert!(!unknown.runs && unknown.tool.is_none());

    let c2 = f.run(&conv, "c2", None, None).await.unwrap();
    assert_eq!(view_content(&c2), ("c2", AiToolStatus::Ok, "hi"));
    let c3 = f.run(&conv, "c3", None, None).await.unwrap();
    assert_eq!(view_content(&c3), ("c3", AiToolStatus::Ok, "as a string"));
    let c4 = f.run(&conv, "c4", None, None).await.unwrap();
    assert_eq!(
        view_content(&c4),
        (
            "c4",
            AiToolStatus::Error,
            "The call did not run: its arguments do not match the input schema of \
             mcp__files__echo.\n- `text` must be a string, not an integer.\nCall it again with \
             arguments that match this schema:\n\
             {\"properties\":{\"text\":{\"type\":\"string\"}},\"type\":\"object\"}"
        )
    );
    let c5 = f.run(&conv, "c5", None, None).await.unwrap();
    assert_eq!(view_content(&c5).1, AiToolStatus::Error);
    assert!(
        view_content(&c5)
            .2
            .contains("this conversation has no MCP tool named \"mcp__nope__x\" now"),
        "{c5:?}"
    );
    let c6 = f.run(&conv, "c6", None, None).await.unwrap();
    assert_eq!(view_content(&c6), ("c6", AiToolStatus::Ok, "notified"));

    // openai/codex#33266: the tool that `tools/list_changed` added during the turn is found by
    // the next search, before any request lists the tools again, and then resolves for a call.
    let c7 = f.run(&conv, "c7", None, None).await.unwrap();
    assert!(
        view_content(&c7)
            .2
            .contains("1 match of the 46 MCP tools of this conversation"),
        "{c7:?}"
    );
    assert!(view_content(&c7).2.contains("## mcp__files__added\n"));
    let c8 = f.run(&conv, "c8", None, None).await.unwrap();
    // The mock lists `added` but cannot run it: the server's error, not Hatoba's.
    assert_eq!(view_content(&c8).1, AiToolStatus::Error);
    assert!(
        view_content(&c8).2.contains("Unknown tool: added"),
        "{c8:?}"
    );
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;

    // The next message lists the new list; the one after it, with nothing changed, none.
    f.exchange(Some(&conv), "again", f.context()).await;
    f.exchange(Some(&conv), "and again", f.context()).await;
    let texts = user_texts(&f, &conv);
    assert!(
        texts[1].starts_with("<mcp_tools servers=\"2\" tools=\"46\">\n"),
        "{}",
        texts[1]
    );
    assert!(texts[1].contains("mcp__files__env, mcp__files__added\n</mcp_tools>\n\nagain"));
    assert_eq!(texts[2], "and again");

    let requests = f.requests().await;
    assert_eq!(requests.len(), 5);
    // The tool definitions and the system prompt never changed: the provider's cache serves them.
    assert_eq!(tool_names(&requests[0]), deferred_tools());
    for request in &requests[1..] {
        assert_eq!(prefix(request), prefix(&requests[0]));
    }
    // What the search returned reached the model in its result.
    assert!(
        tool_message(&requests[1], "c1")["content"]
            .as_str()
            .unwrap()
            .contains("## mcp__files__echo")
    );
    f.mcp.shutdown().await;
}

/// The conversation stays deferred when its tools shrink; a server switched off in the tools menu
/// is left out of the next list, a call of its tool does not run, and a call of a server deleted
/// since the request does not run either.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_deferred_conversation_keeps_its_tools_when_servers_go() {
    let f = Fixture::new(vec![
        answer("One."),
        calls_of(&[call_proxy("c1", "mcp__big__tool_00", json!({"value": "x"}))]),
        answer("Two."),
        calls_of(&[call_proxy("c2", "mcp__files__echo", json!({"text": "x"}))]),
        answer("Three."),
        answer("Four."),
    ])
    .await;
    f.set_context_window(WINDOW);
    let (files, big, _mock) = servers(&f).await;
    let (conv, _) = f.exchange(None, "hello", f.context()).await;

    // AI-30: `big` switched off for the conversation.
    let without_big = AiTurnContext {
        disabled_mcp_servers: vec![big.clone()],
        ..f.context()
    };
    let (_, sink) = f
        .send_with(Some(&conv), "no big", without_big.clone())
        .await;
    sink.tool_calls_done(1).await;
    let gone = proxy_info(
        &lock(&f.vault),
        &f.mcp,
        &conv,
        r#"{"tool": "mcp__big__tool_00", "arguments": {"value": "x"}}"#,
    );
    assert!(!gone.runs && gone.tool.is_none());
    let c1 = f.run(&conv, "c1", None, None).await.unwrap();
    assert_eq!(view_content(&c1).1, AiToolStatus::Error);
    assert!(
        view_content(&c1)
            .2
            .contains("no MCP tool named \"mcp__big__tool_00\"")
    );
    sink.ended().await;
    f.idle(&conv).await;

    // A server deleted (on another device, say) after the request offered its tool.
    let (_, sink) = f.send_with(Some(&conv), "files", without_big.clone()).await;
    sink.tool_calls_done(1).await;
    lock(&f.vault).delete(&files).unwrap();
    let deleted = proxy_info(
        &lock(&f.vault),
        &f.mcp,
        &conv,
        r#"{"tool": "mcp__files__echo", "arguments": {"text": "x"}}"#,
    );
    assert!(!deleted.runs);
    let c2 = f.run(&conv, "c2", None, None).await.unwrap();
    assert_eq!(
        view_content(&c2),
        (
            "c2",
            AiToolStatus::Error,
            "The MCP server \"files\" was deleted, so the tool did not run."
        )
    );
    sink.ended().await;
    f.idle(&conv).await;
    f.exchange(Some(&conv), "nothing left", without_big).await;

    let texts = user_texts(&f, &conv);
    assert!(texts[0].starts_with("<mcp_tools servers=\"2\" tools=\"45\">"));
    assert!(
        texts[1].starts_with("<mcp_tools servers=\"1\" tools=\"5\">"),
        "{}",
        texts[1]
    );
    assert!(texts[1].contains("- files (5 tools): mcp__files__echo,"));
    assert_eq!(texts[2], "files");
    assert!(
        texts[3].starts_with(
            "<mcp_tools servers=\"0\" tools=\"0\">\nNo MCP tools are available from this \
             message on.\n</mcp_tools>\n\nnothing left"
        ),
        "{}",
        texts[3]
    );
    // Five tools, then none: well below the threshold, and the tools stay as they were.
    let requests = f.requests().await;
    for request in &requests {
        assert_eq!(prefix(request), prefix(&requests[0]));
    }
    assert_eq!(tool_names(&requests[0]), deferred_tools());
    f.mcp.shutdown().await;
}

/// After a compaction the summary leaves the list out, so the next message lists the tools again,
/// although they did not change. The Compact request and a side question offer the
/// conversation's tools, so they keep its prefix, and the side question lists nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_compaction_lists_the_tools_again_and_keeps_the_prefix() {
    let f = Fixture::new(vec![
        answer("One."),
        answer("SUMMARY: one question."),
        answer("Side answer."),
        answer("Two."),
    ])
    .await;
    f.set_context_window(WINDOW);
    let (_files, _big, _mock) = servers(&f).await;
    let (conv, _) = f.exchange(None, "hello", f.context()).await;
    f.manager
        .compact(&f.vault, f.env.as_ref(), &conv, &f.context())
        .await
        .unwrap();
    let (side, _) = f.side(&conv, "what did I ask?", None).await;
    side.unwrap();
    f.exchange(Some(&conv), "again", f.context()).await;

    let texts = user_texts(&f, &conv);
    let list = &texts[0][..texts[0].find("hello").unwrap()];
    assert!(list.starts_with("<mcp_tools servers=\"2\" tools=\"45\">"));
    assert_eq!(texts[1], format!("{list}again"));

    let requests = f.requests().await;
    for request in &requests {
        assert_eq!(prefix(request), prefix(&requests[0]));
    }
    let side = user_messages(&requests[2]);
    assert!(!side.last().unwrap().contains("<mcp_tools"));
    f.mcp.shutdown().await;
}

/// A message waits for the MCP servers before it is stored; a stop meanwhile stores nothing, and
/// the lock meanwhile creates no conversation.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_message_stopped_while_the_servers_start_is_not_stored() {
    use crate::dto::McpServerState;

    let f = Fixture::new(vec![answer("One.")]).await;
    let (conv, _) = f.exchange(None, "hello", f.context()).await;
    let hanging = put_server(&f.vault, hanging_server("slow"));
    set_enabled(&f.vault, &hanging, true);

    let starting = |f: &Fixture| {
        f.mcp_events
            .states(&hanging)
            .contains(&McpServerState::Starting)
    };
    let (send, _) = spawn_send(&f, &conv, "second");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !starting(&f) {
        assert!(Instant::now() < deadline, "the server did not start");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    f.manager.stop(&f.vault, f.env.as_ref(), &conv);
    let stopped = send.await.unwrap().unwrap_err();
    assert_eq!(stopped.code, ErrorCode::Cancelled);
    assert_eq!(user_texts(&f, &conv), ["hello"]);

    // A new conversation, then the lock, while the server is still starting.
    let sink = Arc::new(Sink::default());
    let (manager, vault, env, context) = (f.manager.clone(), f.vault.clone(), f.env(), f.context());
    let send = tokio::spawn(async move {
        manager
            .send(
                &vault,
                env,
                AiSendInput {
                    conversation_id: None,
                    text: "new".into(),
                    context,
                },
                sink,
            )
            .await
            .map(|(started, _)| started)
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    {
        let mut v = lock(&f.vault);
        f.manager.stop_all(&mut v);
        v.lock();
    }
    assert!(send.await.unwrap().is_err());
    f.mcp.stop_all(&f.vault).await;
    lock(&f.vault).unlock(PW).unwrap();
    let conversations = lock(&f.vault)
        .items()
        .filter(|(_, item)| item.as_ai_conversation().is_some())
        .count();
    assert_eq!(conversations, 1);
    f.mcp.shutdown().await;
}

/// An Anthropic response that calls `name` with `input`, as the call `id`.
fn anthropic_use(id: &str, name: &str, input: &Value) -> ResponseTemplate {
    let events = [
        json!({"type": "message_start", "message": {"id": "msg_1", "type": "message",
               "role": "assistant", "model": "m1", "content": [], "stop_reason": null,
               "usage": {"input_tokens": 20, "cache_read_input_tokens": 80, "output_tokens": 1}}}),
        json!({"type": "content_block_start", "index": 0,
               "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}}),
        json!({"type": "content_block_delta", "index": 0,
               "delta": {"type": "input_json_delta", "partial_json": input.to_string()}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"},
               "usage": {"output_tokens": 5}}),
        json!({"type": "message_stop"}),
    ];
    anthropic_stream(&events)
}

/// AI-44 on Anthropic Messages: the same tools and system prompt (and the top-level
/// `cache_control`) in every request, before and after a search and a call; the schema reaches
/// the model in a `tool_result`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deferred_mcp_tools_keep_the_anthropic_prefix() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(Script(Mutex::new(
            vec![
                anthropic_use("u1", "search_tools", &json!({"query": "echo"})),
                anthropic_use(
                    "u2",
                    "call_mcp_tool",
                    &json!({"tool": "mcp__files__echo", "arguments": {"text": "hi"}}),
                ),
                anthropic_answer("Done."),
                anthropic_answer("Again."),
            ]
            .into(),
        )))
        .mount(&server)
        .await;
    let base_url = server.uri();
    let f = Fixture::with_base_url(Some(server), base_url);
    {
        let mut v = lock(&f.vault);
        let mut provider = v
            .get(&f.provider_id)
            .and_then(Item::as_ai_provider)
            .cloned()
            .unwrap();
        provider.protocol = AiProtocol::Anthropic;
        provider.models[0].context_window = Some(WINDOW);
        v.put(Some(&f.provider_id), Item::AiProvider(provider))
            .unwrap();
    }
    let (_files, _big, _mock) = servers(&f).await;

    let (started, sink) = f.send(None, "use the tools").await;
    let conv = started.conversation.id;
    sink.tool_calls_done(1).await;
    let u1 = f.run(&conv, "u1", None, None).await.unwrap();
    assert!(view_content(&u1).2.contains("## mcp__files__echo\n"));
    sink.tool_calls_done(2).await;
    let u2 = f.run(&conv, "u2", None, None).await.unwrap();
    assert_eq!(view_content(&u2), ("u2", AiToolStatus::Ok, "hi"));
    sink.ended().await;
    f.idle(&conv).await;
    f.exchange(Some(&conv), "again", f.context()).await;

    let requests = f.requests().await;
    assert_eq!(requests.len(), 4);
    let names: Vec<&str> = requests[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, deferred_tools());
    for request in &requests {
        assert_eq!(prefix(request), prefix(&requests[0]));
        assert!(request.get("cache_control").is_some());
    }
    let result = &requests[1]["messages"].as_array().unwrap().last().unwrap()["content"][0];
    assert_eq!(result["type"], "tool_result");
    assert!(
        serde_json::to_string(&result["content"])
            .unwrap()
            .contains("Input schema:")
    );
    f.mcp.shutdown().await;
}
