//! Tabby's SSH profiles (SSH-15), in its `config.yaml`. A profile's settings fall back to those
//! of its group, then to the defaults for SSH profiles, then to Tabby's own (port 22, user
//! `root`). Tabby keeps passwords in the system's credential store or in its vault, and can
//! encrypt the whole file with the vault's passphrase; then nothing can be read from it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use hatoba_core::model::ProxyKind;
use serde_yaml_ng::Value;
use zeroize::Zeroizing;

use super::text;
use super::{Entry, Jump, Proxy, Source, non_empty};
use crate::error::{AppError, AppResult};

pub(super) fn read(path: Option<&Path>) -> AppResult<Source> {
    let Some(path) = path.map(Path::to_path_buf).or_else(default_file) else {
        return Ok(Source::default());
    };
    let mut source = parse(&text::decode(&text::read_file(&path)?))?;
    source.location = Some(path.to_string_lossy().into_owned());
    Ok(source)
}

/// `config.yaml` in Tabby's folder, or in `TABBY_CONFIG_DIRECTORY` when that is set.
fn default_file() -> Option<PathBuf> {
    std::env::var_os("TABBY_CONFIG_DIRECTORY")
        .map(PathBuf::from)
        .or_else(|| dirs::config_dir().map(|d| d.join("tabby")))
        .map(|d| d.join("config.yaml"))
        .filter(|p| p.is_file())
}

fn parse(text: &str) -> AppResult<Source> {
    let config: Value = serde_yaml_ng::from_str(text)
        .map_err(|e| AppError::invalid("path", format!("not a Tabby config: {e}")))?;
    if config.get("encrypted").and_then(Value::as_bool) == Some(true) {
        return Ok(Source {
            encrypted: true,
            ..Source::default()
        });
    }
    let list = |key: &str| {
        config
            .get(key)
            .and_then(Value::as_sequence)
            .map(Vec::as_slice)
            .unwrap_or_default()
    };
    let groups: HashMap<&str, &Value> = list("groups")
        .iter()
        .filter_map(|g| Some((g.get("id")?.as_str()?, g)))
        .collect();
    let profiles: Vec<&Value> = list("profiles")
        .iter()
        .filter(|p| p.get("type").and_then(Value::as_str) == Some("ssh"))
        .collect();
    let defaults = ssh_options(config.get("profileDefaults"));
    let entries = profiles
        .iter()
        .filter_map(|p| entry(p, &profiles, &groups, defaults))
        .collect();
    Ok(Source {
        location: None,
        encrypted: false,
        entries,
        ..Source::default()
    })
}

/// `ssh.options` of a set of defaults.
fn ssh_options(defaults: Option<&Value>) -> Option<&Value> {
    defaults?.get("ssh")?.get("options")
}

fn entry(
    profile: &Value,
    profiles: &[&Value],
    groups: &HashMap<&str, &Value>,
    defaults: Option<&Value>,
) -> Option<Entry> {
    let id = profile.get("id").and_then(Value::as_str)?.to_owned();
    let group_ref = profile
        .get("group")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let group = groups.get(group_ref).copied();
    let layers: Vec<&Value> = [
        profile.get("options"),
        ssh_options(group.and_then(|g| g.get("defaults"))),
        defaults,
    ]
    .into_iter()
    .flatten()
    .collect();
    let option = |key: &str| {
        layers
            .iter()
            .find_map(|l| l.get(key).filter(|v| !v.is_null()))
    };
    let text_option = |key: &str| non_empty(option(key).and_then(Value::as_str));
    let number_option = |key: &str, default: u16| {
        option(key)
            .and_then(|v| v.as_u64().or_else(|| v.as_str()?.trim().parse().ok()))
            .and_then(|p| u16::try_from(p).ok())
            .filter(|p| *p != 0)
            .unwrap_or(default)
    };

    let address = text_option("host")?;
    let mut identity_files = Vec::new();
    let mut kept_key = None;
    for key in option("privateKeys")
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if key.starts_with("vault://") {
            kept_key = Some(String::new());
        } else if let Some(path) = key_path(key) {
            identity_files.push(path);
        }
    }
    let proxy = [
        ("socksProxyHost", "socksProxyPort", ProxyKind::Socks5, 1080),
        ("httpProxyHost", "httpProxyPort", ProxyKind::Http, 8080),
    ]
    .into_iter()
    .find_map(|(host, port, kind, default_port)| {
        Some(Proxy {
            kind,
            address: text_option(host)?,
            port: number_option(port, default_port),
            username: String::new(),
            password: Zeroizing::new(String::new()),
        })
    });
    let jump = text_option("jumpHost").map(|jump| {
        // The profile's id; a name in configs from before ids were used.
        let by_name = profiles.iter().find_map(|p| {
            (p.get("name").and_then(Value::as_str) == Some(jump.as_str()))
                .then(|| p.get("id").and_then(Value::as_str))
                .flatten()
        });
        let known = profiles
            .iter()
            .any(|p| p.get("id").and_then(Value::as_str) == Some(jump.as_str()));
        Jump::Entry(if known {
            jump.clone()
        } else {
            by_name.map_or(jump, str::to_owned)
        })
    });
    Some(Entry {
        name: non_empty(profile.get("name").and_then(Value::as_str))
            .unwrap_or_else(|| address.clone()),
        id,
        port: number_option("port", 22),
        username: Some(text_option("user").unwrap_or_else(|| "root".into())),
        folder: match group {
            Some(group) => group_path(group, groups),
            // A group name, in configs from before groups had ids.
            None => non_empty(Some(group_ref)).into_iter().collect(),
        },
        identity_files,
        kept_key,
        agent: text_option("auth").as_deref() == Some("agent"),
        jump,
        proxy,
        skipped_proxy: text_option("proxyCommand").map(|c| format!("ProxyCommand {c}")),
        address,
        ..Entry::default()
    })
}

/// A private key's path, written `file://` and the path.
fn key_path(key: &str) -> Option<String> {
    let path = key.strip_prefix("file://").unwrap_or(key);
    // `file:///C:/Users/...`
    let path = match path.strip_prefix('/') {
        Some(rest) if rest.get(1..2) == Some(":") => rest,
        _ => path,
    };
    non_empty(Some(path))
}

/// The names of `group` and the groups it is in, outermost first.
fn group_path(group: &Value, groups: &HashMap<&str, &Value>) -> Vec<String> {
    let mut path = Vec::new();
    let mut current = Some(group);
    // At most once per group, in case of a loop.
    for _ in 0..=groups.len() {
        let Some(group) = current else {
            break;
        };
        path.extend(non_empty(group.get("name").and_then(Value::as_str)));
        current = group
            .get("parentGroupId")
            .and_then(Value::as_str)
            .and_then(|id| groups.get(id).copied());
    }
    path.reverse();
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r"version: 8
profiles:
  - type: local
    name: PowerShell
    id: local:custom:powershell:1
    options:
      command: pwsh.exe
  - type: ssh
    name: bastion
    id: ssh:custom:bastion:91ab
    options:
      host: bastion.example.org
      user: jump
      auth: agent
  - type: ssh
    name: web-01
    group: 3f8e6a10
    id: ssh:custom:web-01:7c2f
    options:
      host: 203.0.113.10
      port: 2222
      auth: publicKey
      privateKeys:
        - file://C:\Users\ops\.ssh\id_ed25519
        - vault://9f2c
      jumpHost: ssh:custom:bastion:91ab
      socksProxyHost: 127.0.0.1
      socksProxyPort: 1081
  - type: ssh
    name: legacy
    group: Lab
    id: ssh:custom:legacy:1
    options:
      host: 10.0.0.4
      privateKeys:
        - file:///home/ops/.ssh/id_rsa
      jumpHost: bastion
      proxyCommand: nc -X connect -x proxy:3128 %h %p
  - type: ssh
    name: no-host
    id: ssh:custom:no-host:1
    options: {}
groups:
  - id: 0a1b
    name: Production
  - id: 3f8e6a10
    name: Web
    parentGroupId: 0a1b
    defaults:
      ssh:
        options:
          user: deploy
profileDefaults:
  ssh:
    options:
      port: 22
      user: admin
";

    #[test]
    fn ssh_profiles_parse_with_groups_defaults_jump_hosts_and_proxies() {
        let source = parse(CONFIG).unwrap();
        assert!(!source.encrypted);
        let names: Vec<&str> = source.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["bastion", "web-01", "legacy"]);

        let bastion = &source.entries[0];
        assert_eq!(
            (
                bastion.address.as_str(),
                bastion.port,
                bastion.username.as_deref()
            ),
            ("bastion.example.org", 22, Some("jump"))
        );
        assert!(bastion.agent);

        let web = &source.entries[1];
        assert_eq!(web.id, "ssh:custom:web-01:7c2f");
        assert_eq!(
            (web.port, web.username.as_deref()),
            (2222, Some("deploy")),
            "the group's default"
        );
        assert_eq!(web.folder, ["Production", "Web"]);
        assert_eq!(web.identity_files, [r"C:\Users\ops\.ssh\id_ed25519"]);
        assert_eq!(
            web.kept_key.as_deref(),
            Some(""),
            "the key in Tabby's vault"
        );
        assert_eq!(
            web.jump,
            Some(Jump::Entry("ssh:custom:bastion:91ab".into()))
        );
        assert_eq!(web.proxy.as_ref().unwrap().label(), "SOCKS5 127.0.0.1:1081");
        assert!(!web.agent);

        let legacy = &source.entries[2];
        assert_eq!(
            legacy.username.as_deref(),
            Some("admin"),
            "the defaults for SSH profiles"
        );
        assert_eq!(legacy.folder, ["Lab"]);
        assert_eq!(legacy.identity_files, ["/home/ops/.ssh/id_rsa"]);
        assert_eq!(
            legacy.jump,
            Some(Jump::Entry("ssh:custom:bastion:91ab".into())),
            "by name"
        );
        assert_eq!(
            legacy.skipped_proxy.as_deref(),
            Some("ProxyCommand nc -X connect -x proxy:3128 %h %p")
        );
    }

    #[test]
    fn tabbys_own_defaults_apply_and_encrypted_configs_are_reported() {
        let source =
            parse("profiles:\n  - type: ssh\n    id: a\n    options:\n      host: h\n").unwrap();
        let entry = &source.entries[0];
        assert_eq!(
            (entry.name.as_str(), entry.port, entry.username.as_deref()),
            ("h", 22, Some("root"))
        );

        let encrypted = parse("encrypted: true\nvault:\n  version: 1\n  contents: abc\n").unwrap();
        assert!(encrypted.encrypted && encrypted.entries.is_empty());
        assert!(parse("profiles: [").is_err());
        assert_eq!(key_path("file:///C:/keys/id"), Some("C:/keys/id".into()));
    }
}
