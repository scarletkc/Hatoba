use hatoba_assistant::ai::memory;
use hatoba_core::KdfParams;
use hatoba_core::model::{Group, HostProxy, KeyAlgorithm, SshKey, TelnetHost};
use zeroize::Zeroizing;

use super::*;

fn vault() -> Vault {
    let mut vault = Vault::open_in_memory().unwrap();
    vault
        .create_with_params(&hatoba_core::new_id(), KdfParams::for_tests())
        .unwrap();
    vault
}

fn host(name: &str, auth: HostAuth) -> Host {
    Host {
        name: name.to_owned(),
        address: format!("{}.internal", name.to_lowercase().replace(' ', "-")),
        port: 22,
        username: "deploy".to_owned(),
        auth,
        proxy: HostProxy::Direct,
        ..Host::default()
    }
}

#[test]
fn hosts_are_found_by_id_or_by_name() {
    let mut v = vault();
    let db = v
        .put(None, Item::Host(host("db", HostAuth::Agent)))
        .unwrap();
    let web = v
        .put(None, Item::Host(host("My Server", HostAuth::Agent)))
        .unwrap();
    assert_eq!(find_host(&v, &db).unwrap().0, db);
    assert_eq!(find_host(&v, "db").unwrap().0, db);
    // As `hatoba <name>` finds it: without case, and around spaces.
    assert_eq!(find_host(&v, " my server ").unwrap().0, web);

    let missing = find_host(&v, "nope").unwrap_err();
    assert!(missing.contains("hatoba hosts"), "{missing}");

    let twin = v
        .put(None, Item::Host(host("DB", HostAuth::Agent)))
        .unwrap();
    let both = find_host(&v, "db").unwrap_err();
    assert!(both.contains(&db) && both.contains(&twin), "{both}");
    assert_eq!(find_host(&v, &twin).unwrap().0, twin);
}

#[test]
fn a_telnet_host_is_left_out_and_named_as_one() {
    let mut v = vault();
    let switch = v
        .put(
            None,
            Item::TelnetHost(TelnetHost {
                name: "Core Switch".to_owned(),
                address: "10.0.0.2".to_owned(),
                ..TelnetHost::default()
            }),
        )
        .unwrap();
    assert!(host_list(&v).is_empty());
    for wanted in [switch.as_str(), "core switch"] {
        let error = find_host(&v, wanted).unwrap_err();
        assert!(
            error.contains("is a Telnet host") && error.contains("Run hatoba Core Switch to open"),
            "{error}"
        );
    }
    // An SSH host of the same name is the one a command means.
    let ssh = v
        .put(None, Item::Host(host("core switch", HostAuth::Agent)))
        .unwrap();
    assert_eq!(find_host(&v, "Core Switch").unwrap().0, ssh);
}

#[test]
fn the_host_list_names_groups_and_jump_hosts_and_holds_no_secret() {
    let mut v = vault();
    let key = v
        .put(
            None,
            Item::Key(SshKey {
                name: "deploy key".into(),
                algorithm: KeyAlgorithm::Ed25519,
                private_key: Zeroizing::new("PRIVATE-KEY-SECRET".into()),
                passphrase: Some(Zeroizing::new("PASSPHRASE-SECRET".into())),
                public_key: "ssh-ed25519 AAAA".into(),
                fingerprint: "SHA256:x".into(),
                comment: String::new(),
                created_at: 0,
                updated_at: 0,
            }),
        )
        .unwrap();
    let group = v
        .put(
            None,
            Item::Group(Group {
                name: "Production".into(),
                ..Group::default()
            }),
        )
        .unwrap();
    let password = HostAuth::Password {
        password: Zeroizing::new("PASSWORD-SECRET".into()),
    };
    let bastion = v.put(None, Item::Host(host("bastion", password))).unwrap();
    let mut app = host("app", HostAuth::Key { key_id: key });
    app.group_id = Some(group);
    app.jump_host_id = Some(bastion);
    app.ai_notes = "Logs are in /var/log/app.".into();
    v.put(None, Item::Host(app)).unwrap();
    let empty = HostAuth::Password {
        password: Zeroizing::new(String::new()),
    };
    v.put(None, Item::Host(host("Zeta", empty))).unwrap();

    let hosts = host_list(&v);
    let names: Vec<&str> = hosts.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(names, ["app", "bastion", "Zeta"]);
    assert_eq!(hosts[0].auth, AuthKind::Key);
    assert_eq!(hosts[0].group.as_deref(), Some("Production"));
    assert_eq!(hosts[0].jump.as_deref(), Some("bastion"));
    assert_eq!(hosts[0].ai_notes, "Logs are in /var/log/app.");
    assert_eq!(hosts[1].auth, AuthKind::Password);
    // A password that is not saved is asked for.
    assert_eq!(hosts[2].auth, AuthKind::Ask);

    let json = serde_json::to_string(&hosts).unwrap();
    for secret in ["PRIVATE-KEY-SECRET", "PASSPHRASE-SECRET", "PASSWORD-SECRET"] {
        assert!(!json.contains(secret), "{json}");
    }
}

#[test]
fn a_missing_password_says_how_to_give_it() {
    let mut v = vault();
    let id = v.put(None, Item::Host(host("db", HostAuth::Ask))).unwrap();
    let db = v.get(&id).and_then(Item::as_host).cloned().unwrap();
    let error = ssh::build_config(&v, &db, Some(&id), Some(&Overrides::default()))
        .err()
        .unwrap();
    let message = config_error(&error, "db");
    assert!(message.starts_with("db has no saved password") && message.contains("--no-prompt"));
    // Asked for in the window instead.
    let asking = Overrides {
        ask_password: true,
        ..Overrides::default()
    };
    let cfg = ssh::build_config(&v, &db, Some(&id), Some(&asking)).unwrap();
    assert!(matches!(cfg.auth, hatoba_ssh::AuthMethod::Ask));

    assert_eq!(config_error(&AppError::locked(), "db"), LOCKED);
}

/// HOST-15: `hatoba memory` saves, lists and deletes under the `memory` tool's rules, among the
/// global memories or a saved SSH host's.
#[test]
fn memory_commands_follow_the_memory_tools_rules() {
    use hatoba_ai::tools::MemoryAction::{Delete, Save};
    use hatoba_core::model::SETTINGS_ID;

    let mut v = vault();
    let db = v
        .put(None, Item::Host(host("db", HostAuth::Agent)))
        .unwrap();
    v.put(
        None,
        Item::TelnetHost(TelnetHost {
            name: "Core Switch".to_owned(),
            address: "10.0.0.2".to_owned(),
            ..TelnetHost::default()
        }),
    )
    .unwrap();

    // Nothing yet: the listing says so beside an empty stdout, and JSON is an empty list.
    assert_eq!(
        memories::list(&v, None, false).unwrap(),
        memories::Listing {
            text: String::new(),
            notice: Some("no global memories are saved in Hatoba yet".into()),
        }
    );
    assert_eq!(
        memories::list(&v, Some("db"), false)
            .unwrap()
            .notice
            .as_deref(),
        Some("no global memories and no memories of db are saved in Hatoba yet")
    );
    assert_eq!(
        memories::list(&v, Some("db"), true).unwrap(),
        memories::Listing {
            text: "[]\n".into(),
            notice: None,
        }
    );

    let mut change = |host: Option<&str>, action, name: &str, content: Option<&str>| {
        memories::change(
            &mut v,
            host,
            action,
            name.to_owned(),
            content.map(str::to_owned),
        )
    };
    assert_eq!(
        change(None, Save, "reply language", Some("Chinese.\n")).unwrap(),
        "Saved the global memory \"reply language\"."
    );
    // A host is named by its name or its id, as for exec.
    assert_eq!(
        change(Some("DB"), Save, "sudo use", Some("Uses doas.")).unwrap(),
        "Saved the host memory \"sudo use\" of db."
    );
    // A save under a name that exists replaces that memory.
    change(Some(&db), Save, "sudo use", Some("Uses doas,\n\nnot sudo.")).unwrap();
    assert_eq!(
        change(None, Save, "empty", Some(" ")).unwrap_err(),
        "The memory is empty. To remove a memory, delete it."
    );
    assert!(
        change(Some("nope"), Save, "x", Some("y"))
            .unwrap_err()
            .contains("no saved host is named nope")
    );
    assert!(
        change(Some("core switch"), Save, "x", Some("y"))
            .unwrap_err()
            .contains("is a Telnet host")
    );
    assert_eq!(
        change(None, Delete, "sudo use", None).unwrap_err(),
        "There is no global memory named \"sudo use\". The global memories: \"reply language\"."
    );

    let stored: Vec<(String, Option<String>)> = v
        .ai_memories()
        .into_iter()
        .map(|(_, m)| (m.name, m.host_id))
        .collect();
    assert_eq!(
        stored,
        [
            ("reply language".into(), None),
            ("sudo use".into(), Some(db.clone()))
        ]
    );

    // Without a host, the global ones; with it, the host's after them.
    let day = memory::listed(&v, None)[0].updated.clone();
    assert_eq!(day.len(), 10, "{day}");
    assert_eq!(
        memories::list(&v, None, false).unwrap(),
        memories::Listing {
            text: format!("reply language (global, {day})\n  Chinese.\n"),
            notice: None,
        }
    );
    assert_eq!(
        memories::list(&v, Some("db"), false).unwrap().text,
        format!(
            "reply language (global, {day})\n  Chinese.\n\nsudo use (host db, {day})\n  Uses \
             doas,\n\n  not sudo.\n"
        )
    );
    let json: serde_json::Value =
        serde_json::from_str(&memories::list(&v, Some("db"), true).unwrap().text).unwrap();
    assert_eq!(
        json,
        serde_json::json!([
            {
                "name": "reply language",
                "scope": "global",
                "updated": day,
                "content": "Chinese.",
            },
            {
                "name": "sudo use",
                "scope": "host",
                "host": "db",
                "updated": day,
                "content": "Uses doas,\n\nnot sudo.",
            },
        ])
    );

    assert_eq!(
        memories::change(&mut v, Some("db"), Delete, "sudo use".into(), None).unwrap(),
        "Deleted the host memory \"sudo use\" of db."
    );
    assert_eq!(v.ai_memories().len(), 1);

    // While memory is off, nothing is listed or saved.
    let mut settings = v.settings();
    settings.ai.memory = false;
    v.put(Some(SETTINGS_ID), Item::Settings(settings)).unwrap();
    assert_eq!(memories::list(&v, None, true).unwrap_err(), memory::OFF);
    assert_eq!(
        memories::change(&mut v, None, Save, "x".into(), Some("y".into())).unwrap_err(),
        memory::OFF
    );
    assert_eq!(v.ai_memories().len(), 1);
}

#[test]
fn remote_paths_join_with_one_slash() {
    assert_eq!(join_remote("/home/me", "a.txt"), "/home/me/a.txt");
    assert_eq!(join_remote("/tmp/", "a.txt"), "/tmp/a.txt");
}

#[test]
fn a_sudo_password_is_kept_for_its_host_until_unused_for_15_minutes() {
    let cli = CliAccess::default();
    cli.keep_sudo("h1\nv1".into(), Zeroizing::new("pw".into()));
    assert_eq!(
        cli.kept_sudo("h1\nv1").as_deref().map(String::as_str),
        Some("pw")
    );
    // The host connects another way now.
    assert!(cli.kept_sudo("h1\nv2").is_none());
    lock(&cli.sudo).get_mut("h1\nv1").unwrap().used = SystemTime::now() - SUDO_KEPT;
    assert!(cli.kept_sudo("h1\nv1").is_none());
    assert_eq!(detail(true, "ls".into()), "sudo ls");
    assert_eq!(detail(false, "ls".into()), "ls");
}

#[test]
fn requests_for_one_host_find_its_sudo_password_one_at_a_time() {
    let cli = CliAccess::default();
    let first = cli.sudo_turn("h1\nv1").try_lock_owned().unwrap();
    assert!(cli.sudo_turn("h1\nv1").try_lock_owned().is_err());
    assert!(cli.sudo_turn("h2\nv1").try_lock_owned().is_ok());
    drop(first);
    assert!(cli.sudo_turn("h1\nv1").try_lock_owned().is_ok());
}
