//! Port forwards that run without a terminal (FWD-05). Each saved host gets one connection with
//! no shell, opened by the first forward started in the host editor and closed with the last one
//! stopped. Hidden to the tray, Hatoba keeps them running like terminals (WIN-10); the tray menu
//! lists them, and a connection that ends on its own tells the WebView, which notifies.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use hatoba_core::model::{ForwardKind, Item};
use hatoba_ssh::{ConnectConfig, SshSession};
use tauri::AppHandle;
use tauri_specta::Event;

use crate::dto::{TunnelForwardView, TunnelLostEvent, TunnelView, TunnelsEvent};
use crate::error::{AppError, AppResult};
use crate::ssh::{LiveSession, Overrides, build_config, connect_with};
use crate::state::{AppState, now_ms, state};

#[derive(Default)]
pub struct Tunnels {
    /// One start or stop at a time per host, so that two starts never open two connections.
    hosts: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    version: AtomicU64,
}

impl Tunnels {
    fn host_gate(&self, host_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut hosts = self.hosts.lock().unwrap_or_else(|p| p.into_inner());
        Arc::clone(hosts.entry(host_id.to_owned()).or_default())
    }
}

/// Where the host's connection for forwards comes from.
enum Via {
    /// HOST-14: the dashboard's, which then reads over it, as a terminal takes it.
    Dashboard(SshSession),
    Connect(Box<ConnectConfig>),
}

/// Starts the saved forward on the host's connection for forwards, connecting first when there is
/// none, and returns the local port. A forward already running there returns its port.
pub async fn start(
    app: &AppHandle,
    host_id: &str,
    forward_id: &str,
    overrides: &Overrides,
) -> AppResult<u16> {
    let state = state(app);
    let gate = state.tunnels.host_gate(host_id);
    let _held = gate.lock().await;
    let forward = state.with_unlocked(|v| {
        v.get(forward_id)
            .and_then(Item::as_forward)
            .filter(|f| f.host_id == host_id && f.kind == ForwardKind::Local)
            .cloned()
            .ok_or_else(|| AppError::not_found("forward"))
    })?;
    let (session_id, live) = match state.ssh.tunnel_for_host(host_id) {
        Some(open) => open,
        None => open(app, &state, host_id, overrides).await?,
    };
    if let Some(port) = live.forward_port(forward_id) {
        return Ok(port);
    }
    let started = crate::commands::forwards::open(&live, forward_id, &forward).await;
    // A connection opened for this forward alone goes with it.
    if started.is_err() && live.forward_count() == 0 && state.ssh.remove_if_same(&session_id, &live)
    {
        live.close().await;
    }
    changed(app);
    started
}

/// Stops the forward on the host's connection for forwards, which closes once none is left.
pub async fn stop(app: &AppHandle, host_id: &str, forward_id: &str) {
    let state = state(app);
    // Not behind a start that is still connecting: there is nothing to stop yet.
    if state.ssh.tunnel_for_host(host_id).is_none() {
        return;
    }
    let gate = state.tunnels.host_gate(host_id);
    let _held = gate.lock().await;
    let Some((session_id, live)) = state.ssh.tunnel_for_host(host_id) else {
        return;
    };
    if !live.stop_forward(forward_id) {
        return;
    }
    if live.forward_count() == 0 && state.ssh.remove_if_same(&session_id, &live) {
        live.close().await;
    }
    changed(app);
}

/// The forward's local port while it runs without a terminal.
pub fn running_port(state: &AppState, host_id: &str, forward_id: &str) -> Option<u16> {
    let (_, live) = state.ssh.tunnel_for_host(host_id)?;
    live.forward_port(forward_id)
}

/// Every forward running without a terminal, by host as the connections were made.
pub fn list(state: &AppState) -> TunnelsEvent {
    let tunnels = state
        .ssh
        .tunnels()
        .into_iter()
        .filter_map(|(_, live)| {
            let forwards: Vec<TunnelForwardView> = live
                .running_forwards()
                .into_iter()
                .map(|(forward_id, local_port, dest)| TunnelForwardView {
                    forward_id,
                    local_port,
                    dest,
                })
                .collect();
            (!forwards.is_empty()).then(|| TunnelView {
                host_id: live.host_id.clone().unwrap_or_default(),
                host_name: live.tunnel.clone().unwrap_or_default(),
                forwards,
            })
        })
        .collect();
    TunnelsEvent {
        version: state.tunnels.version.load(Ordering::Relaxed),
        tunnels,
    }
}

/// Tells the WebView and the tray menu what runs now.
pub fn changed(app: &AppHandle) {
    let state = state(app);
    state.tunnels.version.fetch_add(1, Ordering::Relaxed);
    let event = list(&state);
    crate::tray::set_tunnels(app, event.version, &event.tunnels);
    let _ = event.emit(app);
}

/// Connects to the host without a shell and registers the connection.
async fn open(
    app: &AppHandle,
    state: &AppState,
    host_id: &str,
    overrides: &Overrides,
) -> AppResult<(String, Arc<LiveSession>)> {
    let (via, config, name) = state.with_unlocked(|v| {
        let host = v
            .get(host_id)
            .and_then(Item::as_host)
            .cloned()
            .ok_or_else(|| AppError::not_found("host"))?;
        let config = crate::dashboard::config_version(v, &host);
        let via = match state.dashboard.take_connection(host_id, &config) {
            Some(session) => Via::Dashboard(session),
            None => Via::Connect(Box::new(build_config(
                v,
                &host,
                Some(host_id),
                Some(overrides),
            )?)),
        };
        Ok((via, config, host.name.clone()))
    })?;
    let session = match via {
        Via::Dashboard(session) => session,
        Via::Connect(cfg) => connect_with(app, *cfg, None).await?,
    };
    let session_id = uuid::Uuid::now_v7().to_string();
    let live = Arc::new(LiveSession::tunnel(
        session.clone(),
        host_id.to_owned(),
        config,
        name,
    ));
    state.ssh.insert(session_id.clone(), Arc::clone(&live));
    // HOST-14: the dashboard reads the host over this connection from now on.
    state.dashboard.wake(host_id);
    // HOST-06, HOST-11, as for a terminal.
    let os = session.server_os().map(hatoba_ssh::ServerOs::as_str);
    if let Err(e) = state.with_unlocked(|v| {
        v.set_last_connected(host_id, now_ms())?;
        Ok(v.set_host_os(host_id, os)?)
    }) {
        tracing::debug!(
            "last-connected time and server OS not recorded: {}",
            e.detail
        );
    }
    watch(app.clone(), session_id.clone(), Arc::clone(&live));
    Ok((session_id, live))
}

/// Waits for the connection to end. One that ends while still registered ended on its own: a
/// stop, the lock (SEC-03) and quitting take it out first.
fn watch(app: AppHandle, session_id: String, live: Arc<LiveSession>) {
    tauri::async_runtime::spawn(async move {
        let err = live.session.closed().await;
        let state = state(&app);
        let lost = state.ssh.remove_if_same(&session_id, &live) && live.forward_count() > 0;
        live.close().await;
        if lost {
            let error = AppError::from(err);
            tracing::info!(kind = ?error.ssh_kind, "connection for port forwards lost");
            let _ = TunnelLostEvent {
                host_id: live.host_id.clone().unwrap_or_default(),
                host_name: live.tunnel.clone().unwrap_or_default(),
                error,
            }
            .emit(&app);
        }
        changed(&app);
    });
}
