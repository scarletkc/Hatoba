//! Session tests against a mock stdio server (this test binary, started again) and a mock
//! Streamable HTTP server (wiremock).

mod http;
mod stdio_mock;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use super::*;
use stdio_mock::{MOCK_ENV, mock_args};

const CONNECT: Duration = Duration::from_secs(60);
const CALL: Duration = Duration::from_secs(20);

fn secret(value: &str) -> Zeroizing<String> {
    Zeroizing::new(value.to_owned())
}

fn mock_command(mode: &str) -> McpTransportConfig {
    McpTransportConfig::Stdio {
        command: std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        args: mock_args(),
        env: vec![
            (MOCK_ENV.to_owned(), secret(mode)),
            ("MOCK_TOKEN".to_owned(), secret("env-secret-value")),
        ],
    }
}

fn args(pairs: Value) -> Map<String, Value> {
    pairs.as_object().cloned().unwrap_or_default()
}

async fn call(conn: &McpConnection, tool: &str, arguments: Value) -> McpToolResult {
    conn.call_tool(tool, args(arguments), &CancellationToken::new(), CALL)
        .await
        .unwrap_or_else(|e| panic!("{tool}: {e}"))
}

/// Polls `check` every 50 ms until it holds or `within` passes.
async fn eventually(within: Duration, mut check: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if check() {
            return true;
        }
        if start.elapsed() > within {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Whether a process with this id is running (a zombie counts as gone).
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
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat
                .rsplit_once(')')
                .is_some_and(|(_, rest)| !rest.trim_start().starts_with('Z')),
            Err(_) if std::path::Path::new("/proc/self").exists() => false,
            Err(_) => std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .status()
                .is_ok_and(|s| s.success()),
        }
    }
}

/// A directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir =
            std::env::temp_dir().join(format!("hatoba-mcp-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn http_client() -> reqwest::Client {
    crate::provider::http_client()
}

#[test]
fn debug_never_prints_env_or_header_values() {
    let stdio = mock_command("serve");
    let debug = format!("{stdio:?}");
    assert!(
        debug.contains("MOCK_TOKEN") && !debug.contains("env-secret-value"),
        "{debug}"
    );
    let http = McpTransportConfig::Http {
        url: "https://mcp.example.com/mcp".into(),
        headers: vec![("Authorization".into(), secret("Bearer header-secret"))],
    };
    let debug = format!("{http:?}");
    assert!(
        debug.contains("Authorization") && !debug.contains("header-secret"),
        "{debug}"
    );
    assert_eq!(stdio.kind_str(), "stdio");
    assert_eq!(http.kind_str(), "http");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stdio_session_lists_calls_and_shuts_down() {
    let conn = McpConnection::connect(&mock_command("serve"), &http_client(), CONNECT)
        .await
        .unwrap();
    assert!(!conn.is_closed());
    assert!(!format!("{conn:?}").contains("secret"));

    // Two pages.
    let tools = conn.list_tools().await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "echo",
            "rich",
            "fail",
            "notify",
            "pid",
            "spawn_child",
            "slow",
            "stderr_burst",
            "capabilities",
            "cancelled"
        ]
    );
    assert_eq!(tools[0].description, "Echoes text");
    assert_eq!(tools[0].input_schema["required"], json!(["text"]));
    assert_eq!(tools[0].annotations.title.as_deref(), Some("Echo"));
    assert_eq!(tools[0].annotations.read_only_hint, Some(true));
    assert_eq!(tools[0].annotations.open_world_hint, Some(false));
    assert!(!conn.tools_changed());

    // No client capability is declared (§13.9).
    assert_eq!(call(&conn, "capabilities", json!({})).await.content, "{}");

    assert_eq!(
        call(&conn, "echo", json!({"text": "hello"})).await,
        McpToolResult {
            is_error: false,
            content: "hello".into()
        }
    );
    assert_eq!(
        call(&conn, "rich", json!({})).await.content,
        "before\n[image content omitted]\n[audio content omitted]\n\
         [resource_link content omitted]\nembedded\nafter"
    );
    assert_eq!(
        call(&conn, "fail", json!({})).await,
        McpToolResult {
            is_error: true,
            content: "boom".into()
        }
    );
    let err = conn
        .call_tool("nope", Map::new(), &CancellationToken::new(), CALL)
        .await
        .unwrap_err();
    assert_eq!(
        err,
        McpError::Server("Unknown tool: nope (code -32602)".into())
    );

    // tools/list_changed flips the flag until the next listing.
    assert_eq!(call(&conn, "notify", json!({})).await.content, "notified");
    assert!(eventually(Duration::from_secs(5), || conn.tools_changed()).await);
    assert_eq!(conn.list_tools().await.unwrap().len(), 10);
    assert!(!conn.tools_changed());

    // stderr goes to the ring, cleaned, and only the last 50 lines stay.
    assert_eq!(call(&conn, "stderr_burst", json!({})).await.content, "ok");
    assert!(
        eventually(Duration::from_secs(5), || conn
            .stderr_tail()
            .last()
            .is_some_and(|l| l == "red end"))
        .await,
        "{:?}",
        conn.stderr_tail()
    );
    let tail = conn.stderr_tail();
    assert_eq!(tail.len(), STDERR_LINES);
    assert_eq!(tail[0], "burst line 12");
    assert!(!tail.iter().any(|l| l.contains("mock server starting")));

    // A timeout and a cancellation each send notifications/cancelled and leave the session usable.
    let started = Instant::now();
    let err = conn
        .call_tool(
            "slow",
            Map::new(),
            &CancellationToken::new(),
            Duration::from_millis(300),
        )
        .await
        .unwrap_err();
    assert_eq!(err, McpError::Timeout);
    assert!(started.elapsed() < Duration::from_secs(10));
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        trigger.cancel();
    });
    let err = conn
        .call_tool("slow", Map::new(), &cancel, CALL)
        .await
        .unwrap_err();
    assert_eq!(err, McpError::Cancelled);
    let cancelled: Value =
        serde_json::from_str(&call(&conn, "cancelled", json!({})).await.content).unwrap();
    assert_eq!(cancelled.as_array().map(Vec::len), Some(2), "{cancelled}");
    let already = CancellationToken::new();
    already.cancel();
    assert_eq!(
        conn.call_tool("echo", Map::new(), &already, CALL).await,
        Err(McpError::Cancelled)
    );

    // Shutdown kills the server and the process it started.
    let pid: u32 = call(&conn, "pid", json!({})).await.content.parse().unwrap();
    let grandchild: u32 = call(&conn, "spawn_child", json!({}))
        .await
        .content
        .parse()
        .unwrap();
    assert!(alive(pid) && alive(grandchild));
    let clone = conn.clone();
    conn.shutdown().await;
    assert!(clone.is_closed());
    assert!(
        eventually(Duration::from_secs(10), || !alive(pid)
            && !alive(grandchild))
        .await,
        "the server or its child survived shutdown"
    );
    assert_eq!(
        clone
            .call_tool("echo", Map::new(), &CancellationToken::new(), CALL)
            .await,
        Err(McpError::Closed)
    );
    assert_eq!(clone.list_tools().await, Err(McpError::Closed));
    clone.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_the_last_handle_kills_the_server() {
    let conn = McpConnection::connect(&mock_command("serve"), &http_client(), CONNECT)
        .await
        .unwrap();
    let pid: u32 = call(&conn, "pid", json!({})).await.content.parse().unwrap();
    let grandchild: u32 = call(&conn, "spawn_child", json!({}))
        .await
        .content
        .parse()
        .unwrap();
    drop(conn);
    assert!(
        eventually(Duration::from_secs(10), || !alive(pid)
            && !alive(grandchild))
        .await,
        "the server or its child survived the drop"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_that_dies_closes_the_session() {
    let conn = McpConnection::connect(&mock_command("serve"), &http_client(), CONNECT)
        .await
        .unwrap();
    assert_eq!(
        conn.call_tool("exit", Map::new(), &CancellationToken::new(), CALL)
            .await,
        Err(McpError::Closed)
    );
    assert!(eventually(Duration::from_secs(5), || conn.is_closed()).await);
    assert_eq!(conn.list_tools().await, Err(McpError::Closed));
    assert!(
        eventually(Duration::from_secs(5), || conn
            .stderr_tail()
            .contains(&"exiting on request".to_owned()))
        .await
    );
    conn.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_that_exits_reports_its_code_and_keeps_stderr() {
    let stderr = StderrTail::new();
    let err = McpConnection::connect_with_stderr(
        &mock_command("crash"),
        &http_client(),
        CONNECT,
        stderr.clone(),
    )
    .await
    .unwrap_err();
    let McpError::Spawn(message) = &err else {
        panic!("{err:?}");
    };
    assert!(message.contains("exited with code 3"), "{message}");
    assert!(
        !message.contains("MOCK_TOKEN"),
        "stderr stays out of messages"
    );
    assert!(
        eventually(Duration::from_secs(5), || stderr
            .lines()
            .contains(&"fatal: MOCK_TOKEN is not set".to_owned()))
        .await,
        "{:?}",
        stderr.lines()
    );
}

#[tokio::test]
async fn a_missing_command_is_reported() {
    let cfg = McpTransportConfig::Stdio {
        command: "hatoba-mcp-no-such-command".into(),
        args: Vec::new(),
        env: Vec::new(),
    };
    assert_eq!(
        McpConnection::connect(&cfg, &http_client(), CONNECT)
            .await
            .unwrap_err(),
        McpError::CommandNotFound("hatoba-mcp-no-such-command".into())
    );
    let empty = McpTransportConfig::Stdio {
        command: " ".into(),
        args: Vec::new(),
        env: Vec::new(),
    };
    assert!(matches!(
        McpConnection::connect(&empty, &http_client(), CONNECT).await,
        Err(McpError::InvalidConfig(_))
    ));
}

/// A shim on the `PATH` given in `env`, found without its extension like `npx` finds
/// `npx.cmd`: a `.cmd` file on Windows (run through `cmd.exe`), a shell script elsewhere.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shims_on_the_env_path_are_found_and_started() {
    let dir = TempDir::new("shim");
    let exe = std::env::current_exe().unwrap();
    #[cfg(windows)]
    std::fs::write(
        dir.0.join("hatoba-mock-shim.cmd"),
        format!("@echo off\r\n\"{}\" %*\r\n", exe.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.0.join("hatoba-mock-shim");
        std::fs::write(
            &script,
            format!("#!/bin/sh\nexec '{}' \"$@\"\n", exe.display()),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut path = std::ffi::OsString::from(&dir.0);
    path.push(if cfg!(windows) { ";" } else { ":" });
    path.push(std::env::var_os("PATH").unwrap_or_default());
    // Arguments a package runner gets, and characters cmd.exe treats specially; the test
    // harness takes them as extra name filters, which match no other test.
    let extra = [
        "@modelcontextprotocol/server-filesystem@1.2.3",
        r"C:\Users\Some One\My Files",
        r"C:\trailing\",
        "a&b|c<d>e",
        "100% of %PATH%",
        "^caret (paren) !bang",
        "say \"hi\" twice",
        "",
    ];
    let mut shim_args = mock_args();
    shim_args.extend(extra.iter().map(|a| (*a).to_owned()));
    let cfg = McpTransportConfig::Stdio {
        command: "hatoba-mock-shim".into(),
        args: shim_args.clone(),
        env: vec![
            (MOCK_ENV.to_owned(), secret("serve")),
            ("PATH".to_owned(), secret(&path.to_string_lossy())),
        ],
    };
    let conn = McpConnection::connect(&cfg, &http_client(), CONNECT)
        .await
        .unwrap();
    assert_eq!(
        call(&conn, "echo", json!({"text": "through the shim"}))
            .await
            .content,
        "through the shim"
    );
    let argv: Vec<String> =
        serde_json::from_str(&call(&conn, "argv", json!({})).await.content).unwrap();
    assert_eq!(
        argv, shim_args,
        "arguments arrive unchanged through the shim"
    );
    // On Windows the server is cmd.exe's child, and its own child (which ignores stdin) is
    // three levels down: dropping the connection kills the whole tree at once.
    let pid: u32 = call(&conn, "pid", json!({})).await.content.parse().unwrap();
    let grandchild: u32 = call(&conn, "spawn_child", json!({}))
        .await
        .content
        .parse()
        .unwrap();
    drop(conn);
    assert!(
        eventually(Duration::from_secs(10), || !alive(pid)
            && !alive(grandchild))
        .await,
        "a process behind the shim survived"
    );

    // An argument cmd.exe cannot receive intact is refused before anything runs.
    #[cfg(windows)]
    {
        let McpTransportConfig::Stdio { command, env, .. } = &cfg else {
            unreachable!()
        };
        let mut args = mock_args();
        args.push("line\nbreak".into());
        let refused = McpTransportConfig::Stdio {
            command: command.clone(),
            args,
            env: env.clone(),
        };
        let err = McpConnection::connect(&refused, &http_client(), CONNECT)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, McpError::Spawn(m) if m.contains("arguments")),
            "{err:?}"
        );
    }

    // Without the env PATH, the shim is not found.
    let McpTransportConfig::Stdio { command, args, .. } = cfg else {
        unreachable!()
    };
    let without_path = McpTransportConfig::Stdio {
        command,
        args,
        env: Vec::new(),
    };
    assert!(matches!(
        McpConnection::connect(&without_path, &http_client(), CONNECT).await,
        Err(McpError::CommandNotFound(_))
    ));
}
