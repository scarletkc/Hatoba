use std::collections::BTreeMap;

use hatoba_ai::mcp::{McpError, McpTransportConfig};
use hatoba_core::Vault;
use hatoba_core::model::{Item, McpServer, McpTransport};
use serde::{Deserialize, Serialize};

use super::{McpManager, tool_view};
use crate::dto::{McpProxyInfo, McpToolInfo, McpTransportView};
use crate::error::{AppError, AppResult};

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
    /// This entry was chosen while the server used `stdio`.
    pub stdio: bool,
}

impl DeviceState {
    /// The stored state; an unreadable one counts as empty. While unlocked, the entry of a
    /// `stdio` server that was chosen while the server used `http` counts as absent, so the
    /// server is off here, with no Always allow, until the user turns it on here (AI-29).
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
            if matches!(server.transport, McpTransport::Stdio { .. })
                && state.servers.get(&id).is_some_and(|s| !s.stdio)
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
        here.stdio = matches!(transport, McpTransport::Stdio { .. });
        here
    }

    /// Keeps the entry of a server that is edited on this device, chosen for its new transport:
    /// the user typed it here (AI-29).
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
pub(super) fn with_default_path(
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
pub(super) fn error_kind(e: &McpError) -> &'static str {
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

// ───────────────────────── this device's choices and approval cards ─────────────────────────

pub fn find_server(v: &Vault, id: &str) -> AppResult<McpServer> {
    v.get(id)
        .and_then(Item::as_mcp_server)
        .cloned()
        .ok_or_else(|| AppError::not_found("MCP server"))
}

pub fn set_enabled(v: &mut Vault, id: &str, enabled: bool) -> AppResult<()> {
    let server = find_server(v, id)?;
    let mut device = DeviceState::load(v);
    device.server_mut(id, &server.transport).enabled = Some(enabled);
    device.save(v)
}

/// AI-31 Always allow on this device: one tool (the server's own name) or the whole server.
pub fn set_always_allow(v: &mut Vault, id: &str, tool: Option<&str>, allow: bool) -> AppResult<()> {
    let server = find_server(v, id)?;
    let mut device = DeviceState::load(v);
    let here = device.server_mut(id, &server.transport);
    match tool.map(str::trim) {
        None => here.always_allow = allow,
        Some("") => return Err(AppError::invalid("tool", "The tool name is empty.")),
        Some(tool) => {
            here.always_allow_tools.retain(|t| t != tool);
            if allow {
                here.always_allow_tools.push(tool.to_owned());
            }
        }
    }
    device.save(v)
}

/// The tool behind a name the model called in a conversation, for the approval card (AI-31):
/// resolved like the call itself (`McpManager::offered`), so the card names the server that runs
/// it. `None` when the conversation's offer has no such tool, or its server was deleted.
pub fn tool_info(
    v: &Vault,
    mcp: &McpManager,
    conversation_id: &str,
    name: &str,
) -> Option<McpToolInfo> {
    let tool = mcp.offered(conversation_id, name)?;
    let server = v.get(&tool.server_id).and_then(Item::as_mcp_server)?;
    let device = DeviceState::load(v);
    Some(McpToolInfo {
        server_id: tool.server_id.clone(),
        server_name: server.name.clone(),
        always_ask: server.always_ask,
        tool: tool_view(&device, &tool),
    })
}

/// AI-45: what the approval card of a `call_mcp_tool` call needs, as `ai_tool_run` resolves the
/// call: the tool it names, and whether it runs. A call that does not run (arguments that do not
/// match the tool's input schema, a tool the conversation does not have, a deleted server) gets
/// its error result without asking.
pub fn proxy_info(
    v: &Vault,
    mcp: &McpManager,
    conversation_id: &str,
    arguments: &str,
) -> McpProxyInfo {
    match crate::ai::proxy_call(v, mcp, conversation_id, arguments) {
        Ok((tool, _)) => McpProxyInfo {
            tool: tool_info(v, mcp, conversation_id, &tool.name),
            runs: true,
        },
        Err(_) => McpProxyInfo {
            tool: hatoba_ai::mcp::deferred::parse_call(arguments)
                .ok()
                .and_then(|call| tool_info(v, mcp, conversation_id, &call.tool)),
            runs: false,
        },
    }
}
