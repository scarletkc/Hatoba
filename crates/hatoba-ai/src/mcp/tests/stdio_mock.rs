//! The mock stdio MCP server: this test binary started again with [`MOCK_ENV`] set, running
//! only [`mock_stdio_server`]. It speaks newline-delimited JSON-RPC on stdin and stdout; the
//! harness's own lines before it ("running 1 test") are not JSON, which clients skip.

use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

/// Selects the mock's behaviour: `serve`, `crash` (stderr line, exit code 3) or `sleep`.
pub(super) const MOCK_ENV: &str = "HATOBA_MCP_MOCK";
/// The test that runs the mock, as the harness names it.
const ENTRY_TEST: &str = "mcp::tests::stdio_mock::mock_stdio_server";

/// Arguments that make this test binary run only the mock.
pub(super) fn mock_args() -> Vec<String> {
    [ENTRY_TEST, "--exact", "--nocapture", "--test-threads=1"]
        .map(str::to_owned)
        .to_vec()
}

/// The mock's entry point; does nothing in a normal test run.
#[test]
fn mock_stdio_server() {
    match std::env::var(MOCK_ENV).as_deref() {
        Ok("serve") => serve(),
        Ok("crash") => {
            let _ = writeln!(std::io::stderr(), "fatal: MOCK_TOKEN is not set");
            std::process::exit(3);
        }
        Ok("sleep") => {
            std::thread::sleep(Duration::from_secs(60));
            std::process::exit(0);
        }
        _ => {}
    }
}

type Out = Arc<Mutex<std::io::Stdout>>;

fn send(out: &Out, message: &Value) {
    let mut out = out.lock().unwrap();
    let _ = writeln!(out, "{message}");
    let _ = out.flush();
}

fn reply(out: &Out, id: &Value, result: Value) {
    send(out, &json!({"jsonrpc": "2.0", "id": id, "result": result}));
}

fn text(value: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": value.into()}]})
}

fn tool(name: &str, description: &str) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}
    })
}

fn serve() -> ! {
    let out: Out = Arc::new(Mutex::new(std::io::stdout()));
    // The harness has printed "test <name> ... " without a line break; end that line so the
    // first message starts a line of its own.
    {
        let mut out = out.lock().unwrap();
        let _ = writeln!(out);
        let _ = out.flush();
    }
    let _ = writeln!(std::io::stderr(), "mock server starting");
    let mut capabilities = Value::Null;
    let mut cancelled: Vec<Value> = Vec::new();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            continue;
        };
        let Some(id) = message.get("id").cloned() else {
            if method == "notifications/cancelled" {
                cancelled.push(message["params"]["requestId"].clone());
            }
            continue;
        };
        match method {
            "initialize" => {
                capabilities = message["params"]["capabilities"].clone();
                reply(
                    &out,
                    &id,
                    json!({
                        "protocolVersion": "2025-06-18",
                        "capabilities": {"tools": {"listChanged": true}},
                        "serverInfo": {"name": "mock", "version": "1.0.0"}
                    }),
                );
            }
            "ping" => reply(&out, &id, json!({})),
            "tools/list" => {
                let page = match message["params"]["cursor"].as_str() {
                    None => json!({
                        "tools": [
                            {
                                "name": "echo",
                                "title": "Echo",
                                "description": "Echoes text",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {"text": {"type": "string"}},
                                    "required": ["text"]
                                },
                                "annotations": {"readOnlyHint": true, "openWorldHint": false}
                            },
                            tool("rich", "Returns every content type"),
                            tool("fail", "Fails")
                        ],
                        "nextCursor": "page-2"
                    }),
                    Some("page-2") => json!({
                        "tools": [
                            tool("notify", "Sends tools/list_changed"),
                            tool("pid", "Returns the process id"),
                            tool("spawn_child", "Starts a child process"),
                            tool("slow", "Answers after 30 s"),
                            tool("stderr_burst", "Writes 60 lines to stderr"),
                            tool("capabilities", "Returns the client capabilities"),
                            tool("cancelled", "Returns the cancelled request ids")
                        ]
                    }),
                    Some(_) => {
                        send(
                            &out,
                            &json!({"jsonrpc": "2.0", "id": id,
                                    "error": {"code": -32602, "message": "bad cursor"}}),
                        );
                        continue;
                    }
                };
                reply(&out, &id, page);
            }
            "tools/call" => call(&out, &id, &message["params"], &capabilities, &cancelled),
            _ => send(
                &out,
                &json!({"jsonrpc": "2.0", "id": id,
                        "error": {"code": -32601, "message": "Method not found"}}),
            ),
        }
    }
    std::process::exit(0);
}

fn call(out: &Out, id: &Value, params: &Value, capabilities: &Value, cancelled: &[Value]) {
    let name = params["name"].as_str().unwrap_or_default();
    match name {
        "echo" => reply(
            out,
            id,
            text(params["arguments"]["text"].as_str().unwrap_or("?")),
        ),
        "rich" => reply(
            out,
            id,
            json!({"content": [
                {"type": "text", "text": "before"},
                {"type": "image", "data": "aGk=", "mimeType": "image/png"},
                {"type": "audio", "data": "aGk=", "mimeType": "audio/wav"},
                {"type": "resource_link", "uri": "file:///tmp/a.txt", "name": "a.txt"},
                {"type": "resource", "resource": {"uri": "file:///tmp/b.txt", "text": "embedded"}},
                {"type": "text", "text": "after"}
            ]}),
        ),
        "fail" => reply(
            out,
            id,
            json!({"content": [{"type": "text", "text": "boom"}], "isError": true}),
        ),
        "notify" => {
            send(
                out,
                &json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}),
            );
            reply(out, id, text("notified"));
        }
        "pid" => reply(out, id, text(std::process::id().to_string())),
        "argv" => {
            let argv: Vec<String> = std::env::args().skip(1).collect();
            reply(out, id, text(json!(argv).to_string()));
        }
        "exit" => {
            let _ = writeln!(std::io::stderr(), "exiting on request");
            std::process::exit(7);
        }
        "spawn_child" => {
            #[expect(
                clippy::zombie_processes,
                reason = "the child must outlive the call: the client's shutdown has to kill it"
            )]
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(mock_args())
                .env(MOCK_ENV, "sleep")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            reply(out, id, text(child.id().to_string()));
        }
        "slow" => {
            let (out, id) = (out.clone(), id.clone());
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(30));
                reply(&out, &id, text("late"));
            });
        }
        "stderr_burst" => {
            let mut err = std::io::stderr().lock();
            for i in 1..=60 {
                let _ = writeln!(err, "burst line {i}");
            }
            let _ = writeln!(err, "\x1b[31mred\x1b[0m end");
            let _ = err.flush();
            reply(out, id, text("ok"));
        }
        "capabilities" => reply(out, id, text(capabilities.to_string())),
        "cancelled" => reply(out, id, text(Value::Array(cancelled.to_vec()).to_string())),
        other => send(
            out,
            &json!({"jsonrpc": "2.0", "id": id,
                    "error": {"code": -32602, "message": format!("Unknown tool: {other}")}}),
        ),
    }
}
