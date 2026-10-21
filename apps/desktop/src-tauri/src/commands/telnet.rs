//! Telnet hosts and their terminal tabs (spec §7.6): saving them, and relaying a tab's
//! connection over its channel in the frames SSH tabs use (§10.3).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use hatoba_core::model::{Item, MAX_HOST_AI_NOTES_BYTES, TelnetHost};
use hatoba_core::vault::Vault;
use hatoba_telnet::{AutoLogin, TelnetEvent, TelnetSession};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use crate::commands::hosts::{MAX_NAME_CHARS, check_item_size, proxy_from_input, tags_from_input};
use crate::commands::ssh::{FRAME_CLOSED, FRAME_DATA, TermFrame, emit_state};
use crate::convert::telnet_host_view;
use crate::dto::{LocalOpenOptions, SessionState, TelnetHostInput, TelnetHostView};
use crate::error::{AppError, AppResult, SshErrorKind};
use crate::state::{AppState, now_ms};
use crate::sync;
use crate::telnet::TelnetLive;

/// How long name resolution and the connect may take, as for SSH (SSH-01).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// The longest saved user name or password, in bytes.
const MAX_CREDENTIAL_BYTES: usize = 255;

fn find(vault: &Vault, id: &str) -> AppResult<TelnetHost> {
    vault
        .get(id)
        .and_then(Item::as_telnet_host)
        .cloned()
        .ok_or_else(|| AppError::not_found("host"))
}

/// Validates the form and builds the stored model, as `host_from_input` does for SSH hosts.
pub fn telnet_host_from_input(
    input: &TelnetHostInput,
    existing: Option<&TelnetHost>,
    vault: &Vault,
) -> AppResult<TelnetHost> {
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
    let username = input.username.trim();
    if username.len() > MAX_CREDENTIAL_BYTES || username.contains(char::is_control) {
        return Err(AppError::invalid("username", "username is not valid"));
    }
    // HOST-08: no password in the input keeps the saved one; an empty one removes it.
    let password = match (&input.password, existing) {
        (Some(new), _) => Zeroizing::new(new.clone()),
        (None, Some(saved)) => saved.password.clone(),
        (None, None) => Zeroizing::new(String::new()),
    };
    if password.len() > MAX_CREDENTIAL_BYTES || password.contains(char::is_control) {
        return Err(AppError::invalid("password", "password is not valid"));
    }
    if input.ai_notes.len() > MAX_HOST_AI_NOTES_BYTES {
        return Err(AppError::invalid(
            "ai_notes",
            "AI notes are limited to 16 KB",
        ));
    }
    if let Some(group) = &input.group_id
        && vault.get(group).and_then(Item::as_group).is_none()
    {
        return Err(AppError::invalid("group_id", "group not found"));
    }
    let proxy = proxy_from_input(input.proxy_mode, input.proxy_id.as_deref(), vault)?;
    Ok(TelnetHost {
        // HOST-01: a host without a name is named after its address.
        name: if name.is_empty() {
            address.chars().take(MAX_NAME_CHARS).collect()
        } else {
            name.to_owned()
        },
        address: address.to_owned(),
        port: input.port,
        username: username.to_owned(),
        password,
        group_id: input.group_id.clone(),
        tags: tags_from_input(&input.tags),
        favorite: input.favorite,
        proxy,
        note: input.note.clone(),
        ai_notes: input.ai_notes.clone(),
        updated_at: 0,
    })
}

#[tauri::command]
#[specta::specta]
pub fn telnet_hosts_list(state: State<'_, AppState>) -> AppResult<Vec<TelnetHostView>> {
    state.with_unlocked(|v| {
        let mut hosts: Vec<TelnetHostView> = v
            .telnet_hosts()
            .iter()
            .map(|(id, h)| telnet_host_view(id, h, v))
            .collect();
        hosts.sort_by_cached_key(|h| h.name.to_lowercase());
        Ok(hosts)
    })
}

#[tauri::command]
#[specta::specta]
pub fn telnet_host_save(
    app: AppHandle,
    state: State<'_, AppState>,
    input: TelnetHostInput,
) -> AppResult<TelnetHostView> {
    let view = state.with_unlocked(|v| {
        let existing = match &input.id {
            Some(id) => Some(find(v, id)?),
            None => None,
        };
        let item = Item::TelnetHost(telnet_host_from_input(&input, existing.as_ref(), v)?);
        check_item_size(v, &item)?;
        let id = v.put(input.id.as_deref(), item)?;
        Ok(telnet_host_view(&id, &find(v, &id)?, v))
    })?;
    sync::local_change(&app);
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub fn telnet_host_delete(app: AppHandle, state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.with_unlocked(|v| Ok(v.delete_telnet_host(&id)?))?;
    sync::local_change(&app);
    // Its tabs close with it.
    for live in state.telnet.take_host(&id) {
        live.close();
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn telnet_host_duplicate(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<TelnetHostView> {
    let view = state.with_unlocked(|v| {
        let mut host = find(v, &id)?;
        let names: HashSet<String> = v
            .telnet_hosts()
            .into_iter()
            .map(|(_, h)| h.name)
            .chain(v.hosts().into_iter().map(|(_, h)| h.name))
            .collect();
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
        let new_id = v.put(None, Item::TelnetHost(host))?;
        Ok(telnet_host_view(&new_id, &find(v, &new_id)?, v))
    })?;
    sync::local_change(&app);
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub fn telnet_host_set_favorite(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    favorite: bool,
) -> AppResult<()> {
    state.with_unlocked(|v| {
        let mut host = find(v, &id)?;
        if host.favorite != favorite {
            host.favorite = favorite;
            v.put(Some(&id), Item::TelnetHost(host))?;
        }
        Ok(())
    })?;
    sync::local_change(&app);
    Ok(())
}

/// SEC-08: copies a Telnet host's saved password to the clipboard from Rust, as
/// `host_copy_password` does for SSH hosts.
#[tauri::command]
#[specta::specta]
pub fn telnet_host_copy_password(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    let password = state.with_unlocked(|v| {
        let password = find(v, &id)?.password;
        if password.is_empty() {
            return Err(AppError::invalid(
                "password",
                "this host has no saved password",
            ));
        }
        Ok(password)
    })?;
    crate::commands::hosts::copy_secret(&app, &state, password)
}

/// Connects a tab to the Telnet host `host_id` (TELNET-02) and returns the session id. Its
/// output streams on `channel` until the connection ends or `telnet_close`.
#[tauri::command]
#[specta::specta]
pub async fn telnet_open(
    app: AppHandle,
    state: State<'_, AppState>,
    host_id: String,
    options: LocalOpenOptions,
    channel: Channel<TermFrame>,
) -> AppResult<String> {
    let (host, proxy) = state.with_unlocked(|v| {
        let host = find(v, &host_id)?;
        let proxy = crate::ssh::resolve_proxy(v, &host.proxy)?;
        Ok((host, proxy))
    })?;
    let stream = hatoba_ssh::dial(proxy.as_ref(), &host.address, host.port, CONNECT_TIMEOUT)
        .await
        .map_err(AppError::from)?;
    let cols = u16::try_from(options.cols.clamp(10, 1000)).unwrap_or(80);
    let rows = u16::try_from(options.rows.clamp(2, 500)).unwrap_or(24);
    // TELNET-03: the saved user name and password are typed at the login prompts.
    let login = AutoLogin::new(
        Some(host.username.clone()),
        Some(host.password.as_str().to_owned()),
    );
    let (session, events) = TelnetSession::start(stream, cols, rows, login);
    let session_id = uuid::Uuid::now_v7().to_string();
    let live = Arc::new(TelnetLive::new(session, host_id.clone()));
    state.telnet.insert(session_id.clone(), live.clone());
    if let Err(e) = state.with_unlocked(|v| Ok(v.set_last_connected(&host_id, now_ms())?)) {
        tracing::debug!("last-connected time not recorded: {}", e.detail);
    }
    tracing::info!("telnet connected");
    emit_state(
        &app,
        &session_id,
        Some(&host_id),
        SessionState::Connected,
        |_| {},
    );
    pump(app, session_id.clone(), live, events, channel);
    Ok(session_id)
}

/// Relays the connection's output to the WebView until it ends, then drops the session.
fn pump(
    app: AppHandle,
    session_id: String,
    live: Arc<TelnetLive>,
    mut events: mpsc::Receiver<TelnetEvent>,
    channel: Channel<TermFrame>,
) {
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                TelnetEvent::Data(bytes) => {
                    if channel.send(TermFrame::new(FRAME_DATA, &bytes)).is_err() {
                        // The WebView went away (reload / window closed): the connection goes too.
                        live.close();
                        break;
                    }
                }
                TelnetEvent::Closed { error } => {
                    // A connection Hatoba closed, on lock or quit, ended without an error.
                    let error = error.filter(|_| !live.is_closed());
                    let _ = channel.send(TermFrame::new(FRAME_CLOSED, b""));
                    emit_state(
                        &app,
                        &session_id,
                        Some(&live.host_id),
                        SessionState::Disconnected,
                        |e| {
                            e.error = error.map(|m| AppError::ssh(SshErrorKind::Disconnected, m));
                        },
                    );
                    break;
                }
            }
        }
        crate::state::state(&app)
            .telnet
            .remove_if_same(&session_id, &live);
        live.close();
    });
}

#[tauri::command]
#[specta::specta]
pub async fn telnet_write(
    state: State<'_, AppState>,
    session_id: String,
    data: String,
) -> AppResult<()> {
    write(&state, &session_id, data.into_bytes())
}

/// Writes bytes as they are, for xterm's binary mouse reports (see `ssh_write_bytes`).
#[tauri::command]
#[specta::specta]
pub async fn telnet_write_bytes(
    state: State<'_, AppState>,
    session_id: String,
    data: Vec<u8>,
) -> AppResult<()> {
    write(&state, &session_id, data)
}

fn write(state: &AppState, session_id: &str, data: Vec<u8>) -> AppResult<()> {
    // SEC-03: like SSH sessions, Telnet tabs take no input behind the lock screen.
    if !state.vault().is_unlocked() {
        return Err(AppError::locked());
    }
    state
        .telnet
        .get(session_id)?
        .session
        .write(data)
        .map_err(|_| AppError::not_found("session"))
}

#[tauri::command]
#[specta::specta]
pub async fn telnet_resize(
    state: State<'_, AppState>,
    session_id: String,
    cols: u32,
    rows: u32,
) -> AppResult<()> {
    let live = state.telnet.get(&session_id)?;
    live.session.resize(
        u16::try_from(cols.clamp(10, 1000)).unwrap_or(80),
        u16::try_from(rows.clamp(2, 500)).unwrap_or(24),
    );
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn telnet_close(state: State<'_, AppState>, session_id: String) -> AppResult<()> {
    if let Some(live) = state.telnet.take(&session_id) {
        live.close();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use hatoba_core::KdfParams;

    use super::*;
    use crate::dto::ProxyMode;

    fn vault() -> Vault {
        let mut v = Vault::open_in_memory().unwrap();
        v.create_with_params(&hatoba_core::new_id(), KdfParams::for_tests())
            .unwrap();
        v
    }

    fn input() -> TelnetHostInput {
        TelnetHostInput {
            id: None,
            name: String::new(),
            address: " 10.0.0.2 ".into(),
            port: 23,
            username: "admin".into(),
            password: Some("pw".into()),
            group_id: None,
            tags: vec!["lab".into(), " lab ".into(), String::new()],
            favorite: false,
            proxy_mode: ProxyMode::DeviceDefault,
            proxy_id: None,
            note: String::new(),
            ai_notes: String::new(),
        }
    }

    #[test]
    fn a_telnet_host_is_named_after_its_address_and_keeps_its_password() {
        let v = vault();
        let host = telnet_host_from_input(&input(), None, &v).unwrap();
        assert_eq!(host.name, "10.0.0.2");
        assert_eq!(host.tags, ["lab"]);
        let kept = telnet_host_from_input(
            &TelnetHostInput {
                password: None,
                ..input()
            },
            Some(&host),
            &v,
        )
        .unwrap();
        assert_eq!(kept.password.as_str(), "pw");
        let removed = telnet_host_from_input(
            &TelnetHostInput {
                password: Some(String::new()),
                ..input()
            },
            Some(&host),
            &v,
        )
        .unwrap();
        assert!(removed.password.is_empty());
    }

    #[test]
    fn a_telnet_host_form_is_checked() {
        let v = vault();
        let bad = |edit: fn(&mut TelnetHostInput)| {
            let mut i = input();
            edit(&mut i);
            telnet_host_from_input(&i, None, &v).unwrap_err().field
        };
        assert_eq!(bad(|i| i.address = " ".into()).as_deref(), Some("address"));
        assert_eq!(bad(|i| i.port = 0).as_deref(), Some("port"));
        assert_eq!(
            bad(|i| i.password = Some("a\rb".into())).as_deref(),
            Some("password")
        );
        assert_eq!(
            bad(|i| i.group_id = Some("nope".into())).as_deref(),
            Some("group_id")
        );
        assert_eq!(
            bad(|i| i.proxy_mode = ProxyMode::Proxy).as_deref(),
            Some("proxy_id")
        );
    }
}
