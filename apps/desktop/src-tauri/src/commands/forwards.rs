//! Port forwarding: local (FWD-01), remote (FWD-03) and dynamic SOCKS (FWD-04), auto-start with
//! the connection (FWD-02), and forwards that run without a terminal (FWD-05).

use hatoba_core::model::{ForwardKind, Item, PortForward};
use hatoba_ssh::{DynamicForward, LocalForward, RemoteForward};
use tauri::{AppHandle, State};
use tauri_specta::Event;
use zeroize::Zeroizing;

use crate::dto::{
    ForwardInput, ForwardState, ForwardStatusEvent, ForwardView, TunnelOptions, TunnelsEvent,
};
use crate::error::{AppError, AppResult};
use crate::ssh::{LiveSession, Overrides, RunningForward};
use crate::state::{AppState, state as app_state};
use crate::{sync, tunnel};

fn view(id: &str, f: &PortForward) -> ForwardView {
    ForwardView {
        id: id.to_owned(),
        host_id: f.host_id.clone(),
        kind: f.kind.into(),
        bind_address: f.bind_address.clone(),
        bind_port: f.bind_port,
        dest_host: f.dest_host.clone().unwrap_or_default(),
        dest_port: f.dest_port.unwrap_or_default(),
        auto_start: f.auto_start,
    }
}

#[tauri::command]
#[specta::specta]
pub fn forwards_list(state: State<'_, AppState>, host_id: String) -> AppResult<Vec<ForwardView>> {
    state.with_unlocked(|v| {
        let mut list: Vec<ForwardView> = v
            .forwards()
            .iter()
            .filter(|(_, f)| f.host_id == host_id)
            .map(|(id, f)| view(id, f))
            .collect();
        list.sort_by_key(|f| (f.bind_port, f.dest_port));
        Ok(list)
    })
}

/// Every host's forwards, for the **Port Forwarding** page, by host and then as `forwards_list`
/// sorts them.
#[tauri::command]
#[specta::specta]
pub fn forwards_list_all(state: State<'_, AppState>) -> AppResult<Vec<ForwardView>> {
    state.with_unlocked(|v| {
        let mut list: Vec<ForwardView> = v
            .forwards()
            .iter()
            .filter(|(_, f)| f.kind == ForwardKind::Local)
            .map(|(id, f)| view(id, f))
            .collect();
        list.sort_by(|a, b| {
            (&a.host_id, a.bind_port, a.dest_port).cmp(&(&b.host_id, b.bind_port, b.dest_port))
        });
        Ok(list)
    })
}

#[tauri::command]
#[specta::specta]
pub fn forward_save(
    app: AppHandle,
    state: State<'_, AppState>,
    input: ForwardInput,
) -> AppResult<ForwardView> {
    let kind = ForwardKind::from(input.kind);
    // A remote forward listens where `ssh -R` does by default.
    let bind_address = match (input.bind_address.trim(), kind) {
        ("", ForwardKind::Remote) => "localhost".to_owned(),
        ("", _) => "127.0.0.1".to_owned(),
        (a, _) => a.to_owned(),
    };
    if bind_address.chars().any(char::is_whitespace) {
        return Err(AppError::invalid("bind_address", "invalid bind address"));
    }
    let dest = match kind {
        ForwardKind::Dynamic => None,
        ForwardKind::Local | ForwardKind::Remote => {
            let dest_host = input.dest_host.trim().to_owned();
            if dest_host.is_empty() || dest_host.chars().any(char::is_whitespace) {
                return Err(AppError::invalid(
                    "dest_host",
                    "destination host is required",
                ));
            }
            if input.dest_port == 0 {
                return Err(AppError::invalid(
                    "dest_port",
                    "port must be between 1 and 65535",
                ));
            }
            Some((dest_host, input.dest_port))
        }
    };
    let saved = state.with_unlocked(|v| {
        v.get(&input.host_id)
            .and_then(Item::as_host)
            .ok_or_else(|| AppError::not_found("host"))?;
        let (dest_host, dest_port) = dest.unzip();
        let forward = PortForward {
            host_id: input.host_id.clone(),
            kind,
            bind_address,
            bind_port: input.bind_port,
            dest_host,
            dest_port,
            auto_start: input.auto_start,
            updated_at: 0,
        };
        let id = v.put(input.id.as_deref(), Item::Forward(forward))?;
        let f = v
            .get(&id)
            .and_then(Item::as_forward)
            .cloned()
            .ok_or_else(|| AppError::not_found("forward"))?;
        Ok(view(&id, &f))
    })?;
    sync::local_change(&app);
    Ok(saved)
}

/// Deletes the rule, and stops it wherever it runs.
#[tauri::command]
#[specta::specta]
pub async fn forward_delete(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    let host_id = state.with_unlocked(|v| {
        let host_id = v
            .get(&id)
            .and_then(Item::as_forward)
            .map(|f| f.host_id.clone())
            .ok_or_else(|| AppError::not_found("forward"))?;
        v.delete(&id)?;
        Ok(host_id)
    })?;
    sync::local_change(&app);
    for (session_id, live) in state.ssh.terminals_for_host(&host_id) {
        if live.stop_forward(&id) {
            emit(&app, &session_id, &id, ForwardState::Stopped, None, None);
        }
    }
    tunnel::stop(&app, &host_id, &id).await;
    Ok(())
}

fn emit(
    app: &AppHandle,
    session_id: &str,
    forward_id: &str,
    state: ForwardState,
    port: Option<u16>,
    error: Option<String>,
) {
    let _ = ForwardStatusEvent {
        session_id: session_id.to_owned(),
        forward_id: forward_id.to_owned(),
        state,
        port,
        error,
    }
    .emit(app);
}

/// `host:port`, with an IPv6 address in brackets.
fn host_port(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Starts one saved forward on a live session and returns the port it listens on.
pub async fn open(live: &LiveSession, forward_id: &str, f: &PortForward) -> AppResult<u16> {
    let bind_address = f.bind_address.clone();
    let dest_host = f.dest_host.clone().unwrap_or_default();
    let dest_port = f.dest_port.unwrap_or_default();
    let dest = host_port(&dest_host, dest_port);
    let (handle, address, dest) = match f.kind {
        ForwardKind::Local => {
            let spec = LocalForward {
                bind_address,
                bind_port: f.bind_port,
                dest_host,
                dest_port,
            };
            let handle = live.session.local_forward(spec).await?;
            let address = handle
                .local_addr()
                .map(tunnel::reachable)
                .unwrap_or_default();
            (handle, address, dest)
        }
        ForwardKind::Remote => {
            let spec = RemoteForward {
                bind_address,
                bind_port: f.bind_port,
                dest_host,
                dest_port,
            };
            let handle = live.session.remote_forward(spec).await?;
            let address = host_port(&f.bind_address, handle.port());
            (handle, address, dest)
        }
        ForwardKind::Dynamic => {
            let spec = DynamicForward {
                bind_address,
                bind_port: f.bind_port,
            };
            let handle = live.session.dynamic_forward(spec).await?;
            let address = handle
                .local_addr()
                .map(tunnel::reachable)
                .unwrap_or_default();
            (handle, address, String::new())
        }
    };
    let port = handle.port();
    live.add_forward(
        forward_id,
        RunningForward {
            handle,
            kind: f.kind.into(),
            address,
            dest,
        },
    );
    Ok(port)
}

/// Starts one saved forward on a terminal's session and reports its state.
pub async fn start(
    app: &AppHandle,
    session_id: &str,
    live: &LiveSession,
    forward_id: &str,
    f: &PortForward,
) -> AppResult<u16> {
    match open(live, forward_id, f).await {
        Ok(port) => {
            emit(
                app,
                session_id,
                forward_id,
                ForwardState::Running,
                Some(port),
                None,
            );
            Ok(port)
        }
        Err(e) => {
            emit(
                app,
                session_id,
                forward_id,
                ForwardState::Failed,
                None,
                Some(e.detail.clone()),
            );
            Err(e)
        }
    }
}

/// FWD-02: called after a session connects. A forward already running without a terminal
/// (FWD-05) stays there.
pub async fn start_auto(app: &AppHandle, session_id: &str, live: &LiveSession, host_id: &str) {
    let state = app_state(app);
    let auto: Vec<(String, PortForward)> = state
        .with_unlocked(|v| {
            Ok(v.forwards()
                .into_iter()
                .filter(|(id, f)| {
                    f.host_id == host_id
                        && f.auto_start
                        && tunnel::running_port(&state, host_id, id).is_none()
                })
                .collect())
        })
        .unwrap_or_default();
    for (id, f) in auto {
        if let Err(e) = start(app, session_id, live, &id, &f).await {
            tracing::warn!(forward_id = %id, "auto-start forward failed: {}", e.detail);
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn forward_start(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    forward_id: String,
) -> AppResult<u16> {
    let live = state.ssh.get(&session_id)?;
    let f = state.with_unlocked(|v| {
        v.get(&forward_id)
            .and_then(Item::as_forward)
            .cloned()
            .ok_or_else(|| AppError::not_found("forward"))
    })?;
    if let Some(port) = live.forward_port(&forward_id) {
        return Ok(port);
    }
    start(&app, &session_id, &live, &forward_id, &f).await
}

#[tauri::command]
#[specta::specta]
pub fn forward_stop(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    forward_id: String,
) -> AppResult<()> {
    let live = state.ssh.get(&session_id)?;
    live.stop_forward(&forward_id);
    emit(
        &app,
        &session_id,
        &forward_id,
        ForwardState::Stopped,
        None,
        None,
    );
    Ok(())
}

/// Forwards currently running on a session: `(forward_id, port)`.
#[tauri::command]
#[specta::specta]
pub fn forwards_active(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<Vec<(String, u16)>> {
    Ok(state.ssh.get(&session_id)?.active_forwards())
}

/// FWD-05: starts the saved forward without a terminal, on the host's connection for forwards,
/// and returns the port it listens on.
#[tauri::command]
#[specta::specta]
pub async fn tunnel_forward_start(
    app: AppHandle,
    host_id: String,
    forward_id: String,
    options: TunnelOptions,
) -> AppResult<u16> {
    let overrides = Overrides {
        password: options
            .password
            .filter(|p| !p.is_empty())
            .map(Zeroizing::new),
        passphrase: options
            .passphrase
            .filter(|p| !p.is_empty())
            .map(Zeroizing::new),
        ask_password: false,
    };
    tunnel::start(&app, &host_id, &forward_id, &overrides).await
}

/// FWD-05: stops a forward running without a terminal; the host's connection for forwards closes
/// with its last one.
#[tauri::command]
#[specta::specta]
pub async fn tunnel_forward_stop(app: AppHandle, host_id: String, forward_id: String) {
    tunnel::stop(&app, &host_id, &forward_id).await;
}

/// FWD-05: every forward running without a terminal.
#[tauri::command]
#[specta::specta]
pub fn tunnels_list(state: State<'_, AppState>) -> TunnelsEvent {
    tunnel::list(&state)
}
