//! Streamable HTTP sessions against a wiremock server that answers like an MCP server: JSON for
//! most requests, an event stream for `notify`, a session id, 405 for the GET stream.

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::*;

const SESSION: &str = "session-1";

fn json_reply(id: &Value, result: Value) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_json(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn text(value: &str) -> Value {
    json!({"content": [{"type": "text", "text": value}]})
}

/// The mock server's whole behaviour.
fn respond(request: &Request) -> ResponseTemplate {
    match request.method.as_str() {
        // No standalone event stream; session deletion succeeds.
        "GET" => return ResponseTemplate::new(405),
        "DELETE" => return ResponseTemplate::new(200),
        _ => {}
    }
    let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
    let Some(id) = body.get("id").cloned() else {
        // Notifications.
        return ResponseTemplate::new(202);
    };
    match body["method"].as_str().unwrap_or_default() {
        "initialize" => json_reply(
            &id,
            json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {"tools": {"listChanged": true}},
                "serverInfo": {"name": "http-mock", "version": "1.0.0"}
            }),
        )
        .insert_header("mcp-session-id", SESSION),
        "tools/list" => match body["params"]["cursor"].as_str() {
            None => json_reply(
                &id,
                json!({
                    "tools": [{"name": "echo", "inputSchema": {"type": "object"}}],
                    "nextCursor": "2"
                }),
            ),
            Some(_) => json_reply(
                &id,
                json!({"tools": [
                    {"name": "notify", "description": "d", "inputSchema": {"type": "object"}},
                    {"name": "fail", "inputSchema": {"type": "object"}},
                    // Not valid for the SDK: no inputSchema, a hint that is not a boolean.
                    {"name": "loose", "annotations": {"readOnlyHint": "yes", "title": "Loose"}},
                    {"description": "no name"}
                ]}),
            ),
        },
        "tools/call" => match body["params"]["name"].as_str().unwrap_or_default() {
            "echo" => json_reply(
                &id,
                text(body["params"]["arguments"]["text"].as_str().unwrap_or("?")),
            ),
            "notify" => {
                let notice =
                    json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"});
                let result = json!({"jsonrpc": "2.0", "id": id, "result": text("notified")});
                ResponseTemplate::new(200).set_body_raw(
                    format!("event: message\ndata: {notice}\n\nevent: message\ndata: {result}\n\n"),
                    "text/event-stream",
                )
            }
            "fail" => json_reply(
                &id,
                json!({"content": [{"type": "text", "text": "bad input"}], "isError": true}),
            ),
            _ => ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(json!({"jsonrpc": "2.0", "id": id,
                    "error": {"code": -32602, "message": "Unknown tool"}})),
        },
        _ => ResponseTemplate::new(200)
            .insert_header("content-type", "application/json")
            .set_body_json(json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": "Method not found"}})),
    }
}

async fn mock_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/mcp"))
        .respond_with(respond)
        .mount(&server)
        .await;
    server
}

fn config(server: &MockServer) -> McpTransportConfig {
    McpTransportConfig::Http {
        url: format!("{}/mcp", server.uri()),
        headers: vec![
            ("Authorization".into(), secret("Bearer http-secret")),
            ("X-Team".into(), secret("t1")),
        ],
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_session_lists_calls_and_closes() {
    let server = mock_server().await;
    let conn = McpConnection::connect(&config(&server), &http_client(), CONNECT)
        .await
        .unwrap();

    let tools = conn.list_tools().await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["echo", "notify", "fail", "loose"]);
    let loose = &tools[3];
    assert_eq!(loose.input_schema, json!({"type": "object"}));
    assert_eq!(loose.annotations.title.as_deref(), Some("Loose"));
    assert_eq!(loose.annotations.read_only_hint, None);
    assert_eq!(
        call(&conn, "echo", json!({"text": "hi"})).await.content,
        "hi"
    );

    // A notification in the response's event stream, before the result.
    assert_eq!(call(&conn, "notify", json!({})).await.content, "notified");
    assert!(eventually(Duration::from_secs(5), || conn.tools_changed()).await);

    assert_eq!(
        call(&conn, "fail", json!({})).await,
        McpToolResult {
            is_error: true,
            content: "bad input".into()
        }
    );
    assert_eq!(
        conn.call_tool("nope", Map::new(), &CancellationToken::new(), CALL)
            .await,
        Err(McpError::Server("Unknown tool (code -32602)".into()))
    );
    assert!(conn.stderr_tail().is_empty());

    conn.shutdown().await;
    assert!(conn.is_closed());
    assert_eq!(conn.list_tools().await, Err(McpError::Closed));

    let requests = server.received_requests().await.unwrap();
    let posts: Vec<&Request> = requests
        .iter()
        .filter(|r| r.method.as_str() == "POST")
        .collect();
    assert!(posts.len() >= 7, "{}", posts.len());
    for post in &posts {
        assert_eq!(post.headers["authorization"], "Bearer http-secret");
        assert_eq!(post.headers["x-team"], "t1");
    }
    // Every request after initialize carries the session.
    for post in &posts[1..] {
        assert_eq!(post.headers["mcp-session-id"], SESSION);
    }
    assert!(
        requests.iter().any(|r| r.method.as_str() == "DELETE"
            && r.headers
                .get("mcp-session-id")
                .is_some_and(|v| v == SESSION)),
        "shutdown deletes the session"
    );
}

#[tokio::test]
async fn http_errors_name_no_secret_or_url() {
    // 401 with a challenge: Hatoba has no OAuth sign-in.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(401).insert_header("www-authenticate", "Bearer realm=\"mcp\""),
        )
        .mount(&server)
        .await;
    let err = McpConnection::connect(&config(&server), &http_client(), CONNECT)
        .await
        .unwrap_err();
    let McpError::Connect(message) = &err else {
        panic!("{err:?}");
    };
    assert!(message.contains("authorization"), "{message}");

    // 401 without one, and with a body.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string("invalid token"))
        .mount(&server)
        .await;
    let err = McpConnection::connect(&config(&server), &http_client(), CONNECT)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, McpError::Connect(m) if m.contains("401") && !m.contains("http-secret")),
        "{err:?}"
    );

    // Nothing listening: the message has neither the URL nor its query.
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let cfg = McpTransportConfig::Http {
        url: format!("http://127.0.0.1:{port}/mcp?token=url-secret"),
        headers: Vec::new(),
    };
    let err = McpConnection::connect(&cfg, &http_client(), CONNECT)
        .await
        .unwrap_err();
    let message = err.to_string();
    assert!(matches!(err, McpError::Connect(_)), "{err:?}");
    assert!(
        !message.contains("url-secret") && !message.contains("127.0.0.1"),
        "{message}"
    );

    for (url, expected) in [
        ("http://8.8.8.8/mcp", "use https"),
        ("ftp://example.com/mcp", "server URL"),
        ("not a url", "server URL"),
    ] {
        let cfg = McpTransportConfig::Http {
            url: url.into(),
            headers: Vec::new(),
        };
        let err = McpConnection::connect(&cfg, &http_client(), CONNECT)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, McpError::InvalidUrl(m) if m.contains(expected)),
            "{url}: {err:?}"
        );
    }

    let cfg = McpTransportConfig::Http {
        url: "https://mcp.example.com/mcp".into(),
        headers: vec![("Mcp-Session-Id".into(), secret("x"))],
    };
    assert!(matches!(
        McpConnection::connect(&cfg, &http_client(), CONNECT).await,
        Err(McpError::InvalidConfig(_))
    ));
}
