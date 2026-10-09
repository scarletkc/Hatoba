//! MCP server command tests (AI-29, AI-31, AI-33): saving with values that never come back,
//! device-local enablement and Always allow, import, export and their errors.

use hatoba_core::KdfParams;

use super::*;
use crate::error::ErrorCode;

const ENV_VALUE: &str = "env-value-NEVER-SHOWN";
const HEADER_VALUE: &str = "Bearer header-value-NEVER-SHOWN";

fn vault() -> Vault {
    let mut vault = Vault::open_in_memory().unwrap();
    vault
        .create_with_params("correct horse battery staple", KdfParams::for_tests())
        .unwrap();
    vault
}

fn secret(key: &str, value: Option<&str>) -> McpSecretInput {
    McpSecretInput {
        key: key.into(),
        value: value.map(str::to_owned),
    }
}

fn stdio(name: &str, env: Vec<McpSecretInput>) -> McpServerInput {
    McpServerInput {
        id: None,
        name: name.into(),
        transport: McpTransportInput::Stdio {
            command: " npx ".into(),
            args: vec![
                "-y".into(),
                "@modelcontextprotocol/server-filesystem".into(),
            ],
            env,
        },
        always_ask: false,
    }
}

fn http(name: &str, headers: Vec<McpSecretInput>) -> McpServerInput {
    McpServerInput {
        id: None,
        name: name.into(),
        transport: McpTransportInput::Http {
            url: "https://mcp.example.com/mcp".into(),
            headers,
        },
        always_ask: true,
    }
}

fn url() -> Option<String> {
    Some("https://mcp.example.com/mcp".into())
}

fn field(result: AppResult<Saved>) -> String {
    result.err().unwrap().field.unwrap_or_default()
}

fn stored(v: &Vault, id: &str) -> McpTransport {
    find_server(v, id).unwrap().transport
}

#[test]
fn saved_values_stay_in_the_vault_and_out_of_views() {
    let mut v = vault();
    let input = stdio("files", vec![secret("API_KEY", Some(ENV_VALUE))]);
    assert!(!format!("{input:?}").contains(ENV_VALUE));
    let saved = save_server(&mut v, &input, None).unwrap();
    assert!(saved.changed);
    let id = saved.view.id.clone();
    assert_eq!(
        saved.view.transport,
        crate::dto::McpTransportView::Stdio {
            command: "npx".into(),
            args: vec![
                "-y".into(),
                "@modelcontextprotocol/server-filesystem".into()
            ],
            env_keys: vec!["API_KEY".into()],
        }
    );
    // Created on this device: enabled here (AI-29).
    assert!(saved.view.enabled);
    let shown = serde_json::to_string(&servers_list(&v)).unwrap();
    assert!(!shown.contains(ENV_VALUE), "{shown}");
    assert!(!format!("{:?}", stored(&v, &id)).contains(ENV_VALUE));
    let McpTransport::Stdio { env, .. } = stored(&v, &id) else {
        panic!("stdio");
    };
    assert_eq!(env["API_KEY"].as_str(), ENV_VALUE);

    // `null` keeps the saved value; an unchanged configuration does not restart anything.
    let mut keep = stdio("files", vec![secret("API_KEY", None)]);
    keep.id = Some(id.clone());
    let again = save_server(&mut v, &keep, None).unwrap();
    assert!(!again.changed);
    let McpTransport::Stdio { env, .. } = stored(&v, &id) else {
        panic!("stdio");
    };
    assert_eq!(env["API_KEY"].as_str(), ENV_VALUE);
    // A new key needs a value; a removed key is gone.
    keep.transport = McpTransportInput::Stdio {
        command: "npx".into(),
        args: Vec::new(),
        env: vec![secret("OTHER", None)],
    };
    assert_eq!(field(save_server(&mut v, &keep, None)), "env");
    keep.transport = McpTransportInput::Stdio {
        command: "npx".into(),
        args: Vec::new(),
        env: Vec::new(),
    };
    assert!(save_server(&mut v, &keep, None).unwrap().changed);
    let McpTransport::Stdio { env, .. } = stored(&v, &id) else {
        panic!("stdio");
    };
    assert!(env.is_empty());

    // Switching to http does not reuse values of the other kind.
    let mut switched = http("files", vec![secret("API_KEY", None)]);
    switched.id = Some(id.clone());
    assert_eq!(field(save_server(&mut v, &switched, url())), "headers");
    let mut switched = http("files", vec![secret("Authorization", Some(HEADER_VALUE))]);
    switched.id = Some(id.clone());
    let view = save_server(&mut v, &switched, url()).unwrap().view;
    assert!(view.always_ask);
    assert_eq!(
        view.transport,
        crate::dto::McpTransportView::Http {
            url: "https://mcp.example.com/mcp".into(),
            header_keys: vec!["Authorization".into()],
        }
    );
    assert!(
        !serde_json::to_string(&view)
            .unwrap()
            .contains("NEVER-SHOWN")
    );
}

#[test]
fn invalid_servers_are_refused_with_short_messages() {
    let mut v = vault();
    save_server(&mut v, &stdio("Files", Vec::new()), None).unwrap();
    let cases: Vec<(McpServerInput, Option<String>, &str)> = vec![
        (stdio("  ", Vec::new()), None, "name"),
        (stdio(&"n".repeat(65), Vec::new()), None, "name"),
        // Names are unique, ignoring case.
        (stdio(" files ", Vec::new()), None, "name"),
        (
            McpServerInput {
                transport: McpTransportInput::Stdio {
                    command: " ".into(),
                    args: Vec::new(),
                    env: Vec::new(),
                },
                ..stdio("a", Vec::new())
            },
            None,
            "command",
        ),
        (stdio("b", vec![secret("A=B", Some("x"))]), None, "env"),
        (stdio("c", vec![secret(" ", Some("x"))]), None, "env"),
        (
            stdio("d", vec![secret("K", Some("1")), secret("K", Some("2"))]),
            None,
            "env",
        ),
        (http("e", Vec::new()), None, "url"),
        (
            http("f", vec![secret("Content-Type", Some("x"))]),
            url(),
            "headers",
        ),
        (
            http("g", vec![secret("Bad Header", Some("x"))]),
            url(),
            "headers",
        ),
        (
            http("h", vec![secret("X-Key", Some("a\nb"))]),
            url(),
            "headers",
        ),
    ];
    for (input, url, expected) in cases {
        let err = save_server(&mut v, &input, url).err().unwrap();
        assert_eq!(err.field.as_deref(), Some(expected), "{input:?}");
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert!(
            err.detail.ends_with('.') && err.detail.len() < 120,
            "{}",
            err.detail
        );
    }
    assert_eq!(v.mcp_servers().len(), 1);
}

#[tokio::test]
async fn http_urls_follow_the_provider_rules() {
    let field = |e: AppError| e.field.unwrap_or_default();
    let with_url = |url: &str| McpTransportInput::Http {
        url: url.into(),
        headers: Vec::new(),
    };
    assert_eq!(field(checked_url(&with_url(" ")).await.unwrap_err()), "url");
    let err = checked_url(&with_url("ftp://mcp.example.com"))
        .await
        .unwrap_err();
    assert_eq!(field(err.clone()), "url");
    assert!(err.detail.starts_with("The URL"), "{}", err.detail);
    assert_eq!(
        checked_url(&with_url(" http://127.0.0.1:3000/mcp "))
            .await
            .unwrap(),
        Some("http://127.0.0.1:3000/mcp".into())
    );
    let stdio = McpTransportInput::Stdio {
        command: "x".into(),
        args: Vec::new(),
        env: Vec::new(),
    };
    assert_eq!(checked_url(&stdio).await.unwrap(), None);
}

#[test]
fn enablement_and_always_allow_are_device_local() {
    let mut v = vault();
    // Servers that arrived from another device: no device state yet.
    let stdio_id = v
        .put(
            None,
            Item::McpServer(McpServer {
                name: "local tool".into(),
                ..McpServer::default()
            }),
        )
        .unwrap();
    let http_id = v
        .put(
            None,
            Item::McpServer(McpServer {
                name: "remote".into(),
                transport: McpTransport::Http {
                    url: "https://mcp.example.com".into(),
                    headers: Default::default(),
                },
                ..McpServer::default()
            }),
        )
        .unwrap();
    let view = |v: &Vault, id: &str| servers_list(v).into_iter().find(|s| s.id == id).unwrap();
    assert!(!view(&v, &stdio_id).enabled);
    assert!(view(&v, &http_id).enabled);
    let pending = v.pending_count();
    set_enabled(&mut v, &stdio_id, true).unwrap();
    set_enabled(&mut v, &http_id, false).unwrap();
    assert!(view(&v, &stdio_id).enabled && !view(&v, &http_id).enabled);
    // Nothing synced changed: only this device's state did.
    assert_eq!(v.pending_count(), pending);

    set_always_allow(&mut v, &stdio_id, Some(" read "), true).unwrap();
    set_always_allow(&mut v, &stdio_id, Some("write"), true).unwrap();
    set_always_allow(&mut v, &stdio_id, Some("read"), true).unwrap();
    assert_eq!(view(&v, &stdio_id).always_allow_tools, ["write", "read"]);
    set_always_allow(&mut v, &stdio_id, Some("write"), false).unwrap();
    assert_eq!(view(&v, &stdio_id).always_allow_tools, ["read"]);
    set_always_allow(&mut v, &http_id, None, true).unwrap();
    assert!(view(&v, &http_id).always_allow);
    assert_eq!(
        set_always_allow(&mut v, &http_id, Some(" "), true)
            .unwrap_err()
            .field
            .as_deref(),
        Some("tool")
    );
    assert_eq!(
        set_enabled(&mut v, "0190a0a0-0000-7000-8000-000000000000", true)
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );

    // Deleting a server forgets its device state.
    delete_server(&mut v, &stdio_id).unwrap();
    assert_eq!(DeviceState::load(&v).server(&stdio_id), Default::default());
    assert_eq!(servers_list(&v).len(), 1);
}

#[test]
fn a_command_line_changed_on_another_device_is_off_here_until_turned_on() {
    let mut v = vault();
    let input = stdio("files", vec![secret("TOKEN", Some(ENV_VALUE))]);
    let id = save_server(&mut v, &input, None).unwrap().view.id;
    set_always_allow(&mut v, &id, None, true).unwrap();
    let view = |v: &Vault| servers_list(v).into_iter().find(|s| s.id == id).unwrap();
    let on = |v: &Vault| {
        let view = view(v);
        view.enabled && view.always_allow
    };

    // Edited here, where the form shows the new command line: still on, still allowed.
    let mut edit = stdio("files", Vec::new());
    edit.id = Some(id.clone());
    edit.transport = McpTransportInput::Stdio {
        command: "uvx".into(),
        args: vec!["mcp-files".into()],
        env: vec![secret("TOKEN", None)],
    };
    save_server(&mut v, &edit, None).unwrap();
    assert!(on(&v));

    let sync = |v: &mut Vault, transport: McpTransport| {
        let mut server = find_server(v, &id).unwrap();
        server.transport = transport;
        v.put(Some(&id), Item::McpServer(server)).unwrap();
    };
    let synced = |command: &str, args: &[&str], env: &[(&str, &str)]| McpTransport::Stdio {
        command: command.into(),
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        env: env
            .iter()
            .map(|(k, v)| ((*k).to_owned(), Zeroizing::new((*v).to_owned())))
            .collect(),
    };
    // A new environment value from another device leaves the command line as it was.
    sync(
        &mut v,
        synced("uvx", &["mcp-files"], &[("TOKEN", "rotated")]),
    );
    assert!(on(&v));

    // Another command, other arguments or other environment names: off here, with no Always
    // allow, until turned on here again.
    for changed in [
        synced("sh", &["mcp-files"], &[("TOKEN", "rotated")]),
        synced("uvx", &["mcp-files", "--all"], &[("TOKEN", "rotated")]),
        synced(
            "uvx",
            &["mcp-files"],
            &[("TOKEN", "rotated"), ("EXTRA", "x")],
        ),
    ] {
        set_enabled(&mut v, &id, true).unwrap();
        set_always_allow(&mut v, &id, None, true).unwrap();
        set_always_allow(&mut v, &id, Some("read"), true).unwrap();
        sync(&mut v, changed);
        let off = view(&v);
        assert!(!off.enabled && !off.always_allow && off.always_allow_tools.is_empty());
    }
    set_enabled(&mut v, &id, true).unwrap();
    let back = view(&v);
    assert!(back.enabled && !back.always_allow && back.always_allow_tools.is_empty());
}

const IMPORT: &str = r#"{
  "mcpServers": {
    "files": {"command": "npx", "args": ["-y", "server-files"], "env": {"TOKEN": "env-value-NEVER-SHOWN"}},
    "web": {"type": "http", "url": "https://mcp.example.com/mcp", "headers": {"Authorization": "Bearer header-value-NEVER-SHOWN"}},
    "old": {"type": "sse", "url": "https://mcp.example.com/sse"}
  }
}"#;

#[test]
fn imports_add_servers_and_exports_hide_values() {
    let mut v = vault();
    save_server(&mut v, &stdio("Files", Vec::new()), None).unwrap();

    let report = parse(IMPORT).unwrap();
    let shown = import_preview(&v, &report);
    let names: Vec<(&str, bool)> = shown
        .servers
        .iter()
        .map(|s| (s.name.as_str(), s.exists))
        .collect();
    assert_eq!(names, [("files", true), ("web", false)]);
    assert_eq!(shown.skipped.len(), 1);
    assert_eq!(shown.skipped[0].name, "old");
    assert!(
        shown.skipped[0]
            .reason
            .starts_with("SSE servers are not supported")
    );
    assert!(shown.skipped[0].reason.ends_with('.'));
    let text = format!("{shown:?}") + &format!("{report:?}");
    assert!(!text.contains("NEVER-SHOWN"), "{text}");

    // A taken name gets a suffix: importing adds another server, never replaces one.
    let views = import_servers(&mut v, report).unwrap();
    let names: Vec<&str> = views.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["files-2", "web"]);
    assert!(views.iter().all(|s| s.enabled));
    assert!(
        !serde_json::to_string(&views)
            .unwrap()
            .contains("NEVER-SHOWN")
    );
    let McpTransport::Stdio { env, .. } = stored(&v, &views[0].id) else {
        panic!("stdio");
    };
    assert_eq!(env["TOKEN"].as_str(), ENV_VALUE);
    let again = import_servers(&mut v, parse(IMPORT).unwrap()).unwrap();
    assert_eq!(again[0].name, "files-3");
    assert_eq!(again[1].name, "web-2");

    // AI-33: placeholders in place of every value.
    let exported = export(&v);
    assert!(!exported.contains("NEVER-SHOWN"), "{exported}");
    assert!(exported.contains("<TOKEN>") && exported.contains("<Authorization>"));
    let json: serde_json::Value = serde_json::from_str(&exported).unwrap();
    assert_eq!(json["mcpServers"].as_object().unwrap().len(), 5);

    // Text that is no server list is refused with a sentence.
    for bad in ["not json", r#"{"other": 1}"#] {
        let err = parse(bad).unwrap_err();
        assert_eq!(err.field.as_deref(), Some("json"));
        let first = err.detail.chars().next().unwrap();
        assert!(
            first.is_uppercase() && err.detail.ends_with('.'),
            "{}",
            err.detail
        );
    }
}

#[test]
fn free_names_stay_within_the_limit() {
    let mut v = vault();
    let long = "n".repeat(70);
    let report = parse(&format!(
        r#"{{"mcpServers": {{"{long}": {{"command": "x"}}}}}}"#
    ))
    .unwrap();
    let first = import_servers(&mut v, report).unwrap();
    assert_eq!(first[0].name.chars().count(), NAME_MAX);
    let report = parse(&format!(
        r#"{{"mcpServers": {{"{long}": {{"command": "x"}}}}}}"#
    ))
    .unwrap();
    let second = import_servers(&mut v, report).unwrap();
    assert_eq!(second[0].name.chars().count(), NAME_MAX);
    assert!(second[0].name.ends_with("-2"));
}

#[test]
fn sentences_start_upper_case_and_end_with_a_period() {
    assert_eq!(sentence("the url is missing"), "The url is missing.");
    assert_eq!(sentence(" Done! "), "Done!");
    assert_eq!(sentence(""), ".");
}
