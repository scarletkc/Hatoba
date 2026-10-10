//! MCP servers in the shell (spec §13.9): what each server is on this device, its live
//! connection, and the tools a request offers the model.
//!
//! - [`DeviceState`]: whether a server is enabled on this device and what it always allows
//!   (AI-29, AI-31), stored in the vault's device-local meta, readable while locked. A server
//!   with no entry is enabled iff it uses `http`, so a `stdio` server that arrives from another
//!   device stays off until the user turns it on here. What this device chose for a `stdio`
//!   server holds for the command line it was chosen for: one changed on another device is off
//!   here again, with no Always allow, until the user turns it on here.
//! - [`McpManager`]: one entry per server started since unlock. A server starts when a request
//!   needs its tools ([`McpManager::offer`]) or when the user starts it, lists its tools once,
//!   and lists them again before the next request after `tools/list_changed`. Its state
//!   (`stopped | starting | running | failed`, the error and the stderr tail) lives in memory
//!   and goes to the WebView as `ai://mcp-status` on every change. Locking the vault and quitting
//!   stop every server; a server whose configuration changed restarts.
//! - [`Offer`]: the MCP tools one request offered, with the names the model sees (AI-30). A
//!   conversation keeps the offer of its latest request, so a call maps back to the server and
//!   tool that request named.
//!
//! Locks: the vault mutex comes before the manager's own locks, never after; the servers map
//! before a server's state, and one server's state at a time. Status events read the device
//! state from the vault, so they are sent only without the vault guard.
//!
//! SEC-04: logs carry server ids, states, counts and error kinds; never environment or header
//! values, tool arguments or results, a server's own messages, or its stderr.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use futures::future::join_all;
use hatoba_ai::chat::ToolDef;
use hatoba_ai::entry::ToolStatus;
use hatoba_ai::mcp::{self, McpConnection, McpError, McpTool, McpTransportConfig, StderrTail};
use hatoba_ai::provider::Protocol;
use hatoba_ai::tools;
use hatoba_core::Vault;
use hatoba_core::model::{Item, McpTransport};
use hatoba_core::sync::SharedVault;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::dto::{
    AiTurnContext, McpServerState, McpServerStatus, McpToolAnnotations, McpToolView,
    McpTransportView,
};
use crate::error::{AppError, AppResult};
use crate::platform;

/// How long a server gets to start and answer `initialize` (and each `tools/list` page). Long
/// enough for `npx -y` to download a package the first time.
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// How long one tool call may take.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// How long stopping every server waits for the starts in flight to end (their children are
/// killed as soon as they see the stop).
const START_STOP_WAIT: Duration = Duration::from_secs(5);

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs a task on the async runtime: the current one when there is one (commands, turns,
/// tests), Tauri's otherwise.
fn spawn(task: impl Future<Output = ()> + Send + 'static) {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn(task);
        }
        Err(_) => {
            tauri::async_runtime::spawn(task);
        }
    }
}

// ───────────────────────── device-local state ─────────────────────────

/// What this device keeps about each MCP server (AI-29, AI-31). Holds no secret.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceState {
    #[serde(default)]
    servers: BTreeMap<String, DeviceServer>,
}

/// One server on this device.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceServer {
    /// `None` until this device chose: the server is then enabled iff it uses `http` (AI-29).
    pub enabled: Option<bool>,
    /// Every tool of the server runs without asking in manual mode.
    pub always_allow: bool,
    /// These tools (the server's own names) run without asking in manual mode.
    pub always_allow_tools: Vec<String>,
    /// [`stdio_fingerprint`] of the command line this entry was chosen for; `None` for `http`.
    pub stdio: Option<String>,
}

/// What identifies a `stdio` server's command line: a hash of the command, the arguments and
/// the environment names. Environment values are secrets and the device state is not encrypted,
/// so they are left out. `None` for `http`.
pub fn stdio_fingerprint(transport: &McpTransport) -> Option<String> {
    let McpTransport::Stdio { command, args, env } = transport else {
        return None;
    };
    let names: Vec<&String> = env.keys().collect();
    let encoded = serde_json::to_vec(&(command, args, names)).unwrap_or_default();
    Some(hatoba_core::crypto::sha256_hex(&encoded))
}

impl DeviceState {
    /// The stored state; an unreadable one counts as empty. While unlocked, the entry of a
    /// `stdio` server whose command line is not the one it was chosen for counts as absent.
    pub fn load(v: &Vault) -> Self {
        let mut state = match v.mcp_device_state() {
            Ok(Some(json)) => serde_json::from_str(&json).unwrap_or_else(|_| {
                tracing::warn!("the MCP device state is unreadable; starting from defaults");
                Self::default()
            }),
            Ok(None) => Self::default(),
            Err(_) => {
                tracing::warn!("could not read the MCP device state");
                Self::default()
            }
        };
        for (id, server) in v.mcp_servers() {
            let Some(fingerprint) = stdio_fingerprint(&server.transport) else {
                continue;
            };
            if state
                .servers
                .get(&id)
                .is_some_and(|s| s.stdio.as_ref() != Some(&fingerprint))
            {
                state.servers.remove(&id);
            }
        }
        state
    }

    pub fn save(&self, v: &mut Vault) -> AppResult<()> {
        let json = serde_json::to_string(self)
            .map_err(|_| AppError::internal("could not encode the MCP device state"))?;
        Ok(v.set_mcp_device_state(&json)?)
    }

    /// The entry of a server, or the defaults.
    pub fn server(&self, id: &str) -> DeviceServer {
        self.servers.get(id).cloned().unwrap_or_default()
    }

    /// The entry of a server, to change: what it holds is chosen for `transport`.
    pub fn server_mut(&mut self, id: &str, transport: &McpTransport) -> &mut DeviceServer {
        let here = self.servers.entry(id.to_owned()).or_default();
        here.stdio = stdio_fingerprint(transport);
        here
    }

    /// Keeps the entry of a server that is edited on this device, where the form showed the new
    /// command line (AI-29), chosen for its new transport.
    pub fn edited(&mut self, id: &str, transport: &McpTransport) {
        if self.servers.contains_key(id) {
            self.server_mut(id, transport);
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.servers.remove(id);
    }

    /// Whether the server is enabled on this device (AI-29).
    pub fn enabled(&self, id: &str, transport: &McpTransport) -> bool {
        self.servers
            .get(id)
            .and_then(|s| s.enabled)
            .unwrap_or(matches!(transport, McpTransport::Http { .. }))
    }

    /// Whether `tool` of the server runs without asking in manual mode (AI-31).
    pub fn allows(&self, id: &str, tool: &str) -> bool {
        self.servers
            .get(id)
            .is_some_and(|s| s.always_allow || s.always_allow_tools.iter().any(|t| t == tool))
    }
}

// ───────────────────────── conversions ─────────────────────────

/// What [`McpConnection::connect`] needs from a stored server.
pub fn transport_config(t: &McpTransport) -> McpTransportConfig {
    match t {
        McpTransport::Stdio { command, args, env } => McpTransportConfig::Stdio {
            command: command.clone(),
            args: args.clone(),
            env: env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        },
        McpTransport::Http { url, headers } => McpTransportConfig::Http {
            url: url.clone(),
            headers: headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        },
    }
}

/// A `stdio` server's configuration with `path` as its `PATH`, unless the server sets its own.
/// The server is looked up on it and started with it. `http` servers and `None` change nothing.
pub fn with_default_path(
    config: McpTransportConfig,
    path: Option<&std::ffi::OsStr>,
) -> McpTransportConfig {
    match (config, path) {
        (
            McpTransportConfig::Stdio {
                command,
                args,
                mut env,
            },
            Some(path),
        ) if !env.iter().any(|(name, _)| name == "PATH") => {
            env.push(("PATH".into(), path.to_string_lossy().into_owned().into()));
            McpTransportConfig::Stdio { command, args, env }
        }
        (config, _) => config,
    }
}

/// A stored server's transport for the vault, from an import.
pub fn core_transport(c: McpTransportConfig) -> McpTransport {
    match c {
        McpTransportConfig::Stdio { command, args, env } => McpTransport::Stdio {
            command,
            args,
            env: env.into_iter().collect(),
        },
        McpTransportConfig::Http { url, headers } => McpTransport::Http {
            url,
            headers: headers.into_iter().collect(),
        },
    }
}

/// The transport for the WebView: names only, never values (AI-29).
pub fn transport_view(t: &McpTransport) -> McpTransportView {
    match t {
        McpTransport::Stdio { command, args, env } => McpTransportView::Stdio {
            command: command.clone(),
            args: args.clone(),
            env_keys: env.keys().cloned().collect(),
        },
        McpTransport::Http { url, headers } => McpTransportView::Http {
            url: url.clone(),
            header_keys: headers.keys().cloned().collect(),
        },
    }
}

/// [`transport_view`] of a configuration that is not stored yet (an import preview).
pub fn config_view(c: &McpTransportConfig) -> McpTransportView {
    match c {
        McpTransportConfig::Stdio { command, args, env } => McpTransportView::Stdio {
            command: command.clone(),
            args: args.clone(),
            env_keys: env.iter().map(|(k, _)| k.clone()).collect(),
        },
        McpTransportConfig::Http { url, headers } => McpTransportView::Http {
            url: url.clone(),
            header_keys: headers.iter().map(|(k, _)| k.clone()).collect(),
        },
    }
}

/// Whether the server's item is in the vault: one deleted on another device keeps its
/// connection until the next request stops it, and must not run a call meanwhile.
pub fn server_exists(v: &Vault, id: &str) -> bool {
    v.get(id).and_then(Item::as_mcp_server).is_some()
}

/// The error result of a call whose server was deleted.
pub fn server_gone(server_name: &str) -> String {
    format!("The MCP server \"{server_name}\" was deleted, so the tool did not run.")
}

/// The kind of a failure, for logs (never its message, which may quote the server).
pub fn error_kind(e: &McpError) -> &'static str {
    match e {
        McpError::Spawn(_) => "spawn",
        McpError::CommandNotFound(_) => "command_not_found",
        McpError::Connect(_) => "connect",
        McpError::Protocol(_) => "protocol",
        McpError::Server(_) => "server",
        McpError::Closed => "closed",
        McpError::Timeout => "timeout",
        McpError::Cancelled => "cancelled",
        McpError::InvalidUrl(_) => "invalid_url",
        McpError::InvalidConfig(_) => "invalid_config",
    }
}

// ───────────────────────── offers ─────────────────────────

/// An MCP tool as a request offered it.
#[derive(Clone, Debug)]
pub struct OfferedTool {
    /// The name the model sees, `mcp__<server>__<tool>` after cleaning (AI-30).
    pub name: String,
    pub server_id: String,
    pub server_name: String,
    pub tool: McpTool,
}

/// The MCP tools of one request, in the order they were offered.
#[derive(Clone, Debug, Default)]
pub struct Offer {
    tools: Vec<OfferedTool>,
}

impl Offer {
    /// Names every tool of `servers`, which are `(id, name, tools)` sorted by name, then id.
    fn new(servers: &[(String, String, Vec<McpTool>)]) -> Self {
        let input: Vec<(&str, Vec<&str>)> = servers
            .iter()
            .map(|(_, name, tools)| {
                (
                    name.as_str(),
                    tools.iter().map(|t| t.name.as_str()).collect(),
                )
            })
            .collect();
        let names = mcp::tool_names(&input);
        let tools = servers
            .iter()
            .zip(names)
            .flat_map(|((id, server_name, tools), names)| {
                tools.iter().zip(names).map(|(tool, name)| OfferedTool {
                    name,
                    server_id: id.clone(),
                    server_name: server_name.clone(),
                    tool: tool.clone(),
                })
            })
            .collect();
        Self { tools }
    }

    /// The tool definitions for a provider: descriptions as the server wrote them, schemas
    /// cleaned for the protocol (AI-30).
    pub fn tool_defs(&self, protocol: Protocol) -> Vec<ToolDef> {
        self.tools
            .iter()
            .map(|t| ToolDef {
                name: t.name.clone(),
                description: t.tool.description.clone(),
                input_schema: mcp::clean_schema(&t.tool.input_schema, protocol),
            })
            .collect()
    }

    fn find(&self, name: &str) -> Option<&OfferedTool> {
        self.tools.iter().find(|t| t.name == name)
    }
}

/// The view of an offered tool, with its Always allow on this device.
pub fn tool_view(device: &DeviceState, tool: &OfferedTool) -> McpToolView {
    let a = &tool.tool.annotations;
    McpToolView {
        name: tool.name.clone(),
        tool: tool.tool.name.clone(),
        description: tool.tool.description.clone(),
        annotations: McpToolAnnotations {
            title: a.title.clone(),
            read_only_hint: a.read_only_hint,
            destructive_hint: a.destructive_hint,
            idempotent_hint: a.idempotent_hint,
            open_world_hint: a.open_world_hint,
        },
        always_allow: device.allows(&tool.server_id, &tool.tool.name),
    }
}

// ───────────────────────── the manager ─────────────────────────

/// Where status changes go: the `ai://mcp-status` event in the app, a list in tests.
pub trait McpEvents: Send + Sync + 'static {
    fn status(&self, status: McpServerStatus);
}

/// Why a server cannot take a call.
enum Unavailable {
    Stopped,
    Failed(String),
    Cancelled,
}

/// A start in flight, so that a stop can abort it: the start's task drops the connect, which
/// kills a stdio server's process tree (an `npx` download, say) instead of letting it run on
/// for up to [`START_TIMEOUT`] behind the lock screen, or past the app's exit.
struct Start {
    /// Aborts the start.
    cancel: CancellationToken,
    /// Cancelled when the start's task has ended, its child killed (a drop guard in the task).
    ended: CancellationToken,
}

/// What a start needs from [`ServerState::begin`].
struct Begun {
    generation: u64,
    /// The stderr buffer the start writes.
    stderr: StderrTail,
    /// The previous connection, which the start shuts down first.
    old: Option<McpConnection>,
    cancel: CancellationToken,
    ended: CancellationToken,
}

/// What a stop leaves to finish outside the locks.
struct Halted {
    /// The connection to shut down.
    conn: Option<McpConnection>,
    /// Ends when the aborted start's task has ended.
    start_ended: Option<CancellationToken>,
}

/// One server's live state.
struct ServerState {
    /// Bumped by every start and stop, so that a start that was overtaken drops its result.
    generation: u64,
    phase: McpServerState,
    /// The name and configuration of the current start.
    name: String,
    config: Option<McpTransportConfig>,
    conn: Option<McpConnection>,
    /// The start in flight, if any.
    start: Option<Start>,
    /// Shown with `failed` (AI-32); may quote the server, so never logged.
    error: Option<String>,
    /// The stdio server's stderr, kept in memory only.
    stderr: StderrTail,
    /// The tools from the last successful listing.
    tools: Vec<McpTool>,
}

impl Default for ServerState {
    fn default() -> Self {
        Self {
            generation: 0,
            phase: McpServerState::Stopped,
            name: String::new(),
            config: None,
            conn: None,
            start: None,
            error: None,
            stderr: StderrTail::new(),
            tools: Vec::new(),
        }
    }
}

impl ServerState {
    /// Begins a start with `config`, aborting one that is still in flight.
    fn begin(&mut self, name: &str, config: &McpTransportConfig) -> Begun {
        self.generation += 1;
        self.phase = McpServerState::Starting;
        name.clone_into(&mut self.name);
        self.config = Some(config.clone());
        self.error = None;
        self.tools.clear();
        self.stderr = StderrTail::new();
        if let Some(previous) = self.start.take() {
            previous.cancel.cancel();
        }
        let start = Start {
            cancel: CancellationToken::new(),
            ended: CancellationToken::new(),
        };
        let begun = Begun {
            generation: self.generation,
            stderr: self.stderr.clone(),
            old: self.conn.take(),
            cancel: start.cancel.clone(),
            ended: start.ended.clone(),
        };
        self.start = Some(start);
        begun
    }

    /// Stops, aborting a start in flight.
    fn halt(&mut self) -> Halted {
        self.generation += 1;
        self.phase = McpServerState::Stopped;
        self.config = None;
        self.error = None;
        self.tools.clear();
        self.stderr = StderrTail::new();
        let start_ended = self.start.take().map(|start| {
            start.cancel.cancel();
            start.ended
        });
        Halted {
            conn: self.conn.take(),
            start_ended,
        }
    }

    /// The connection closed by itself (the child exited, the transport broke): `failed`, with
    /// the stderr tail kept. Returns the dead connection, to drop outside the lock.
    fn closed(&mut self) -> Option<McpConnection> {
        self.phase = McpServerState::Failed;
        self.error = Some(McpError::Closed.to_string());
        self.tools.clear();
        self.conn.take()
    }
}

struct Server {
    state: Mutex<ServerState>,
    /// Woken when a start ends or the server stops.
    settled: tokio::sync::Notify,
}

impl Server {
    fn halt(&self) -> Halted {
        let halted = guard(&self.state).halt();
        self.settled.notify_waiters();
        halted
    }
}

/// The MCP servers' live state. Cheap to clone; every clone is the same manager.
#[derive(Clone, Default)]
pub struct McpManager(Arc<Inner>);

#[derive(Default)]
struct Inner {
    http: OnceLock<reqwest::Client>,
    events: OnceLock<Arc<dyn McpEvents>>,
    servers: Mutex<HashMap<String, Arc<Server>>>,
    /// The latest offer of each conversation.
    offers: Mutex<HashMap<String, Arc<Offer>>>,
}

impl McpManager {
    /// Where status events go; set once at startup.
    pub fn set_events(&self, events: Arc<dyn McpEvents>) {
        let _ = self.0.events.set(events);
    }

    fn http(&self) -> reqwest::Client {
        self.0
            .http
            .get_or_init(hatoba_ai::provider::http_client)
            .clone()
    }

    fn server(&self, id: &str) -> Arc<Server> {
        Arc::clone(
            guard(&self.0.servers)
                .entry(id.to_owned())
                .or_insert_with(|| {
                    Arc::new(Server {
                        state: Mutex::new(ServerState::default()),
                        settled: tokio::sync::Notify::new(),
                    })
                }),
        )
    }

    fn existing(&self, id: &str) -> Option<Arc<Server>> {
        guard(&self.0.servers).get(id).cloned()
    }

    // ---- status ----

    /// The tools of every running server, named as one request offering all of them would
    /// name them (servers sorted by name, then id).
    fn current_tools(&self) -> Offer {
        let servers: Vec<(String, Arc<Server>)> = guard(&self.0.servers)
            .iter()
            .map(|(id, s)| (id.clone(), Arc::clone(s)))
            .collect();
        let mut running: Vec<(String, String, Vec<McpTool>)> = servers
            .into_iter()
            .filter_map(|(id, server)| {
                let s = guard(&server.state);
                (s.phase == McpServerState::Running).then(|| (id, s.name.clone(), s.tools.clone()))
            })
            .collect();
        running.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        Offer::new(&running)
    }

    /// A server's state for the WebView. Never logged: `error` and `stderr` may quote it.
    pub fn status(&self, device: &DeviceState, server_id: &str) -> McpServerStatus {
        let current = self.current_tools();
        let Some(server) = self.existing(server_id) else {
            return McpServerStatus {
                server_id: server_id.to_owned(),
                state: McpServerState::Stopped,
                error: None,
                stderr: Vec::new(),
                tools: Vec::new(),
            };
        };
        let s = guard(&server.state);
        let tools = current
            .tools
            .iter()
            .filter(|t| t.server_id == server_id)
            .map(|t| tool_view(device, t))
            .collect();
        McpServerStatus {
            server_id: server_id.to_owned(),
            state: s.phase,
            error: s.error.clone(),
            stderr: s.stderr.lines(),
            tools,
        }
    }

    /// Sends the server's status. Reads the device state, so never call it with the vault
    /// guard held.
    pub fn emit(&self, vault: &SharedVault, server_id: &str) {
        let Some(events) = self.0.events.get() else {
            return;
        };
        let device = DeviceState::load(&guard(vault));
        events.status(self.status(&device, server_id));
    }

    // ---- requests (AI-30) ----

    /// The MCP tools a request offers: those of every server enabled on this device and not
    /// switched off for the conversation, with or without a terminal tab (AI-09). Starts the
    /// servers that are stopped and waits for them (until `cancel`), and relists a server's tools
    /// after `tools/list_changed`. A server that cannot start offers nothing; its status says why.
    pub async fn offer(
        &self,
        vault: &SharedVault,
        context: &AiTurnContext,
        cancel: &CancellationToken,
    ) -> Offer {
        let (wanted, live, off) = {
            let v = guard(vault);
            if !v.is_unlocked() {
                return Offer::default();
            }
            let device = DeviceState::load(&v);
            let servers = v.mcp_servers();
            let live: Vec<String> = servers.iter().map(|(id, _)| id.clone()).collect();
            let mut wanted: Vec<(String, String, McpTransportConfig)> = Vec::new();
            let mut off = Vec::new();
            for (id, s) in servers {
                if !device.enabled(&id, &s.transport) {
                    off.push(id);
                } else if !context.disabled_mcp_servers.contains(&id) {
                    let config = transport_config(&s.transport);
                    wanted.push((id, s.name, config));
                }
            }
            wanted.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
            (wanted, live, off)
        };
        self.prune(vault, &live);
        self.stop_off(vault, &off);
        let lists = join_all(
            wanted
                .iter()
                .map(|(id, name, config)| self.ensure_running(vault, id, name, config, cancel)),
        )
        .await;
        let servers: Vec<(String, String, Vec<McpTool>)> = wanted
            .into_iter()
            .zip(lists)
            .filter_map(|((id, name, _), tools)| tools.map(|tools| (id, name, tools)))
            .collect();
        Offer::new(&servers)
    }

    /// Stops the servers whose item is gone (deleted on another device).
    fn prune(&self, vault: &SharedVault, live: &[String]) {
        let gone: Vec<(String, Arc<Server>)> = {
            let mut servers = guard(&self.0.servers);
            let ids: Vec<String> = servers
                .keys()
                .filter(|id| !live.contains(id))
                .cloned()
                .collect();
            ids.into_iter()
                .filter_map(|id| servers.remove(&id).map(|s| (id, s)))
                .collect()
        };
        for (id, server) in gone {
            let conn = server.halt().conn;
            let (manager, vault) = (self.clone(), Arc::clone(vault));
            spawn(async move {
                if let Some(conn) = conn {
                    conn.shutdown().await;
                }
                tracing::info!(server_id = %id, "MCP server stopped: it was deleted");
                manager.emit(&vault, &id);
            });
        }
    }

    /// Stops the servers that run although they are off on this device: a `stdio` server whose
    /// command line changed on another device (the switch already stops one turned off here).
    fn stop_off(&self, vault: &SharedVault, off: &[String]) {
        for id in off {
            let Some(server) = self.existing(id) else {
                continue;
            };
            let conn = {
                let mut s = guard(&server.state);
                if s.phase == McpServerState::Stopped {
                    continue;
                }
                s.halt().conn
            };
            server.settled.notify_waiters();
            let (manager, vault, id) = (self.clone(), Arc::clone(vault), id.clone());
            spawn(async move {
                if let Some(conn) = conn {
                    conn.shutdown().await;
                }
                tracing::info!(server_id = %id, "MCP server stopped: it is off on this device");
                manager.emit(&vault, &id);
            });
        }
    }

    /// The tools of a server, starting it when it is stopped (or restarting it when its
    /// configuration changed). `None` when it is not running in the end.
    async fn ensure_running(
        &self,
        vault: &SharedVault,
        id: &str,
        name: &str,
        config: &McpTransportConfig,
        cancel: &CancellationToken,
    ) -> Option<Vec<McpTool>> {
        enum Next {
            Ready(Vec<McpTool>),
            Wait,
            Relist(McpConnection, u64),
            Start(Begun),
            Dead(Option<McpConnection>),
        }
        let server = self.server(id);
        // Set once this call started a start or waited for one: a server that is stopped after
        // that was stopped on purpose, so it is not started again.
        let mut waited = false;
        loop {
            if cancel.is_cancelled() {
                return None;
            }
            let settled = server.settled.notified();
            tokio::pin!(settled);
            settled.as_mut().enable();
            let next = {
                let mut s = guard(&server.state);
                let changed = s.config.as_ref() != Some(config);
                // A rename synced from another device changes only the tool names.
                if s.name != name {
                    name.clone_into(&mut s.name);
                }
                match s.phase {
                    McpServerState::Running | McpServerState::Failed if changed && !waited => {
                        Next::Start(s.begin(name, config))
                    }
                    McpServerState::Running => match s.conn.clone() {
                        Some(conn) if conn.is_closed() => Next::Dead(s.closed()),
                        Some(conn) if conn.tools_changed() => Next::Relist(conn, s.generation),
                        Some(_) => Next::Ready(s.tools.clone()),
                        None => return None,
                    },
                    McpServerState::Starting => Next::Wait,
                    McpServerState::Failed => return None,
                    McpServerState::Stopped if waited => return None,
                    McpServerState::Stopped => Next::Start(s.begin(name, config)),
                }
            };
            match next {
                Next::Ready(tools) => return Some(tools),
                Next::Relist(conn, generation) => {
                    self.relist(vault, id, &server, &conn, generation).await;
                    let s = guard(&server.state);
                    return (s.phase == McpServerState::Running).then(|| s.tools.clone());
                }
                Next::Start(begun) => {
                    // `starting` goes out before the start can end.
                    self.emit(vault, id);
                    self.spawn_start(vault, id, &server, config.clone(), begun);
                }
                Next::Dead(conn) => {
                    drop(conn);
                    tracing::warn!(server_id = id, "MCP server connection closed");
                    self.emit(vault, id);
                    server.settled.notify_waiters();
                    return None;
                }
                Next::Wait => {}
            }
            waited = true;
            tokio::select! {
                biased;
                () = cancel.cancelled() => return None,
                () = &mut settled => {}
            }
        }
    }

    /// Starts a server in a task of its own, so a turn that stops waiting does not abort it:
    /// connects, lists the tools, and records the outcome unless a stop or a newer start came
    /// first, or the vault locked meanwhile. A stop or a newer start aborts it: dropping the
    /// connect kills a stdio server's process tree.
    fn spawn_start(
        &self,
        vault: &SharedVault,
        id: &str,
        server: &Arc<Server>,
        config: McpTransportConfig,
        begun: Begun,
    ) {
        let (manager, vault, server, id) = (
            self.clone(),
            Arc::clone(vault),
            Arc::clone(server),
            id.to_owned(),
        );
        let Begun {
            generation,
            stderr,
            old,
            cancel,
            ended,
        } = begun;
        spawn(async move {
            let _ended = ended.drop_guard();
            let start = async {
                if let Some(old) = old {
                    old.shutdown().await;
                }
                tracing::info!(server_id = %id, transport = config.kind_str(), "MCP server starting");
                let http = manager.http();
                // The login shell's PATH on macOS, resolved when the first stdio server starts.
                let path = if matches!(config, McpTransportConfig::Stdio { .. }) {
                    tauri::async_runtime::spawn_blocking(platform::shell_path::default_path)
                        .await
                        .ok()
                        .flatten()
                } else {
                    None
                };
                let config = with_default_path(config.clone(), path);
                match McpConnection::connect_with_stderr(&config, &http, START_TIMEOUT, stderr)
                    .await
                {
                    Ok(conn) => match conn.list_tools().await {
                        Ok(tools) => Ok((conn, tools)),
                        Err(e) => {
                            conn.shutdown().await;
                            Err(e)
                        }
                    },
                    Err(e) => Err(e),
                }
            };
            let started = tokio::select! {
                biased;
                () = cancel.cancelled() => Err(McpError::Cancelled),
                started = start => started,
            };
            drop(config);
            let unlocked = guard(&vault).is_unlocked();
            let (current, stale) = {
                let mut s = guard(&server.state);
                let current = s.generation == generation;
                if current {
                    s.start = None;
                }
                if !current {
                    (false, started.ok().map(|(conn, _)| conn))
                } else if !unlocked {
                    s.halt();
                    (true, started.ok().map(|(conn, _)| conn))
                } else {
                    match started {
                        Ok((conn, tools)) => {
                            tracing::info!(server_id = %id, tools = tools.len(), "MCP server running");
                            s.phase = McpServerState::Running;
                            s.conn = Some(conn);
                            s.tools = tools;
                        }
                        Err(e) => {
                            tracing::warn!(server_id = %id, kind = error_kind(&e), "MCP server failed to start");
                            s.phase = McpServerState::Failed;
                            s.error = Some(e.to_string());
                        }
                    }
                    (true, None)
                }
            };
            if let Some(conn) = stale {
                conn.shutdown().await;
            }
            // The status goes out before the waiters go on, so it precedes what they do next.
            if current {
                manager.emit(&vault, &id);
            }
            server.settled.notify_waiters();
        });
    }

    /// Lists the tools again after `tools/list_changed` (AI-30). A failed listing keeps the
    /// old list (the flag stays set, so the next request tries again); a closed session fails
    /// the server.
    async fn relist(
        &self,
        vault: &SharedVault,
        id: &str,
        server: &Server,
        conn: &McpConnection,
        generation: u64,
    ) {
        let result = conn.list_tools().await;
        let dead = {
            let mut s = guard(&server.state);
            if s.generation != generation {
                return;
            }
            match result {
                Ok(tools) => {
                    tracing::info!(server_id = id, tools = tools.len(), "MCP tools relisted");
                    s.tools = tools;
                    None
                }
                Err(McpError::Closed) => s.closed(),
                Err(e) => {
                    tracing::warn!(
                        server_id = id,
                        kind = error_kind(&e),
                        "MCP tool list refresh failed"
                    );
                    return;
                }
            }
        };
        drop(dead);
        self.emit(vault, id);
        server.settled.notify_waiters();
    }

    /// Keeps the offer a conversation's request was made with, so its calls map back to it.
    pub fn record_offer(&self, conversation_id: &str, offer: Offer) {
        guard(&self.0.offers).insert(conversation_id.to_owned(), Arc::new(offer));
    }

    /// The tool behind `name` in a call of the conversation: from the offer of its latest
    /// request, or, when this run of the app made none, from the running servers. The approval
    /// card (`mcp_tool_info`) and the call (`ai_tool_run`) both resolve names here, so they
    /// always name the same server: names are per request, since cleaned server names can
    /// collide and the tools menu changes which servers a conversation's requests offer.
    pub fn offered(&self, conversation_id: &str, name: &str) -> Option<OfferedTool> {
        let offer = guard(&self.0.offers).get(conversation_id).cloned();
        match offer {
            Some(offer) => offer.find(name).cloned(),
            None => self.current_tools().find(name).cloned(),
        }
    }

    // ---- calls (AI-30, AI-32) ----

    /// Calls an offered tool and returns the result to store: `ok`, `error` (the tool failed,
    /// the server is not running or failed, named with its error, or its item was deleted), or
    /// `cancelled`.
    pub async fn call(
        &self,
        vault: &SharedVault,
        tool: &OfferedTool,
        arguments: Map<String, Value>,
        cancel: &CancellationToken,
    ) -> (ToolStatus, String) {
        let server_name = &tool.server_name;
        let (conn, generation) = match self.connection(vault, &tool.server_id, cancel).await {
            Ok(found) => found,
            Err(Unavailable::Cancelled) => return (ToolStatus::Cancelled, String::new()),
            Err(Unavailable::Stopped) => {
                return (
                    ToolStatus::Error,
                    format!(
                        "The MCP server \"{server_name}\" is not running, so the tool did not run."
                    ),
                );
            }
            Err(Unavailable::Failed(error)) => {
                return (
                    ToolStatus::Error,
                    format!(
                        "The MCP server \"{server_name}\" is not available, so the tool did not \
                         run: {error}"
                    ),
                );
            }
        };
        // Deleted on another device while the connection still runs (the next request stops
        // it): nothing runs, whatever the call was approved as. Checked after any wait for a
        // start, right before the call.
        {
            let v = guard(vault);
            if !v.is_unlocked() {
                return (ToolStatus::Cancelled, String::new());
            }
            if !server_exists(&v, &tool.server_id) {
                return (ToolStatus::Error, server_gone(server_name));
            }
        }
        let result = conn
            .call_tool(&tool.tool.name, arguments, cancel, CALL_TIMEOUT)
            .await;
        let outcome = match result {
            Ok(result) => {
                let status = if result.is_error {
                    ToolStatus::Error
                } else {
                    ToolStatus::Ok
                };
                (status, tools::truncate_result(&result.content))
            }
            Err(McpError::Cancelled) => (ToolStatus::Cancelled, String::new()),
            Err(McpError::Closed) => {
                self.lost(vault, &tool.server_id, generation);
                (
                    ToolStatus::Error,
                    format!(
                        "The MCP server \"{server_name}\" stopped before the call finished: {}.",
                        McpError::Closed
                    ),
                )
            }
            Err(McpError::Timeout) => (
                ToolStatus::Error,
                format!(
                    "The MCP server \"{server_name}\" did not answer within {} seconds.",
                    CALL_TIMEOUT.as_secs()
                ),
            ),
            Err(e) => {
                tracing::warn!(server_id = %tool.server_id, kind = error_kind(&e), "MCP tool call failed");
                (
                    ToolStatus::Error,
                    format!("The MCP server \"{server_name}\" could not run the tool: {e}"),
                )
            }
        };
        tracing::info!(server_id = %tool.server_id, status = ?outcome.0, "MCP tool call finished");
        outcome
    }

    /// The live connection of a server, waiting while it starts.
    async fn connection(
        &self,
        vault: &SharedVault,
        id: &str,
        cancel: &CancellationToken,
    ) -> Result<(McpConnection, u64), Unavailable> {
        let Some(server) = self.existing(id) else {
            return Err(Unavailable::Stopped);
        };
        loop {
            if cancel.is_cancelled() {
                return Err(Unavailable::Cancelled);
            }
            let settled = server.settled.notified();
            tokio::pin!(settled);
            settled.as_mut().enable();
            let dead = {
                let mut s = guard(&server.state);
                match s.phase {
                    McpServerState::Running => match s.conn.clone() {
                        Some(conn) if !conn.is_closed() => return Ok((conn, s.generation)),
                        _ => Some(s.closed()),
                    },
                    McpServerState::Starting => None,
                    McpServerState::Failed => {
                        return Err(Unavailable::Failed(s.error.clone().unwrap_or_default()));
                    }
                    McpServerState::Stopped => return Err(Unavailable::Stopped),
                }
            };
            if let Some(dead) = dead {
                drop(dead);
                tracing::warn!(server_id = id, "MCP server connection closed");
                self.emit(vault, id);
                server.settled.notify_waiters();
                return Err(Unavailable::Failed(McpError::Closed.to_string()));
            }
            tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(Unavailable::Cancelled),
                () = &mut settled => {}
            }
        }
    }

    /// A call found the session closed: the server fails, unless it was restarted meanwhile.
    fn lost(&self, vault: &SharedVault, id: &str, generation: u64) {
        let Some(server) = self.existing(id) else {
            return;
        };
        let dead = {
            let mut s = guard(&server.state);
            if s.generation != generation || s.phase != McpServerState::Running {
                return;
            }
            s.closed()
        };
        drop(dead);
        tracing::warn!(server_id = id, "MCP server connection closed");
        self.emit(vault, id);
        server.settled.notify_waiters();
    }

    // ---- lifecycle (AI-32) ----

    /// `mcp_server_start`: starts the server now, or restarts it, and waits until it runs or
    /// fails. Returns its status.
    pub async fn start_now(
        &self,
        vault: &SharedVault,
        id: &str,
        name: &str,
        config: McpTransportConfig,
    ) -> McpServerStatus {
        let server = self.server(id);
        let begun = guard(&server.state).begin(name, &config);
        let generation = begun.generation;
        server.settled.notify_waiters();
        self.emit(vault, id);
        self.spawn_start(vault, id, &server, config, begun);
        loop {
            let settled = server.settled.notified();
            tokio::pin!(settled);
            settled.as_mut().enable();
            {
                let s = guard(&server.state);
                if s.generation != generation || s.phase != McpServerState::Starting {
                    break;
                }
            }
            settled.await;
        }
        let device = DeviceState::load(&guard(vault));
        self.status(&device, id)
    }

    /// The configuration of a server was saved: one that is starting, running or failed starts
    /// again with it when it is enabled on this device, and stops otherwise.
    pub async fn restart_if_active(
        &self,
        vault: &SharedVault,
        id: &str,
        name: &str,
        config: McpTransportConfig,
        enabled: bool,
    ) {
        let Some(server) = self.existing(id) else {
            return;
        };
        let begun = {
            let mut s = guard(&server.state);
            if s.phase == McpServerState::Stopped {
                return;
            }
            if enabled {
                Ok(s.begin(name, &config))
            } else {
                Err(s.halt())
            }
        };
        server.settled.notify_waiters();
        match begun {
            Ok(begun) => {
                tracing::info!(
                    server_id = id,
                    "MCP server restarting with its new configuration"
                );
                self.emit(vault, id);
                self.spawn_start(vault, id, &server, config, begun);
            }
            Err(halted) => {
                if let Some(conn) = halted.conn {
                    conn.shutdown().await;
                }
                self.emit(vault, id);
            }
        }
    }

    /// Stops a server (`mcp_server_stop`, or disabled on this device).
    pub async fn stop(&self, vault: &SharedVault, id: &str) {
        if let Some(conn) = self.existing(id).and_then(|server| server.halt().conn) {
            conn.shutdown().await;
            tracing::info!(server_id = id, "MCP server stopped");
        }
        self.emit(vault, id);
    }

    /// A deleted server: stopped and forgotten.
    pub async fn forget(&self, vault: &SharedVault, id: &str) {
        let server = guard(&self.0.servers).remove(id);
        if let Some(conn) = server.and_then(|s| s.halt().conn) {
            conn.shutdown().await;
            tracing::info!(server_id = id, "MCP server stopped: it was deleted");
        }
        self.emit(vault, id);
    }

    /// The vault locked (AI-32): stops every server (closing `http` sessions too) and forgets
    /// their state, stderr and the offers of every conversation.
    pub async fn stop_all(&self, vault: &SharedVault) {
        for id in self.stop_everything().await {
            self.emit(vault, &id);
        }
    }

    /// The app quits: stops every server, without events.
    pub async fn shutdown(&self) {
        self.stop_everything().await;
    }

    /// Stops every server and aborts every start in flight, and waits until the aborted starts
    /// have killed their children (at most [`START_STOP_WAIT`]).
    async fn stop_everything(&self) -> Vec<String> {
        let servers: Vec<(String, Arc<Server>)> = guard(&self.0.servers).drain().collect();
        guard(&self.0.offers).clear();
        let mut conns = Vec::new();
        let mut starts = Vec::new();
        for (_, server) in &servers {
            let halted = server.halt();
            conns.extend(halted.conn);
            starts.extend(halted.start_ended);
        }
        if !conns.is_empty() || !starts.is_empty() {
            tracing::info!(
                count = conns.len(),
                starting = starts.len(),
                "MCP servers stopping"
            );
        }
        let starts_ended = join_all(
            starts
                .iter()
                .map(|ended| tokio::time::timeout(START_STOP_WAIT, ended.cancelled())),
        );
        futures::join!(
            join_all(conns.iter().map(McpConnection::shutdown)),
            starts_ended
        );
        servers.into_iter().map(|(id, _)| id).collect()
    }
}

#[cfg(test)]
pub(crate) mod tests;
