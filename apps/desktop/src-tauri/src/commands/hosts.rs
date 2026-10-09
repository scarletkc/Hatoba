//! Hosts, groups, tags, reachability probes and `~/.ssh/config` import (spec §8.2, SSH-11).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use hatoba_core::model::{Group, Host, HostAuth, Item, MAX_HOST_AI_NOTES_CHARS, SshKey};
use hatoba_core::vault::Vault;
use hatoba_ssh::{ParsedKey, SshConfigHost};
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

use crate::convert::{group_view, host_view};
use crate::dto::{
    AuthKind, GroupInput, GroupView, HostInput, HostView, ImportResult, ProbeResult,
    SshConfigCandidate, TagCount,
};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, blocking, now_ms};
use crate::sync;

fn find_host(vault: &Vault, id: &str) -> AppResult<Host> {
    vault
        .get(id)
        .and_then(Item::as_host)
        .cloned()
        .ok_or_else(|| AppError::not_found("host"))
}

/// Validates the form (spec §9 "field validation errors") and builds the stored model.
pub fn host_from_input(
    input: &HostInput,
    existing: Option<&Host>,
    vault: &Vault,
) -> AppResult<Host> {
    let name = input.name.trim();
    let address = input.address.trim();
    if name.is_empty() {
        return Err(AppError::invalid("name", "name is required"));
    }
    if name.chars().count() > 128 {
        return Err(AppError::invalid("name", "name is too long"));
    }
    if address.is_empty() {
        return Err(AppError::invalid("address", "address is required"));
    }
    if address.chars().any(char::is_whitespace) || address.len() > 255 {
        return Err(AppError::invalid(
            "address",
            "address is not a valid host name or IP",
        ));
    }
    if input.port == 0 {
        return Err(AppError::invalid(
            "port",
            "port must be between 1 and 65535",
        ));
    }
    if input.ai_notes.chars().count() > MAX_HOST_AI_NOTES_CHARS {
        return Err(AppError::invalid(
            "ai_notes",
            "AI notes are limited to 2,000 characters",
        ));
    }
    if let Some(jump) = &input.jump_host_id {
        if input.id.as_deref() == Some(jump.as_str()) {
            return Err(AppError::invalid(
                "jump_host_id",
                "a host can't be its own jump host",
            ));
        }
        if vault.get(jump).and_then(Item::as_host).is_none() {
            return Err(AppError::invalid("jump_host_id", "jump host not found"));
        }
    }
    if let Some(group) = &input.group_id
        && vault.get(group).and_then(Item::as_group).is_none()
    {
        return Err(AppError::invalid("group_id", "group not found"));
    }
    let auth = match input.auth_kind {
        AuthKind::Password => match (&input.password, existing.map(|h| &h.auth)) {
            (Some(pw), _) => HostAuth::Password {
                password: Zeroizing::new(pw.clone()),
            },
            // HOST-08: keep the saved password; it is never sent to the WebView.
            (None, Some(HostAuth::Password { password })) => HostAuth::Password {
                password: password.clone(),
            },
            (None, _) => HostAuth::Password {
                password: Zeroizing::new(String::new()),
            },
        },
        AuthKind::Key => {
            let key_id = input
                .key_id
                .clone()
                .ok_or_else(|| AppError::invalid("key_id", "choose a key"))?;
            if vault.get(&key_id).and_then(Item::as_key).is_none() {
                return Err(AppError::invalid("key_id", "key not found"));
            }
            HostAuth::Key { key_id }
        }
        AuthKind::Agent => HostAuth::Agent,
        AuthKind::Ask => HostAuth::Ask,
    };
    let mut tags: Vec<String> = Vec::new();
    for tag in &input.tags {
        let tag = tag.trim();
        if !tag.is_empty() && !tags.iter().any(|t| t == tag) {
            tags.push(tag.to_owned());
        }
    }
    Ok(Host {
        name: name.to_owned(),
        address: address.to_owned(),
        port: input.port,
        username: input.username.trim().to_owned(),
        auth,
        group_id: input.group_id.clone(),
        tags,
        favorite: input.favorite,
        jump_host_id: input.jump_host_id.clone(),
        note: input.note.clone(),
        ai_notes: input.ai_notes.clone(),
        updated_at: 0,
    })
}

#[tauri::command]
#[specta::specta]
pub fn hosts_list(state: State<'_, AppState>) -> AppResult<Vec<HostView>> {
    state.with_unlocked(|v| {
        let mut hosts: Vec<HostView> = v
            .hosts()
            .iter()
            .map(|(id, h)| host_view(id, h, v))
            .collect();
        hosts.sort_by_cached_key(|h| h.name.to_lowercase());
        Ok(hosts)
    })
}

#[tauri::command]
#[specta::specta]
pub fn host_get(state: State<'_, AppState>, id: String) -> AppResult<HostView> {
    state.with_unlocked(|v| Ok(host_view(&id, &find_host(v, &id)?, v)))
}

#[tauri::command]
#[specta::specta]
pub fn host_save(
    app: AppHandle,
    state: State<'_, AppState>,
    input: HostInput,
) -> AppResult<HostView> {
    let view = state.with_unlocked(|v| {
        let existing = match &input.id {
            Some(id) => Some(find_host(v, id)?),
            None => None,
        };
        let host = host_from_input(&input, existing.as_ref(), v)?;
        let id = v.put(input.id.as_deref(), Item::Host(host))?;
        Ok(host_view(&id, &find_host(v, &id)?, v))
    })?;
    sync::local_change(&app);
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub fn host_delete(app: AppHandle, state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.with_unlocked(|v| {
        find_host(v, &id)?;
        // Hosts that jumped through this one now connect directly.
        for (other_id, mut other) in v.hosts() {
            if other.jump_host_id.as_deref() == Some(id.as_str()) {
                other.jump_host_id = None;
                v.put(Some(&other_id), Item::Host(other))?;
            }
        }
        for (fwd_id, fwd) in v.forwards() {
            if fwd.host_id == id {
                v.delete(&fwd_id)?;
            }
        }
        v.delete(&id)?;
        Ok(())
    })?;
    sync::local_change(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn host_duplicate(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<HostView> {
    let view = state.with_unlocked(|v| {
        let mut host = find_host(v, &id)?;
        let names: HashSet<String> = v.hosts().into_iter().map(|(_, h)| h.name).collect();
        let base = format!("{}-copy", host.name);
        host.name = (1..)
            .map(|n| {
                if n == 1 {
                    base.clone()
                } else {
                    format!("{base}-{n}")
                }
            })
            .find(|n| !names.contains(n))
            .unwrap_or(base);
        host.favorite = false;
        let new_id = v.put(None, Item::Host(host))?;
        Ok(host_view(&new_id, &find_host(v, &new_id)?, v))
    })?;
    sync::local_change(&app);
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub fn host_set_favorite(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    favorite: bool,
) -> AppResult<()> {
    state.with_unlocked(|v| {
        let mut host = find_host(v, &id)?;
        if host.favorite != favorite {
            host.favorite = favorite;
            v.put(Some(&id), Item::Host(host))?;
        }
        Ok(())
    })?;
    sync::local_change(&app);
    Ok(())
}

/// Turns the resource usage in a host's terminals on or off on this device (TERM-12).
#[tauri::command]
#[specta::specta]
pub fn host_set_show_stats(state: State<'_, AppState>, id: String, on: bool) -> AppResult<()> {
    state.with_unlocked(|v| {
        find_host(v, &id)?;
        Ok(v.set_host_show_stats(&id, on)?)
    })
}

#[tauri::command]
#[specta::specta]
pub fn groups_list(state: State<'_, AppState>) -> AppResult<Vec<GroupView>> {
    state.with_unlocked(|v| {
        let mut groups: Vec<GroupView> =
            v.groups().iter().map(|(id, g)| group_view(id, g)).collect();
        groups.sort_by(|a, b| a.sort.cmp(&b.sort).then_with(|| a.name.cmp(&b.name)));
        Ok(groups)
    })
}

#[tauri::command]
#[specta::specta]
pub fn group_save(
    app: AppHandle,
    state: State<'_, AppState>,
    input: GroupInput,
) -> AppResult<GroupView> {
    let view = state.with_unlocked(|v| {
        let name = input.name.trim();
        if name.is_empty() {
            return Err(AppError::invalid("name", "name is required"));
        }
        // MVP: one level of nesting (HOST-02).
        if let Some(parent) = &input.parent_id {
            let parent_group = v
                .get(parent)
                .and_then(Item::as_group)
                .ok_or_else(|| AppError::invalid("parent_id", "group not found"))?;
            if parent_group.parent_id.is_some() || input.id.as_deref() == Some(parent.as_str()) {
                return Err(AppError::invalid(
                    "parent_id",
                    "groups can only be nested one level",
                ));
            }
        }
        let group = Group {
            name: name.to_owned(),
            parent_id: input.parent_id.clone(),
            sort: input.sort.into(),
            updated_at: 0,
        };
        let id = v.put(input.id.as_deref(), Item::Group(group))?;
        let saved = v
            .get(&id)
            .and_then(Item::as_group)
            .cloned()
            .ok_or_else(|| AppError::not_found("group"))?;
        Ok(group_view(&id, &saved))
    })?;
    sync::local_change(&app);
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub fn group_delete(app: AppHandle, state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.with_unlocked(|v| {
        v.get(&id)
            .and_then(Item::as_group)
            .ok_or_else(|| AppError::not_found("group"))?;
        for (host_id, mut host) in v.hosts() {
            if host.group_id.as_deref() == Some(id.as_str()) {
                host.group_id = None;
                v.put(Some(&host_id), Item::Host(host))?;
            }
        }
        for (child_id, mut child) in v.groups() {
            if child.parent_id.as_deref() == Some(id.as_str()) {
                child.parent_id = None;
                v.put(Some(&child_id), Item::Group(child))?;
            }
        }
        v.delete(&id)?;
        Ok(())
    })?;
    sync::local_change(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn tags_list(state: State<'_, AppState>) -> AppResult<Vec<TagCount>> {
    state.with_unlocked(|v| {
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        for (_, host) in v.hosts() {
            for tag in host.tags {
                *counts.entry(tag).or_default() += 1;
            }
        }
        let mut tags: Vec<TagCount> = counts
            .into_iter()
            .map(|(name, count)| TagCount { name, count })
            .collect();
        tags.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
        Ok(tags)
    })
}

/// HOST-10: TCP connect only (no authentication), 3 s timeout, probed concurrently.
#[tauri::command]
#[specta::specta]
pub async fn hosts_probe(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> AppResult<Vec<ProbeResult>> {
    let targets: Vec<(String, String, u16)> = state.with_unlocked(|v| {
        Ok(ids
            .iter()
            .take(500)
            .filter_map(|id| {
                v.get(id)
                    .and_then(Item::as_host)
                    .map(|h| (id.clone(), h.address.clone(), h.port))
            })
            .collect())
    })?;
    let probes = targets.into_iter().map(|(id, host, port)| async move {
        // Hosts behind a jump host are usually unreachable directly; report them as unknown/offline.
        let latency = hatoba_ssh::tcp_probe(&host, port, Duration::from_secs(3)).await;
        ProbeResult {
            id,
            online: latency.is_some(),
            latency_ms: latency,
        }
    });
    Ok(futures::future::join_all(probes).await)
}

fn ssh_config_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".ssh").join("config"))
}

fn read_ssh_config() -> AppResult<Vec<SshConfigHost>> {
    let path = ssh_config_path().ok_or_else(|| AppError::not_found("home directory"))?;
    let text = std::fs::read_to_string(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            AppError::not_found("~/.ssh/config")
        } else {
            AppError::io(e)
        }
    })?;
    let home = dirs::home_dir().map(|h| h.to_string_lossy().into_owned());
    Ok(hatoba_ssh::config::parse_ssh_config_with_home(
        &text,
        home.as_deref(),
    ))
}

fn default_user() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default()
}

#[tauri::command]
#[specta::specta]
pub fn ssh_config_preview(state: State<'_, AppState>) -> AppResult<Vec<SshConfigCandidate>> {
    // Which identity files exist is checked before the vault is locked.
    let entries: Vec<(SshConfigHost, Option<String>)> = read_ssh_config()?
        .into_iter()
        .map(|e| {
            let found = identity_path(&e);
            (e, found)
        })
        .collect();
    state.with_unlocked(|v| {
        let names: HashSet<String> = v
            .hosts()
            .into_iter()
            .map(|(_, h)| h.name.to_lowercase())
            .collect();
        Ok(entries
            .into_iter()
            .map(|(e, found)| SshConfigCandidate {
                exists: names.contains(&e.alias.to_lowercase()),
                address: e.hostname.clone().unwrap_or_else(|| e.alias.clone()),
                port: e.port.unwrap_or(22),
                username: e.user.clone().unwrap_or_else(default_user),
                identity_file_found: found.is_some(),
                identity_file: found.or_else(|| e.identity_files.first().cloned()),
                proxy_jump: e.proxy_jump.clone(),
                alias: e.alias,
            })
            .collect())
    })
}

/// The identity file an entry signs in with: the first `IdentityFile` that exists.
fn identity_path(entry: &SshConfigHost) -> Option<String> {
    entry
        .identity_files
        .iter()
        .find(|p| Path::new(p).is_file())
        .cloned()
}

/// SSH-11: creates hosts for the chosen aliases. With `import_keys` it also imports the
/// unencrypted identity files they use as keys; without it no private key file is read, and
/// those hosts ask how to sign in.
#[tauri::command]
#[specta::specta]
pub async fn ssh_config_import(
    app: AppHandle,
    state: State<'_, AppState>,
    aliases: Vec<String>,
    import_keys: bool,
) -> AppResult<ImportResult> {
    let wanted: HashSet<String> = aliases.into_iter().collect();
    let (entries, identities) =
        blocking(move || Ok(prepare_import(read_ssh_config()?, &wanted, import_keys))).await?;
    let result = state.with_unlocked(|v| import_hosts(v, entries, identities))?;
    sync::local_change(&app);
    Ok(result)
}

/// The identity files an import reads, by path: the parsed key, or why it cannot be imported.
type Identities = HashMap<String, Result<ParsedKey, String>>;

/// The first step of an import, before the vault is locked: picks the chosen entries and, with
/// `import_keys`, the identity file of each, then reads and parses each file once.
fn prepare_import(
    entries: Vec<SshConfigHost>,
    wanted: &HashSet<String>,
    import_keys: bool,
) -> (Vec<(SshConfigHost, Option<String>)>, Identities) {
    let entries: Vec<(SshConfigHost, Option<String>)> = entries
        .into_iter()
        .filter(|e| wanted.contains(&e.alias))
        .map(|e| {
            let path = import_keys.then(|| identity_path(&e)).flatten();
            (e, path)
        })
        .collect();
    let mut identities = Identities::new();
    for path in entries.iter().filter_map(|(_, path)| path.as_ref()) {
        if !identities.contains_key(path) {
            identities.insert(path.clone(), read_identity(path));
        }
    }
    (entries, identities)
}

/// The second step, with the vault locked: saves the keys and creates the hosts.
fn import_hosts(
    v: &mut Vault,
    entries: Vec<(SshConfigHost, Option<String>)>,
    mut identities: Identities,
) -> AppResult<ImportResult> {
    let mut result = ImportResult {
        hosts_created: 0,
        keys_imported: 0,
        warnings: Vec::new(),
    };
    let mut created: Vec<(String, SshConfigHost)> = Vec::new();
    for (entry, path) in entries {
        let auth = match path.and_then(|p| identities.get_mut(&p).map(|slot| (p, slot))) {
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
                        result.warnings.push(format!("{}: {reason}", entry.alias));
                        // Hosts that share the file get the same warning.
                        *slot = Err(reason);
                        HostAuth::Ask
                    }
                }
            }
        };
        let host = Host {
            name: entry.alias.clone(),
            address: entry
                .hostname
                .clone()
                .unwrap_or_else(|| entry.alias.clone()),
            port: entry.port.unwrap_or(22),
            username: entry.user.clone().unwrap_or_else(default_user),
            auth,
            ..Host::default()
        };
        let id = v.put(None, Item::Host(host))?;
        result.hosts_created += 1;
        created.push((id, entry));
    }
    // Resolve ProxyJump (first hop) against hosts by name, now that all are created.
    let by_name: BTreeMap<String, String> = v
        .hosts()
        .into_iter()
        .map(|(id, h)| (h.name.to_lowercase(), id))
        .collect();
    for (id, entry) in created {
        let Some(jump) = entry.proxy_jump.as_deref() else {
            continue;
        };
        let first = jump.split(',').next().unwrap_or_default();
        let alias = first
            .rsplit('@')
            .next()
            .unwrap_or(first)
            .split(':')
            .next()
            .unwrap_or_default()
            .to_lowercase();
        match by_name.get(&alias) {
            Some(jump_id) if *jump_id != id => {
                if let Some(mut host) = v.get(&id).and_then(Item::as_host).cloned() {
                    host.jump_host_id = Some(jump_id.clone());
                    v.put(Some(&id), Item::Host(host))?;
                }
            }
            _ => result.warnings.push(format!(
                "{}: ProxyJump {jump} was not imported",
                entry.alias
            )),
        }
    }
    Ok(result)
}

/// Reads and parses an identity file referenced by ssh config, with the Keys page's size limit
/// and wiping. Passphrase-protected keys are left to the Keys page.
fn read_identity(path: &str) -> Result<ParsedKey, String> {
    let text = crate::commands::keys::read_key_file(Path::new(path))
        .map_err(|e| format!("can't read {path}: {}", e.detail))?;
    hatoba_ssh::parse_private_key(&text, None).map_err(|e| match e {
        hatoba_ssh::KeyError::PassphraseRequired => {
            format!("{path} is passphrase-protected; import it from the Keys page")
        }
        other => format!("{path}: {other}"),
    })
}

/// Saves a parsed identity file as a key, or reuses the key with the same fingerprint. The
/// private key moves into the vault item rather than being copied; a later host with the same
/// file finds the saved key by its fingerprint.
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
    let name = Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
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

/// SEC-08: copies a saved host password to the clipboard from Rust (it never reaches the WebView)
/// and clears it after 30 s if the clipboard still holds it.
#[tauri::command]
#[specta::specta]
pub fn host_copy_password(app: AppHandle, state: State<'_, AppState>, id: String) -> AppResult<()> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let password = state.with_unlocked(|v| match find_host(v, &id)?.auth {
        HostAuth::Password { password } if !password.is_empty() => Ok(password),
        _ => Err(AppError::invalid(
            "password",
            "this host has no saved password",
        )),
    })?;
    app.clipboard()
        .write_text(password.as_str())
        .map_err(|e| AppError::internal(e.to_string()))?;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let clipboard = app.clipboard();
        if clipboard
            .read_text()
            .is_ok_and(|current| Zeroizing::new(current).as_str() == password.as_str())
        {
            let _ = clipboard.write_text(String::new());
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use hatoba_core::KdfParams;
    use hatoba_ssh::GenerateKind;

    use super::*;

    fn unlocked_vault() -> Vault {
        let mut v = Vault::open_in_memory().unwrap();
        v.create_with_params("correct horse battery staple", KdfParams::for_tests())
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

    fn entry(alias: &str, identity_files: &[&str]) -> SshConfigHost {
        SshConfigHost {
            alias: alias.into(),
            hostname: Some(format!("{alias}.example.org")),
            user: Some("ops".into()),
            port: None,
            identity_files: identity_files.iter().map(|p| (*p).to_owned()).collect(),
            proxy_jump: None,
        }
    }

    fn import(
        v: &mut Vault,
        entries: Vec<SshConfigHost>,
        aliases: &[&str],
        import_keys: bool,
    ) -> ImportResult {
        let wanted = aliases.iter().map(|a| (*a).to_owned()).collect();
        let (entries, identities) = prepare_import(entries, &wanted, import_keys);
        import_hosts(v, entries, identities).unwrap()
    }

    fn auth_of(v: &Vault, alias: &str) -> HostAuth {
        v.hosts()
            .into_iter()
            .find(|(_, h)| h.name == alias)
            .map(|(_, h)| h.auth)
            .unwrap()
    }

    #[test]
    fn hosts_import_without_their_keys_unless_asked() {
        let dir = KeyDir::new();
        let (path, _) = dir.key("id_ed25519", None);
        let mut v = unlocked_vault();

        let (entries, identities) = prepare_import(
            vec![entry("web", &[&path])],
            &HashSet::from(["web".into()]),
            false,
        );
        assert_eq!(entries[0].1, None, "no identity file is chosen");
        assert!(identities.is_empty(), "no key file is read");
        let result = import_hosts(&mut v, entries, identities).unwrap();

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
        let mut v = unlocked_vault();
        // The vault already holds the second key, under another name.
        let parsed = read_identity(&existing).unwrap();
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
            true,
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
    fn unreadable_keys_become_warnings_and_their_hosts_ask() {
        let dir = KeyDir::new();
        let (locked, _) = dir.key("id_locked", Some("synthetic passphrase"));
        let big = dir.file("id_big", &"A".repeat(256 * 1024 + 1));
        let not_a_key = dir.file("id_text", "not a key\n");
        let mut v = unlocked_vault();

        let result = import(
            &mut v,
            vec![
                entry("locked", &[&locked]),
                entry("locked-too", &[&locked]),
                entry("big", &[&big]),
                entry("text", &[&not_a_key]),
            ],
            &["locked", "locked-too", "big", "text"],
            true,
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

    fn input(ai_notes: String) -> HostInput {
        HostInput {
            id: None,
            name: "prod-db".into(),
            address: "10.0.0.5".into(),
            port: 22,
            username: "ops".into(),
            auth_kind: AuthKind::Agent,
            password: None,
            key_id: None,
            group_id: None,
            tags: Vec::new(),
            favorite: false,
            jump_host_id: None,
            note: String::new(),
            ai_notes,
        }
    }

    #[test]
    fn ai_notes_are_saved_with_the_host_up_to_their_limit() {
        let mut v = Vault::open_in_memory().unwrap();
        v.create_with_params("correct horse battery staple", KdfParams::for_tests())
            .unwrap();
        // AI-37: kept as typed, line breaks and placeholders included.
        let notes = "PostgreSQL 16 primary.\nRestart with `systemctl restart <unit>`.";
        let host = host_from_input(&input(notes.into()), None, &v).unwrap();
        assert_eq!(host.ai_notes, notes);
        let id = v.put(None, Item::Host(host)).unwrap();
        assert_eq!(
            host_view(&id, &find_host(&v, &id).unwrap(), &v).ai_notes,
            notes
        );

        assert!(host_from_input(&input("日".repeat(MAX_HOST_AI_NOTES_CHARS)), None, &v).is_ok());
        let err = host_from_input(&input("日".repeat(MAX_HOST_AI_NOTES_CHARS + 1)), None, &v)
            .unwrap_err();
        assert_eq!(err.field.as_deref(), Some("ai_notes"));
    }
}
