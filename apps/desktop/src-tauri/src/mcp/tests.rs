//! MCP manager tests (spec §12, AI bullet) against a mock stdio server (this test binary started
//! again with [`MOCK_ENV`] set, running only [`mock_stdio_server`]) and a mock Streamable HTTP
//! server (wiremock). The turn tests in `crate::ai::tests` use the same mocks.

use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hatoba_core::model::{Item, McpServer, McpTransport};
use hatoba_core::sync::SharedVault;
use hatoba_core::{KdfParams, Vault};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, Request, ResponseTemplate};
use zeroize::Zeroizing;

use super::{DeviceState, McpEvents, McpManager, OfferedTool};
use crate::dto::{AiTurnContext, McpServerState, McpServerStatus};
use hatoba_ai::entry::ToolStatus;

/// Selects the mock's behaviour: `serve`.
const MOCK_ENV: &str = "HATOBA_DESKTOP_MCP_MOCK";
/// The test that runs the mock, as the harness names it.
const ENTRY_TEST: &str = "mcp::tests::mock_stdio_server";
/// An environment value that must reach the child and nothing else.
pub(crate) const ENV_SECRET: &str = "env-secret-NEVER-SHOWN";
/// A header value that must reach the HTTP server and nothing else.
pub(crate) const HEADER_SECRET: &str = "Bearer header-secret-NEVER-SHOWN";

// ───────────────────────── the stdio mock ─────────────────────────

/// The mock's entry point; does nothing in a normal test run.
#[test]
fn mock_stdio_server() {
    match std::env::var(MOCK_ENV).as_deref() {
        Ok("serve") => serve(),
        Ok("hang") => hang(),
        _ => {}
    }
}

/// A server that never answers `initialize`, like `npx` downloading a package: it says its
/// process id on stderr and waits.
fn hang() -> ! {
    let _ = writeln!(std::io::stderr(), "pid {}", std::process::id());
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

fn send(message: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{message}");
    let _ = out.flush();
}

fn reply(id: &Value, result: Value) {
    send(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
}

fn text(value: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": value.into()}]})
}

fn schema() -> Value {
    json!({"type": "object", "properties": {"text": {"type": "string"}}})
}

fn serve() -> ! {
    // The harness printed "test <name> ... " without a line break.
    send(&Value::Null);
    let _ = writeln!(std::io::stderr(), "mock server starting");
    let mut notified = false;
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let (Some(method), Some(id)) = (message["method"].as_str(), message.get("id").cloned())
        else {
            continue;
        };
        match method {
            "initialize" => reply(
                &id,
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"tools": {"listChanged": true}},
                    "serverInfo": {"name": "mock", "version": "1.0.0"}
                }),
            ),
            "ping" => reply(&id, json!({})),
            "tools/list" => {
                let mut tools = vec![
                    json!({"name": "echo", "description": "Echoes text", "inputSchema": schema(),
                           "annotations": {"readOnlyHint": true, "title": "Echo"}}),
                    json!({"name": "fail", "description": "Fails", "inputSchema": schema()}),
                    json!({"name": "notify", "description": "Changes the tool list", "inputSchema": schema()}),
                    json!({"name": "pid", "description": "Returns the process id", "inputSchema": schema()}),
                    json!({"name": "env", "description": "Says whether the secret arrived", "inputSchema": schema()}),
                ];
                if notified {
                    tools.push(
                        json!({"name": "added", "description": "Appears later", "inputSchema": schema()}),
                    );
                }
                reply(&id, json!({"tools": tools}));
            }
            "tools/call" => match message["params"]["name"].as_str().unwrap_or_default() {
                "echo" => reply(
                    &id,
                    text(
                        message["params"]["arguments"]["text"]
                            .as_str()
                            .unwrap_or("?"),
                    ),
                ),
                "fail" => reply(
                    &id,
                    json!({"content": [{"type": "text", "text": "boom"}], "isError": true}),
                ),
                "notify" => {
                    notified = true;
                    send(&json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}));
                    reply(&id, text("notified"));
                }
                "pid" => reply(&id, text(std::process::id().to_string())),
                "env" => {
                    let ok = std::env::var("MOCK_TOKEN").as_deref() == Ok(ENV_SECRET);
                    reply(&id, text(if ok { "secret arrived" } else { "no secret" }));
                }
                other => send(&json!({"jsonrpc": "2.0", "id": id,
                    "error": {"code": -32602, "message": format!("Unknown tool: {other}")}})),
            },
            _ => send(&json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": "Method not found"}})),
        }
    }
    std::process::exit(0);
}

/// A stdio server item that runs the mock.
pub(crate) fn stdio_server(name: &str) -> McpServer {
    mock_server(name, "serve")
}

/// A stdio server item whose start never ends ([`hang`]).
fn hanging_server(name: &str) -> McpServer {
    mock_server(name, "hang")
}

fn mock_server(name: &str, mode: &str) -> McpServer {
    McpServer {
        name: name.into(),
        transport: McpTransport::Stdio {
            command: std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            args: [ENTRY_TEST, "--exact", "--nocapture", "--test-threads=1"]
                .map(str::to_owned)
                .to_vec(),
            env: [
                (MOCK_ENV.to_owned(), Zeroizing::new(mode.to_owned())),
                (
                    "MOCK_TOKEN".to_owned(),
                    Zeroizing::new(ENV_SECRET.to_owned()),
                ),
            ]
            .into(),
        },
        always_ask: false,
        updated_at: 0,
    }
}

/// A stdio server whose command does not exist.
pub(crate) fn missing_server(name: &str) -> McpServer {
    McpServer {
        name: name.into(),
        transport: McpTransport::Stdio {
            command: "hatoba-no-such-mcp-command".into(),
            args: Vec::new(),
            env: [(
                "MOCK_TOKEN".to_owned(),
                Zeroizing::new(ENV_SECRET.to_owned()),
            )]
            .into(),
        },
        always_ask: false,
        updated_at: 0,
    }
}

// ───────────────────────── the HTTP mock ─────────────────────────

fn json_reply(id: &Value, result: Value) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_json(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn respond(request: &Request) -> ResponseTemplate {
    match request.method.as_str() {
        "GET" => return ResponseTemplate::new(405),
        "DELETE" => return ResponseTemplate::new(200),
        _ => {}
    }
    // The header value reaches the server.
    let authorized = request
        .headers
        .get("authorization")
        .is_some_and(|v| v.as_bytes() == HEADER_SECRET.as_bytes());
    if !authorized {
        return ResponseTemplate::new(401);
    }
    let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
    let Some(id) = body.get("id").cloned() else {
        return ResponseTemplate::new(202);
    };
    match body["method"].as_str().unwrap_or_default() {
        "initialize" => json_reply(
            &id,
            json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "http-mock", "version": "1.0.0"}
            }),
        )
        .insert_header("mcp-session-id", "session-1"),
        "tools/list" => json_reply(
            &id,
            json!({"tools": [{
                "name": "lookup",
                "description": "Looks things up",
                "inputSchema": {"type": "object", "properties": {"q": {"type": "string"}},
                                "$schema": "http://json-schema.org/draft-07/schema#"},
                "annotations": {"destructiveHint": false}
            }]}),
        ),
        "tools/call" => json_reply(
            &id,
            text(format!(
                "found {}",
                body["params"]["arguments"]["q"].as_str().unwrap_or("?")
            )),
        ),
        _ => ResponseTemplate::new(200)
            .insert_header("content-type", "application/json")
            .set_body_json(json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": "Method not found"}})),
    }
}

pub(crate) async fn http_mock() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/mcp"))
        .respond_with(respond)
        .mount(&server)
        .await;
    server
}

pub(crate) fn http_server(name: &str, mock: &MockServer) -> McpServer {
    McpServer {
        name: name.into(),
        transport: McpTransport::Http {
            url: format!("{}/mcp", mock.uri()),
            headers: [(
                "Authorization".to_owned(),
                Zeroizing::new(HEADER_SECRET.to_owned()),
            )]
            .into(),
        },
        always_ask: false,
        updated_at: 0,
    }
}

// ───────────────────────── fixtures ─────────────────────────

/// Collects status events.
#[derive(Default)]
pub(crate) struct Events(Mutex<Vec<McpServerStatus>>);

impl McpEvents for Events {
    fn status(&self, status: McpServerStatus) {
        self.0.lock().unwrap().push(status);
    }
}

impl Events {
    pub(crate) fn all(&self) -> Vec<McpServerStatus> {
        self.0.lock().unwrap().clone()
    }

    pub(crate) fn states(&self, id: &str) -> Vec<McpServerState> {
        self.all()
            .into_iter()
            .filter(|s| s.server_id == id)
            .map(|s| s.state)
            .collect()
    }
}

pub(crate) fn vault() -> SharedVault {
    let mut vault = Vault::open_in_memory().unwrap();
    vault
        .create_with_params("correct horse battery staple", KdfParams::for_tests())
        .unwrap();
    Arc::new(Mutex::new(vault))
}

/// Stores a server as if it arrived from another device (no device state).
pub(crate) fn put_server(vault: &SharedVault, server: McpServer) -> String {
    vault
        .lock()
        .unwrap()
        .put(None, Item::McpServer(server))
        .unwrap()
}

/// Enables or disables a server on this device.
pub(crate) fn set_enabled(vault: &SharedVault, id: &str, enabled: bool) {
    let mut v = vault.lock().unwrap();
    let mut device = DeviceState::load(&v);
    device.server_mut(id).enabled = Some(enabled);
    device.save(&mut v).unwrap();
}

pub(crate) fn manager() -> (McpManager, Arc<Events>) {
    let mcp = McpManager::default();
    let events = Arc::new(Events::default());
    mcp.set_events(events.clone());
    (mcp, events)
}

fn context(disabled: &[&str]) -> AiTurnContext {
    AiTurnContext {
        provider_id: "p".into(),
        model_id: "m".into(),
        effort: None,
        host_id: None,
        tab: true,
        session_id: None,
        disabled_mcp_servers: disabled.iter().map(|s| (*s).to_owned()).collect(),
    }
}

fn args(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn names(tools: &[OfferedTool]) -> Vec<&str> {
    tools.iter().map(|t| t.name.as_str()).collect()
}

/// Whether a process with this id is running.
fn alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        let filter = format!("PID eq {pid}");
        let output = std::process::Command::new("tasklist")
            .args(["/FI", &filter, "/NH", "/FO", "CSV"])
            .output()
            .expect("tasklist runs");
        String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\""))
    }
    #[cfg(unix)]
    {
        std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
            stat.rsplit_once(')')
                .is_some_and(|(_, rest)| !rest.trim_start().starts_with('Z'))
        }) || (!std::path::Path::new("/proc/self").exists()
            && std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .status()
                .is_ok_and(|s| s.success()))
    }
}

async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !check() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// ───────────────────────── tests ─────────────────────────

#[test]
fn device_defaults_enable_http_servers_only() {
    let mut device = DeviceState::default();
    let stdio = stdio_server("s").transport;
    let http = McpTransport::Http {
        url: "https://mcp.example.com".into(),
        headers: Default::default(),
    };
    // AI-29: a server from another device is enabled iff it uses http.
    assert!(!device.enabled("a", &stdio));
    assert!(device.enabled("a", &http));
    device.server_mut("a").enabled = Some(true);
    device.server_mut("b").enabled = Some(false);
    assert!(device.enabled("a", &stdio));
    assert!(!device.enabled("b", &http));

    // AI-31: per tool, or the whole server.
    assert!(!device.allows("a", "echo"));
    device
        .server_mut("a")
        .always_allow_tools
        .push("echo".into());
    assert!(device.allows("a", "echo") && !device.allows("a", "fail"));
    device.server_mut("b").always_allow = true;
    assert!(device.allows("b", "anything"));

    // Stored and read back; garbage reads as the defaults.
    let vault = vault();
    let mut v = vault.lock().unwrap();
    device.save(&mut v).unwrap();
    assert_eq!(DeviceState::load(&v), device);
    v.set_mcp_device_state("not json").unwrap();
    assert_eq!(DeviceState::load(&v), DeviceState::default());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn requests_start_servers_lazily_and_offer_their_tools() {
    let vault = vault();
    let mock = http_mock().await;
    let (mcp, events) = manager();
    let files = put_server(&vault, stdio_server("files"));
    let web = put_server(&vault, http_server("Alpha Web", &mock));
    let other = put_server(&vault, stdio_server("other device"));
    // `files` was turned on here; `other` arrived from another device and stays off (AI-29).
    set_enabled(&vault, &files, true);
    let cancel = CancellationToken::new();

    // With or without a terminal tab (AI-09).
    let offer = mcp
        .offer(
            &vault,
            &AiTurnContext {
                tab: false,
                ..context(&[])
            },
            &cancel,
        )
        .await;
    // Sorted by server name, then the server's own order; cleaned names (AI-30).
    assert_eq!(
        names(&offer.tools),
        [
            "mcp__Alpha_Web__lookup",
            "mcp__files__echo",
            "mcp__files__fail",
            "mcp__files__notify",
            "mcp__files__pid",
            "mcp__files__env"
        ]
    );
    assert_eq!(
        events.states(&files),
        [McpServerState::Starting, McpServerState::Running]
    );
    assert_eq!(
        events.states(&web),
        [McpServerState::Starting, McpServerState::Running]
    );
    assert!(events.states(&other).is_empty());

    // Schemas are cleaned for the protocol; descriptions are the server's.
    let defs = offer.tool_defs(hatoba_ai::provider::Protocol::Anthropic);
    assert_eq!(defs[0].description, "Looks things up");
    assert!(defs[0].input_schema.get("$schema").is_none());

    // The running status names the tools; the annotations are shown.
    let device = DeviceState::load(&vault.lock().unwrap());
    let status = mcp.status(&device, &files);
    assert_eq!(status.state, McpServerState::Running);
    assert_eq!(status.tools.len(), 5);
    assert_eq!(status.tools[0].name, "mcp__files__echo");
    assert_eq!(status.tools[0].annotations.read_only_hint, Some(true));
    assert_eq!(status.tools[0].annotations.title.as_deref(), Some("Echo"));
    assert!(
        status
            .stderr
            .iter()
            .any(|l| l.contains("mock server starting"))
    );

    // A server switched off for the conversation is left out (AI-30); started servers stay.
    let offer = mcp.offer(&vault, &context(&[&files]), &cancel).await;
    assert_eq!(names(&offer.tools), ["mcp__Alpha_Web__lookup"]);

    // Calls reach the servers with their secrets, which no status carries.
    let call = |name: &str| offer_tool(&mcp, name);
    let (status, content) = mcp
        .call(&vault, &call("mcp__files__env"), args(json!({})), &cancel)
        .await;
    assert_eq!(
        (status, content.as_str()),
        (ToolStatus::Ok, "secret arrived")
    );
    let (status, content) = mcp
        .call(
            &vault,
            &call("mcp__Alpha_Web__lookup"),
            args(json!({"q": "dns"})),
            &cancel,
        )
        .await;
    assert_eq!((status, content.as_str()), (ToolStatus::Ok, "found dns"));
    let (status, content) = mcp
        .call(&vault, &call("mcp__files__fail"), args(json!({})), &cancel)
        .await;
    assert_eq!((status, content.as_str()), (ToolStatus::Error, "boom"));
    let shown = format!("{:?}", events.all()) + &serde_json::to_string(&events.all()).unwrap();
    assert!(!shown.contains(ENV_SECRET) && !shown.contains("header-secret"));

    mcp.shutdown().await;
}

/// The tool named `name` among the running servers.
fn offer_tool(mcp: &McpManager, name: &str) -> OfferedTool {
    mcp.current_tools()
        .find(name)
        .cloned()
        .unwrap_or_else(|| panic!("no tool {name}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_that_cannot_start_offers_nothing_and_its_calls_fail() {
    let vault = vault();
    let (mcp, events) = manager();
    let broken = put_server(&vault, missing_server("broken"));
    set_enabled(&vault, &broken, true);
    let cancel = CancellationToken::new();

    let offer = mcp.offer(&vault, &context(&[]), &cancel).await;
    assert!(offer.tools.is_empty());
    assert_eq!(
        events.states(&broken),
        [McpServerState::Starting, McpServerState::Failed]
    );
    let failed = events.all().pop().unwrap();
    assert!(
        failed
            .error
            .as_deref()
            .is_some_and(|e| e.contains("hatoba-no-such-mcp-command")),
        "{:?}",
        failed.error
    );

    // A failed server is not retried by every request; its calls name it and its error.
    mcp.offer(&vault, &context(&[]), &cancel).await;
    assert_eq!(events.states(&broken).len(), 2);
    let tool = OfferedTool {
        name: "mcp__broken__x".into(),
        server_id: broken.clone(),
        server_name: "broken".into(),
        tool: hatoba_ai::mcp::McpTool {
            name: "x".into(),
            description: String::new(),
            input_schema: json!({"type": "object"}),
            annotations: Default::default(),
        },
    };
    let (status, content) = mcp.call(&vault, &tool, Map::new(), &cancel).await;
    assert_eq!(status, ToolStatus::Error);
    assert!(
        content.contains("\"broken\"") && content.contains("command not found"),
        "{content}"
    );

    // A stopped server's calls say so.
    mcp.stop(&vault, &broken).await;
    let (status, content) = mcp.call(&vault, &tool, Map::new(), &cancel).await;
    assert_eq!(status, ToolStatus::Error);
    assert!(content.contains("is not running"), "{content}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_changed_tool_list_is_listed_again_before_the_next_request() {
    let vault = vault();
    let (mcp, events) = manager();
    let files = put_server(&vault, stdio_server("files"));
    set_enabled(&vault, &files, true);
    let cancel = CancellationToken::new();

    let offer = mcp.offer(&vault, &context(&[]), &cancel).await;
    assert_eq!(offer.tools.len(), 5);
    let (status, _) = mcp
        .call(
            &vault,
            &offer_tool(&mcp, "mcp__files__notify"),
            Map::new(),
            &cancel,
        )
        .await;
    assert_eq!(status, ToolStatus::Ok);

    let offer = mcp.offer(&vault, &context(&[]), &cancel).await;
    assert_eq!(offer.tools.len(), 6);
    assert_eq!(offer.tools[5].name, "mcp__files__added");
    // The relisting was announced with the new tools.
    let last = events.all().pop().unwrap();
    assert_eq!(last.state, McpServerState::Running);
    assert_eq!(last.tools.len(), 6);
    mcp.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn locking_stops_every_server_and_kills_the_process() {
    let vault = vault();
    let mock = http_mock().await;
    let (mcp, events) = manager();
    let files = put_server(&vault, stdio_server("files"));
    let web = put_server(&vault, http_server("web", &mock));
    set_enabled(&vault, &files, true);
    let cancel = CancellationToken::new();
    mcp.offer(&vault, &context(&[]), &cancel).await;
    let (_, pid) = mcp
        .call(
            &vault,
            &offer_tool(&mcp, "mcp__files__pid"),
            Map::new(),
            &cancel,
        )
        .await;
    let pid: u32 = pid.parse().unwrap();
    assert!(alive(pid));
    mcp.record_offer("conv", mcp.current_tools());

    vault.lock().unwrap().lock();
    mcp.stop_all(&vault).await;
    assert_eq!(events.states(&files).last(), Some(&McpServerState::Stopped));
    assert_eq!(events.states(&web).last(), Some(&McpServerState::Stopped));
    eventually("the server process to exit", || !alive(pid)).await;
    // The HTTP session was deleted.
    let deleted = mock
        .received_requests()
        .await
        .unwrap()
        .iter()
        .any(|r| r.method.as_str() == "DELETE");
    assert!(deleted);
    // Offers and stderr are forgotten; a locked vault starts nothing.
    assert!(mcp.offered("conv", "mcp__files__pid").is_none());
    assert!(
        mcp.offer(&vault, &context(&[]), &cancel)
            .await
            .tools
            .is_empty()
    );
    let device = DeviceState::default();
    let status = mcp.status(&device, &files);
    assert_eq!(status.state, McpServerState::Stopped);
    assert!(status.stderr.is_empty() && status.tools.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_saved_configuration_restarts_a_running_server_and_explicit_starts_wait() {
    let vault = vault();
    let (mcp, events) = manager();
    let files = put_server(&vault, stdio_server("files"));
    set_enabled(&vault, &files, true);

    let status = mcp
        .start_now(
            &vault,
            &files,
            "files",
            super::transport_config(&stdio_server("files").transport),
        )
        .await;
    assert_eq!(status.state, McpServerState::Running);
    assert_eq!(status.tools.len(), 5);
    let cancel = CancellationToken::new();
    let (_, first_pid) = mcp
        .call(
            &vault,
            &offer_tool(&mcp, "mcp__files__pid"),
            Map::new(),
            &cancel,
        )
        .await;

    // A new configuration: the server starts again, the old process goes away.
    let mut renamed = stdio_server("files");
    renamed.name = "files2".into();
    mcp.restart_if_active(
        &vault,
        &files,
        &renamed.name,
        super::transport_config(&renamed.transport),
        true,
    )
    .await;
    eventually("the restart", || {
        events.states(&files).last() == Some(&McpServerState::Running)
            && events.states(&files).len() >= 4
    })
    .await;
    let (_, second_pid) = mcp
        .call(
            &vault,
            &offer_tool(&mcp, "mcp__files2__pid"),
            Map::new(),
            &cancel,
        )
        .await;
    assert_ne!(first_pid, second_pid);
    eventually("the old process to exit", || {
        !alive(first_pid.parse().unwrap())
    })
    .await;

    // Disabled on this device: a configuration change stops it instead.
    mcp.restart_if_active(
        &vault,
        &files,
        "files",
        super::transport_config(&stdio_server("files").transport),
        false,
    )
    .await;
    assert_eq!(events.states(&files).last(), Some(&McpServerState::Stopped));
    // A stopped server does not restart on a save.
    mcp.restart_if_active(
        &vault,
        &files,
        "files",
        super::transport_config(&stdio_server("files").transport),
        true,
    )
    .await;
    assert_eq!(events.states(&files).last(), Some(&McpServerState::Stopped));
    mcp.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_deleted_elsewhere_is_stopped_by_the_next_request() {
    let vault = vault();
    let (mcp, events) = manager();
    let files = put_server(&vault, stdio_server("files"));
    set_enabled(&vault, &files, true);
    let cancel = CancellationToken::new();
    assert_eq!(
        mcp.offer(&vault, &context(&[]), &cancel).await.tools.len(),
        5
    );
    vault.lock().unwrap().delete(&files).unwrap();
    assert!(
        mcp.offer(&vault, &context(&[]), &cancel)
            .await
            .tools
            .is_empty()
    );
    eventually("the stop", || {
        events.states(&files).last() == Some(&McpServerState::Stopped)
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_called_name_resolves_through_its_conversations_offer() {
    let vault = vault();
    let (mcp, _events) = manager();
    let spaced = put_server(&vault, stdio_server("my server"));
    let snake = put_server(&vault, stdio_server("my_server"));
    set_enabled(&vault, &spaced, true);
    set_enabled(&vault, &snake, true);
    let cancel = CancellationToken::new();
    // Both names clean to `my_server`, and conversation b switched `my server` off in its tools
    // menu, so the same name means another server there (AI-30).
    let all = mcp.offer(&vault, &context(&[]), &cancel).await;
    let without = mcp
        .offer(&vault, &context(&[spaced.as_str()]), &cancel)
        .await;
    mcp.record_offer("a", all);
    mcp.record_offer("b", without);
    let name = "mcp__my_server__echo";
    let server_of = |conversation: &str| {
        let v = vault.lock().unwrap();
        let info = crate::commands::mcp::tool_info(&v, &mcp, conversation, name)
            .expect("the approval card's tool");
        // The card names the server the call runs on.
        assert_eq!(
            mcp.offered(conversation, name).unwrap().server_id,
            info.server_id
        );
        info.server_id
    };
    assert_eq!(server_of("a"), spaced);
    assert_eq!(server_of("b"), snake);
    // A conversation without a request in this run of the app: the running servers' names.
    assert_eq!(server_of("c"), spaced);

    // Deleted on another device, before a request stopped it: no card info, and its calls do
    // not run although its connection is up.
    let tool = mcp.offered("a", name).unwrap();
    vault.lock().unwrap().delete(&spaced).unwrap();
    assert!(crate::commands::mcp::tool_info(&vault.lock().unwrap(), &mcp, "a", name).is_none());
    let (status, content) = mcp
        .call(&vault, &tool, args(json!({"text": "hi"})), &cancel)
        .await;
    assert_eq!(status, ToolStatus::Error);
    assert!(content.contains("was deleted"), "{content}");
    mcp.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn locking_aborts_a_start_in_flight_and_kills_its_process() {
    let vault = vault();
    let (mcp, events) = manager();
    let slow = put_server(&vault, hanging_server("slow"));
    set_enabled(&vault, &slow, true);
    // A request starts the server and stops waiting for it; the start goes on.
    let cancel = CancellationToken::new();
    let request = {
        let (mcp, vault, cancel) = (mcp.clone(), vault.clone(), cancel.clone());
        tokio::spawn(async move { mcp.offer(&vault, &context(&[]), &cancel).await })
    };
    let mut pid = None;
    eventually("the server process", || {
        pid = mcp
            .status(&DeviceState::default(), &slow)
            .stderr
            .iter()
            .find_map(|line| line.strip_prefix("pid ")?.trim().parse::<u32>().ok());
        pid.is_some()
    })
    .await;
    let pid = pid.unwrap();
    assert!(alive(pid));
    cancel.cancel();
    assert!(request.await.unwrap().tools.is_empty());
    assert_eq!(events.states(&slow).last(), Some(&McpServerState::Starting));

    vault.lock().unwrap().lock();
    let stopping = Instant::now();
    mcp.stop_all(&vault).await;
    assert!(stopping.elapsed() < Duration::from_secs(15));
    assert_eq!(events.states(&slow).last(), Some(&McpServerState::Stopped));
    eventually("the server process to exit", || !alive(pid)).await;
}
