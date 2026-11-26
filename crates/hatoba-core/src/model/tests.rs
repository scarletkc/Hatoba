use std::collections::BTreeMap;

use serde_json::json;
use zeroize::{Zeroize, Zeroizing};

use super::*;
use crate::Error;

fn sample_host() -> Host {
    Host {
        name: "prod-api-tokyo".into(),
        address: "10.0.0.7".into(),
        username: "deploy".into(),
        auth: HostAuth::password("hunter2"),
        tags: vec!["production".into(), "tokyo".into()],
        favorite: true,
        updated_at: 1_700_000_000_000,
        ..Host::default()
    }
}

#[test]
fn host_json_matches_the_spec_shape() {
    let value = serde_json::to_value(Item::Host(sample_host())).unwrap();
    assert_eq!(
        value,
        json!({
            "type": "host",
            "name": "prod-api-tokyo",
            "address": "10.0.0.7",
            "port": 22,
            "username": "deploy",
            "auth": { "kind": "password", "password": "hunter2" },
            "group_id": null,
            "tags": ["production", "tokyo"],
            "favorite": true,
            "jump_host_id": null,
            "note": "",
            "updated_at": 1_700_000_000_000_i64
        })
    );

    // AI-37: the AI notes are written only when there are some, and read back.
    let host = Host {
        ai_notes: "Debian 12; the app lives in /srv/api.".into(),
        ..sample_host()
    };
    let value = serde_json::to_value(Item::Host(host.clone())).unwrap();
    assert_eq!(value["ai_notes"], "Debian 12; the app lives in /srv/api.");
    let back = Item::from_plaintext(&Item::Host(host.clone()).to_plaintext().unwrap()).unwrap();
    assert_eq!(back, Item::Host(host));
    let old: Item = serde_json::from_value(json!({"type": "host", "name": "old"})).unwrap();
    assert_eq!(old.as_host().unwrap().ai_notes, "");
}

#[test]
fn proxies_match_the_spec_shape() {
    let proxy = Item::Proxy(Proxy {
        name: "Office".into(),
        kind: ProxyKind::Http,
        address: "proxy.example.com".into(),
        port: 3128,
        username: "alice".into(),
        password: Zeroizing::new("s3cret".into()),
        updated_at: 7,
    });
    assert_eq!(
        serde_json::to_value(&proxy).unwrap(),
        json!({
            "type": "proxy",
            "name": "Office",
            "kind": "http",
            "address": "proxy.example.com",
            "port": 3128,
            "username": "alice",
            "password": "s3cret",
            "updated_at": 7
        })
    );
    let text = format!("{proxy:?}");
    assert!(text.contains("alice") && !text.contains("s3cret"), "{text}");
    let old: Item = serde_json::from_value(json!({"type": "proxy", "name": "x"})).unwrap();
    let old = old.as_proxy().unwrap();
    assert_eq!((old.kind, old.port), (ProxyKind::Socks5, 1080));

    // A host writes its proxy only when it is not the device default.
    let value = serde_json::to_value(Item::Host(sample_host())).unwrap();
    assert!(value.get("proxy").is_none());
    for (choice, expected) in [
        (HostProxy::Direct, json!({"kind": "direct"})),
        (
            HostProxy::Proxy {
                proxy_id: "p1".into(),
            },
            json!({"kind": "proxy", "proxy_id": "p1"}),
        ),
    ] {
        let host = Host {
            proxy: choice.clone(),
            ..sample_host()
        };
        let value = serde_json::to_value(Item::Host(host.clone())).unwrap();
        assert_eq!(value["proxy"], expected);
        let back = Item::from_plaintext(&Item::Host(host.clone()).to_plaintext().unwrap());
        assert_eq!(back.unwrap(), Item::Host(host));
    }
    let old: Item = serde_json::from_value(json!({"type": "host", "name": "old"})).unwrap();
    assert_eq!(old.as_host().unwrap().proxy, HostProxy::DeviceDefault);
    assert_eq!(
        HostProxy::Proxy {
            proxy_id: "p1".into()
        }
        .proxy_id(),
        Some("p1")
    );
}

#[test]
fn host_env_is_written_only_when_set_and_read_back() {
    // SSH-14: a host without variables keeps the old shape, so older versions read it as
    // before.
    let value = serde_json::to_value(Item::Host(sample_host())).unwrap();
    assert!(value.get("env").is_none());

    let host = Host {
        env: vec![EnvVar::new("TZ", "Asia/Tokyo"), EnvVar::new("EMPTY", "")],
        ..sample_host()
    };
    let value = serde_json::to_value(Item::Host(host.clone())).unwrap();
    assert_eq!(
        value["env"],
        json!([{"name": "TZ", "value": "Asia/Tokyo"}, {"name": "EMPTY", "value": ""}])
    );
    let back = Item::from_plaintext(&Item::Host(host.clone()).to_plaintext().unwrap()).unwrap();
    assert_eq!(back, Item::Host(host));

    // An item from before SSH-14 has no variables; a newer one with extra fields still reads.
    let old: Item = serde_json::from_value(json!({"type": "host", "name": "old"})).unwrap();
    assert!(old.as_host().unwrap().env.is_empty());
    let newer: Item = serde_json::from_value(json!({
        "type": "host",
        "name": "newer",
        "env": [{"name": "A", "value": "1", "secret": false}, {"name": "B"}]
    }))
    .unwrap();
    assert_eq!(
        newer.as_host().unwrap().env,
        [EnvVar::new("A", "1"), EnvVar::new("B", "")]
    );
}

#[test]
fn host_env_check_rules() {
    let ok = [
        EnvVar::new("LANG", "ja_JP.UTF-8"),
        EnvVar::new("_private", ""),
        EnvVar::new("LC_ALL2", "spaces and = signs"),
        EnvVar::new("lang", "names are case-sensitive"),
    ];
    assert_eq!(check_host_env(&ok), Ok(()));
    assert_eq!(check_host_env(&[]), Ok(()));

    let one = |name: &str, value: &str| check_host_env(&[EnvVar::new(name, value)]);
    assert_eq!(one("", "x"), Err(EnvVarError::NameMissing));
    for bad in ["1ST", "A-B", "A B", "A=B", "日本", "a.b", " A"] {
        assert_eq!(
            one(bad, "x"),
            Err(EnvVarError::NameInvalid(bad.into())),
            "{bad}"
        );
    }
    assert_eq!(one(&"N".repeat(MAX_ENV_NAME_CHARS), ""), Ok(()));
    assert_eq!(
        one(&"N".repeat(MAX_ENV_NAME_CHARS + 1), ""),
        Err(EnvVarError::NameTooLong)
    );
    for bad in ["a\nb", "a\rb", "a\0b", "a\x1bb", "a\x7fb", "a\u{85}b"] {
        assert_eq!(one("V", bad), Err(EnvVarError::ValueInvalid("V".into())));
    }
    assert_eq!(one("V", "\ttab is fine"), Ok(()));
    assert_eq!(one("V", &"値".repeat(MAX_ENV_VALUE_CHARS)), Ok(()));
    assert_eq!(
        one("V", &"値".repeat(MAX_ENV_VALUE_CHARS + 1)),
        Err(EnvVarError::ValueTooLong("V".into()))
    );

    assert_eq!(
        check_host_env(&[
            EnvVar::new("A", "1"),
            EnvVar::new("B", ""),
            EnvVar::new("A", "2")
        ]),
        Err(EnvVarError::NameDuplicate("A".into()))
    );
    let many: Vec<EnvVar> = (0..=MAX_HOST_ENV_VARS)
        .map(|i| EnvVar::new(format!("V{i}"), ""))
        .collect();
    assert_eq!(check_host_env(&many[..MAX_HOST_ENV_VARS]), Ok(()));
    assert_eq!(check_host_env(&many), Err(EnvVarError::TooMany));

    // Names and values together, in UTF-8 bytes.
    let full = MAX_ENV_VALUE_CHARS;
    let mut big = vec![
        EnvVar::new("A", "x".repeat(full)),
        EnvVar::new("B", "x".repeat(full)),
        EnvVar::new("C", "x".repeat(full)),
        EnvVar::new("D", "x".repeat(MAX_HOST_ENV_BYTES - 3 * full - 4)),
    ];
    assert_eq!(env_bytes(&big), MAX_HOST_ENV_BYTES);
    assert_eq!(check_host_env(&big), Ok(()));
    big[3].value.push('x');
    assert_eq!(check_host_env(&big), Err(EnvVarError::TooLarge));
    assert_eq!(
        check_host_env(&[EnvVar::new("V", "値".repeat(MAX_HOST_ENV_BYTES / 3))]),
        Err(EnvVarError::ValueTooLong("V".into()))
    );
}

#[test]
fn the_largest_host_env_fits_an_item() {
    // SSH-14: a host whose variables use the whole budget, with the escaping that costs most
    // (every value byte a quote), still fits the plaintext a synced item may have, with 6,000
    // bytes of AI notes beside them. Longer notes and such variables together may not fit,
    // and saving the host while sync is on then says so.
    let quotes = "\"".repeat(MAX_ENV_VALUE_CHARS);
    let mut env: Vec<EnvVar> = (0..4)
        .map(|i| EnvVar::new(format!("N{i}"), quotes.clone()))
        .collect();
    env.extend((4..MAX_HOST_ENV_VARS).map(|i| EnvVar::new(format!("N{i}"), "")));
    while env_bytes(&env) > MAX_HOST_ENV_BYTES {
        env[3].value.pop();
    }
    assert_eq!(check_host_env(&env), Ok(()));
    let item = Item::Host(Host {
        ai_notes: "日".repeat(2_000),
        env,
        ..sample_host()
    });
    let len = item.plaintext_len().unwrap();
    assert_eq!(len, item.to_plaintext().unwrap().len());
    assert!(len <= MAX_ITEM_PLAINTEXT_BYTES, "{len}");
}

#[test]
fn the_longest_notes_and_instructions_fit_an_item() {
    // AI-36, AI-37: as plain text, whose JSON is as long as the text. The longest notes fit
    // beside environment variables that use the whole budget (SSH-14).
    let value = "v".repeat(MAX_ENV_VALUE_CHARS);
    let mut env: Vec<EnvVar> = (0..4)
        .map(|i| EnvVar::new(format!("N{i}"), value.clone()))
        .collect();
    env.extend((4..MAX_HOST_ENV_VARS).map(|i| EnvVar::new(format!("N{i}"), "")));
    while env_bytes(&env) > MAX_HOST_ENV_BYTES {
        env[3].value.pop();
    }
    assert_eq!(env_bytes(&env), MAX_HOST_ENV_BYTES);
    assert_eq!(check_host_env(&env), Ok(()));
    let host = Item::Host(Host {
        ai_notes: "日".repeat(MAX_HOST_AI_NOTES_BYTES / 3),
        env,
        ..sample_host()
    });
    assert!(host.plaintext_len().unwrap() <= MAX_ITEM_PLAINTEXT_BYTES);
    let mut settings = Settings::default();
    settings.ai.custom_instructions = "x".repeat(MAX_CUSTOM_INSTRUCTIONS_BYTES);
    let settings = Item::Settings(settings);
    assert!(settings.plaintext_len().unwrap() <= MAX_ITEM_PLAINTEXT_BYTES);
}

#[test]
fn telnet_hosts_keep_their_password_out_of_debug_and_empty_fields_out_of_json() {
    let host = TelnetHost {
        name: "core-switch".into(),
        address: "10.0.0.2".into(),
        password: Zeroizing::new("hunter2".into()),
        ..TelnetHost::default()
    };
    assert!(!format!("{host:?}").contains("hunter2"));
    let value = serde_json::to_value(Item::TelnetHost(host.clone())).unwrap();
    assert_eq!(value["port"], 23);
    assert_eq!(value["password"], "hunter2");
    assert!(value.get("proxy").is_none());
    assert!(value.get("ai_notes").is_none());
    let bare = TelnetHost {
        password: Zeroizing::default(),
        ..host
    };
    let value = serde_json::to_value(Item::TelnetHost(bare)).unwrap();
    assert!(value.get("password").is_none());
    // Missing fields take their defaults.
    let read = Item::from_plaintext(br#"{"type":"telnet_host","address":"r1"}"#).unwrap();
    assert_eq!(read.as_telnet_host().unwrap().port, 23);
}

#[test]
fn every_type_tag_round_trips() {
    let items = [
        (Item::Host(Host::default()), "host"),
        (Item::TelnetHost(TelnetHost::default()), "telnet_host"),
        (Item::Group(Group::default()), "group"),
        (Item::Key(SshKey::default()), "key"),
        (Item::KnownHost(KnownHost::default()), "known_host"),
        (Item::Forward(PortForward::default()), "forward"),
        (Item::Snippet(Snippet::default()), "snippet"),
        (Item::Proxy(Proxy::default()), "proxy"),
        (Item::AiProvider(AiProvider::default()), "ai_provider"),
        (
            Item::SearchProvider(SearchProvider::default()),
            "search_provider",
        ),
        (
            Item::AiConversation(AiConversation::default()),
            "ai_conversation",
        ),
        (Item::AiMessage(AiMessage::default()), "ai_message"),
        (Item::Skill(Skill::default()), "skill"),
        (Item::SkillFile(SkillFile::default()), "skill_file"),
        (Item::McpServer(McpServer::default()), "mcp_server"),
        (Item::AiMemory(AiMemory::default()), "ai_memory"),
        (Item::Settings(Settings::default()), "settings"),
    ];
    for (item, tag) in items {
        assert_eq!(item.kind_str(), tag);
        let value = serde_json::to_value(&item).unwrap();
        assert_eq!(value["type"], tag);
        let back = Item::from_plaintext(&item.to_plaintext().unwrap()).unwrap();
        assert_eq!(back, item);
    }
}

#[test]
fn ai_items_match_the_spec_shape() {
    let provider = Item::AiProvider(AiProvider {
        name: "Anthropic".into(),
        protocol: AiProtocol::Anthropic,
        base_url: "https://api.anthropic.com".into(),
        api_key: Zeroizing::new("sk-ant-key".into()),
        auth_header: AiAuthHeader::Authorization,
        models: vec![
            AiModel {
                id: "claude-x".into(),
                name: "Claude X".into(),
                context_window: Some(200_000),
                max_output_tokens: None,
                efforts: Some(vec![AiEffort::Low, AiEffort::High, AiEffort::Max]),
                adaptive_thinking: Some(true),
            },
            AiModel {
                efforts: Some(vec![]),
                ..AiModel::default()
            },
            AiModel::default(),
        ],
        chatgpt: None,
        updated_at: 7,
    });
    assert_eq!(
        serde_json::to_value(&provider).unwrap(),
        json!({
            "type": "ai_provider",
            "name": "Anthropic",
            "protocol": "anthropic",
            "base_url": "https://api.anthropic.com",
            "api_key": "sk-ant-key",
            "auth_header": "authorization",
            "models": [
                {"id": "claude-x", "name": "Claude X",
                 "context_window": 200_000, "max_output_tokens": null,
                 "efforts": ["low", "high", "max"], "adaptive_thinking": true},
                {"id": "", "name": "", "context_window": null, "max_output_tokens": null,
                 "efforts": []},
                {"id": "", "name": "", "context_window": null, "max_output_tokens": null}
            ],
            "updated_at": 7
        })
    );
    assert_eq!(
        serde_json::to_value(Item::AiProvider(AiProvider::default())).unwrap(),
        json!({
            "type": "ai_provider", "name": "", "protocol": "chat_completions",
            "base_url": "", "api_key": "", "auth_header": "x-api-key",
            "models": [], "updated_at": 0
        })
    );
    // AI-52: the account a ChatGPT provider signs in with syncs; its tokens do not.
    let chatgpt = Item::AiProvider(AiProvider {
        name: "ChatGPT".into(),
        protocol: AiProtocol::Responses,
        base_url: "https://api.openai.com/v1".into(),
        chatgpt: Some(AiChatgptAccount {
            client_id: "oaiapp_1".into(),
            subject: "user-1".into(),
            email: "user@example.com".into(),
        }),
        ..AiProvider::default()
    });
    let value = serde_json::to_value(&chatgpt).unwrap();
    assert_eq!(
        value["chatgpt"],
        json!({"client_id": "oaiapp_1", "subject": "user-1", "email": "user@example.com"})
    );
    assert_eq!(serde_json::from_value::<Item>(value).unwrap(), chatgpt);
    assert!(!format!("{chatgpt:?}").contains("user@example.com"));
    let Item::AiProvider(mut wiped) = chatgpt else {
        unreachable!()
    };
    wiped.zeroize();
    assert_eq!(wiped.chatgpt, None);

    let search = Item::SearchProvider(SearchProvider {
        kind: SearchKind::Searxng,
        base_url: Some("https://searx.example.org".into()),
        api_key: Zeroizing::new(String::new()),
        updated_at: 8,
    });
    assert_eq!(
        serde_json::to_value(&search).unwrap(),
        json!({
            "type": "search_provider", "kind": "searxng",
            "base_url": "https://searx.example.org", "api_key": "", "updated_at": 8
        })
    );
    for (kind, text) in [
        (SearchKind::Brave, "brave"),
        (SearchKind::Tavily, "tavily"),
        (SearchKind::Searxng, "searxng"),
    ] {
        assert_eq!(serde_json::to_value(kind).unwrap(), text);
        assert_eq!(kind.as_str(), text);
    }

    let conversation = Item::AiConversation(AiConversation {
        title: "Why is nginx down".into(),
        host_id: Some("h1".into()),
        quick_target: None,
        origin: None,
        local: None,
        pinned: true,
        context_start: Some("e1".into()),
        effort: Some(AiEffort::Xhigh),
        created_at: 5,
        updated_at: 9,
    });
    assert_eq!(
        serde_json::to_value(&conversation).unwrap(),
        json!({
            "type": "ai_conversation", "title": "Why is nginx down", "host_id": "h1",
            "pinned": true, "context_start": "e1", "effort": "xhigh", "created_at": 5,
            "updated_at": 9
        })
    );
    assert_eq!(
        serde_json::to_value(Item::AiConversation(AiConversation::default())).unwrap(),
        json!({
            "type": "ai_conversation", "title": "", "host_id": null, "pinned": false,
            "context_start": null, "created_at": 0, "updated_at": 0
        })
    );
    // On a quick connection (HOST-12), the target in place of a host.
    let target = AiQuickTarget::new("root", "[FE80::1]", 2222);
    assert_eq!(target, AiQuickTarget::new("root", "fe80::1", 2222));
    assert_eq!(
        serde_json::to_value(Item::AiConversation(AiConversation {
            quick_target: Some(target),
            ..AiConversation::default()
        }))
        .unwrap()["quick_target"],
        json!({"username": "root", "address": "fe80::1", "port": 2222})
    );
    // A fork whose copies name a server it cannot tell (AI-43).
    assert_eq!(
        serde_json::to_value(Item::AiConversation(AiConversation {
            origin: Some("prod".into()),
            ..AiConversation::default()
        }))
        .unwrap()["origin"],
        json!("prod")
    );

    let message = Item::AiMessage(AiMessage {
        conversation_id: "c1".into(),
        entry_id: "e1".into(),
        part: 1,
        part_count: 3,
        data: r#"{"role":"user"}"#.into(),
        updated_at: 10,
    });
    assert_eq!(
        serde_json::to_value(&message).unwrap(),
        json!({
            "type": "ai_message", "conversation_id": "c1", "entry_id": "e1",
            "part": 1, "part_count": 3, "data": r#"{"role":"user"}"#, "updated_at": 10
        })
    );

    let mut frontmatter = serde_json::Map::new();
    frontmatter.insert("license".into(), json!("MIT"));
    frontmatter.insert("metadata".into(), json!({"author": "x", "tags": [1, 2]}));
    let skill = Item::Skill(Skill {
        name: "nginx-ops".into(),
        description: "Operate nginx".into(),
        frontmatter,
        enabled: true,
        updated_at: 11,
    });
    assert_eq!(
        serde_json::to_value(&skill).unwrap(),
        json!({
            "type": "skill", "name": "nginx-ops", "description": "Operate nginx",
            "frontmatter": {"license": "MIT", "metadata": {"author": "x", "tags": [1, 2]}},
            "enabled": true, "updated_at": 11
        })
    );

    let file = Item::SkillFile(SkillFile {
        skill_id: "s1".into(),
        path: "references/nginx.md".into(),
        content: "# nginx".into(),
        updated_at: 12,
    });
    assert_eq!(
        serde_json::to_value(&file).unwrap(),
        json!({
            "type": "skill_file", "skill_id": "s1", "path": "references/nginx.md",
            "content": "# nginx", "updated_at": 12
        })
    );

    let stdio = Item::McpServer(McpServer {
        name: "fs".into(),
        transport: McpTransport::Stdio {
            command: "npx".into(),
            args: vec!["-y".into(), "server-fs".into()],
            env: BTreeMap::from([("TOKEN".to_owned(), Zeroizing::new("t0k".to_owned()))]),
        },
        always_ask: true,
        updated_at: 13,
    });
    assert_eq!(
        serde_json::to_value(&stdio).unwrap(),
        json!({
            "type": "mcp_server", "name": "fs",
            "transport": {"kind": "stdio", "command": "npx", "args": ["-y", "server-fs"],
                          "env": {"TOKEN": "t0k"}},
            "always_ask": true, "updated_at": 13
        })
    );
    let http = Item::McpServer(McpServer {
        name: "remote".into(),
        transport: McpTransport::Http {
            url: "https://mcp.example.org/mcp".into(),
            headers: BTreeMap::from([(
                "Authorization".to_owned(),
                Zeroizing::new("Bearer b".to_owned()),
            )]),
        },
        always_ask: false,
        updated_at: 14,
    });
    assert_eq!(
        serde_json::to_value(&http).unwrap(),
        json!({
            "type": "mcp_server", "name": "remote",
            "transport": {"kind": "http", "url": "https://mcp.example.org/mcp",
                          "headers": {"Authorization": "Bearer b"}},
            "always_ask": false, "updated_at": 14
        })
    );
    let memory = Item::AiMemory(AiMemory {
        name: "web-1 sudo".into(),
        content: "web-1 uses doas, not sudo.".into(),
        updated_at: 15,
        ..AiMemory::default()
    });
    assert_eq!(
        serde_json::to_value(&memory).unwrap(),
        json!({
            "type": "ai_memory", "name": "web-1 sudo",
            "content": "web-1 uses doas, not sudo.", "updated_at": 15
        })
    );
    let host_memory = Item::AiMemory(AiMemory {
        name: "sudo".into(),
        content: "Uses doas, not sudo.".into(),
        host_id: Some("h1".into()),
        updated_at: 16,
        ..AiMemory::default()
    });
    assert_eq!(
        serde_json::to_value(&host_memory).unwrap(),
        json!({
            "type": "ai_memory", "name": "sudo", "content": "Uses doas, not sudo.",
            "host_id": "h1", "updated_at": 16
        })
    );
    let local_memory = Item::AiMemory(AiMemory {
        name: "shell".into(),
        content: "PowerShell 7.".into(),
        local: Some("KC-PC".into()),
        updated_at: 17,
        ..AiMemory::default()
    });
    assert_eq!(
        serde_json::to_value(&local_memory).unwrap(),
        json!({
            "type": "ai_memory", "name": "shell", "content": "PowerShell 7.",
            "local": "KC-PC", "updated_at": 17
        })
    );

    let mut settings = Settings::default();
    settings.ai.default_model = Some(AiModelRef {
        provider_id: "p1".into(),
        model_id: "m1".into(),
    });
    settings.ai.search_provider_id = Some("sp1".into());
    settings.ai.default_effort = Some(AiEffort::Medium);
    settings.ai.custom_instructions = "Answer in English.".into();
    settings.ai.judge_model = Some(AiModelRef {
        provider_id: "p2".into(),
        model_id: "jev-latest".into(),
    });
    let value = serde_json::to_value(Item::Settings(settings)).unwrap();
    assert_eq!(
        value["ai"],
        json!({
            "default_model": {"provider_id": "p1", "model_id": "m1"},
            "default_effort": "medium",
            "search_provider_id": "sp1",
            "native_search": true,
            "builtin_skill_enabled": true,
            "custom_instructions": "Answer in English.",
            "send_terminal_screen": true,
            "judge_model": {"provider_id": "p2", "model_id": "jev-latest"},
            "memory": true,
            "text_size": 13
        })
    );
    assert_eq!(
        serde_json::to_value(Item::Settings(Settings::default())).unwrap()["ai"],
        json!({
            "default_model": null,
            "search_provider_id": null,
            "native_search": true,
            "builtin_skill_enabled": true,
            "send_terminal_screen": true,
            "memory": true,
            "text_size": 13
        })
    );

    // Every shape reads back as written.
    for item in [
        provider,
        search,
        conversation,
        message,
        skill,
        file,
        stdio,
        http,
        memory,
        host_memory,
        local_memory,
    ] {
        let back = Item::from_plaintext(&item.to_plaintext().unwrap()).unwrap();
        assert_eq!(back, item);
    }
}

#[test]
fn ai_items_tolerate_unknown_and_missing_fields() {
    let read = |value: serde_json::Value| -> Item { serde_json::from_value(value).unwrap() };

    // Settings written before the AI assistant existed have no `ai`.
    let old = read(json!({"type": "settings", "auto_lock_minutes": 5}));
    assert_eq!(old.as_settings().unwrap().ai, AiSettings::default());
    // Those written before the built-in skill (AI-34) have it on.
    let before = read(json!({"type": "settings", "ai": {"search_provider_id": "s"}}));
    let ai = &before.as_settings().unwrap().ai;
    assert!(ai.builtin_skill_enabled);
    assert_eq!(ai.search_provider_id.as_deref(), Some("s"));
    let off = read(json!({"type": "settings", "ai": {"builtin_skill_enabled": false}}));
    assert!(!off.as_settings().unwrap().ai.builtin_skill_enabled);
    // Those written before native search (AI-14) search with the chat provider.
    assert!(ai.native_search);
    let none = read(json!({"type": "settings", "ai": {"native_search": false}}));
    assert!(!none.as_settings().unwrap().ai.native_search);
    // And those written before the screen note (AI-46) send the screen.
    assert!(ai.send_terminal_screen);
    let quiet = read(json!({"type": "settings", "ai": {"send_terminal_screen": false}}));
    assert!(!quiet.as_settings().unwrap().ai.send_terminal_screen);
    // Those written before memory (AI-53) have it on.
    assert!(ai.memory);
    let forgetful = read(json!({"type": "settings", "ai": {"memory": false}}));
    assert!(!forgetful.as_settings().unwrap().ai.memory);
    // And those written before the AI text size (TERM-13) have the default.
    assert_eq!(ai.text_size, AI_TEXT_SIZE_DEFAULT);
    let memory = read(json!({"type": "ai_memory", "name": "web-1", "extra": 1}));
    assert_eq!(
        memory,
        Item::AiMemory(AiMemory {
            name: "web-1".into(),
            ..AiMemory::default()
        })
    );
    // AI-36: settings from before custom instructions have none.
    assert_eq!(ai.custom_instructions, "");
    let with = read(json!({
        "type": "settings",
        "ai": {"custom_instructions": "Reply in English.\nUse zsh."}
    }));
    assert_eq!(
        with.as_settings().unwrap().ai.custom_instructions,
        "Reply in English.\nUse zsh."
    );

    // AI-05: settings, conversations and models from before thinking levels read as Default
    // and unknown, and a level a newer version added does not make the item unreadable.
    assert_eq!(ai.default_effort, None);
    let newer = read(json!({"type": "settings", "ai": {"default_effort": "ultra"}}));
    assert_eq!(newer.as_settings().unwrap().ai.default_effort, None);
    let high = read(json!({"type": "settings", "ai": {"default_effort": "high"}}));
    assert_eq!(
        high.as_settings().unwrap().ai.default_effort,
        Some(AiEffort::High)
    );
    let old = read(json!({"type": "ai_conversation", "title": "t", "pinned": true}));
    assert_eq!(old.as_ai_conversation().unwrap().effort, None);
    for effort in [json!("ultra"), json!(3), json!(null)] {
        let c = read(json!({"type": "ai_conversation", "effort": effort}));
        assert_eq!(c.as_ai_conversation().unwrap().effort, None, "{effort}");
    }
    let levels = read(json!({
        "type": "ai_provider",
        "models": [
            {"id": "a", "efforts": ["max", "ultra", "low", "low"], "adaptive_thinking": false},
            {"id": "b", "efforts": "all"},
            {"id": "c"}
        ]
    }));
    let models = &levels.as_ai_provider().unwrap().models;
    assert_eq!(models[0].efforts, Some(vec![AiEffort::Low, AiEffort::Max]));
    assert_eq!(models[0].adaptive_thinking, Some(false));
    assert_eq!(models[1].efforts, None);
    assert_eq!(
        (models[2].efforts.clone(), models[2].adaptive_thinking),
        (None, None)
    );

    // AI-47: settings from before Auto mode have no judge.
    assert_eq!(ai.judge_model, None);
    let judged = read(json!({
        "type": "settings",
        "ai": {"judge_model": {"provider_id": "p", "model_id": "clef-flash"}}
    }));
    assert_eq!(
        judged.as_settings().unwrap().ai.judge_model,
        Some(AiModelRef {
            provider_id: "p".into(),
            model_id: "clef-flash".into(),
        })
    );
    let system_one = read(json!({"type": "ai_provider", "protocol": "system_one"}));
    assert_eq!(
        system_one.as_ai_provider().unwrap().protocol,
        AiProtocol::SystemOne
    );
    assert_eq!(AiProtocol::SystemOne.as_str(), "system_one");
    let responses = read(json!({"type": "ai_provider", "protocol": "responses"}));
    assert_eq!(
        responses.as_ai_provider().unwrap().protocol,
        AiProtocol::Responses
    );
    assert_eq!(AiProtocol::Responses.as_str(), "responses");

    // A missing or unknown protocol / auth header reads as the default.
    let provider = read(json!({
        "type": "ai_provider", "name": "p", "protocol": "grpc", "auth_header": "cookie",
        "base_url": "https://x", "future": [1],
        "models": [{"id": "m", "extra": true}]
    }));
    let provider = provider.as_ai_provider().unwrap();
    assert_eq!(provider.protocol, AiProtocol::ChatCompletions);
    assert_eq!(provider.auth_header, AiAuthHeader::XApiKey);
    assert_eq!(provider.models[0].id, "m");
    assert_eq!(provider.models[0].context_window, None);
    assert_eq!(provider.api_key.as_str(), "");
    let bare = read(json!({"type": "ai_provider"}));
    assert_eq!(
        bare.as_ai_provider().unwrap().protocol,
        AiProtocol::ChatCompletions
    );

    // A search kind from the future is not guessed: the item is unreadable instead.
    assert!(matches!(
        Item::from_plaintext(br#"{"type":"search_provider","kind":"kagi"}"#),
        Err(Error::Format(_))
    ));
    assert_eq!(
        read(json!({"type": "search_provider"}))
            .as_search_provider()
            .unwrap()
            .kind,
        SearchKind::Brave
    );

    // MCP transports tolerate missing fields; their kind is required to be known.
    let mcp = read(json!({
        "type": "mcp_server", "name": "m", "extra": 1,
        "transport": {"kind": "http", "url": "https://m", "future": 1}
    }));
    assert_eq!(
        mcp.as_mcp_server().unwrap().transport,
        McpTransport::Http {
            url: "https://m".into(),
            headers: BTreeMap::new()
        }
    );
    let stdio = read(json!({"type": "mcp_server", "transport": {"kind": "stdio"}}));
    assert_eq!(
        stdio.as_mcp_server().unwrap().transport,
        McpTransport::default()
    );
    assert!(
        Item::from_plaintext(br#"{"type":"mcp_server","transport":{"kind":"carrier-pigeon"}}"#)
            .is_err()
    );
    let missing = read(json!({"type": "mcp_server"}));
    assert_eq!(
        missing.as_mcp_server().unwrap().transport.kind_str(),
        "stdio"
    );

    let skill = read(json!({"type": "skill", "name": "s", "unknown": {}}));
    assert!(skill.as_skill().unwrap().frontmatter.is_empty());
    assert!(!skill.as_skill().unwrap().enabled);
}

#[test]
fn ai_item_names_and_accessors() {
    let provider = Item::AiProvider(AiProvider {
        name: "Anthropic".into(),
        updated_at: 3,
        ..AiProvider::default()
    });
    assert_eq!(provider.display_name(), "Anthropic");
    assert_eq!(provider.updated_at(), 3);
    assert_eq!(
        provider.with_name_suffix(" (copy)").display_name(),
        "Anthropic (copy)"
    );
    assert!(provider.as_ai_provider().is_some() && provider.as_host().is_none());
    assert!(!provider.is_key());

    let search = Item::SearchProvider(SearchProvider {
        kind: SearchKind::Tavily,
        ..SearchProvider::default()
    });
    assert_eq!(search.display_name(), "tavily");
    assert_eq!(search.with_name_suffix(" (copy)"), search);
    assert!(search.as_search_provider().is_some());

    let mut conversation = Item::AiConversation(AiConversation {
        title: "Disk full".into(),
        ..AiConversation::default()
    });
    assert_eq!(conversation.display_name(), "Disk full");
    conversation.set_updated_at(99);
    assert_eq!(conversation.updated_at(), 99);
    assert!(conversation.as_ai_conversation().is_some());

    let mut message = Item::AiMessage(AiMessage {
        data: "x".into(),
        ..AiMessage::default()
    });
    assert_eq!(message.display_name(), "message");
    message.set_updated_at(4);
    assert_eq!(message.updated_at(), 4);
    assert_eq!(message.as_ai_message().unwrap().data, "x");
    message.strip_message_data();
    assert_eq!(message.as_ai_message().unwrap().data, "");

    let mut skill = Item::Skill(Skill {
        name: "nginx".into(),
        ..Skill::default()
    });
    skill.strip_message_data(); // a no-op for every other type
    assert_eq!(skill.display_name(), "nginx");
    assert!(skill.as_skill().is_some());
    let file = Item::SkillFile(SkillFile {
        path: "SKILL.md".into(),
        ..SkillFile::default()
    });
    assert_eq!(file.display_name(), "SKILL.md");
    assert!(file.as_skill_file().is_some());
    let mcp = Item::McpServer(McpServer {
        name: "fs".into(),
        ..McpServer::default()
    });
    assert_eq!(mcp.display_name(), "fs");
    assert_eq!(mcp.with_name_suffix("!").display_name(), "fs!");
    assert!(mcp.as_mcp_server().is_some());
}

#[test]
fn auth_variants_use_the_kind_tag() {
    let cases = [
        (
            HostAuth::password("p"),
            json!({"kind":"password","password":"p"}),
        ),
        (
            HostAuth::Key { key_id: "k".into() },
            json!({"kind":"key","key_id":"k"}),
        ),
        (HostAuth::Agent, json!({"kind":"agent"})),
        (HostAuth::Ask, json!({"kind":"ask"})),
    ];
    for (auth, expected) in cases {
        assert_eq!(serde_json::to_value(&auth).unwrap(), expected);
        assert_eq!(serde_json::from_value::<HostAuth>(expected).unwrap(), auth);
    }
}

#[test]
fn settings_defaults_match_the_spec() {
    let s = Settings::default();
    assert_eq!(s.terminal.font_family, DEFAULT_FONT_FAMILY);
    assert_eq!(
        DEFAULT_FONT_FAMILY,
        if cfg!(target_os = "macos") {
            "Menlo"
        } else {
            "Cascadia Mono"
        }
    );
    assert_eq!(s.terminal.font_size, 13);
    assert_eq!(s.terminal.theme, ThemeMode::Dark);
    assert_eq!(s.terminal.cursor_style, CursorStyle::Block);
    assert_eq!(s.terminal.scrollback, 10_000);
    assert_eq!(s.terminal.right_click(), RightClick::platform_default());
    assert!(s.terminal.confirm_multiline_paste());
    assert!(s.terminal.suggest_commands());
    let value = serde_json::to_value(Item::Settings(s)).unwrap();
    assert_eq!(value["terminal"]["theme"], "dark");
    assert_eq!(value["terminal"]["cursor_style"], "block");
    // Unset fields are left out, as builds that kept them device-local wrote them.
    assert!(value["terminal"].get("right_click").is_none());
    assert!(value["terminal"].get("confirm_multiline_paste").is_none());
    assert!(value["terminal"].get("suggest_commands").is_none());
    assert!(value["terminal"].get("dark_scheme").is_none());
    assert!(value["terminal"].get("light_scheme").is_none());
}

#[test]
fn terminal_behaviour_tells_unset_from_default() {
    let read = |terminal: serde_json::Value| -> TerminalSettings {
        let item: Item =
            serde_json::from_value(json!({"type":"settings","terminal":terminal})).unwrap();
        item.as_settings().unwrap().terminal.clone()
    };
    let unset = read(json!({"font_size": 14}));
    assert_eq!(unset.right_click, None);
    assert_eq!(unset.confirm_multiline_paste, None);
    assert_eq!(unset.suggest_commands, None);
    let set = read(json!({
        "right_click": "copy_paste",
        "confirm_multiline_paste": true,
        "suggest_commands": true
    }));
    assert_eq!(set.right_click, Some(RightClick::CopyPaste));
    assert_eq!(set.right_click(), RightClick::CopyPaste);
    assert_eq!(set.confirm_multiline_paste, Some(true));
    assert_eq!(set.suggest_commands, Some(true));
    let set = read(json!({
        "right_click": "menu",
        "confirm_multiline_paste": false,
        "suggest_commands": false
    }));
    assert_eq!(set.right_click(), RightClick::Menu);
    assert!(!set.confirm_multiline_paste());
    assert!(!set.suggest_commands());
    let value = serde_json::to_value(&set).unwrap();
    assert_eq!(value["right_click"], "menu");
    assert_eq!(value["confirm_multiline_paste"], false);
    assert_eq!(value["suggest_commands"], false);
    // An unknown future value still counts as a choice the user made.
    assert_eq!(
        read(json!({"right_click":"middle_paste"})).right_click,
        Some(RightClick::CopyPaste)
    );
}

#[test]
fn colour_schemes_round_trip_as_stored() {
    let item: Item = serde_json::from_value(json!({
        "type": "settings",
        "terminal": {"theme": "system", "dark_scheme": "dracula", "light_scheme": "from-the-future"}
    }))
    .unwrap();
    let t = &item.as_settings().unwrap().terminal;
    assert_eq!(t.dark_scheme.as_deref(), Some("dracula"));
    // The desktop app decides what an id it does not know reads as; the item keeps it.
    assert_eq!(t.light_scheme.as_deref(), Some("from-the-future"));
    let value = serde_json::to_value(t).unwrap();
    assert_eq!(value["dark_scheme"], "dracula");
    assert_eq!(value["light_scheme"], "from-the-future");
}

#[test]
fn recording_the_default_leaves_an_unset_field_unset() {
    // The default depends on the platform, so name the value that is not it.
    let default = RightClick::platform_default();
    let other = match default {
        RightClick::Menu => RightClick::CopyPaste,
        RightClick::CopyPaste => RightClick::Menu,
    };
    let mut t = TerminalSettings::default();
    t.set_right_click(default);
    t.set_confirm_multiline_paste(true);
    t.set_suggest_commands(true);
    assert_eq!(t.right_click, None);
    assert_eq!(t.right_click(), default);
    assert_eq!(t.confirm_multiline_paste, None);
    assert_eq!(t.suggest_commands, None);

    t.set_right_click(other);
    t.set_confirm_multiline_paste(false);
    t.set_suggest_commands(false);
    assert_eq!(t.right_click, Some(other));
    assert_eq!(t.right_click(), other);
    assert_eq!(t.confirm_multiline_paste, Some(false));
    assert_eq!(t.suggest_commands, Some(false));
    assert!(!t.suggest_commands());

    // Once set, changing back to the default is recorded too.
    t.set_right_click(default);
    t.set_confirm_multiline_paste(true);
    t.set_suggest_commands(true);
    assert_eq!(t.right_click, Some(default));
    assert_eq!(t.confirm_multiline_paste, Some(true));
    assert_eq!(t.suggest_commands, Some(true));
}

#[test]
fn tolerant_of_unknown_and_missing_fields() {
    let item: Item = serde_json::from_value(json!({
        "type": "host",
        "name": "x",
        "address": "example.org",
        "from_the_future": { "nested": [1, 2, 3] }
    }))
    .unwrap();
    let host = item.as_host().unwrap();
    assert_eq!(host.port, 22);
    assert_eq!(host.auth, HostAuth::Ask);
    assert!(host.tags.is_empty());
    assert_eq!(host.group_id, None);

    // Unknown enum values degrade gracefully instead of failing the whole item.
    let item: Item = serde_json::from_value(
        json!({"type":"settings","terminal":{"theme":"solarized","cursor_style":"beam"}}),
    )
    .unwrap();
    let s = item.as_settings().unwrap();
    assert_eq!(s.terminal.theme, ThemeMode::System);
    assert_eq!(s.terminal.cursor_style, CursorStyle::Block);
}

#[test]
fn unknown_type_is_a_format_error() {
    let err = Item::from_plaintext(br#"{"type":"teleporter","name":"x"}"#);
    assert!(matches!(err, Err(Error::Format(_))));
    assert!(matches!(Item::from_plaintext(b"{}"), Err(Error::Format(_))));
}

#[test]
fn updated_at_and_names() {
    let mut item = Item::Host(sample_host());
    assert_eq!(item.updated_at(), 1_700_000_000_000);
    item.set_updated_at(5);
    assert_eq!(item.updated_at(), 5);
    assert_eq!(item.display_name(), "prod-api-tokyo");
    let key = Item::Key(SshKey {
        name: "laptop".into(),
        ..SshKey::default()
    });
    assert_eq!(
        key.with_name_suffix(" (copy)").display_name(),
        "laptop (copy)"
    );
    assert!(key.is_key());
    let kh = Item::KnownHost(KnownHost {
        host: "h".into(),
        port: 2222,
        ..KnownHost::default()
    });
    assert_eq!(kh.display_name(), "h:2222");
    let forward = |name: &str| {
        Item::Forward(PortForward {
            name: name.into(),
            bind_address: "127.0.0.1".into(),
            bind_port: 15432,
            ..PortForward::default()
        })
    };
    assert_eq!(forward("Postgres").display_name(), "Postgres");
    assert_eq!(forward("").display_name(), "127.0.0.1:15432");
}

#[test]
fn ids_are_uuid_v7_and_time_ordered() {
    let a = new_id();
    let b = new_id();
    let parsed = uuid::Uuid::parse_str(&a).unwrap();
    assert_eq!(parsed.get_version_num(), 7);
    assert_ne!(a, b);
}

#[test]
fn debug_never_prints_secrets() {
    let host = Item::Host(sample_host());
    assert!(!format!("{host:?}").contains("hunter2"));
    let key = Item::Key(SshKey {
        private_key: Zeroizing::new("-----BEGIN OPENSSH PRIVATE KEY-----SECRETBODY".into()),
        passphrase: Some(Zeroizing::new("pass-phrase-secret".into())),
        ..SshKey::default()
    });
    let dbg = format!("{key:?}");
    assert!(!dbg.contains("SECRETBODY"));
    assert!(!dbg.contains("pass-phrase-secret"));

    let provider = Item::AiProvider(AiProvider {
        name: "openai".into(),
        api_key: Zeroizing::new("sk-provider-secret".into()),
        ..AiProvider::default()
    });
    let search = Item::SearchProvider(SearchProvider {
        api_key: Zeroizing::new("search-api-secret".into()),
        ..SearchProvider::default()
    });
    let stdio = Item::McpServer(McpServer {
        name: "fs".into(),
        transport: McpTransport::Stdio {
            command: "npx".into(),
            args: vec!["server".into()],
            env: BTreeMap::from([(
                "API_TOKEN".to_owned(),
                Zeroizing::new("env-secret".to_owned()),
            )]),
        },
        ..McpServer::default()
    });
    let http = Item::McpServer(McpServer {
        transport: McpTransport::Http {
            url: "https://mcp.example.org".into(),
            headers: BTreeMap::from([(
                "Authorization".to_owned(),
                Zeroizing::new("header-secret".to_owned()),
            )]),
        },
        ..McpServer::default()
    });
    let message = Item::AiMessage(AiMessage {
        entry_id: "e1".into(),
        data: "conversation-secret".into(),
        ..AiMessage::default()
    });
    for (item, secret) in [
        (&provider, "sk-provider-secret"),
        (&search, "search-api-secret"),
        (&stdio, "env-secret"),
        (&http, "header-secret"),
        (&message, "conversation-secret"),
    ] {
        let dbg = format!("{item:?}");
        assert!(!dbg.contains(secret), "{dbg}");
        let pretty = format!("{item:#?}");
        assert!(!pretty.contains(secret), "{pretty}");
    }
    // What is not secret stays readable.
    assert!(format!("{provider:?}").contains("openai"));
    assert!(format!("{stdio:?}").contains("API_TOKEN"));
    assert!(format!("{http:?}").contains("https://mcp.example.org"));
    assert!(format!("{message:?}").contains("e1"));
}

#[test]
fn zeroize_wipes_secret_fields() {
    let mut item = Item::Key(SshKey {
        private_key: Zeroizing::new("PRIVATE".into()),
        passphrase: Some(Zeroizing::new("PASS".into())),
        name: "n".into(),
        ..SshKey::default()
    });
    item.zeroize();
    let key = item.as_key().unwrap();
    assert!(key.private_key.is_empty());
    assert!(key.passphrase.is_none());
    assert!(key.name.is_empty());

    let mut provider = Item::AiProvider(AiProvider {
        name: "n".into(),
        api_key: Zeroizing::new("KEY".into()),
        models: vec![AiModel {
            id: "m".into(),
            ..AiModel::default()
        }],
        ..AiProvider::default()
    });
    provider.zeroize();
    let provider = provider.as_ai_provider().unwrap();
    assert!(provider.api_key.is_empty() && provider.name.is_empty());
    assert!(provider.models.is_empty());

    let mut search = Item::SearchProvider(SearchProvider {
        base_url: Some("https://s".into()),
        api_key: Zeroizing::new("KEY".into()),
        ..SearchProvider::default()
    });
    search.zeroize();
    let search = search.as_search_provider().unwrap();
    assert!(search.api_key.is_empty() && search.base_url.is_none());

    let mut mcp = Item::McpServer(McpServer {
        name: "n".into(),
        transport: McpTransport::Stdio {
            command: "cmd".into(),
            args: vec!["a".into()],
            env: BTreeMap::from([("K".to_owned(), Zeroizing::new("V".to_owned()))]),
        },
        ..McpServer::default()
    });
    mcp.zeroize();
    let mcp = mcp.as_mcp_server().unwrap();
    assert!(mcp.name.is_empty());
    assert_eq!(mcp.transport, McpTransport::default());
    let mut http = McpTransport::Http {
        url: "https://u".into(),
        headers: BTreeMap::from([("H".to_owned(), Zeroizing::new("V".to_owned()))]),
    };
    http.zeroize();
    assert_eq!(
        http,
        McpTransport::Http {
            url: String::new(),
            headers: BTreeMap::new()
        }
    );

    let mut message = Item::AiMessage(AiMessage {
        data: "DATA".into(),
        conversation_id: "c".into(),
        ..AiMessage::default()
    });
    message.zeroize();
    let message = message.as_ai_message().unwrap();
    assert!(message.data.is_empty() && message.conversation_id.is_empty());
}

#[test]
fn hand_written_as_str_matches_serde_name_for_every_variant() {
    for v in [ProxyKind::Socks5, ProxyKind::Http] {
        match v {
            ProxyKind::Socks5 | ProxyKind::Http => {}
        }
        assert_eq!(serde_json::to_value(v).unwrap(), v.as_str());
    }
    for v in [
        AiProtocol::Anthropic,
        AiProtocol::Responses,
        AiProtocol::SystemOne,
        AiProtocol::ChatCompletions,
    ] {
        match v {
            AiProtocol::Anthropic
            | AiProtocol::Responses
            | AiProtocol::SystemOne
            | AiProtocol::ChatCompletions => {}
        }
        assert_eq!(serde_json::to_value(v).unwrap(), v.as_str());
    }
    for v in [AiAuthHeader::Authorization, AiAuthHeader::XApiKey] {
        match v {
            AiAuthHeader::Authorization | AiAuthHeader::XApiKey => {}
        }
        assert_eq!(serde_json::to_value(v).unwrap(), v.as_str());
    }
    for v in AiEffort::ALL {
        match v {
            AiEffort::Low | AiEffort::Medium | AiEffort::High | AiEffort::Xhigh | AiEffort::Max => {
            }
        }
        assert_eq!(serde_json::to_value(v).unwrap(), v.as_str());
    }
    for v in [SearchKind::Brave, SearchKind::Tavily, SearchKind::Searxng] {
        match v {
            SearchKind::Brave | SearchKind::Tavily | SearchKind::Searxng => {}
        }
        assert_eq!(serde_json::to_value(v).unwrap(), v.as_str());
    }
    for v in [
        McpTransport::Stdio {
            command: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
        },
        McpTransport::Http {
            url: String::new(),
            headers: BTreeMap::new(),
        },
    ] {
        match &v {
            McpTransport::Stdio { .. } | McpTransport::Http { .. } => {}
        }
        assert_eq!(serde_json::to_value(&v).unwrap()["kind"], v.kind_str());
    }
    for v in [
        Item::Host(Host::default()),
        Item::TelnetHost(TelnetHost::default()),
        Item::Group(Group::default()),
        Item::Key(SshKey::default()),
        Item::KnownHost(KnownHost::default()),
        Item::Forward(PortForward::default()),
        Item::Snippet(Snippet::default()),
        Item::Proxy(Proxy::default()),
        Item::AiProvider(AiProvider::default()),
        Item::SearchProvider(SearchProvider::default()),
        Item::AiConversation(AiConversation::default()),
        Item::AiMessage(AiMessage::default()),
        Item::Skill(Skill::default()),
        Item::SkillFile(SkillFile::default()),
        Item::McpServer(McpServer::default()),
        Item::AiMemory(AiMemory::default()),
        Item::Settings(Settings::default()),
    ] {
        match &v {
            Item::Host(_)
            | Item::TelnetHost(_)
            | Item::Group(_)
            | Item::Key(_)
            | Item::KnownHost(_)
            | Item::Forward(_)
            | Item::Snippet(_)
            | Item::Proxy(_)
            | Item::AiProvider(_)
            | Item::SearchProvider(_)
            | Item::AiConversation(_)
            | Item::AiMessage(_)
            | Item::Skill(_)
            | Item::SkillFile(_)
            | Item::McpServer(_)
            | Item::AiMemory(_)
            | Item::Settings(_) => {}
        }
        assert_eq!(serde_json::to_value(&v).unwrap()["type"], v.kind_str());
    }
}

#[test]
fn golden_json_fixtures_cover_every_item_and_nested_enum() {
    let mut frontmatter = serde_json::Map::new();
    frontmatter.insert("license".into(), json!("MIT"));
    frontmatter.insert(
        "metadata".into(),
        json!({"author": "alice", "tags": [1, 2]}),
    );

    let cases: Vec<(&str, &str, Item)> = vec![
        (
            "host_password.json",
            include_str!("../../tests/fixtures/items/host_password.json"),
            Item::Host(Host {
                name: "prod-api-tokyo".into(),
                address: "10.0.0.7".into(),
                port: 22,
                username: "deploy".into(),
                auth: HostAuth::password("hunter2"),
                group_id: Some("grp-1".into()),
                tags: vec!["production".into(), "tokyo".into()],
                favorite: true,
                sort: 1000,
                jump_host_id: Some("bastion-1".into()),
                proxy: HostProxy::Proxy {
                    proxy_id: "proxy-1".into(),
                },
                note: "Primary API node".into(),
                ai_notes: "Debian 12; app lives in /srv/api.".into(),
                updated_at: 1_700_000_000_000,
                env: vec![EnvVar::new("TZ", "Asia/Tokyo")],
            }),
        ),
        (
            "host_key.json",
            include_str!("../../tests/fixtures/items/host_key.json"),
            Item::Host(Host {
                name: "staging-web".into(),
                address: "10.0.1.8".into(),
                port: 2222,
                username: "ubuntu".into(),
                auth: HostAuth::Key {
                    key_id: "key-1".into(),
                },
                proxy: HostProxy::Direct,
                updated_at: 1_700_000_000_001,
                ..Host::default()
            }),
        ),
        (
            "host_agent.json",
            include_str!("../../tests/fixtures/items/host_agent.json"),
            Item::Host(Host {
                name: "bastion".into(),
                address: "bastion.example.org".into(),
                port: 22,
                username: "ops".into(),
                auth: HostAuth::Agent,
                tags: vec!["jump".into()],
                proxy: HostProxy::DeviceDefault,
                updated_at: 1_700_000_000_002,
                ..Host::default()
            }),
        ),
        (
            "host_ask.json",
            include_str!("../../tests/fixtures/items/host_ask.json"),
            Item::Host(Host {
                name: "sandbox".into(),
                address: "192.0.2.10".into(),
                port: 22,
                username: "alice".into(),
                auth: HostAuth::Ask,
                updated_at: 1_700_000_000_003,
                ..Host::default()
            }),
        ),
        (
            "telnet_host.json",
            include_str!("../../tests/fixtures/items/telnet_host.json"),
            Item::TelnetHost(TelnetHost {
                name: "core-switch-1".into(),
                address: "10.0.0.2".into(),
                port: 23,
                username: "admin".into(),
                password: Zeroizing::new("switch-password".into()),
                group_id: Some("grp-1".into()),
                tags: vec!["network".into()],
                favorite: true,
                sort: 2000,
                proxy: HostProxy::Proxy {
                    proxy_id: "proxy-1".into(),
                },
                note: "Core rack".into(),
                ai_notes: "Cisco IOS".into(),
                updated_at: 1_700_000_000_004,
            }),
        ),
        (
            "group.json",
            include_str!("../../tests/fixtures/items/group.json"),
            Item::Group(Group {
                name: "Production".into(),
                parent_id: Some("grp-root".into()),
                sort: 1000,
                updated_at: 1_700_000_000_005,
            }),
        ),
        (
            "key_ed25519.json",
            include_str!("../../tests/fixtures/items/key_ed25519.json"),
            Item::Key(SshKey {
                name: "laptop-ed25519".into(),
                algorithm: KeyAlgorithm::Ed25519,
                private_key: Zeroizing::new(
                    "-----BEGIN OPENSSH PRIVATE KEY-----\nTEST\n-----END OPENSSH PRIVATE KEY-----"
                        .into(),
                ),
                passphrase: Some(Zeroizing::new("key-passphrase".into())),
                public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest alice@laptop".into(),
                fingerprint: "SHA256:abcdef0123456789".into(),
                comment: "alice@laptop".into(),
                created_at: 1_700_000_000_000,
                updated_at: 1_700_000_000_006,
            }),
        ),
        (
            "key_ecdsa.json",
            include_str!("../../tests/fixtures/items/key_ecdsa.json"),
            Item::Key(SshKey {
                name: "server-ecdsa".into(),
                algorithm: KeyAlgorithm::Ecdsa,
                private_key: Zeroizing::new(
                    "-----BEGIN OPENSSH PRIVATE KEY-----\nECDSA\n-----END OPENSSH PRIVATE KEY-----"
                        .into(),
                ),
                passphrase: None,
                public_key: "ecdsa-sha2-nistp256 AAAATest".into(),
                fingerprint: "SHA256:ecdsa123456".into(),
                comment: String::new(),
                created_at: 1_700_000_000_000,
                updated_at: 1_700_000_000_007,
            }),
        ),
        (
            "key_rsa.json",
            include_str!("../../tests/fixtures/items/key_rsa.json"),
            Item::Key(SshKey {
                name: "legacy-rsa".into(),
                algorithm: KeyAlgorithm::Rsa,
                private_key: Zeroizing::new(
                    "-----BEGIN OPENSSH PRIVATE KEY-----\nRSA\n-----END OPENSSH PRIVATE KEY-----"
                        .into(),
                ),
                passphrase: None,
                public_key: "ssh-rsa AAAATest".into(),
                fingerprint: "SHA256:rsa123456".into(),
                comment: "legacy".into(),
                created_at: 1_700_000_000_000,
                updated_at: 1_700_000_000_008,
            }),
        ),
        (
            "known_host.json",
            include_str!("../../tests/fixtures/items/known_host.json"),
            Item::KnownHost(KnownHost {
                host: "10.0.0.7".into(),
                port: 22,
                key_type: "ssh-ed25519".into(),
                public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITestHost".into(),
                fingerprint: "SHA256:host123456".into(),
                first_seen_at: 1_700_000_000_000,
                updated_at: 1_700_000_000_009,
            }),
        ),
        (
            "forward_local.json",
            include_str!("../../tests/fixtures/items/forward_local.json"),
            Item::Forward(PortForward {
                host_id: "host-1".into(),
                name: "Postgres".into(),
                kind: ForwardKind::Local,
                bind_address: "127.0.0.1".into(),
                bind_port: 15432,
                dest_host: Some("localhost".into()),
                dest_port: Some(5432),
                auto_start: true,
                updated_at: 1_700_000_000_010,
            }),
        ),
        (
            "forward_remote.json",
            include_str!("../../tests/fixtures/items/forward_remote.json"),
            Item::Forward(PortForward {
                host_id: "host-1".into(),
                name: "Webhook".into(),
                kind: ForwardKind::Remote,
                bind_address: "0.0.0.0".into(),
                bind_port: 8080,
                dest_host: Some("127.0.0.1".into()),
                dest_port: Some(3000),
                auto_start: false,
                updated_at: 1_700_000_000_011,
            }),
        ),
        (
            "forward_dynamic.json",
            include_str!("../../tests/fixtures/items/forward_dynamic.json"),
            Item::Forward(PortForward {
                host_id: "host-1".into(),
                name: "SOCKS".into(),
                kind: ForwardKind::Dynamic,
                bind_address: "127.0.0.1".into(),
                bind_port: 1080,
                dest_host: None,
                dest_port: None,
                auto_start: false,
                updated_at: 1_700_000_000_012,
            }),
        ),
        (
            "snippet.json",
            include_str!("../../tests/fixtures/items/snippet.json"),
            Item::Snippet(Snippet {
                name: "Check disk".into(),
                command: "df -h".into(),
                tags: vec!["linux".into(), "disk".into()],
                updated_at: 1_700_000_000_013,
            }),
        ),
        (
            "proxy_socks5.json",
            include_str!("../../tests/fixtures/items/proxy_socks5.json"),
            Item::Proxy(Proxy {
                name: "Office SOCKS".into(),
                kind: ProxyKind::Socks5,
                address: "socks.example.org".into(),
                port: 1080,
                username: "alice".into(),
                password: Zeroizing::new("proxy-password".into()),
                updated_at: 1_700_000_000_014,
            }),
        ),
        (
            "proxy_http.json",
            include_str!("../../tests/fixtures/items/proxy_http.json"),
            Item::Proxy(Proxy {
                name: "Corporate HTTP".into(),
                kind: ProxyKind::Http,
                address: "proxy.example.org".into(),
                port: 3128,
                username: String::new(),
                password: Zeroizing::default(),
                updated_at: 1_700_000_000_015,
            }),
        ),
        (
            "ai_provider_anthropic.json",
            include_str!("../../tests/fixtures/items/ai_provider_anthropic.json"),
            Item::AiProvider(AiProvider {
                name: "Anthropic".into(),
                protocol: AiProtocol::Anthropic,
                base_url: "https://api.anthropic.com".into(),
                api_key: Zeroizing::new("sk-ant-test".into()),
                auth_header: AiAuthHeader::Authorization,
                models: vec![AiModel {
                    id: "claude-x".into(),
                    name: "Claude X".into(),
                    context_window: Some(200_000),
                    max_output_tokens: Some(16_384),
                    efforts: Some(AiEffort::ALL.to_vec()),
                    adaptive_thinking: Some(true),
                }],
                chatgpt: None,
                updated_at: 1_700_000_000_016,
            }),
        ),
        (
            "ai_provider_responses.json",
            include_str!("../../tests/fixtures/items/ai_provider_responses.json"),
            Item::AiProvider(AiProvider {
                name: "ChatGPT".into(),
                protocol: AiProtocol::Responses,
                base_url: "https://api.openai.com/v1".into(),
                api_key: Zeroizing::default(),
                auth_header: AiAuthHeader::XApiKey,
                models: Vec::new(),
                chatgpt: Some(AiChatgptAccount {
                    client_id: "oaiapp_1".into(),
                    subject: "user-1".into(),
                    email: "user@example.com".into(),
                }),
                updated_at: 1_700_000_000_017,
            }),
        ),
        (
            "ai_provider_system_one.json",
            include_str!("../../tests/fixtures/items/ai_provider_system_one.json"),
            Item::AiProvider(AiProvider {
                name: "System One".into(),
                protocol: AiProtocol::SystemOne,
                base_url: "https://api.typesafe.ai/v1".into(),
                api_key: Zeroizing::new("sys1-key".into()),
                auth_header: AiAuthHeader::XApiKey,
                models: vec![AiModel {
                    id: "jev-latest".into(),
                    name: "Jev Latest".into(),
                    ..AiModel::default()
                }],
                chatgpt: None,
                updated_at: 1_700_000_000_018,
            }),
        ),
        (
            "ai_provider_chat_completions.json",
            include_str!("../../tests/fixtures/items/ai_provider_chat_completions.json"),
            Item::AiProvider(AiProvider {
                name: "OpenAI Compat".into(),
                protocol: AiProtocol::ChatCompletions,
                base_url: "https://api.openai.com/v1".into(),
                api_key: Zeroizing::new("sk-openai-test".into()),
                auth_header: AiAuthHeader::XApiKey,
                models: Vec::new(),
                chatgpt: None,
                updated_at: 1_700_000_000_019,
            }),
        ),
        (
            "search_provider_brave.json",
            include_str!("../../tests/fixtures/items/search_provider_brave.json"),
            Item::SearchProvider(SearchProvider {
                kind: SearchKind::Brave,
                base_url: None,
                api_key: Zeroizing::new("brave-key".into()),
                updated_at: 1_700_000_000_020,
            }),
        ),
        (
            "search_provider_tavily.json",
            include_str!("../../tests/fixtures/items/search_provider_tavily.json"),
            Item::SearchProvider(SearchProvider {
                kind: SearchKind::Tavily,
                base_url: None,
                api_key: Zeroizing::new("tavily-key".into()),
                updated_at: 1_700_000_000_021,
            }),
        ),
        (
            "search_provider_searxng.json",
            include_str!("../../tests/fixtures/items/search_provider_searxng.json"),
            Item::SearchProvider(SearchProvider {
                kind: SearchKind::Searxng,
                base_url: Some("https://searx.example.org".into()),
                api_key: Zeroizing::default(),
                updated_at: 1_700_000_000_022,
            }),
        ),
        (
            "ai_conversation.json",
            include_str!("../../tests/fixtures/items/ai_conversation.json"),
            Item::AiConversation(AiConversation {
                title: "Why is nginx down".into(),
                host_id: Some("host-1".into()),
                quick_target: Some(AiQuickTarget::new("root", "[FE80::1]", 2222)),
                origin: Some("prod".into()),
                local: Some("workstation".into()),
                pinned: true,
                context_start: Some("entry-1".into()),
                effort: Some(AiEffort::Xhigh),
                created_at: 1_700_000_000_000,
                updated_at: 1_700_000_000_023,
            }),
        ),
        (
            "ai_message.json",
            include_str!("../../tests/fixtures/items/ai_message.json"),
            Item::AiMessage(AiMessage {
                conversation_id: "conv-1".into(),
                entry_id: "entry-1".into(),
                part: 0,
                part_count: 1,
                data: r#"{"role":"user","content":"hello"}"#.into(),
                updated_at: 1_700_000_000_024,
            }),
        ),
        (
            "skill.json",
            include_str!("../../tests/fixtures/items/skill.json"),
            Item::Skill(Skill {
                name: "nginx-ops".into(),
                description: "Operate nginx".into(),
                frontmatter,
                enabled: true,
                updated_at: 1_700_000_000_025,
            }),
        ),
        (
            "skill_file.json",
            include_str!("../../tests/fixtures/items/skill_file.json"),
            Item::SkillFile(SkillFile {
                skill_id: "skill-1".into(),
                path: "references/nginx.md".into(),
                content: "# nginx".into(),
                updated_at: 1_700_000_000_026,
            }),
        ),
        (
            "mcp_server_stdio.json",
            include_str!("../../tests/fixtures/items/mcp_server_stdio.json"),
            Item::McpServer(McpServer {
                name: "fs".into(),
                transport: McpTransport::Stdio {
                    command: "npx".into(),
                    args: vec!["-y".into(), "server-fs".into()],
                    env: BTreeMap::from([("TOKEN".to_owned(), Zeroizing::new("t0k".to_owned()))]),
                },
                always_ask: true,
                updated_at: 1_700_000_000_027,
            }),
        ),
        (
            "mcp_server_http.json",
            include_str!("../../tests/fixtures/items/mcp_server_http.json"),
            Item::McpServer(McpServer {
                name: "remote".into(),
                transport: McpTransport::Http {
                    url: "https://mcp.example.org/mcp".into(),
                    headers: BTreeMap::from([(
                        "Authorization".to_owned(),
                        Zeroizing::new("Bearer secret".to_owned()),
                    )]),
                },
                always_ask: false,
                updated_at: 1_700_000_000_028,
            }),
        ),
        (
            "ai_memory.json",
            include_str!("../../tests/fixtures/items/ai_memory.json"),
            Item::AiMemory(AiMemory {
                name: "sudo".into(),
                content: "Uses doas, not sudo.".into(),
                host_id: Some("host-1".into()),
                local: Some("workstation".into()),
                updated_at: 1_700_000_000_029,
            }),
        ),
        (
            "settings_dark_bar_menu.json",
            include_str!("../../tests/fixtures/items/settings_dark_bar_menu.json"),
            Item::Settings(Settings {
                terminal: TerminalSettings {
                    font_family: "Cascadia Mono".into(),
                    font_size: 13,
                    theme: ThemeMode::Dark,
                    dark_scheme: Some("dracula".into()),
                    light_scheme: Some("github-light".into()),
                    cursor_style: CursorStyle::Bar,
                    scrollback: 10_000,
                    right_click: Some(RightClick::Menu),
                    confirm_multiline_paste: Some(true),
                    suggest_commands: Some(false),
                },
                ai: AiSettings {
                    default_model: Some(AiModelRef {
                        provider_id: "p1".into(),
                        model_id: "m1".into(),
                    }),
                    default_effort: Some(AiEffort::High),
                    search_provider_id: Some("sp1".into()),
                    native_search: true,
                    builtin_skill_enabled: true,
                    custom_instructions: "Answer in English.".into(),
                    send_terminal_screen: true,
                    judge_model: Some(AiModelRef {
                        provider_id: "p2".into(),
                        model_id: "jev-latest".into(),
                    }),
                    memory: true,
                    text_size: 13,
                },
                updated_at: 1_700_000_000_030,
            }),
        ),
        (
            "settings_light_underline_copypaste.json",
            include_str!("../../tests/fixtures/items/settings_light_underline_copypaste.json"),
            Item::Settings(Settings {
                terminal: TerminalSettings {
                    font_family: "Menlo".into(),
                    font_size: 14,
                    theme: ThemeMode::Light,
                    dark_scheme: None,
                    light_scheme: None,
                    cursor_style: CursorStyle::Underline,
                    scrollback: 5_000,
                    right_click: Some(RightClick::CopyPaste),
                    confirm_multiline_paste: None,
                    suggest_commands: None,
                },
                ai: AiSettings::default(),
                updated_at: 1_700_000_000_031,
            }),
        ),
        (
            "settings_system_block.json",
            include_str!("../../tests/fixtures/items/settings_system_block.json"),
            Item::Settings(Settings {
                terminal: TerminalSettings {
                    font_family: "Cascadia Mono".into(),
                    font_size: 13,
                    theme: ThemeMode::System,
                    dark_scheme: None,
                    light_scheme: None,
                    cursor_style: CursorStyle::Block,
                    scrollback: 10_000,
                    right_click: None,
                    confirm_multiline_paste: None,
                    suggest_commands: None,
                },
                ai: AiSettings::default(),
                updated_at: 1_700_000_000_032,
            }),
        ),
    ];

    let mut seen_items = std::collections::BTreeSet::new();
    let mut seen_auth = std::collections::BTreeSet::new();
    let mut seen_proxy_choice = std::collections::BTreeSet::new();
    let mut seen_proxy_kind = std::collections::BTreeSet::new();
    let mut seen_key_alg = std::collections::BTreeSet::new();
    let mut seen_fwd_kind = std::collections::BTreeSet::new();
    let mut seen_protocol = std::collections::BTreeSet::new();
    let mut seen_auth_header = std::collections::BTreeSet::new();
    let mut seen_effort = std::collections::BTreeSet::new();
    let mut seen_search = std::collections::BTreeSet::new();
    let mut seen_mcp = std::collections::BTreeSet::new();
    let mut seen_theme = std::collections::BTreeSet::new();
    let mut seen_cursor = std::collections::BTreeSet::new();
    let mut seen_right_click = std::collections::BTreeSet::new();

    for (name, raw, item) in cases {
        let expected: serde_json::Value =
            serde_json::from_str(raw).unwrap_or_else(|e| panic!("{name}: {e}"));
        let actual = serde_json::to_value(&item).unwrap();
        assert_eq!(actual, expected, "{name}");
        let back = Item::from_plaintext(raw.as_bytes()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(back, item, "{name}");

        seen_items.insert(item.kind_str());
        match &item {
            Item::Host(h) => {
                seen_auth.insert(match &h.auth {
                    HostAuth::Password { .. } => "password",
                    HostAuth::Key { .. } => "key",
                    HostAuth::Agent => "agent",
                    HostAuth::Ask => "ask",
                });
                seen_proxy_choice.insert(match &h.proxy {
                    HostProxy::DeviceDefault => "device_default",
                    HostProxy::Direct => "direct",
                    HostProxy::Proxy { .. } => "proxy",
                });
            }
            Item::TelnetHost(h) => {
                seen_proxy_choice.insert(match &h.proxy {
                    HostProxy::DeviceDefault => "device_default",
                    HostProxy::Direct => "direct",
                    HostProxy::Proxy { .. } => "proxy",
                });
            }
            Item::Key(k) => {
                seen_key_alg.insert(match k.algorithm {
                    KeyAlgorithm::Ed25519 => "ed25519",
                    KeyAlgorithm::Ecdsa => "ecdsa",
                    KeyAlgorithm::Rsa => "rsa",
                });
            }
            Item::Forward(f) => {
                seen_fwd_kind.insert(match f.kind {
                    ForwardKind::Local => "local",
                    ForwardKind::Remote => "remote",
                    ForwardKind::Dynamic => "dynamic",
                });
            }
            Item::Proxy(p) => {
                seen_proxy_kind.insert(p.kind.as_str());
            }
            Item::AiProvider(p) => {
                seen_protocol.insert(p.protocol.as_str());
                seen_auth_header.insert(p.auth_header.as_str());
                for m in &p.models {
                    if let Some(efforts) = &m.efforts {
                        for e in efforts {
                            seen_effort.insert(e.as_str());
                        }
                    }
                }
            }
            Item::SearchProvider(s) => {
                seen_search.insert(s.kind.as_str());
            }
            Item::McpServer(m) => {
                seen_mcp.insert(m.transport.kind_str());
            }
            Item::Settings(s) => {
                seen_theme.insert(match s.terminal.theme {
                    ThemeMode::Light => "light",
                    ThemeMode::Dark => "dark",
                    ThemeMode::System => "system",
                });
                seen_cursor.insert(match s.terminal.cursor_style {
                    CursorStyle::Bar => "bar",
                    CursorStyle::Underline => "underline",
                    CursorStyle::Block => "block",
                });
                if let Some(rc) = s.terminal.right_click {
                    seen_right_click.insert(match rc {
                        RightClick::Menu => "menu",
                        RightClick::CopyPaste => "copy_paste",
                    });
                }
            }
            Item::Group(_)
            | Item::KnownHost(_)
            | Item::Snippet(_)
            | Item::AiConversation(_)
            | Item::AiMessage(_)
            | Item::Skill(_)
            | Item::SkillFile(_)
            | Item::AiMemory(_) => {}
        }
    }

    assert_eq!(seen_items.len(), 17);
    assert_eq!(seen_auth.len(), 4);
    assert_eq!(seen_proxy_choice.len(), 3);
    assert_eq!(seen_proxy_kind.len(), 2);
    assert_eq!(seen_key_alg.len(), 3);
    assert_eq!(seen_fwd_kind.len(), 3);
    assert_eq!(seen_protocol.len(), 4);
    assert_eq!(seen_auth_header.len(), 2);
    assert_eq!(seen_effort.len(), 5);
    assert_eq!(seen_search.len(), 3);
    assert_eq!(seen_mcp.len(), 2);
    assert_eq!(seen_theme.len(), 3);
    assert_eq!(seen_cursor.len(), 3);
    assert_eq!(seen_right_click.len(), 2);
}
