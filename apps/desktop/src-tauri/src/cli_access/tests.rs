use hatoba_core::KdfParams;
use hatoba_core::model::{Group, HostProxy, KeyAlgorithm, SshKey};
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

#[test]
fn remote_paths_join_with_one_slash() {
    assert_eq!(join_remote("/home/me", "a.txt"), "/home/me/a.txt");
    assert_eq!(join_remote("/tmp/", "a.txt"), "/tmp/a.txt");
}
