//! Host import (SSH-11, SSH-12, SSH-15): a preview of the hosts in `~/.ssh/config` or another SSH
//! client's sessions, then the import of the ones the user ticked, with their folders as groups,
//! their proxies and jump hosts, and, when the user asks, their private key files.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use hatoba_core::model::{
    EnvVar, EnvVarError, Group, Host, HostAuth, HostProxy, Item, MAX_HOST_ENV_BYTES,
    MAX_HOST_ENV_VARS, Proxy, ProxyKind, SshKey, check_env_var,
};
use hatoba_core::vault::Vault;
use hatoba_ssh::ParsedKey;
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

use crate::commands::hosts::{MAX_NAME_CHARS, too_large_to_sync};
use crate::commands::proxies::MAX_CREDENTIAL_BYTES;
use crate::dto::{ImportCandidate, ImportPreview, ImportResult, ImportSource};
use crate::error::AppResult;
use crate::import::{self, Entry, Gateway, Jump};
use crate::platform::system::default_user;
use crate::state::{AppState, blocking, now_ms};
use crate::sync;

/// What a source holds, read before the vault is locked: a file on an unreachable network path
/// can take long to answer. Addresses lose the brackets of an IPv6 address, as hosts keep them
/// without, and entries whose address no host could have are left out.
fn read(source: ImportSource, path: Option<&str>) -> AppResult<import::Source> {
    let mut found = import::read(source, path.map(Path::new))?;
    for entry in &mut found.entries {
        entry.address = unbracketed(&entry.address).to_owned();
        if let Some(Jump::Gateways(list)) = &mut entry.jump {
            for gateway in list {
                gateway.address = unbracketed(&gateway.address).to_owned();
            }
        }
    }
    found
        .entries
        .retain(|e| e.address.len() <= 255 && !e.address.contains(char::is_whitespace));
    Ok(found)
}

/// `[2001:db8::1]` without its brackets, and without a `:port` after them.
fn unbracketed(address: &str) -> &str {
    let address = address.trim();
    address
        .strip_prefix('[')
        .and_then(|a| a.split_once(']'))
        .map_or(address, |(inner, _)| inner)
}

#[tauri::command]
#[specta::specta]
pub async fn hosts_import_preview(
    state: State<'_, AppState>,
    source: ImportSource,
    path: Option<String>,
) -> AppResult<ImportPreview> {
    let (found, identities) = blocking(move || {
        let found = read(source, path.as_deref())?;
        let identities: Vec<Option<String>> = found
            .entries
            .iter()
            .map(|e| identity_path(e, &found.key_files))
            .collect();
        Ok((found, identities))
    })
    .await?;
    state.with_unlocked(|v| {
        let names: HashSet<String> = v
            .hosts()
            .into_iter()
            .map(|(_, h)| h.name.to_lowercase())
            .collect();
        let by_id: HashMap<&str, &str> = found
            .entries
            .iter()
            .map(|e| (e.id.as_str(), e.name.as_str()))
            .collect();
        let candidates = found
            .entries
            .iter()
            .zip(identities)
            .map(|(e, found)| ImportCandidate {
                id: e.id.clone(),
                name: host_name(e),
                address: e.address.clone(),
                port: e.port,
                username: e.username.clone().unwrap_or_else(default_user),
                folder: (!e.folder.is_empty()).then(|| e.folder.join(" / ")),
                identity_file_found: found.is_some(),
                identity_file: found.or_else(|| e.identity_files.first().cloned()),
                kept_key: e.kept_key.clone(),
                jump: e.jump.as_ref().map(|j| match j {
                    Jump::Named(raw) => raw.clone(),
                    Jump::Entry(id) => by_id.get(id.as_str()).unwrap_or(&id.as_str()).to_string(),
                    Jump::Gateways(list) => list
                        .iter()
                        .map(Gateway::label)
                        .collect::<Vec<_>>()
                        .join(", "),
                }),
                proxy: e.proxy.as_ref().map(import::Proxy::label),
                skipped_proxy: e.skipped_proxy.clone(),
                exists: names.contains(&host_name(e).to_lowercase()),
            })
            .collect();
        Ok(ImportPreview {
            location: found.location.clone(),
            encrypted: found.encrypted,
            candidates,
        })
    })
}

/// Creates hosts for the chosen entries of `source`, read again from `path`. It imports as keys
/// only the unencrypted key files in `key_files`, the ones the preview listed and the user
/// confirmed. A host whose key file is not among them (the sessions or the files changed after
/// the preview) asks how to sign in, with a warning. With no `key_files`, no private key file is
/// read.
#[tauri::command]
#[specta::specta]
pub async fn hosts_import(
    app: AppHandle,
    state: State<'_, AppState>,
    source: ImportSource,
    path: Option<String>,
    ids: Vec<String>,
    key_files: Vec<String>,
) -> AppResult<ImportResult> {
    let wanted: HashSet<String> = ids.into_iter().collect();
    let confirmed: HashSet<String> = key_files.into_iter().collect();
    let prepared = blocking(move || {
        let found = read(source, path.as_deref())?;
        Ok(prepare_import(
            found.entries,
            &found.key_files,
            &wanted,
            &confirmed,
        ))
    })
    .await?;
    let result = state.with_unlocked(|v| import_hosts(v, prepared))?;
    sync::local_change(&app);
    Ok(result)
}

/// An entry's name as a host's: at most [`MAX_NAME_CHARS`] characters, and its address when the
/// entry has none.
fn host_name(entry: &Entry) -> String {
    let name = entry.name.trim();
    let name = if name.is_empty() {
        &entry.address
    } else {
        name
    };
    name.chars().take(MAX_NAME_CHARS).collect()
}

/// The key file an entry signs in with: the first one that the source holds or that exists.
fn identity_path(entry: &Entry, held: &KeyFiles) -> Option<String> {
    entry
        .identity_files
        .iter()
        .find(|p| held.contains_key(*p) || Path::new(p).is_file())
        .cloned()
}

/// Key files a source holds itself, by path.
type KeyFiles = HashMap<String, Zeroizing<Vec<u8>>>;

/// The key files an import reads, by path: the parsed key, or why it cannot be imported.
type Identities = HashMap<String, Result<ParsedKey, String>>;

/// The first step of an import, before the vault is locked.
struct Prepared {
    /// The chosen entries, each with its key file when key files were confirmed.
    chosen: Vec<(Entry, Option<String>)>,
    identities: Identities,
    /// The names of all the source's entries by id, for jump hosts that name one.
    names: HashMap<String, String>,
}

/// Picks the chosen entries and, when key files were confirmed, the key file of each: the first
/// one that the source holds or that exists, or else a confirmed one that has gone since the
/// preview, so the host gets a warning rather than silently none. Each confirmed file is read and
/// parsed once, from the source when it holds it; any other file is left unread.
fn prepare_import(
    entries: Vec<Entry>,
    held: &KeyFiles,
    wanted: &HashSet<String>,
    confirmed: &HashSet<String>,
) -> Prepared {
    let names = entries
        .iter()
        .map(|e| (e.id.clone(), host_name(e)))
        .collect();
    let chosen: Vec<(Entry, Option<String>)> = entries
        .into_iter()
        .filter(|e| wanted.contains(&e.id))
        .map(|e| {
            let path = if confirmed.is_empty() {
                None
            } else {
                identity_path(&e, held).or_else(|| {
                    e.identity_files
                        .iter()
                        .find(|p| confirmed.contains(*p))
                        .cloned()
                })
            };
            (e, path)
        })
        .collect();
    let mut identities = Identities::new();
    for path in chosen.iter().filter_map(|(_, path)| path.as_ref()) {
        if !identities.contains_key(path) {
            let identity = if confirmed.contains(path) {
                read_identity(path, held.get(path))
            } else {
                Err(format!(
                    "{path} was not among the confirmed key files and was not imported"
                ))
            };
            identities.insert(path.clone(), identity);
        }
    }
    Prepared {
        chosen,
        identities,
        names,
    }
}

/// The second step, with the vault locked: saves the keys, groups and proxies, creates the hosts,
/// then links their jump hosts.
fn import_hosts(v: &mut Vault, prepared: Prepared) -> AppResult<ImportResult> {
    let Prepared {
        chosen,
        mut identities,
        names,
    } = prepared;
    let mut result = ImportResult {
        hosts_created: 0,
        keys_imported: 0,
        warnings: Vec::new(),
    };
    let mut groups: HashMap<String, String> = HashMap::new();
    let mut created: Vec<(String, String, Option<Jump>)> = Vec::new();
    let mut by_entry: HashMap<String, String> = HashMap::new();
    for (entry, path) in chosen {
        let name = host_name(&entry);
        let auth = match path.and_then(|p| identities.get_mut(&p).map(|slot| (p, slot))) {
            None if entry.agent => HostAuth::Agent,
            None => HostAuth::Ask,
            Some((path, slot)) => {
                let saved = match slot {
                    Ok(parsed) => store_identity(v, &path, parsed),
                    Err(reason) => Err(reason.clone()),
                };
                match saved {
                    Ok((key_id, fresh)) => {
                        result.keys_imported += u32::from(fresh);
                        HostAuth::Key { key_id }
                    }
                    Err(reason) => {
                        result.warnings.push(format!("{name}: {reason}"));
                        // Hosts that share the file get the same warning.
                        *slot = Err(reason);
                        HostAuth::Ask
                    }
                }
            }
        };
        if let Some(key) = &entry.kept_key
            && matches!(auth, HostAuth::Ask)
        {
            let key = if key.is_empty() {
                String::new()
            } else {
                format!(" {key}")
            };
            result.warnings.push(format!(
                "{name}: the private key{key} stays in the client it came from; import its file from the Keys page"
            ));
        }
        if let Some(skipped) = &entry.skipped_proxy {
            result.warnings.push(format!(
                "{name}: {skipped} was not imported; set a proxy or jump host for it"
            ));
        }
        let proxy = match &entry.proxy {
            None => HostProxy::DeviceDefault,
            Some(proxy) => match proxy_for(v, proxy)? {
                Ok(proxy_id) => HostProxy::Proxy { proxy_id },
                Err(reason) => {
                    result.warnings.push(format!(
                        "{name}: proxy {} was not imported: {reason}",
                        proxy.label()
                    ));
                    HostProxy::DeviceDefault
                }
            },
        };
        let mut host = Host {
            group_id: group_for(v, &entry.folder, &mut groups)?,
            env: import_env(&name, &entry.set_env, &mut result.warnings),
            name: name.clone(),
            address: entry.address,
            port: entry.port,
            username: entry.username.unwrap_or_else(default_user),
            auth,
            proxy,
            note: entry.note,
            ..Host::default()
        };
        // As the host editor does while sync is on, a host too large to sync is not saved. Its
        // note, which other clients don't bound, is left out when that is enough.
        let mut item = Item::Host(host.clone());
        if too_large_to_sync(v, &item)? {
            host.note.clear();
            item = Item::Host(host);
            if too_large_to_sync(v, &item)? {
                result.warnings.push(format!(
                    "{name}: it is too large to sync and was not imported"
                ));
                continue;
            }
            result.warnings.push(format!(
                "{name}: its note was too long to sync and was not imported"
            ));
        }
        let id = v.put(None, item)?;
        result.hosts_created += 1;
        by_entry.insert(entry.id, id.clone());
        created.push((id, name, entry.jump));
    }

    // Jump hosts, now that all hosts exist.
    let by_name: HashMap<String, String> = v
        .hosts()
        .into_iter()
        .map(|(id, h)| (h.name.to_lowercase(), id))
        .collect();
    let mut gateways: HashMap<GatewayKey, String> = HashMap::new();
    for (id, name, jump) in created {
        let (jump_id, what) = match &jump {
            None => continue,
            Some(Jump::Named(raw)) => {
                let first = raw.split(',').next().unwrap_or_default();
                let alias = first
                    .rsplit('@')
                    .next()
                    .unwrap_or(first)
                    .split(':')
                    .next()
                    .unwrap_or_default()
                    .to_lowercase();
                (by_name.get(&alias).cloned(), format!("ProxyJump {raw}"))
            }
            Some(Jump::Entry(entry)) => {
                let jump_name = names.get(entry).cloned().unwrap_or_else(|| entry.clone());
                let found = by_entry
                    .get(entry)
                    .or_else(|| by_name.get(&jump_name.to_lowercase()))
                    .cloned();
                (found, format!("jump host {jump_name}"))
            }
            Some(Jump::Gateways(list)) => (
                gateway_chain(v, list, &mut gateways, &mut result)?,
                format!(
                    "gateway {}",
                    list.iter()
                        .map(Gateway::label)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ),
        };
        match jump_id {
            Some(jump_id) if jump_id != id => {
                if let Some(mut host) = v.get(&id).and_then(Item::as_host).cloned() {
                    host.jump_host_id = Some(jump_id);
                    v.put(Some(&id), Item::Host(host))?;
                }
            }
            _ => result
                .warnings
                .push(format!("{name}: {what} was not imported")),
        }
    }
    Ok(result)
}

/// The group for a folder path, named after the path with ` / ` between its folders, as the host
/// list shows groups flat (HOST-02). A top-level group with that name is reused.
fn group_for(
    v: &mut Vault,
    folder: &[String],
    cache: &mut HashMap<String, String>,
) -> AppResult<Option<String>> {
    if folder.is_empty() {
        return Ok(None);
    }
    let name: String = folder.join(" / ").chars().take(MAX_NAME_CHARS).collect();
    let key = name.to_lowercase();
    if let Some(id) = cache.get(&key) {
        return Ok(Some(id.clone()));
    }
    let groups = v.groups();
    let id = match groups
        .iter()
        .find(|(_, g)| g.parent_id.is_none() && g.name.to_lowercase() == key)
    {
        Some((id, _)) => id.clone(),
        None => {
            let sort = groups
                .iter()
                .map(|(_, g)| g.sort)
                .max()
                .map_or(0, |s| s + 1);
            v.put(
                None,
                Item::Group(Group {
                    name,
                    parent_id: None,
                    sort,
                    updated_at: 0,
                }),
            )?
        }
    };
    cache.insert(key, id.clone());
    Ok(Some(id))
}

/// The saved proxy with the same protocol, address, port, username and password, or a new one; or
/// why the proxy can't be saved, by the rules of the proxy editor (SSH-13).
fn proxy_for(v: &mut Vault, proxy: &import::Proxy) -> AppResult<Result<String, String>> {
    let address = unbracketed(&proxy.address);
    if address.is_empty()
        || address.len() > 255
        || address.contains(|c: char| c.is_whitespace() || c.is_control())
        || address.contains(['@', '/', '\\'])
    {
        return Ok(Err("the address is not a valid host name or IP".into()));
    }
    let username = proxy.username.trim();
    if username.len() > MAX_CREDENTIAL_BYTES || username.contains(char::is_control) {
        return Ok(Err("the username is not valid".into()));
    }
    if proxy.kind == ProxyKind::Http && username.contains(':') {
        return Ok(Err("an HTTP proxy username can't contain a colon".into()));
    }
    if proxy.password.len() > MAX_CREDENTIAL_BYTES {
        return Ok(Err("the password is longer than 255 bytes".into()));
    }
    let password = if username.is_empty() {
        zeroize::Zeroizing::new(String::new())
    } else {
        proxy.password.clone()
    };
    if let Some((id, _)) = v.proxies().into_iter().find(|(_, p)| {
        p.kind == proxy.kind
            && p.address.eq_ignore_ascii_case(address)
            && p.port == proxy.port
            && p.username == username
            && p.password == password
    }) {
        return Ok(Ok(id));
    }
    let id = v.put(
        None,
        Item::Proxy(Proxy {
            name: format!("{address}:{}", proxy.port),
            kind: proxy.kind,
            address: address.to_owned(),
            port: proxy.port,
            username: username.to_owned(),
            password,
            updated_at: 0,
        }),
    )?;
    Ok(Ok(id))
}

/// A gateway host already found or added in this import: its address, port and user, and the host
/// before it.
type GatewayKey = (String, u16, Option<String>, Option<String>);

/// The host a chain of gateways ends at, outermost first: a saved host with the gateway's address,
/// port and user that goes through the gateway before it (any saved one for the outermost), or a
/// new one named after its address. A new one signs in with the SSH agent, as MobaXterm and PuTTY
/// try first, since a jump host can't ask for a password.
fn gateway_chain(
    v: &mut Vault,
    list: &[Gateway],
    cache: &mut HashMap<GatewayKey, String>,
    result: &mut ImportResult,
) -> AppResult<Option<String>> {
    let mut previous: Option<String> = None;
    for gateway in list {
        let key = (
            gateway.address.to_lowercase(),
            gateway.port,
            gateway.username.clone(),
            previous.clone(),
        );
        if let Some(id) = cache.get(&key) {
            previous = Some(id.clone());
            continue;
        }
        let existing = v.hosts().into_iter().find(|(_, h)| {
            h.address.eq_ignore_ascii_case(&gateway.address)
                && h.port == gateway.port
                && gateway.username.as_ref().is_none_or(|u| *u == h.username)
                && (previous.is_none() || h.jump_host_id == previous)
        });
        let id = match existing {
            Some((id, _)) => id,
            None => {
                result.hosts_created += 1;
                v.put(
                    None,
                    Item::Host(Host {
                        name: gateway.address.chars().take(MAX_NAME_CHARS).collect(),
                        address: gateway.address.clone(),
                        port: gateway.port,
                        username: gateway.username.clone().unwrap_or_else(default_user),
                        auth: HostAuth::Agent,
                        jump_host_id: previous.clone(),
                        ..Host::default()
                    }),
                )?
            }
        };
        cache.insert(key, id.clone());
        previous = Some(id);
    }
    Ok(previous)
}

/// The `SetEnv` variables an imported host keeps (SSH-14): the ones the host editor would accept,
/// in order until the host has as many, or as many bytes, as it can. Each one left out gets a
/// warning.
fn import_env(name: &str, set_env: &[(String, String)], warnings: &mut Vec<String>) -> Vec<EnvVar> {
    let mut env: Vec<EnvVar> = Vec::new();
    let mut bytes = 0;
    for (var_name, value) in set_env {
        let var = EnvVar::new(var_name.as_str(), value.as_str());
        let size = var_name.len() + value.len();
        let problem = if env.iter().any(|v| v.name == *var_name) {
            Err(EnvVarError::NameDuplicate(var_name.clone()))
        } else if env.len() == MAX_HOST_ENV_VARS {
            Err(EnvVarError::TooMany)
        } else if bytes + size > MAX_HOST_ENV_BYTES {
            Err(EnvVarError::TooLarge)
        } else {
            check_env_var(&var)
        };
        match problem {
            Ok(()) => {
                bytes += size;
                env.push(var);
            }
            Err(e) => warnings.push(format!("{name}: SetEnv {var_name} was not imported: {e}")),
        }
    }
    env
}

/// Reads and parses a key file, or the source's copy when it `held` one, with the Keys page's size
/// limit and wiping. Passphrase-protected keys are left to the Keys page.
fn read_identity(path: &str, held: Option<&Zeroizing<Vec<u8>>>) -> Result<ParsedKey, String> {
    let text = match held {
        Some(bytes) => crate::commands::keys::key_file_text(bytes.clone()),
        None => crate::commands::keys::read_key_file(Path::new(path)),
    }
    .map_err(|e| format!("can't read {path}: {}", e.detail))?;
    hatoba_ssh::parse_private_key(&text, None).map_err(|e| match e {
        hatoba_ssh::KeyError::PassphraseRequired => {
            format!("{path} is passphrase-protected; import it from the Keys page")
        }
        other => format!("{path}: {other}"),
    })
}

/// Saves a parsed key file as a key, or reuses the key with the same fingerprint. The private key
/// moves into the vault item rather than being copied; a later host with the same file finds the
/// saved key by its fingerprint.
fn store_identity(
    v: &mut Vault,
    path: &str,
    parsed: &mut ParsedKey,
) -> Result<(String, bool), String> {
    if let Some((id, _)) = v
        .keys()
        .into_iter()
        .find(|(_, k)| k.fingerprint == parsed.fingerprint)
    {
        return Ok((id, false));
    }
    // A source may hold a key file by a path written on another system.
    let name = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_owned();
    let key = SshKey {
        name,
        algorithm: crate::commands::keys::core_algorithm(parsed.algorithm),
        private_key: std::mem::take(&mut parsed.openssh_private),
        passphrase: None,
        public_key: parsed.public_openssh.clone(),
        fingerprint: parsed.fingerprint.clone(),
        comment: parsed.comment.clone(),
        created_at: now_ms(),
        updated_at: 0,
    };
    let id = v.put(None, Item::Key(key)).map_err(|e| e.to_string())?;
    Ok((id, true))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use hatoba_core::KdfParams;
    use hatoba_ssh::GenerateKind;

    use super::*;

    /// An unlocked vault in memory. Its master password is made up for each run.
    fn vault() -> Vault {
        let mut v = Vault::open_in_memory().unwrap();
        v.create_with_params(&hatoba_core::new_id(), KdfParams::for_tests())
            .unwrap();
        v
    }

    /// A fresh directory for synthetic key files, removed when dropped.
    struct KeyDir(PathBuf);

    impl KeyDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "hatoba-ssh-import-{}-{}",
                std::process::id(),
                uuid::Uuid::now_v7()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self, name: &str, text: &str) -> String {
            let path = self.0.join(name);
            std::fs::write(&path, text).unwrap();
            path.to_string_lossy().into_owned()
        }

        /// A newly generated Ed25519 key, and its fingerprint.
        fn key(&self, name: &str, passphrase: Option<&str>) -> (String, String) {
            let key = hatoba_ssh::generate_key(GenerateKind::Ed25519, name, passphrase).unwrap();
            (self.file(name, &key.openssh_private), key.fingerprint)
        }
    }

    impl Drop for KeyDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn entry(alias: &str, identity_files: &[&str]) -> Entry {
        Entry {
            id: alias.into(),
            name: alias.into(),
            address: format!("{alias}.example.org"),
            port: 22,
            username: Some("ops".into()),
            identity_files: identity_files.iter().map(|p| (*p).to_owned()).collect(),
            ..Entry::default()
        }
    }

    fn import(
        v: &mut Vault,
        entries: Vec<Entry>,
        ids: &[&str],
        key_files: &[&str],
    ) -> ImportResult {
        let wanted = ids.iter().map(|a| (*a).to_owned()).collect();
        let confirmed = key_files.iter().map(|f| (*f).to_owned()).collect();
        import_hosts(
            v,
            prepare_import(entries, &KeyFiles::new(), &wanted, &confirmed),
        )
        .unwrap()
    }

    fn host(v: &Vault, name: &str) -> (String, Host) {
        v.hosts().into_iter().find(|(_, h)| h.name == name).unwrap()
    }

    fn auth_of(v: &Vault, alias: &str) -> HostAuth {
        host(v, alias).1.auth
    }

    #[test]
    fn hosts_import_without_their_keys_unless_asked() {
        let dir = KeyDir::new();
        let (path, _) = dir.key("id_ed25519", None);
        let mut v = vault();

        let prepared = prepare_import(
            vec![entry("web", &[&path])],
            &KeyFiles::new(),
            &HashSet::from(["web".into()]),
            &HashSet::new(),
        );
        assert_eq!(prepared.chosen[0].1, None, "no key file is chosen");
        assert!(prepared.identities.is_empty(), "no key file is read");
        let result = import_hosts(&mut v, prepared).unwrap();

        assert_eq!((result.hosts_created, result.keys_imported), (1, 0));
        assert!(result.warnings.is_empty());
        assert!(v.keys().is_empty());
        assert!(matches!(auth_of(&v, "web"), HostAuth::Ask));
    }

    #[test]
    fn keys_import_once_and_reuse_keys_already_in_the_vault() {
        let dir = KeyDir::new();
        let (shared, shared_fp) = dir.key("id_shared", None);
        let (existing, existing_fp) = dir.key("id_existing", None);
        let mut v = vault();
        // The vault already holds the second key, under another name.
        let parsed = read_identity(&existing, None).unwrap();
        v.put(
            None,
            Item::Key(SshKey {
                name: "already here".into(),
                private_key: parsed.openssh_private.clone(),
                fingerprint: parsed.fingerprint.clone(),
                ..SshKey::default()
            }),
        )
        .unwrap();
        let missing = dir.0.join("id_missing").to_string_lossy().into_owned();

        let result = import(
            &mut v,
            vec![
                entry("a", &[&missing, &shared]),
                entry("b", &[&shared]),
                entry("c", &[&existing]),
                entry("skipped", &[&shared]),
            ],
            &["a", "b", "c"],
            &[&shared, &existing],
        );

        assert_eq!((result.hosts_created, result.keys_imported), (3, 1));
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        let keys = v.keys();
        assert_eq!(keys.len(), 2);
        let id_of = |fp: &str| {
            keys.iter()
                .find(|(_, k)| k.fingerprint == fp)
                .unwrap()
                .0
                .clone()
        };
        let (shared_id, existing_id) = (id_of(&shared_fp), id_of(&existing_fp));
        let saved = &keys.iter().find(|(id, _)| *id == shared_id).unwrap().1;
        assert_eq!(saved.name, "id_shared");
        assert!(saved.private_key.contains("OPENSSH PRIVATE KEY"));
        for (alias, key) in [("a", &shared_id), ("b", &shared_id), ("c", &existing_id)] {
            assert!(
                matches!(auth_of(&v, alias), HostAuth::Key { key_id } if key_id == *key),
                "{alias}"
            );
        }
    }

    #[test]
    fn a_securecrt_export_imports_the_key_file_it_holds() {
        let dir = KeyDir::new();
        let key = hatoba_ssh::generate_key(GenerateKind::Ed25519, "Identity", None).unwrap();
        let hex: Vec<String> = key
            .openssh_private
            .bytes()
            .map(|b| format!("{b:02x}"))
            .collect();
        let session = |name: &str| {
            format!(
                "<key name=\"{name}\"><string name=\"Hostname\">{name}.example.org</string>\
                 <string name=\"Protocol Name\">SSH2</string></key>"
            )
        };
        // The key file set for all sessions, by its path on the computer the export came from.
        let export = dir.file(
            "SecureCRT.xml",
            &format!(
                "<VanDyke version=\"3.0\">\n<key name=\"Sessions\">{}{}</key>\n\
                 <key name=\"SSH2\"><string name=\"Identity Filename V2\">D:\\keys\\Identity\
                 </string></key>\n<key name=\"Files\"><key name=\"File 0\">\
                 <string name=\"Path\">D:\\keys\\Identity</string>\
                 <binary name=\"Contents\">{}</binary></key></key>\n</VanDyke>\n",
                session("web"),
                session("db"),
                hex.join(" ")
            ),
        );
        let found = read(ImportSource::Securecrt, Some(&export)).unwrap();
        let path = identity_path(&found.entries[0], &found.key_files).unwrap();
        assert_eq!(path, "D:\\keys\\Identity");
        let mut v = vault();

        let prepared = prepare_import(
            found.entries,
            &found.key_files,
            &HashSet::from(["web".into(), "db".into()]),
            &HashSet::from([path]),
        );
        let result = import_hosts(&mut v, prepared).unwrap();

        assert_eq!((result.hosts_created, result.keys_imported), (2, 1));
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        let keys = v.keys();
        assert_eq!(
            (keys[0].1.name.as_str(), keys[0].1.fingerprint.as_str()),
            ("Identity", key.fingerprint.as_str())
        );
        for alias in ["web", "db"] {
            assert!(
                matches!(auth_of(&v, alias), HostAuth::Key { key_id } if key_id == keys[0].0),
                "{alias}"
            );
        }
    }

    #[test]
    fn unreadable_keys_become_warnings_and_their_hosts_ask() {
        let dir = KeyDir::new();
        let (locked, _) = dir.key("id_locked", Some("synthetic passphrase"));
        let big = dir.file("id_big", &"A".repeat(256 * 1024 + 1));
        let not_a_key = dir.file("id_text", "not a key\n");
        let mut v = vault();

        let result = import(
            &mut v,
            vec![
                entry("locked", &[&locked]),
                entry("locked-too", &[&locked]),
                entry("big", &[&big]),
                entry("text", &[&not_a_key]),
            ],
            &["locked", "locked-too", "big", "text"],
            &[&locked, &big, &not_a_key],
        );

        assert_eq!((result.hosts_created, result.keys_imported), (4, 0));
        let warnings = result.warnings.join("\n");
        assert_eq!(result.warnings.len(), 4, "{warnings}");
        assert!(warnings.contains("passphrase-protected"), "{warnings}");
        assert!(warnings.contains("too large"), "{warnings}");
        for alias in ["locked", "locked-too", "big", "text"] {
            assert!(matches!(auth_of(&v, alias), HostAuth::Ask), "{alias}");
        }
        assert!(v.keys().is_empty());
    }

    #[test]
    fn only_the_confirmed_key_files_are_read() {
        let dir = KeyDir::new();
        let (listed, _) = dir.key("id_listed", None);
        // Missing when the preview ran, so not in the list, and created before the import.
        let (appeared, _) = dir.key("id_appeared", None);
        // In the list, and deleted before the import.
        let (gone, _) = dir.key("id_gone", None);
        std::fs::remove_file(&gone).unwrap();
        let mut v = vault();

        let result = import(
            &mut v,
            vec![
                entry("listed", &[&listed]),
                entry("appeared", &[&appeared]),
                entry("gone", &[&gone]),
            ],
            &["listed", "appeared", "gone"],
            &[&listed, &gone],
        );

        assert_eq!((result.hosts_created, result.keys_imported), (3, 1));
        assert_eq!(result.warnings.len(), 2, "{:?}", result.warnings);
        let warning = |alias: &str| {
            result
                .warnings
                .iter()
                .find(|w| w.starts_with(&format!("{alias}: ")))
                .cloned()
                .unwrap_or_default()
        };
        assert!(
            warning("appeared").contains("was not among the confirmed key files"),
            "{:?}",
            result.warnings
        );
        assert!(
            warning("gone").contains("can't read"),
            "{:?}",
            result.warnings
        );
        assert!(matches!(auth_of(&v, "listed"), HostAuth::Key { .. }));
        assert!(matches!(auth_of(&v, "appeared"), HostAuth::Ask));
        assert!(matches!(auth_of(&v, "gone"), HostAuth::Ask));
        assert_eq!(v.keys().len(), 1);
    }

    #[test]
    fn imported_set_env_keeps_the_variables_a_host_accepts() {
        let pairs = |list: &[(&str, &str)]| -> Vec<(String, String)> {
            list.iter()
                .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
                .collect()
        };
        let mut warnings = Vec::new();
        let env = import_env(
            "app",
            &pairs(&[("TZ", "UTC"), ("BAD-NAME", "x"), ("LANG", "C.UTF-8")]),
            &mut warnings,
        );
        assert_eq!(
            env,
            [EnvVar::new("TZ", "UTC"), EnvVar::new("LANG", "C.UTF-8")]
        );
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].starts_with("app: SetEnv BAD-NAME was not imported"),
            "{warnings:?}"
        );

        // A name the host already has is left out, as the host editor refuses it.
        let mut warnings = Vec::new();
        let env = import_env(
            "twice",
            &pairs(&[("TZ", "UTC"), ("TZ", "Asia/Tokyo")]),
            &mut warnings,
        );
        assert_eq!(env, [EnvVar::new("TZ", "UTC")]);
        assert_eq!(warnings.len(), 1, "{warnings:?}");

        let many: Vec<(String, String)> = (0..=MAX_HOST_ENV_VARS)
            .map(|i| (format!("V{i}"), String::new()))
            .collect();
        let mut warnings = Vec::new();
        assert_eq!(
            import_env("big", &many, &mut warnings).len(),
            MAX_HOST_ENV_VARS
        );
        assert_eq!(warnings.len(), 1);

        // A variable that would take the host over its byte budget is left out; later small
        // ones are still kept.
        let long = "x".repeat(4_000);
        let wide: Vec<(String, String)> = (0..5)
            .map(|i| (format!("L{i}"), long.clone()))
            .chain([("TZ".to_owned(), "UTC".to_owned())])
            .collect();
        let mut warnings = Vec::new();
        let env = import_env("wide", &wide, &mut warnings);
        assert_eq!(
            env.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(),
            ["L0", "L1", "L2", "L3", "TZ"]
        );
        assert_eq!(hatoba_core::model::check_host_env(&env), Ok(()));
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].starts_with("wide: SetEnv L4 was not imported"),
            "{warnings:?}"
        );
    }

    #[test]
    fn folders_become_groups_named_after_their_path() {
        let mut v = vault();
        let prod = v
            .put(
                None,
                Item::Group(Group {
                    name: "Prod".into(),
                    sort: 4,
                    ..Group::default()
                }),
            )
            .unwrap();
        let in_folder = |name: &str, folder: &[&str]| Entry {
            folder: folder.iter().map(|f| (*f).to_owned()).collect(),
            ..entry(name, &[])
        };
        let result = import(
            &mut v,
            vec![
                in_folder("web", &["prod"]),
                in_folder("pg", &["Prod", "DB", "PG"]),
                in_folder("pg-replica", &["prod", "db", "PG"]),
                in_folder("lab", &["Lab"]),
                in_folder("loose", &[]),
            ],
            &["web", "pg", "pg-replica", "lab", "loose"],
            &[],
        );
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);

        let groups = v.groups();
        assert_eq!(groups.len(), 3, "{groups:?}");
        let group_of = |name: &str| host(&v, name).1.group_id;
        assert_eq!(group_of("web"), Some(prod));
        let db = group_of("pg").unwrap();
        assert_eq!(group_of("pg-replica"), Some(db.clone()));
        let db = groups.iter().find(|(id, _)| *id == db).unwrap().1.clone();
        assert_eq!(
            (db.name.as_str(), db.parent_id, db.sort),
            ("Prod / DB / PG", None, 5)
        );
        let lab = group_of("lab").unwrap();
        let lab = groups.iter().find(|(id, _)| *id == lab).unwrap().1.clone();
        assert_eq!((lab.name.as_str(), lab.sort), ("Lab", 6));
        assert_eq!(group_of("loose"), None);
    }

    #[test]
    fn proxies_are_reused_or_saved() {
        let mut v = vault();
        let saved = v
            .put(
                None,
                Item::Proxy(Proxy {
                    name: "Local SOCKS".into(),
                    address: "127.0.0.1".into(),
                    port: 1080,
                    ..Proxy::default()
                }),
            )
            .unwrap();
        let via = |name: &str, kind: ProxyKind, address: &str, port: u16, user: &str| Entry {
            proxy: Some(import::Proxy {
                kind,
                address: address.into(),
                port,
                username: user.into(),
                password: zeroize::Zeroizing::new("proxy secret".into()),
            }),
            ..entry(name, &[])
        };
        let result = import(
            &mut v,
            vec![
                via("a", ProxyKind::Socks5, "127.0.0.1", 1080, ""),
                via("b", ProxyKind::Http, "proxy.example.org", 3128, "ops"),
                via("c", ProxyKind::Http, "proxy.example.org", 3128, "ops"),
                via("d", ProxyKind::Http, "proxy.example.org", 3128, "a:b"),
                entry("e", &[]),
                // The same endpoint and user with another password is another proxy.
                Entry {
                    proxy: Some(import::Proxy {
                        password: zeroize::Zeroizing::new("other secret".into()),
                        ..via("x", ProxyKind::Http, "proxy.example.org", 3128, "ops")
                            .proxy
                            .unwrap()
                    }),
                    ..entry("f", &[])
                },
                via("g", ProxyKind::Socks5, "proxy example", 1080, ""),
                via("h", ProxyKind::Socks5, "[::1]", 1080, ""),
            ],
            &["a", "b", "c", "d", "e", "f", "g", "h"],
            &[],
        );
        assert_eq!(result.warnings.len(), 2, "{:?}", result.warnings);
        assert!(result.warnings[0].starts_with("d: proxy HTTP proxy.example.org:3128"));
        assert!(result.warnings[1].starts_with("g: proxy SOCKS5 proxy example:1080"));

        let proxies = v.proxies();
        assert_eq!(proxies.len(), 4);
        let proxy_of = |name: &str| host(&v, name).1.proxy;
        assert_eq!(proxy_of("a"), HostProxy::Proxy { proxy_id: saved });
        let HostProxy::Proxy { proxy_id } = proxy_of("b") else {
            panic!("b has no proxy");
        };
        assert_eq!(
            proxy_of("c"),
            HostProxy::Proxy {
                proxy_id: proxy_id.clone()
            }
        );
        let new = &proxies.iter().find(|(id, _)| *id == proxy_id).unwrap().1;
        assert_eq!(
            (
                new.name.as_str(),
                new.kind,
                new.username.as_str(),
                new.password.as_str()
            ),
            (
                "proxy.example.org:3128",
                ProxyKind::Http,
                "ops",
                "proxy secret"
            )
        );
        assert_eq!(proxy_of("d"), HostProxy::DeviceDefault);
        assert_eq!(proxy_of("e"), HostProxy::DeviceDefault);
        assert_ne!(proxy_of("f"), proxy_of("b"));
        let HostProxy::Proxy { proxy_id } = proxy_of("h") else {
            panic!("h has no proxy");
        };
        let ipv6 = &proxies.iter().find(|(id, _)| *id == proxy_id).unwrap().1;
        assert_eq!(ipv6.address, "::1");
    }

    #[test]
    fn jump_hosts_resolve_to_imported_or_saved_hosts_and_gateways_are_added() {
        let mut v = vault();
        let old = v
            .put(
                None,
                Item::Host(Host {
                    name: "old-bastion".into(),
                    address: "10.0.0.1".into(),
                    port: 22,
                    username: "ops".into(),
                    ..Host::default()
                }),
            )
            .unwrap();
        // The inner gateway of `pg`, saved without the outer one before it.
        v.put(
            None,
            Item::Host(Host {
                name: "direct-inner".into(),
                address: "10.9.9.9".into(),
                port: 2200,
                ..Host::default()
            }),
        )
        .unwrap();
        let jumping = |name: &str, jump: Jump| Entry {
            jump: Some(jump),
            ..entry(name, &[])
        };
        let gateway = |address: &str, port: u16, user: Option<&str>| Gateway {
            address: address.into(),
            port,
            username: user.map(str::to_owned),
        };
        let chain = || {
            Jump::Gateways(vec![
                gateway("gw.example.org", 22, Some("jump")),
                gateway("10.9.9.9", 2200, None),
            ])
        };
        let entries = vec![
            Entry {
                id: "b1".into(),
                ..entry("bastion", &[])
            },
            jumping("web", Jump::Entry("b1".into())),
            jumping("api", Jump::Entry("missing".into())),
            jumping("proxied", Jump::Named("ops@bastion:2222,other".into())),
            jumping(
                "cache",
                Jump::Gateways(vec![gateway("10.0.0.1", 22, Some("ops"))]),
            ),
            jumping("pg", chain()),
            jumping("pg2", chain()),
            // Not ticked, but another entry names it, and a saved host has its name.
            Entry {
                id: "s1".into(),
                ..entry("old-bastion", &[])
            },
            jumping("queue", Jump::Entry("s1".into())),
        ];
        let result = import(
            &mut v,
            entries,
            &["b1", "web", "api", "proxied", "cache", "pg", "pg2", "queue"],
            &[],
        );
        assert_eq!(result.hosts_created, 8 + 2, "two gateways are added");
        assert_eq!(result.warnings, ["api: jump host missing was not imported"]);

        let jump_of = |name: &str| host(&v, name).1.jump_host_id;
        let bastion = host(&v, "bastion").0;
        assert_eq!(jump_of("web"), Some(bastion.clone()));
        assert_eq!(jump_of("proxied"), Some(bastion));
        assert_eq!(jump_of("api"), None);
        assert_eq!(jump_of("cache"), Some(old.clone()));
        assert_eq!(jump_of("queue"), Some(old));
        let (outer, outer_host) = host(&v, "gw.example.org");
        assert_eq!(
            (outer_host.username.as_str(), outer_host.jump_host_id),
            ("jump", None)
        );
        assert!(
            matches!(outer_host.auth, HostAuth::Agent),
            "a jump host can't ask"
        );
        let (inner, inner_host) = host(&v, "10.9.9.9");
        assert_eq!(
            (inner_host.port, inner_host.jump_host_id),
            (2200, Some(outer))
        );
        assert_eq!(jump_of("pg"), Some(inner.clone()));
        assert_eq!(jump_of("pg2"), Some(inner));
    }

    #[test]
    fn a_gateway_reached_through_another_route_is_another_host() {
        let mut v = vault();
        let via = |name: &str, outer: &str| Entry {
            jump: Some(Jump::Gateways(vec![
                Gateway {
                    address: outer.into(),
                    port: 22,
                    username: None,
                },
                Gateway {
                    address: "inner.example.org".into(),
                    port: 22,
                    username: None,
                },
            ])),
            ..entry(name, &[])
        };
        let result = import(
            &mut v,
            vec![via("a", "one.example.org"), via("b", "two.example.org")],
            &["a", "b"],
            &[],
        );
        assert_eq!(result.hosts_created, 2 + 4);
        let jump_of = |id: &str| {
            v.get(id)
                .and_then(Item::as_host)
                .and_then(|h| h.jump_host_id.clone())
        };
        let inner_a = host(&v, "a").1.jump_host_id.unwrap();
        let inner_b = host(&v, "b").1.jump_host_id.unwrap();
        assert_ne!(inner_a, inner_b);
        assert_eq!(jump_of(&inner_a), Some(host(&v, "one.example.org").0));
        assert_eq!(jump_of(&inner_b), Some(host(&v, "two.example.org").0));
    }

    #[test]
    fn a_note_too_large_to_sync_is_left_out_while_sync_is_on() {
        let mut v = vault();
        let config = hatoba_core::sync::SyncConfig::Worker {
            url: "https://sync.example.workers.dev".into(),
            deployment: None,
        };
        v.set_sync_config(Some(&config)).unwrap();
        let noted = |name: &str, note: String| Entry {
            note,
            ..entry(name, &[])
        };
        let max = hatoba_core::model::MAX_ITEM_PLAINTEXT_BYTES;
        let result = import(
            &mut v,
            vec![
                noted("long", "n".repeat(max)),
                noted("short", "on the rack".into()),
                // Too large without its note too.
                Entry {
                    username: Some("u".repeat(max)),
                    ..noted("huge", "rack A".into())
                },
            ],
            &["long", "short", "huge"],
            &[],
        );
        assert_eq!(
            result.warnings,
            [
                "long: its note was too long to sync and was not imported",
                "huge: it is too large to sync and was not imported",
            ]
        );
        assert_eq!(result.hosts_created, 2);
        assert_eq!(host(&v, "long").1.note, "");
        assert_eq!(host(&v, "short").1.note, "on the rack");
        assert!(v.hosts().iter().all(|(_, h)| h.name != "huge"));
    }

    #[test]
    fn read_drops_ipv6_brackets_and_addresses_no_host_could_have() {
        let dir = KeyDir::new();
        let config = dir.file(
            "config.yaml",
            "profiles:\n\
             - {type: ssh, id: a, name: v6, options: {host: '[2001:db8::1]'}}\n\
             - {type: ssh, id: b, name: spaced, options: {host: 'web example'}}\n\
             - {type: ssh, id: c, name: web, options: {host: web.example.org}}\n",
        );
        let found = read(ImportSource::Tabby, Some(&config)).unwrap();
        let addresses: Vec<&str> = found.entries.iter().map(|e| e.address.as_str()).collect();
        assert_eq!(addresses, ["2001:db8::1", "web.example.org"]);
    }

    #[test]
    fn ipv6_addresses_lose_their_brackets() {
        assert_eq!(unbracketed("[2001:db8::1]"), "2001:db8::1");
        assert_eq!(unbracketed(" [::1]:2222"), "::1");
        assert_eq!(unbracketed("::1"), "::1");
        assert_eq!(unbracketed("web.example.org"), "web.example.org");
    }

    #[test]
    fn kept_keys_and_skipped_proxies_warn_and_agent_entries_use_the_agent() {
        let mut v = vault();
        let result = import(
            &mut v,
            vec![
                Entry {
                    kept_key: Some("id_rsa_2048".into()),
                    ..entry("xshell", &[])
                },
                Entry {
                    skipped_proxy: Some("SOCKS4 proxy 10.0.0.1:1080".into()),
                    ..entry("old", &[])
                },
                Entry {
                    agent: true,
                    ..entry("agent", &[])
                },
                Entry {
                    name: " ".into(),
                    username: None,
                    ..entry("unnamed", &[])
                },
            ],
            &["xshell", "old", "agent", "unnamed"],
            &[],
        );
        assert_eq!(
            result.warnings,
            [
                "xshell: the private key id_rsa_2048 stays in the client it came from; import its file from the Keys page",
                "old: SOCKS4 proxy 10.0.0.1:1080 was not imported; set a proxy or jump host for it",
            ]
        );
        assert!(matches!(auth_of(&v, "agent"), HostAuth::Agent));
        let (_, unnamed) = host(&v, "unnamed.example.org");
        assert_eq!(unnamed.username, default_user());
    }
}
