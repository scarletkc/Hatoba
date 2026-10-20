//! Hosts, groups, tags and reachability probes (spec §8.2).

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use hatoba_core::model::{
    EnvVar, Group, Host, HostAuth, HostProxy, Item, MAX_HOST_AI_NOTES_BYTES,
    MAX_ITEM_PLAINTEXT_BYTES, check_host_env,
};
use hatoba_core::vault::Vault;
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

use crate::convert::{group_view, host_view};
use crate::dto::{
    AuthKind, GroupInput, GroupView, HostInput, HostView, LocalPrefs, ProbeResult, ProxyMode,
    TagCount,
};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::sync;

fn find_host(vault: &Vault, id: &str) -> AppResult<Host> {
    vault
        .get(id)
        .and_then(Item::as_host)
        .cloned()
        .ok_or_else(|| AppError::not_found("host"))
}

/// The longest host name, in characters (HOST-01).
pub(crate) const MAX_NAME_CHARS: usize = 128;

/// Validates the form (spec §9 "field validation errors") and builds the stored model.
pub fn host_from_input(
    input: &HostInput,
    existing: Option<&Host>,
    vault: &Vault,
) -> AppResult<Host> {
    let name = input.name.trim();
    let address = input.address.trim();
    if name.chars().count() > MAX_NAME_CHARS {
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
    if input.ai_notes.len() > MAX_HOST_AI_NOTES_BYTES {
        return Err(AppError::invalid(
            "ai_notes",
            "AI notes are limited to 16 KB",
        ));
    }
    let env: Vec<EnvVar> = input
        .env
        .iter()
        .map(|v| EnvVar::new(v.name.trim(), v.value.as_str()))
        .collect();
    check_host_env(&env).map_err(|e| AppError::invalid("env", e.to_string()))?;
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
    let proxy = match input.proxy_mode {
        ProxyMode::DeviceDefault => HostProxy::DeviceDefault,
        ProxyMode::Direct => HostProxy::Direct,
        ProxyMode::Proxy => {
            let proxy_id = input
                .proxy_id
                .clone()
                .ok_or_else(|| AppError::invalid("proxy_id", "choose a proxy"))?;
            if vault.get(&proxy_id).and_then(Item::as_proxy).is_none() {
                return Err(AppError::invalid("proxy_id", "proxy not found"));
            }
            HostProxy::Proxy { proxy_id }
        }
    };
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
        // HOST-01: a host without a name is named after its address, cut to the name's limit so
        // the host saves again as it is.
        name: if name.is_empty() {
            address.chars().take(MAX_NAME_CHARS).collect()
        } else {
            name.to_owned()
        },
        address: address.to_owned(),
        port: input.port,
        username: input.username.trim().to_owned(),
        auth,
        group_id: input.group_id.clone(),
        tags,
        favorite: input.favorite,
        jump_host_id: input.jump_host_id.clone(),
        proxy,
        note: input.note.clone(),
        ai_notes: input.ai_notes.clone(),
        updated_at: 0,
        env,
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
        let item = Item::Host(host_from_input(&input, existing.as_ref(), v)?);
        check_item_size(v, &item)?;
        let id = v.put(input.id.as_deref(), item)?;
        Ok(host_view(&id, &find_host(v, &id)?, v))
    })?;
    sync::local_change(&app);
    Ok(view)
}

/// With sync on, refuses a host too large to sync: the Worker and D1 would reject its envelope on
/// every sync (§6.2). Long notes or environment variables are what make a host this large.
pub(crate) fn check_item_size(vault: &Vault, item: &Item) -> AppResult<()> {
    if too_large_to_sync(vault, item)? {
        return Err(AppError::invalid(
            "host",
            "this host is too large to sync: shorten its notes or environment variables",
        ));
    }
    Ok(())
}

/// Whether sync is on and `item` is larger than it sends (§6.2).
pub(crate) fn too_large_to_sync(vault: &Vault, item: &Item) -> AppResult<bool> {
    Ok(vault.sync_config()?.is_some() && item.plaintext_len()? > MAX_ITEM_PLAINTEXT_BYTES)
}

#[tauri::command]
#[specta::specta]
pub fn host_delete(app: AppHandle, state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.with_unlocked(|v| {
        find_host(v, &id)?;
        // Hosts that jumped through this one now connect directly; its forwards go with it.
        v.delete_host(&id)?;
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

/// Leaves a host or a group out of the dashboard on this device, or brings it back (HOST-14).
#[tauri::command]
#[specta::specta]
pub fn dashboard_set_shown(state: State<'_, AppState>, id: String, shown: bool) -> AppResult<()> {
    state.with_unlocked(|v| {
        match v.get(&id) {
            Some(Item::Host(_) | Item::Group(_)) => {}
            _ => return Err(AppError::not_found("host or group")),
        }
        Ok(v.set_dashboard_shown(&id, shown)?)
    })
}

#[tauri::command]
#[specta::specta]
pub fn groups_list(state: State<'_, AppState>) -> AppResult<Vec<GroupView>> {
    state.with_unlocked(|v| {
        let mut groups: Vec<GroupView> = v
            .groups()
            .iter()
            .map(|(id, g)| group_view(id, g, v))
            .collect();
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
        Ok(group_view(&id, &saved, v))
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
        // Its hosts and child groups move to the top level.
        v.delete_group(&id)?;
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

/// Where a host probe connects: the address, the port, and the proxy on the way.
type ProbeTarget = (String, u16, Option<hatoba_ssh::ProxyConfig>);

/// The probe target of each existing host among `ids`. SSH-13: the probe goes through the proxy
/// the connection would use, never around it, so a host whose proxy was deleted gets `None` and
/// is reported offline (the list keeps the last result of a host it is not told about).
fn probe_targets(v: &Vault, ids: &[String]) -> Vec<(String, Option<ProbeTarget>)> {
    ids.iter()
        .take(500)
        .filter_map(|id| {
            let h = v.get(id).and_then(Item::as_host)?;
            let target = crate::ssh::connection_proxy(v, h)
                .ok()
                .map(|proxy| (h.address.clone(), h.port, proxy));
            Some((id.clone(), target))
        })
        .collect()
}

/// HOST-10: TCP connect only (no authentication), 3 s timeout, probed concurrently.
#[tauri::command]
#[specta::specta]
pub async fn hosts_probe(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> AppResult<Vec<ProbeResult>> {
    let targets = state.with_unlocked(|v| Ok(probe_targets(v, &ids)))?;
    let probes = targets.into_iter().map(|(id, target)| async move {
        // Hosts behind a jump host are usually unreachable directly; report them as
        // unknown/offline.
        let latency = match target {
            Some((host, port, proxy)) => {
                hatoba_ssh::tcp_probe_via(proxy.as_ref(), &host, port, Duration::from_secs(3)).await
            }
            None => None,
        };
        ProbeResult {
            id,
            online: latency.is_some(),
            latency_ms: latency,
        }
    });
    Ok(futures::future::join_all(probes).await)
}

/// SEC-08: copies a saved host password to the clipboard from Rust (it never reaches the WebView)
/// and clears it after the device's delay (90 s by default; 0 never clears it) if the clipboard
/// still holds it.
#[tauri::command]
#[specta::specta]
pub fn host_copy_password(app: AppHandle, state: State<'_, AppState>, id: String) -> AppResult<()> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let (password, prefs) = state.with_unlocked(|v| match find_host(v, &id)?.auth {
        HostAuth::Password { password } if !password.is_empty() => Ok((password, v.local_prefs()?)),
        _ => Err(AppError::invalid(
            "password",
            "this host has no saved password",
        )),
    })?;
    let clear_secs = prefs
        .map(|json| LocalPrefs::from_stored(&json))
        .unwrap_or_default()
        .clipboard_clear_secs;
    app.clipboard()
        .write_text(password.as_str())
        .map_err(|e| AppError::internal(e.to_string()))?;
    if clear_secs == 0 {
        return Ok(());
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(clear_secs.into())).await;
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
    use crate::dto::HostEnvVar;

    /// An unlocked vault in memory. Its master password is made up for each run.
    fn vault() -> Vault {
        let mut v = Vault::open_in_memory().unwrap();
        v.create_with_params(&hatoba_core::new_id(), KdfParams::for_tests())
            .unwrap();
        v
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
            proxy_mode: ProxyMode::DeviceDefault,
            proxy_id: None,
            note: String::new(),
            ai_notes,
            env: Vec::new(),
        }
    }

    fn env_input(vars: &[(&str, &str)]) -> HostInput {
        HostInput {
            env: vars
                .iter()
                .map(|(name, value)| HostEnvVar {
                    name: (*name).into(),
                    value: (*value).into(),
                })
                .collect(),
            ..input(String::new())
        }
    }

    #[test]
    fn hosts_whose_proxy_was_deleted_are_still_reported() {
        let mut v = vault();
        let host = host_from_input(&input(String::new()), None, &v).unwrap();
        let id = v.put(None, Item::Host(host)).unwrap();
        // This device's default proxy was deleted on another device.
        v.set_local_prefs(r#"{"default_proxy_id":"gone"}"#).unwrap();
        let ids = vec![id.clone(), "no-such-host".to_owned()];
        let targets = probe_targets(&v, &ids);
        assert_eq!(targets.len(), 1, "unknown ids are skipped");
        assert_eq!(targets[0].0, id);
        assert!(targets[0].1.is_none(), "no probe around a deleted proxy");

        v.set_local_prefs(r#"{"default_proxy_id":null}"#).unwrap();
        let targets = probe_targets(&v, &ids);
        let (address, port, proxy) = targets[0].1.clone().unwrap();
        assert_eq!(
            (address.as_str(), port, proxy.is_none()),
            ("10.0.0.5", 22, true)
        );
    }

    #[test]
    fn env_is_saved_with_the_host_and_checked() {
        let mut v = vault();
        // SSH-14: names are trimmed, values kept as typed.
        let host = host_from_input(
            &env_input(&[(" TZ ", "Asia/Tokyo"), ("GREETING", " hi ")]),
            None,
            &v,
        )
        .unwrap();
        assert_eq!(
            host.env,
            [
                EnvVar::new("TZ", "Asia/Tokyo"),
                EnvVar::new("GREETING", " hi ")
            ]
        );
        let id = v.put(None, Item::Host(host)).unwrap();
        let view = host_view(&id, &find_host(&v, &id).unwrap(), &v);
        assert_eq!(view.env.len(), 2);
        assert_eq!(view.env[1].value, " hi ");

        for bad in [
            env_input(&[("1BAD", "x")]),
            env_input(&[("", "x")]),
            env_input(&[("A", "1"), (" A", "2")]),
            env_input(&[("A", "line\nbreak")]),
        ] {
            let err = host_from_input(&bad, None, &v).unwrap_err();
            assert_eq!(err.field.as_deref(), Some("env"), "{}", err.detail);
        }
    }

    #[test]
    fn a_host_too_large_to_sync_is_refused_while_sync_is_on() {
        let mut v = vault();
        let host = |v: &Vault, note: String| {
            Item::Host(
                host_from_input(
                    &HostInput {
                        note,
                        ..input(String::new())
                    },
                    None,
                    v,
                )
                .unwrap(),
            )
        };
        let small = host(&v, "n".repeat(30_000));
        let large = host(&v, "n".repeat(MAX_ITEM_PLAINTEXT_BYTES));
        assert!(check_item_size(&v, &large).is_ok());

        let config = hatoba_core::sync::SyncConfig::Worker {
            url: "https://sync.example.workers.dev".into(),
            deployment: None,
        };
        v.set_sync_config(Some(&config)).unwrap();
        assert!(check_item_size(&v, &small).is_ok());
        let err = check_item_size(&v, &large).unwrap_err();
        assert_eq!(err.field.as_deref(), Some("host"));
    }

    #[test]
    fn a_host_without_a_name_is_named_after_its_address() {
        let v = vault();
        let unnamed = HostInput {
            name: "  ".into(),
            ..input(String::new())
        };
        assert_eq!(
            host_from_input(&unnamed, None, &v).unwrap().name,
            "10.0.0.5"
        );
        let named = host_from_input(&input(String::new()), None, &v).unwrap();
        assert_eq!(named.name, "prod-db");

        // A long address gives a name within the limit, which the host editor saves again.
        let long = format!("{}.example.com", "a".repeat(200));
        let unnamed = HostInput {
            name: String::new(),
            address: long.clone(),
            ..input(String::new())
        };
        let host = host_from_input(&unnamed, None, &v).unwrap();
        assert_eq!(host.name, long[..MAX_NAME_CHARS]);
        let again = HostInput {
            name: host.name.clone(),
            ..unnamed
        };
        assert_eq!(host_from_input(&again, None, &v).unwrap().name, host.name);
    }

    #[test]
    fn ai_notes_are_saved_with_the_host_up_to_their_limit() {
        let mut v = vault();
        // AI-37: kept as typed, line breaks and placeholders included.
        let notes = "PostgreSQL 16 primary.\nRestart with `systemctl restart <unit>`.";
        let host = host_from_input(&input(notes.into()), None, &v).unwrap();
        assert_eq!(host.ai_notes, notes);
        let id = v.put(None, Item::Host(host)).unwrap();
        assert_eq!(
            host_view(&id, &find_host(&v, &id).unwrap(), &v).ai_notes,
            notes
        );

        assert!(host_from_input(&input("x".repeat(MAX_HOST_AI_NOTES_BYTES)), None, &v).is_ok());
        let err =
            host_from_input(&input("x".repeat(MAX_HOST_AI_NOTES_BYTES + 1)), None, &v).unwrap_err();
        assert_eq!(err.field.as_deref(), Some("ai_notes"));
    }
}
