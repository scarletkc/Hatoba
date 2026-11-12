use hatoba_assistant::ai::memory;
use hatoba_cli::protocol::{AuthKind, EnvVar};
use hatoba_core::KdfParams;
use hatoba_core::model::{Group, HostProxy, KeyAlgorithm, SshKey, TelnetHost};
use zeroize::Zeroizing;

use super::*;
use crate::dto;

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
    app.note = "Billing API.".into();
    app.favorite = true;
    app.proxy = HostProxy::DeviceDefault;
    app.env = vec![hatoba_core::model::EnvVar::new("TZ", "Asia/Tokyo")];
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
    // HOST-15: what `hosts edit` sets, so a program can read a host before it changes it.
    assert_eq!(hosts[0].key.as_deref(), Some("deploy key"));
    assert_eq!(hosts[0].note, "Billing API.");
    assert!(hosts[0].favorite);
    assert_eq!(hosts[0].proxy, "default");
    assert_eq!(
        (
            hosts[0].env[0].name.as_str(),
            hosts[0].env[0].value.as_str()
        ),
        ("TZ", "Asia/Tokyo")
    );
    assert_eq!(hosts[1].auth, AuthKind::Password);
    assert_eq!(
        (hosts[1].key.as_deref(), hosts[1].proxy.as_str()),
        (None, "direct")
    );
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

/// HOST-15: `hatoba memory` saves, lists and deletes a saved SSH host's memories under the
/// `memory` tool's rules, and leaves the global ones out.
#[test]
fn memory_commands_follow_the_memory_tools_rules() {
    use hatoba_ai::entry::ToolStatus;
    use hatoba_ai::tools::MemoryAction::{Delete, Save};
    use hatoba_ai::tools::{MemoryArgs, MemoryScope};
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
        memories::list(&v, "db", false).unwrap(),
        memories::Listing {
            text: String::new(),
            notice: Some("no memories of db are saved in Hatoba yet".into()),
        }
    );
    assert_eq!(
        memories::list(&v, "db", true).unwrap(),
        memories::Listing {
            text: "[]\n".into(),
            notice: None,
        }
    );

    // A global memory the assistant saved, which the commands never show.
    let global = MemoryArgs {
        action: Save,
        scope: MemoryScope::Global,
        name: "reply language".into(),
        content: Some("Chinese.".into()),
    };
    assert_eq!(memory::run(&mut v, None, &global).0, ToolStatus::Ok);

    let mut change = |host: &str, action, name: &str, content: Option<&str>| {
        memories::change(
            &mut v,
            host,
            action,
            name.to_owned(),
            content.map(str::to_owned),
        )
    };
    // A host is named by its name or its id, as for exec.
    assert_eq!(
        change("DB", Save, "sudo use", Some("Uses doas.")).unwrap(),
        "Saved the host memory \"sudo use\" of db."
    );
    // A save under a name that exists replaces that memory.
    change(&db, Save, "sudo use", Some("Uses doas,\n\nnot sudo.")).unwrap();
    assert_eq!(
        change("db", Save, "empty", Some(" ")).unwrap_err(),
        "The memory is empty. To remove a memory, delete it."
    );
    assert!(
        change("nope", Save, "x", Some("y"))
            .unwrap_err()
            .contains("no saved host is named nope")
    );
    assert!(
        change("core switch", Save, "x", Some("y"))
            .unwrap_err()
            .contains("is a Telnet host")
    );
    // A miss names the host's memories and says nothing of a global one of that name.
    assert_eq!(
        change("db", Delete, "reply language", None).unwrap_err(),
        "There is no host memory named \"reply language\" of db. The host memories of db: \
         \"sudo use\"."
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

    // Only the host's own memories, without the global one.
    let day = memory::listed(&v, &memory::Scope::Host(db.clone()))[0]
        .updated
        .clone();
    assert_eq!(day.len(), 10, "{day}");
    assert_eq!(
        memories::list(&v, "db", false).unwrap(),
        memories::Listing {
            text: format!("sudo use ({day})\n  Uses doas,\n\n  not sudo.\n"),
            notice: None,
        }
    );
    let json: serde_json::Value =
        serde_json::from_str(&memories::list(&v, "db", true).unwrap().text).unwrap();
    assert_eq!(
        json,
        serde_json::json!([
            {
                "name": "sudo use",
                "updated": day,
                "content": "Uses doas,\n\nnot sudo.",
            },
        ])
    );

    assert_eq!(
        memories::change(&mut v, "db", Delete, "sudo use".into(), None).unwrap(),
        "Deleted the host memory \"sudo use\" of db."
    );
    assert_eq!(v.ai_memories().len(), 1);

    // While memory is off, nothing is listed or saved.
    let mut settings = v.settings();
    settings.ai.memory = false;
    v.put(Some(SETTINGS_ID), Item::Settings(settings)).unwrap();
    assert_eq!(memories::list(&v, "db", true).unwrap_err(), memory::OFF);
    assert_eq!(
        memories::change(&mut v, "db", Save, "x".into(), Some("y".into())).unwrap_err(),
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

fn ssh_key(name: &str) -> Item {
    Item::Key(SshKey {
        name: name.into(),
        algorithm: KeyAlgorithm::Ed25519,
        private_key: Zeroizing::new("PRIVATE-KEY-SECRET".into()),
        passphrase: None,
        public_key: "ssh-ed25519 AAAA".into(),
        fingerprint: "SHA256:x".into(),
        comment: String::new(),
        created_at: 0,
        updated_at: 0,
    })
}

/// `hatoba hosts add` or `edit`, planned, checked and saved as `run_change_host` does.
fn change(v: &mut Vault, edited: Option<&str>, changes: &HostChanges) -> Result<String, String> {
    let host = edited.and_then(|id| v.get(id).and_then(Item::as_host).cloned());
    let plan = hosts::plan(v, edited.zip(host.as_ref()), changes, "me")?;
    hosts::check(v, &plan)?;
    hosts::save(v, plan)
}

fn stored(v: &Vault, id: &str) -> Host {
    v.get(id).and_then(Item::as_host).cloned().unwrap()
}

/// HOST-15: `hosts add` saves a host as **New Host** does, with the names it is given found or,
/// for a group, made.
#[test]
fn hosts_add_saves_a_host_with_what_its_options_name() {
    let mut v = vault();
    let ci = v.put(None, ssh_key("ci")).unwrap();
    let bastion = v
        .put(None, Item::Host(host("bastion", HostAuth::Agent)))
        .unwrap();
    let changes = HostChanges {
        name: Some("web-3".into()),
        address: Some("10.0.0.7".into()),
        username: Some("deploy".into()),
        auth: Some(AuthKind::Key),
        key: Some("CI".into()),
        jump: Some("bastion".into()),
        group: Some("prod".into()),
        tags: vec!["tokyo".into()],
        ..HostChanges::default()
    };
    let id = change(&mut v, None, &changes).unwrap();
    let web = stored(&v, &id);
    assert_eq!(
        (
            web.name.as_str(),
            web.address.as_str(),
            web.port,
            web.username.as_str()
        ),
        ("web-3", "10.0.0.7", 22, "deploy")
    );
    assert_eq!(web.auth, HostAuth::Key { key_id: ci });
    assert_eq!(web.jump_host_id.as_deref(), Some(bastion.as_str()));
    assert_eq!(web.tags, ["tokyo"]);
    // A name no group had made the group, and a second host joins it.
    let groups = v.groups();
    assert_eq!(groups.len(), 1);
    assert_eq!(web.group_id.as_deref(), Some(groups[0].0.as_str()));
    let other = HostChanges {
        address: Some("nas.local".into()),
        group: Some("Prod".into()),
        ..HostChanges::default()
    };
    let nas = change(&mut v, None, &other).unwrap();
    let nas = stored(&v, &nas);
    assert_eq!(v.groups().len(), 1);
    assert_eq!(nas.group_id, web.group_id);
    // As the editor names a host without a name, with this computer's user, and with a password
    // to ask for, as New Host starts.
    assert_eq!(
        (nas.name.as_str(), nas.username.as_str()),
        ("nas.local", "me")
    );
    assert!(matches!(&nas.auth, HostAuth::Password { password } if password.is_empty()));

    let info = host_list(&v);
    let web = info.iter().find(|h| h.name == "web-3").unwrap();
    assert_eq!(
        (
            web.key.as_deref(),
            web.group.as_deref(),
            web.jump.as_deref()
        ),
        (Some("ci"), Some("prod"), Some("bastion"))
    );
}

/// HOST-15: `hosts edit` changes the fields its options name, and the rest, the saved password
/// included, stays as it was (HOST-08).
#[test]
fn hosts_edit_changes_only_what_its_options_name() {
    let mut v = vault();
    let bastion = v
        .put(None, Item::Host(host("bastion", HostAuth::Agent)))
        .unwrap();
    let group = v
        .put(
            None,
            Item::Group(Group {
                name: "prod".into(),
                ..Group::default()
            }),
        )
        .unwrap();
    let mut web = host(
        "web-3",
        HostAuth::Password {
            password: Zeroizing::new("PASSWORD-SECRET".into()),
        },
    );
    web.group_id = Some(group);
    web.jump_host_id = Some(bastion);
    web.tags = vec!["tokyo".into(), "web".into()];
    web.env = vec![
        hatoba_core::model::EnvVar::new("TZ", "UTC"),
        hatoba_core::model::EnvVar::new("LANG", "C"),
    ];
    web.note = "Front end.".into();
    web.favorite = true;
    web.sort = 7;
    let id = v.put(None, Item::Host(web)).unwrap();
    let before = stored(&v, &id);

    let changes = HostChanges {
        port: Some(2222),
        env: vec![EnvVar {
            name: "TZ".into(),
            value: "Asia/Tokyo".into(),
        }],
        ai_notes: Some("Staging; restart with systemctl".into()),
        ..HostChanges::default()
    };
    assert_eq!(change(&mut v, Some(&id), &changes).unwrap(), id);
    let after = stored(&v, &id);
    assert_eq!(after.port, 2222);
    assert_eq!(
        after.env,
        [
            hatoba_core::model::EnvVar::new("TZ", "Asia/Tokyo"),
            hatoba_core::model::EnvVar::new("LANG", "C")
        ]
    );
    assert_eq!(after.ai_notes, "Staging; restart with systemctl");
    let unchanged = Host {
        port: before.port,
        env: before.env.clone(),
        ai_notes: before.ai_notes.clone(),
        updated_at: before.updated_at,
        ..after.clone()
    };
    assert_eq!(unchanged, before);

    // Tags and variables go by name, and an empty text clears a note.
    let changes = HostChanges {
        untags: vec!["tokyo".into()],
        tags: vec!["osaka".into(), "web".into()],
        unset_env: vec!["LANG".into()],
        note: Some(String::new()),
        no_jump: true,
        no_group: true,
        favorite: Some(false),
        ..HostChanges::default()
    };
    change(&mut v, Some(&id), &changes).unwrap();
    let after = stored(&v, &id);
    assert_eq!(after.tags, ["web", "osaka"]);
    assert_eq!(
        after.env,
        [hatoba_core::model::EnvVar::new("TZ", "Asia/Tokyo")]
    );
    assert_eq!(
        (
            after.note.as_str(),
            after.jump_host_id,
            after.group_id,
            after.favorite
        ),
        ("", None, None, false)
    );
    assert_eq!(after.auth, before.auth, "the saved password stays");
}

/// HOST-15: `--auth password` without a password asks for one, and one given replaces the saved
/// one; the password is never in what Settings lists.
#[test]
fn a_password_is_asked_for_only_when_auth_password_brings_none() {
    let mut v = vault();
    let id = v
        .put(None, Item::Host(host("nas", HostAuth::Agent)))
        .unwrap();
    let edited = stored(&v, &id);
    let ask = HostChanges {
        auth: Some(AuthKind::Password),
        ..HostChanges::default()
    };
    let plan = hosts::plan(&v, Some((&id, &edited)), &ask, "me").unwrap();
    assert!(plan.needs_password);
    assert_eq!(plan.name(), "nas");
    let given = HostChanges {
        password: Some("PASSWORD-SECRET".into()),
        ..ask.clone()
    };
    assert!(
        !hosts::plan(&v, Some((&id, &edited)), &given, "me")
            .unwrap()
            .needs_password
    );
    change(&mut v, Some(&id), &given).unwrap();
    assert!(matches!(
        &stored(&v, &id).auth,
        HostAuth::Password { password } if password.as_str() == "PASSWORD-SECRET"
    ));
    // Leaving --auth alone asks nothing.
    let port = HostChanges {
        port: Some(2200),
        ..HostChanges::default()
    };
    let edited = stored(&v, &id);
    assert!(
        !hosts::plan(&v, Some((&id, &edited)), &port, "me")
            .unwrap()
            .needs_password
    );

    let listed = hosts::describe("edit", &given);
    assert_eq!(listed, "hosts edit --password-stdin");
    assert!(!listed.contains("SECRET"));
    let add = HostChanges {
        address: Some("fe80::1".into()),
        username: Some("root".into()),
        port: Some(2222),
        name: Some("lab box".into()),
        auth: Some(AuthKind::Key),
        key: Some("ci".into()),
        ai_notes: Some("x".repeat(50)),
        ..HostChanges::default()
    };
    assert_eq!(
        hosts::describe("add", &add),
        format!(
            "hosts add root@[fe80::1]:2222 --name \"lab box\" --key ci --ai-notes {}…",
            "x".repeat(40)
        )
    );
}

/// HOST-15: a change the host editor refuses fails with its message, and nothing changes, not
/// even the group the change would have made.
#[test]
fn a_change_the_editor_refuses_changes_nothing() {
    let mut v = vault();
    let bastion = v
        .put(None, Item::Host(host("bastion", HostAuth::Agent)))
        .unwrap();
    let mut web = host("web-3", HostAuth::Agent);
    web.jump_host_id = Some(bastion.clone());
    let web = v.put(None, Item::Host(web)).unwrap();
    let before = (stored(&v, &web), stored(&v, &bastion));

    let refused =
        |v: &mut Vault, id: &str, changes: HostChanges| change(v, Some(id), &changes).unwrap_err();
    let port = HostChanges {
        port: Some(0),
        group: Some("new".into()),
        ..HostChanges::default()
    };
    assert_eq!(
        refused(&mut v, &web, port),
        "port must be between 1 and 65535"
    );
    let own = HostChanges {
        jump: Some("web-3".into()),
        ..HostChanges::default()
    };
    assert_eq!(
        refused(&mut v, &web, own),
        "a host can't be its own jump host"
    );
    // bastion → web-3 → bastion.
    let back = HostChanges {
        jump: Some("web-3".into()),
        ..HostChanges::default()
    };
    assert_eq!(
        refused(&mut v, &bastion, back),
        "the jump host chain can't loop back on itself"
    );
    let keyless = HostChanges {
        auth: Some(AuthKind::Key),
        ..HostChanges::default()
    };
    assert_eq!(refused(&mut v, &web, keyless), "choose a key");
    let env = HostChanges {
        env: vec![EnvVar {
            name: "1BAD".into(),
            value: "x".into(),
        }],
        ..HostChanges::default()
    };
    refused(&mut v, &web, env);
    assert_eq!((stored(&v, &web), stored(&v, &bastion)), before);

    // Saved without the check, the host fails, and the group it made goes with it.
    let bad = HostChanges {
        address: Some("10.0.0.9".into()),
        port: Some(0),
        group: Some("new".into()),
        ..HostChanges::default()
    };
    let plan = hosts::plan(&v, None, &bad, "me").unwrap();
    assert!(hosts::save(&mut v, plan).is_err());
    assert!(v.groups().is_empty());
    assert_eq!(v.hosts().len(), 2);
}

/// HOST-15: a key, a proxy, a group and a jump host are named by their id or their name, and a
/// name several share, or none has, fails and says so.
#[test]
fn names_that_fit_several_or_none_fail() {
    let mut v = vault();
    let first = v.put(None, ssh_key("ci")).unwrap();
    let second = v.put(None, ssh_key("CI")).unwrap();
    let office = v
        .put(
            None,
            Item::Proxy(hatoba_core::model::Proxy {
                name: "Office".into(),
                address: "10.0.0.1".into(),
                port: 1080,
                ..hatoba_core::model::Proxy::default()
            }),
        )
        .unwrap();
    let id = v
        .put(None, Item::Host(host("web", HostAuth::Agent)))
        .unwrap();
    let edited = stored(&v, &id);
    let plan =
        |v: &Vault, changes: HostChanges| hosts::plan(v, Some((&id, &edited)), &changes, "me");

    let both = plan(
        &v,
        HostChanges {
            key: Some("ci".into()),
            ..HostChanges::default()
        },
    )
    .unwrap_err();
    assert!(both.starts_with("2 keys in Hatoba are named ci"), "{both}");
    assert!(both.contains(&first) && both.contains(&second), "{both}");
    let by_id = plan(
        &v,
        HostChanges {
            key: Some(second.clone()),
            ..HostChanges::default()
        },
    )
    .unwrap();
    assert_eq!(by_id.input.key_id.as_deref(), Some(second.as_str()));
    let none = plan(
        &v,
        HostChanges {
            key: Some("deploy".into()),
            ..HostChanges::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        none,
        "no key in Hatoba is named deploy. The Keys page in Hatoba lists them."
    );

    let proxy = |name: &str| {
        plan(
            &v,
            HostChanges {
                proxy: Some(name.into()),
                ..HostChanges::default()
            },
        )
        .map(|p| (p.input.proxy_mode, p.input.proxy_id))
    };
    assert_eq!(
        proxy("office").unwrap(),
        (dto::ProxyMode::Proxy, Some(office))
    );
    assert_eq!(proxy("direct").unwrap(), (dto::ProxyMode::Direct, None));
    assert_eq!(
        proxy("default").unwrap(),
        (dto::ProxyMode::DeviceDefault, None)
    );
    assert!(
        proxy("home")
            .unwrap_err()
            .starts_with("no proxy in Hatoba is named home")
    );

    let jump = plan(
        &v,
        HostChanges {
            jump: Some("nope".into()),
            ..HostChanges::default()
        },
    )
    .unwrap_err();
    assert!(jump.contains("hatoba hosts"), "{jump}");
    assert!(
        plan(
            &v,
            HostChanges {
                group: Some(" ".into()),
                ..HostChanges::default()
            }
        )
        .is_err()
    );
}

#[test]
fn a_deletion_says_what_went_with_the_host() {
    assert_eq!(hosts::deleted("web-3", 0), "Deleted the host \"web-3\".");
    assert_eq!(
        hosts::deleted("web-3", 1),
        "Deleted the host \"web-3\" and its forwarding rule."
    );
    assert_eq!(
        hosts::deleted("web-3", 3),
        "Deleted the host \"web-3\" and its 3 forwarding rules."
    );
}
