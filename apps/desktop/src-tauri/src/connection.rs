//! One connection per saved host (SSH-16), which its terminal tabs and port forwards share. The
//! first tab or forward on a host connects, or takes over the dashboard's connection (HOST-14),
//! and starts the host's auto-start forwards (FWD-02); later ones join it without signing in
//! again, up to [`TABS_PER_CONNECTION`] tabs. Closing a tab closes only its shell: the connection
//! closes once no tab and no forward is left on it. Hidden to the tray, Hatoba keeps it running
//! (WIN-10); the tray menu lists its forwards, and a connection that ends on its own with forwards
//! on it tells the WebView, which notifies (FWD-05).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};

use hatoba_core::model::{ForwardKind, Item, PortForward};
use hatoba_ssh::{ConnectConfig, ForwardHandle, LocalForward, SshSession};
use tauri::AppHandle;
use tauri_specta::Event;

use crate::dto::{
    ConnectionsEvent, FailedForwardView, ForwardsLostEvent, HostConnectionView, RunningForwardView,
};
use crate::error::{AppError, AppResult, ErrorCode, SshErrorKind};
use crate::ssh::{Overrides, build_config, connect_with};
use crate::state::{AppState, now_ms, state};

/// The terminal tabs that share one connection; a further tab opens another. OpenSSH allows 10
/// sessions on a connection by default (`MaxSessions`), and each tab may also hold one for SFTP,
/// besides the resource usage readouts of the tab in front and of the dashboard.
pub const TABS_PER_CONNECTION: usize = 4;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

pub struct Connection {
    pub session: SshSession,
    pub host_id: String,
    /// `dashboard::config_version` when it connected: tabs and forwards join it only while the
    /// host connects that way.
    pub config: String,
    /// The host's name when it connected, for the tray menu and the notification.
    pub host_name: String,
    users: Mutex<Users>,
}

#[derive(Default)]
struct Users {
    tabs: usize,
    /// Forward starts in progress on it.
    starting: usize,
    forwards: HashMap<String, RunningForward>,
    /// Forwards that failed to start on it, until they start or stop.
    failed: HashMap<String, AppError>,
    /// The server refused a shell on it: no further tab joins it.
    full: bool,
    /// Nothing is left on it, and it is closing.
    closing: bool,
}

impl Users {
    fn unused(&self) -> bool {
        self.tabs == 0 && self.starting == 0 && self.forwards.is_empty()
    }
}

/// A forward listening on this computer, and the `host:port` it reaches from the server.
struct RunningForward {
    handle: ForwardHandle,
    dest: String,
}

/// A terminal tab's place on a connection. Dropping it leaves, which closes the connection when
/// nothing else is left on it.
pub struct Tab(Arc<Connection>);

impl Tab {
    pub fn connection(&self) -> &Arc<Connection> {
        &self.0
    }
}

impl Drop for Tab {
    fn drop(&mut self) {
        self.0.leave(|u| u.tabs -= 1);
    }
}

/// Keeps a connection open while a forward starts on it, or while a tab joins the connection
/// opened for it.
struct Starting(Arc<Connection>);

impl Drop for Starting {
    fn drop(&mut self) {
        self.0.leave(|u| u.starting -= 1);
    }
}

impl Connection {
    fn users(&self) -> MutexGuard<'_, Users> {
        lock(&self.users)
    }

    fn open(&self) -> bool {
        !self.session.is_closed() && !self.users().closing
    }

    fn join_tab(self: &Arc<Self>) -> Option<Tab> {
        let mut users = self.users();
        if users.closing
            || users.full
            || users.tabs >= TABS_PER_CONNECTION
            || self.session.is_closed()
        {
            return None;
        }
        users.tabs += 1;
        Some(Tab(Arc::clone(self)))
    }

    fn hold(self: &Arc<Self>) -> Option<Starting> {
        let mut users = self.users();
        if users.closing || self.session.is_closed() {
            return None;
        }
        users.starting += 1;
        Some(Starting(Arc::clone(self)))
    }

    /// The server refused a shell on it: later tabs open another connection.
    pub fn mark_full(&self) {
        self.users().full = true;
    }

    fn leave(&self, change: impl FnOnce(&mut Users)) {
        let close = {
            let mut users = self.users();
            change(&mut users);
            let close = users.unused() && !users.closing;
            users.closing |= close;
            close
        };
        if close {
            let session = self.session.clone();
            tauri::async_runtime::spawn(async move { session.disconnect().await });
        }
    }

    fn forward_port(&self, id: &str) -> Option<u16> {
        self.users()
            .forwards
            .get(id)
            .filter(|f| f.handle.is_running())
            .map(|f| f.handle.local_port())
    }

    /// Keeps the forward unless its rule went while it started: a delete removes the rule, then
    /// stops the forward under the same lock, so it either finds the forward or the forward finds
    /// no rule.
    fn add_forward(
        &self,
        state: &AppState,
        id: &str,
        handle: ForwardHandle,
        dest: String,
    ) -> AppResult<()> {
        let mut users = self.users();
        if let Err(e) = saved_forward(state, &self.host_id, id) {
            handle.stop();
            return Err(e);
        }
        users.failed.remove(id);
        if let Some(old) = users
            .forwards
            .insert(id.to_owned(), RunningForward { handle, dest })
        {
            old.handle.stop();
        }
        Ok(())
    }

    fn add_failure(&self, id: &str, error: AppError) {
        self.users().failed.insert(id.to_owned(), error);
    }

    /// Stops the forward and forgets its failure; `false` when it had neither here.
    fn stop_forward(&self, id: &str) -> bool {
        let mut found = false;
        self.leave(|users| {
            if let Some(running) = users.forwards.remove(id) {
                running.handle.stop();
                found = true;
            }
            found |= users.failed.remove(id).is_some();
        });
        found
    }

    /// Stops every forward, and returns how many there were.
    fn stop_forwards(&self) -> usize {
        let mut count = 0;
        self.leave(|users| {
            count = users.forwards.len();
            for (_, running) in users.forwards.drain() {
                running.handle.stop();
            }
            users.failed.clear();
        });
        count
    }

    /// SEC-03: stops its forwards and closes it, whatever runs on it.
    pub async fn close(&self) {
        self.stop_forwards();
        self.session.disconnect().await;
    }
}

#[derive(Default)]
pub struct Connections {
    /// Oldest first.
    list: Mutex<Vec<Arc<Connection>>>,
    /// One sign-in at a time per host, so that tabs and forwards that start together share one
    /// connection, and one forward start at a time.
    gates: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// The list's version, held while the list is read and sent, so that a version names one list.
    version: Mutex<u64>,
}

impl Connections {
    fn gate(&self, host_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(lock(&self.gates).entry(host_id.to_owned()).or_default())
    }

    /// The host's open connections, oldest first; with `config`, only those made the way the host
    /// connects now.
    fn of_host(&self, host_id: &str, config: Option<&str>) -> Vec<Arc<Connection>> {
        lock(&self.list)
            .iter()
            .filter(|c| c.host_id == host_id && config.is_none_or(|v| c.config == v) && c.open())
            .cloned()
            .collect()
    }

    /// A connection of the saved host made the way it connects now (`config`), which the
    /// dashboard (HOST-14) and the `hatoba` command (HOST-15) read over. The newest, when there
    /// are several.
    pub fn session_for(&self, host_id: &str, config: &str) -> Option<SshSession> {
        self.of_host(host_id, Some(config))
            .pop()
            .map(|c| c.session.clone())
    }

    fn remove(&self, conn: &Arc<Connection>) -> bool {
        let mut list = lock(&self.list);
        let before = list.len();
        list.retain(|c| !Arc::ptr_eq(c, conn));
        list.len() < before
    }

    /// SEC-03 "lock disconnects sessions": takes every connection out, for the lock to close once
    /// the tabs on them closed.
    pub fn take_all(&self) -> Vec<Arc<Connection>> {
        std::mem::take(&mut *lock(&self.list))
    }
}

/// The host's saved local forward.
fn saved_forward(state: &AppState, host_id: &str, forward_id: &str) -> AppResult<PortForward> {
    state.with_unlocked(|v| {
        v.get(forward_id)
            .and_then(Item::as_forward)
            .filter(|f| f.host_id == host_id && f.kind == ForwardKind::Local)
            .cloned()
            .ok_or_else(|| AppError::not_found("forward"))
    })
}

fn config_of(state: &AppState, host_id: &str) -> AppResult<String> {
    state.with_unlocked(|v| {
        let host = v
            .get(host_id)
            .and_then(Item::as_host)
            .ok_or_else(|| AppError::not_found("host"))?;
        Ok(crate::dashboard::config_version(v, host))
    })
}

/// HOST-06, HOST-11: device-local, never synced.
pub fn record_connected(state: &AppState, host_id: &str, session: &SshSession) {
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
}

/// A place for a terminal tab on the saved host's connection, which connects first when no
/// connection has room. `session_id` is the tab's, which answers the questions signing in asks.
pub async fn join_tab(
    app: &AppHandle,
    host_id: &str,
    overrides: &Overrides,
    session_id: &str,
) -> AppResult<Tab> {
    let state = state(app);
    let gate = state.connections.gate(host_id);
    let _held = gate.lock().await;
    // Read behind the gate: the host may have changed while a sign-in held it.
    let config = config_of(&state, host_id)?;
    if let Some(tab) = state
        .connections
        .of_host(host_id, Some(&config))
        .iter()
        .find_map(Connection::join_tab)
    {
        return Ok(tab);
    }
    let starting = open(app, &state, host_id, overrides, Some(session_id)).await?;
    starting.0.join_tab().ok_or_else(|| {
        AppError::ssh(
            SshErrorKind::Disconnected,
            "the connection closed as it opened",
        )
    })
}

/// Starts the saved forward on the host's connection, connecting first when there is none, and
/// returns the local port. A forward already running returns its port.
pub async fn start(
    app: &AppHandle,
    host_id: &str,
    forward_id: &str,
    overrides: &Overrides,
) -> AppResult<u16> {
    let state = state(app);
    let gate = state.connections.gate(host_id);
    let _held = gate.lock().await;
    saved_forward(&state, host_id, forward_id)?;
    if let Some(port) = running_port(&state, host_id, forward_id) {
        return Ok(port);
    }
    let config = config_of(&state, host_id)?;
    let starting = match state
        .connections
        .of_host(host_id, Some(&config))
        .iter()
        .find_map(Connection::hold)
    {
        Some(starting) => starting,
        None => {
            let starting = open(app, &state, host_id, overrides, None).await?;
            record_connected(&state, host_id, &starting.0.session);
            starting
        }
    };
    let conn = &starting.0;
    // The host's auto-start forwards started with a connection opened for this one.
    let started = match conn.forward_port(forward_id) {
        Some(port) => Ok(port),
        None => open_forward(&state, conn, forward_id).await,
    };
    drop(starting);
    changed(app);
    started
}

/// Starts the forward on the connection, keeping why it failed there.
async fn open_forward(state: &AppState, conn: &Connection, forward_id: &str) -> AppResult<u16> {
    // Read again: signing in may have waited on the user while the rule changed or went.
    let opened = async {
        let f = saved_forward(state, &conn.host_id, forward_id)?;
        let spec = LocalForward {
            bind_address: f.bind_address,
            bind_port: f.bind_port,
            dest_host: f.dest_host.unwrap_or_default(),
            dest_port: f.dest_port.unwrap_or_default(),
        };
        let dest = host_port(&spec.dest_host, spec.dest_port);
        let handle = conn.session.local_forward(spec).await?;
        let port = handle.local_port();
        conn.add_forward(state, forward_id, handle, dest)?;
        Ok::<_, AppError>(port)
    }
    .await;
    if let Err(e) = &opened
        && e.code != ErrorCode::NotFound
    {
        conn.add_failure(forward_id, e.clone());
    }
    opened
}

/// FWD-02: the host's auto-start forwards, on the connection just opened. One already running on
/// another of the host's connections stays there.
async fn start_auto(state: &AppState, conn: &Connection) {
    let auto: Vec<String> = state
        .with_unlocked(|v| {
            Ok(v.forwards()
                .into_iter()
                .filter(|(_, f)| {
                    f.host_id == conn.host_id && f.auto_start && f.kind == ForwardKind::Local
                })
                .map(|(id, _)| id)
                .collect())
        })
        .unwrap_or_default();
    for id in auto {
        if running_port(state, &conn.host_id, &id).is_some() {
            continue;
        }
        if let Err(e) = open_forward(state, conn, &id).await {
            tracing::warn!(forward_id = %id, "auto-start forward failed: {}", e.detail);
        }
    }
}

/// Stops the forward wherever it runs on the host's connections, which close when nothing else
/// is left on them.
pub fn stop(app: &AppHandle, host_id: &str, forward_id: &str) {
    let state = state(app);
    let mut found = false;
    for conn in state.connections.of_host(host_id, None) {
        found |= conn.stop_forward(forward_id);
    }
    if found {
        changed(app);
    }
}

/// Stops every forward on the host's connections: the host was deleted, and its rules with it.
pub fn forget_host(app: &AppHandle, host_id: &str) {
    let state = state(app);
    for conn in state.connections.of_host(host_id, None) {
        conn.stop_forwards();
    }
    changed(app);
}

/// `host:port`, with an IPv6 address in brackets.
fn host_port(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Where this computer reaches a forward listening on `addr`: `localhost` for a loopback or
/// wildcard address, else that address.
fn reachable(addr: SocketAddr) -> String {
    if addr.ip().is_loopback() || addr.ip().is_unspecified() {
        format!("localhost:{}", addr.port())
    } else {
        addr.to_string()
    }
}

/// The forward's local port while it runs on one of the host's connections.
fn running_port(state: &AppState, host_id: &str, forward_id: &str) -> Option<u16> {
    state
        .connections
        .of_host(host_id, None)
        .iter()
        .find_map(|c| c.forward_port(forward_id))
}

/// Every saved host's open connection, with the forwards on it.
pub fn list(state: &AppState) -> ConnectionsEvent {
    let version = lock(&state.connections.version);
    snapshot(state, *version)
}

fn snapshot(state: &AppState, version: u64) -> ConnectionsEvent {
    let mut connections: Vec<HostConnectionView> = Vec::new();
    for conn in lock(&state.connections.list).iter().filter(|c| c.open()) {
        let at = match connections.iter().position(|v| v.host_id == conn.host_id) {
            Some(at) => at,
            None => {
                connections.push(HostConnectionView {
                    host_id: conn.host_id.clone(),
                    host_name: conn.host_name.clone(),
                    forwards: Vec::new(),
                    failed: Vec::new(),
                });
                connections.len() - 1
            }
        };
        let view = &mut connections[at];
        let users = conn.users();
        view.forwards.extend(
            users
                .forwards
                .iter()
                .filter(|(_, f)| f.handle.is_running())
                .map(|(forward_id, f)| RunningForwardView {
                    forward_id: forward_id.clone(),
                    local_port: f.handle.local_port(),
                    address: reachable(f.handle.local_addr()),
                    dest: f.dest.clone(),
                }),
        );
        view.failed.extend(
            users
                .failed
                .iter()
                .map(|(forward_id, error)| FailedForwardView {
                    forward_id: forward_id.clone(),
                    error: error.clone(),
                }),
        );
    }
    for view in &mut connections {
        view.forwards.sort_by_key(|f| f.local_port);
    }
    ConnectionsEvent {
        version,
        connections,
    }
}

/// Tells the WebView and the tray menu what is open now, in the order the changes happened.
pub fn changed(app: &AppHandle) {
    let state = state(app);
    let mut version = lock(&state.connections.version);
    *version += 1;
    let event = snapshot(&state, *version);
    crate::tray::set_forwards(app, event.version, &event.connections);
    let _ = event.emit(app);
}

/// Where a host's new connection comes from.
enum Via {
    /// HOST-14: the dashboard's, which then reads over it.
    Dashboard(SshSession),
    Connect(Box<ConnectConfig>),
}

/// Connects to the host, registers the connection and starts the host's auto-start forwards on
/// it. Called behind the host's gate.
async fn open(
    app: &AppHandle,
    state: &AppState,
    host_id: &str,
    overrides: &Overrides,
    session_id: Option<&str>,
) -> AppResult<Starting> {
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
        Via::Connect(cfg) => connect_with(app, *cfg, session_id.map(str::to_owned)).await?,
    };
    let conn = Arc::new(Connection {
        session,
        host_id: host_id.to_owned(),
        config,
        host_name: name,
        users: Mutex::new(Users {
            starting: 1,
            ..Users::default()
        }),
    });
    let starting = Starting(Arc::clone(&conn));
    lock(&state.connections.list).push(Arc::clone(&conn));
    // HOST-14: the dashboard reads the host over this connection from now on.
    state.dashboard.wake(host_id);
    watch(app.clone(), Arc::clone(&conn));
    start_auto(state, &conn).await;
    changed(app);
    Ok(starting)
}

/// Waits for the connection to end. One that ends while still registered ended on its own or
/// closed with nothing left on it; the lock (SEC-03) and quitting take it out first.
fn watch(app: AppHandle, conn: Arc<Connection>) {
    tauri::async_runtime::spawn(async move {
        let err = conn.session.closed().await;
        let state = state(&app);
        let removed = state.connections.remove(&conn);
        if removed && conn.stop_forwards() > 0 {
            let error = AppError::from(err);
            tracing::info!(kind = ?error.ssh_kind, "connection with port forwards lost");
            let _ = ForwardsLostEvent {
                host_id: conn.host_id.clone(),
                host_name: conn.host_name.clone(),
                error,
            }
            .emit(&app);
        }
        changed(&app);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forward_is_reached_where_it_listens() {
        let at = |s: &str| reachable(s.parse().unwrap());
        assert_eq!(at("127.0.0.1:5433"), "localhost:5433");
        assert_eq!(at("0.0.0.0:5433"), "localhost:5433");
        assert_eq!(at("[::1]:5433"), "localhost:5433");
        assert_eq!(at("[::]:5433"), "localhost:5433");
        assert_eq!(at("192.168.1.10:5433"), "192.168.1.10:5433");
        assert_eq!(at("[fd00::5]:5433"), "[fd00::5]:5433");
    }
}
