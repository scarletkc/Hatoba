//! Hosts, groups, tags, reachability probes and `~/.ssh/config` import (spec §8.2, SSH-11).

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use hatoba_core::model::{Group, Host, HostAuth, Item, MAX_HOST_AI_NOTES_CHARS, SshKey};
use hatoba_core::vault::Vault;
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

use crate::convert::{group_view, host_view};
use crate::dto::{
    AuthKind, GroupInput, GroupView, HostInput, HostView, ImportResult, ProbeResult,
    SshConfigCandidate, TagCount,
};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, now_ms};
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

fn read_ssh_config() -> AppResult<Vec<hatoba_ssh::SshConfigHost>> {
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
    let entries = read_ssh_config()?;
    state.with_unlocked(|v| {
        let names: HashSet<String> = v
            .hosts()
            .into_iter()
            .map(|(_, h)| h.name.to_lowercase())
            .collect();
        Ok(entries
            .into_iter()
            .map(|e| SshConfigCandidate {
                exists: names.contains(&e.alias.to_lowercase()),
                address: e.hostname.clone().unwrap_or_else(|| e.alias.clone()),
                port: e.port.unwrap_or(22),
                username: e.user.clone().unwrap_or_else(default_user),
                identity_file: e.identity_files.first().cloned(),
                proxy_jump: e.proxy_jump.clone(),
                alias: e.alias,
            })
            .collect())
    })
}

/// SSH-11: creates hosts (and imports unencrypted identity files as keys) for the chosen aliases.
#[tauri::command]
#[specta::specta]
pub fn ssh_config_import(
    app: AppHandle,
    state: State<'_, AppState>,
    aliases: Vec<String>,
) -> AppResult<ImportResult> {
    let entries = read_ssh_config()?;
    let wanted: HashSet<&str> = aliases.iter().map(String::as_str).collect();
    let result = state.with_unlocked(|v| {
        let mut result = ImportResult {
            hosts_created: 0,
            keys_imported: 0,
            warnings: Vec::new(),
        };
        let mut created: Vec<(String, hatoba_ssh::SshConfigHost)> = Vec::new();
        for entry in entries
            .into_iter()
            .filter(|e| wanted.contains(e.alias.as_str()))
        {
            let auth = match entry
                .identity_files
                .iter()
                .find(|p| std::path::Path::new(p).is_file())
            {
                Some(path) => match import_identity(v, path) {
                    Ok((key_id, fresh)) => {
                        result.keys_imported += u32::from(fresh);
                        HostAuth::Key { key_id }
                    }
                    Err(reason) => {
                        result.warnings.push(format!("{}: {reason}", entry.alias));
                        HostAuth::Ask
                    }
                },
                None => HostAuth::Ask,
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
    })?;
    sync::local_change(&app);
    Ok(result)
}

/// Imports an identity file referenced by ssh config. Reuses an existing key with the same fingerprint.
fn import_identity(v: &mut Vault, path: &str) -> Result<(String, bool), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("can't read {path}: {e}"))?;
    let parsed = hatoba_ssh::parse_private_key(&text, None).map_err(|e| match e {
        hatoba_ssh::KeyError::PassphraseRequired => {
            format!("{path} is passphrase-protected; import it from the Keys page")
        }
        other => format!("{path}: {other}"),
    })?;
    if let Some((id, _)) = v
        .keys()
        .into_iter()
        .find(|(_, k)| k.fingerprint == parsed.fingerprint)
    {
        return Ok((id, false));
    }
    let name = std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let key = SshKey {
        name,
        algorithm: crate::commands::keys::core_algorithm(parsed.algorithm),
        private_key: parsed.openssh_private.clone(),
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
    use hatoba_core::KdfParams;

    use super::*;

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
