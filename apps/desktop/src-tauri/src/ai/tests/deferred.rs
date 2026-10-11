//! Deferred MCP tools (AI-45) in turns against mock Chat Completions and Anthropic Messages
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

/// A Streamable HTTP MCP server named `name` that lists `tools` and answers a call of the tool
/// `t` with the text `"{reply} t"`.
async fn http_mcp(name: &'static str, tools: Vec<Value>, reply: &'static str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/mcp"))
        .respond_with(move |request: &Request| {
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
                        "serverInfo": {"name": name, "version": "1.0.0"}
                    }),
                )
                .insert_header("mcp-session-id", format!("{name}-1").as_str()),
                "tools/list" => mcp_reply(&id, json!({ "tools": tools.clone() })),
                "tools/call" => mcp_reply(
                    &id,
                    json!({"content": [{"type": "text", "text": format!(
                        "{reply} {}", body["params"]["name"].as_str().unwrap_or("?"))}]}),
                ),
                _ => mcp_reply(&id, json!({})),
            }
        })
        .mount(&server)
        .await;
    server
}

/// A Streamable HTTP MCP server with [`BIG_TOOLS`] tools, `tool_00` and on.
async fn big_mock() -> MockServer {
    let tools: Vec<Value> = (0..BIG_TOOLS)
        .map(|n| {
            json!({
                "name": format!("tool_{n:02}"),
                "description": format!(
                    "Padding tool {n:02}. {}",
                    "It stands for one of the many tools a large server lists. ".repeat(5)
                ),
                "inputSchema": {"type": "object",
                                "properties": {"value": {"type": "string"}},
                                "required": ["value"]}
            })
        })
        .collect();
    http_mcp("big", tools, "big").await
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

/// Below the threshold the MCP tools go to the provider directly, as before AI-45, and no message
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

/// AI-45 on Chat Completions: past a tenth of the window the first message lists the tools, the
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

/// A Streamable HTTP MCP server named `twin` with one tool, `act`, whose result is `"{label} act"`.
async fn twin(f: &Fixture, label: &'static str) -> (String, MockServer) {
    let tools =
        vec![json!({"name": "act", "description": "Acts.", "inputSchema": {"type": "object"}})];
    let server = http_mcp("twin", tools, label).await;
    let id = put_server(
        &f.vault,
        McpServer {
            name: "twin".into(),
            ..big_server(&server)
        },
    );
    (id, server)
}

/// AI-30 numbers colliding names in the order of the servers a request offers, so a server that
/// goes away mid-turn renames the other's tool. The model was told the names by the message's
/// list, so a call by such a name still means the tool the list named.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_name_keeps_its_tool_when_a_server_goes_mid_turn() {
    let f = Fixture::new(vec![
        calls_of(&[call_proxy("c1", "mcp__files__echo", json!({"text": "x"}))]),
        calls_of(&[call_proxy("c2", "mcp__twin__act", json!({}))]),
        answer("Done."),
    ])
    .await;
    f.set_context_window(WINDOW);
    let (_files, _big, _mock) = servers(&f).await;
    let (a, _a_mock) = twin(&f, "a").await;
    let (b, _b_mock) = twin(&f, "b").await;
    let (started, sink) = f.send(None, "twins").await;
    let conv = started.conversation.id;
    sink.tool_calls_done(1).await;
    let texts = user_texts(&f, &conv);
    assert!(
        texts[0].contains("- twin (1 tool): mcp__twin__act\n- twin (1 tool): mcp__twin__act_2"),
        "{}",
        texts[0]
    );
    // The server whose tool the list names `mcp__twin__act` goes away.
    let first = f.mcp.offered(&conv, "mcp__twin__act").unwrap().server_id;
    let (gone, stays) = if first == a { (&a, "b") } else { (&b, "a") };
    lock(&f.vault).delete(gone).unwrap();
    f.run(&conv, "c1", None, None).await.unwrap();
    sink.tool_calls_done(2).await;

    let info = proxy_info(
        &lock(&f.vault),
        &f.mcp,
        &conv,
        r#"{"tool": "mcp__twin__act", "arguments": {}}"#,
    );
    assert!(!info.runs);
    let c2 = f.run(&conv, "c2", None, None).await.unwrap();
    assert_eq!(
        view_content(&c2),
        (
            "c2",
            AiToolStatus::Error,
            "The MCP server \"twin\" was deleted, so the tool did not run."
        )
    );
    assert!(!view_content(&c2).2.contains(&format!("{stays} act")));
    sink.ended().await;
    f.idle(&conv).await;
    f.mcp.shutdown().await;
}

/// After a restart or the lock nothing is recorded for a deferred conversation, so a retry records
/// the offer of its request: a name means the conversation's tool, not one of every server running
/// on the device numbered together, whatever the conversation's tools menu says.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_retry_after_the_lock_resolves_names_through_the_conversation_tools() {
    let f = Fixture::new(vec![
        ResponseTemplate::new(500).set_body_json(json!({"error": {"message": "overloaded"}})),
        calls_of(&[call_proxy("c1", "mcp__twin__act", json!({}))]),
        answer("Done."),
    ])
    .await;
    f.set_context_window(WINDOW);
    let (_files, _big, _mock) = servers(&f).await;
    let (a, _a_mock) = twin(&f, "a").await;
    let (b, _b_mock) = twin(&f, "b").await;
    // Device-wide, `off` comes first (servers sort by name, then ID) and would own the name.
    let (off, on, label) = if a < b { (a, b, "b") } else { (b, a, "a") };
    let context = AiTurnContext {
        disabled_mcp_servers: vec![off.clone()],
        ..f.context()
    };
    let (conv, failed) = f.exchange(None, "hello", context.clone()).await;
    assert_eq!(failed, AiTurnEndReason::Error);
    assert!(user_texts(&f, &conv)[0].contains("- twin (1 tool): mcp__twin__act\n"));

    // The lock stops the servers and forgets what was recorded.
    {
        let mut v = lock(&f.vault);
        f.manager.stop_all(&mut v);
        v.lock();
    }
    f.mcp.stop_all(&f.vault).await;
    lock(&f.vault).unlock(PW).unwrap();
    // Another conversation's request starts every server, `off` too.
    f.mcp
        .offer(&f.vault, &f.context(), &CancellationToken::new())
        .await;

    let sink = f.retry(&conv, context).unwrap();
    sink.tool_calls_done(1).await;
    let info = proxy_info(
        &lock(&f.vault),
        &f.mcp,
        &conv,
        r#"{"tool": "mcp__twin__act", "arguments": {}}"#,
    );
    assert_eq!(info.tool.unwrap().server_id, on);
    let c1 = f.run(&conv, "c1", None, None).await.unwrap();
    assert_eq!(view_content(&c1).1, AiToolStatus::Ok);
    assert!(view_content(&c1).2.contains(&format!("{label} act")));
    sink.ended().await;
    f.idle(&conv).await;
    f.mcp.shutdown().await;
}

/// More than 128 tools in a request, built-in ones included, defer the MCP tools however little of
/// a large window their definitions take, since OpenAI's Chat Completions API refuses more; 128
/// do not. Once deferred, they stay deferred with 128 again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn more_than_128_tools_defer_them_in_a_large_window() {
    let f = Fixture::new(vec![answer("One."), answer("Two."), answer("Three.")]).await;
    f.set_context_window(1_000_000);
    let mcp_tools = 128 - BUILTIN.len();
    let tiny: Vec<Value> = (0..mcp_tools)
        .map(|n| json!({"name": format!("t{n:03}"), "description": "", "inputSchema": {"type": "object"}}))
        .collect();
    let many = http_mcp("many", tiny, "many").await;
    put_server(
        &f.vault,
        McpServer {
            name: "many".into(),
            ..big_server(&many)
        },
    );
    let (conv, _) = f.exchange(None, "hello", f.context()).await;
    let requests = f.requests().await;
    assert_eq!(tool_names(&requests[0]).len(), 128);
    assert_eq!(user_texts(&f, &conv), ["hello"]);

    let (one_more, _twin) = twin(&f, "a").await;
    f.exchange(Some(&conv), "one more", f.context()).await;
    lock(&f.vault).delete(&one_more).unwrap();
    f.exchange(Some(&conv), "one fewer", f.context()).await;

    let texts = user_texts(&f, &conv);
    let listed = |servers: usize, tools: usize| {
        format!("<mcp_tools servers=\"{servers}\" tools=\"{tools}\">")
    };
    assert!(
        texts[1].starts_with(&listed(2, mcp_tools + 1)),
        "{}",
        texts[1]
    );
    assert!(texts[2].starts_with(&listed(1, mcp_tools)), "{}", texts[2]);
    let requests = f.requests().await;
    assert_eq!(tool_names(&requests[1]), deferred_tools());
    assert_eq!(prefix(&requests[2]), prefix(&requests[1]));
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

/// An answer whose request the provider counted as `prompt_tokens` tokens.
fn answer_using(text: &str, prompt_tokens: u64) -> ResponseTemplate {
    sse(&[
        json!({"choices": [{"index": 0, "delta": {"role": "assistant", "content": text}}]}),
        json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
        json!({"choices": [], "usage": {"prompt_tokens": prompt_tokens, "completion_tokens": 5}}),
    ])
}

/// The message that defers the MCP tools takes their definitions out of its requests, so the
/// check for a compaction (AI-22) does not count them, though the last response's usage does.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_message_that_defers_does_not_compact_for_the_tools_it_drops() {
    let f = Fixture::new(vec![answer_using("One.", 26_900), answer("Two.")]).await;
    // A window large enough for the tools, then a model with a smaller one.
    f.set_context_window(100_000);
    let (_files, _big, _mock) = servers(&f).await;
    let (conv, _) = f.exchange(None, "hello", f.context()).await;
    assert_eq!(user_texts(&f, &conv), ["hello"]);
    f.set_context_window(WINDOW);
    f.exchange(Some(&conv), "again", f.context()).await;

    // 26,905 tokens and the message pass 90% of 30,000, but not without the tools' definitions.
    assert!(
        !f.entries(&conv)
            .iter()
            .any(|e| matches!(e.body, EntryBody::Summary { .. }))
    );
    let texts = user_texts(&f, &conv);
    assert!(texts[1].starts_with("<mcp_tools servers=\"2\" tools=\"45\">"));
    assert_eq!(f.requests().await.len(), 2);
    f.mcp.shutdown().await;
}

/// When the message that defers the MCP tools needs a compaction all the same, its Compact
/// request is the next request (AI-21): it offers `search_tools` and `call_mcp_tool`, as the
/// message's requests do, and not the definitions it leaves out.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_compaction_before_the_message_that_defers_offers_the_deferred_tools() {
    let f = Fixture::new(vec![
        answer_using("One.", 33_000),
        answer("SUMMARY: the user said hello."),
        answer("Two."),
    ])
    .await;
    f.set_context_window(100_000);
    let (_files, _big, _mock) = servers(&f).await;
    let (conv, _) = f.exchange(None, "hello", f.context()).await;
    f.set_context_window(WINDOW);
    f.exchange(Some(&conv), "again", f.context()).await;

    assert!(
        f.entries(&conv)
            .iter()
            .any(|e| matches!(e.body, EntryBody::Summary { .. }))
    );
    let requests = f.requests().await;
    assert_eq!(requests.len(), 3);
    assert_eq!(tool_names(&requests[1]), deferred_tools());
    assert_eq!(tool_names(&requests[2]), deferred_tools());
    assert_eq!(prefix(&requests[1]), prefix(&requests[2]));
    f.mcp.shutdown().await;
}

/// Waits until a turn is registered, which then waits on the MCP servers its message starts.
async fn turn_registered(f: &Fixture) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while crate::ai::guard(&f.manager.0.turns).is_empty() {
        assert!(Instant::now() < deadline, "no turn was registered");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// A message waits for the MCP servers before it is stored; a stop meanwhile stores nothing (an
/// edit's message and the entries after it stay, and a new conversation, whose id the panel gets
/// before the wait, is not created), and the lock meanwhile creates no conversation.
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

    // AI-26: an edit stopped meanwhile leaves the message it edits and the reply after it.
    let entries = f.entries(&conv);
    let hello = f.user_ids(&conv)[0].clone();
    let edit = {
        let (manager, vault, env, context) =
            (f.manager.clone(), f.vault.clone(), f.env(), f.context());
        let conv = conv.clone();
        tokio::spawn(async move {
            manager
                .edit_resend(
                    &vault,
                    env,
                    conv,
                    &hello,
                    "hello again".into(),
                    context,
                    Arc::new(Sink::default()),
                )
                .await
                .map(|(started, _)| started)
        })
    };
    turn_registered(&f).await;
    f.manager.stop(&f.vault, f.env.as_ref(), &conv);
    let stopped = edit.await.unwrap().unwrap_err();
    assert_eq!(stopped.code, ErrorCode::Cancelled);
    assert_eq!(f.entries(&conv), entries);

    // A new conversation: its id reaches the panel before the wait, and a stop by that id stores
    // nothing and creates no conversation.
    let spawn_new = |f: &Fixture| {
        let sink = Arc::new(Sink::default());
        let (manager, vault, env, context) =
            (f.manager.clone(), f.vault.clone(), f.env(), f.context());
        let events = sink.clone();
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
                    events,
                )
                .await
                .map(|(started, _)| started)
        });
        (send, sink)
    };
    let (send, sink) = spawn_new(&f);
    sink.until("opened", |events| {
        events
            .iter()
            .any(|e| matches!(e, AiTurnEvent::Opened { .. }))
    })
    .await;
    let opened = sink
        .events()
        .iter()
        .find_map(|e| match e {
            AiTurnEvent::Opened { conversation_id } => Some(conversation_id.clone()),
            _ => None,
        })
        .unwrap();
    f.manager.stop(&f.vault, f.env.as_ref(), &opened);
    let stopped = send.await.unwrap().unwrap_err();
    assert_eq!(stopped.code, ErrorCode::Cancelled);
    assert!(lock(&f.vault).get(&opened).is_none());

    // A new conversation, then the lock, while the server is still starting.
    let (send, _) = spawn_new(&f);
    turn_registered(&f).await;
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

/// AI-45 on Anthropic Messages: the same tools and system prompt (and the top-level
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
