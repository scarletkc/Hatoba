use std::fmt;

use serde::{Deserialize, Serialize};
use specta::Type;
use zeroize::Zeroize;

use super::providers::redacted;

// ───────────────────────── AI assistant: MCP servers (§13.9) ─────────────────────────
//
// Environment and header values travel one way, WebView → Rust, like API keys: views carry
// only their names, and the inputs that carry them print them redacted and wipe them when
// dropped (AI-29).

/// Environment and header values never reach the WebView, only their names (AI-29).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpTransportView {
    Stdio {
        command: String,
        args: Vec<String>,
        env_keys: Vec<String>,
    },
    Http {
        url: String,
        header_keys: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpServerView {
    pub id: String,
    pub name: String,
    pub transport: McpTransportView,
    /// Ask even in bypass mode. Synced (AI-31).
    pub always_ask: bool,
    /// Enabled on this device; a server from another device starts enabled only for `http`.
    pub enabled: bool,
    /// On this device, every tool of the server runs without asking in manual mode (AI-31).
    pub always_allow: bool,
    /// On this device, these tools (the server's own names) run without asking in manual mode.
    pub always_allow_tools: Vec<String>,
    pub updated_at: i64,
}

#[derive(Clone, Deserialize, Type)]
pub struct McpSecretInput {
    pub key: String,
    /// `None` keeps the saved value for this key.
    pub value: Option<String>,
}

impl fmt::Debug for McpSecretInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpSecretInput")
            .field("key", &self.key)
            .field("value", &redacted(self.value.as_deref()))
            .finish()
    }
}

impl Drop for McpSecretInput {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpTransportInput {
    Stdio {
        command: String,
        args: Vec<String>,
        env: Vec<McpSecretInput>,
    },
    Http {
        url: String,
        headers: Vec<McpSecretInput>,
    },
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct McpServerInput {
    /// `None` creates a server.
    pub id: Option<String>,
    pub name: String,
    pub transport: McpTransportInput,
    pub always_ask: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpServerState {
    Stopped,
    Starting,
    Running,
    Failed,
}

/// Shown on the approval card; never changes whether a call asks (AI-31).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq, Default)]
pub struct McpToolAnnotations {
    pub title: Option<String>,
    pub read_only_hint: Option<bool>,
    pub destructive_hint: Option<bool>,
    pub idempotent_hint: Option<bool>,
    pub open_world_hint: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpToolView {
    /// The name offered to the model, `mcp__<server>__<tool>` after cleaning (AI-30).
    pub name: String,
    /// The server's own tool name.
    pub tool: String,
    pub description: String,
    pub annotations: McpToolAnnotations,
    /// Always allow on this device (per tool, or because the whole server is).
    pub always_allow: bool,
}

/// A server's live state (AI-32), also pushed as `ai://mcp-status` on every change. The stderr
/// lines stay in memory and are never logged.
#[derive(Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpServerStatus {
    pub server_id: String,
    pub state: McpServerState,
    pub error: Option<String>,
    /// The last stderr lines of a `stdio` server, kept in memory only (AI-32).
    pub stderr: Vec<String>,
    /// The tools from the last successful listing.
    pub tools: Vec<McpToolView>,
    /// The prompts from the last successful listing, which the panel's `/` menu offers (AI-50).
    pub prompts: Vec<McpPromptView>,
    /// The resources from the last successful listing, which the panel's `@` menu offers (AI-51).
    pub resources: Vec<McpResourceView>,
}

impl fmt::Debug for McpServerStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpServerStatus")
            .field("server_id", &self.server_id)
            .field("state", &self.state)
            .field("error", &self.error.is_some())
            .field("stderr", &format_args!("<{} lines>", self.stderr.len()))
            .field("tools", &self.tools.len())
            .field("prompts", &self.prompts.len())
            .field("resources", &self.resources.len())
            .finish()
    }
}

/// A server's prompt (AI-50): the `/` menu offers it as `/mcp__<server>__<prompt>`, which the
/// panel names across the servers a conversation uses, as tool names are (AI-30).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpPromptView {
    /// The prompt's name on its server.
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    pub arguments: Vec<McpPromptArgumentView>,
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpPromptArgumentView {
    pub name: String,
    pub description: String,
    pub required: bool,
}

/// A server's resource (AI-51), which the `@` menu offers.
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpResourceView {
    pub uri: String,
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    pub mime_type: Option<String>,
}

/// A value for one argument of an MCP prompt (AI-50).
#[derive(Clone, Deserialize, Type)]
pub struct McpPromptArgumentInput {
    pub name: String,
    pub value: String,
}

impl fmt::Debug for McpPromptArgumentInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpPromptArgumentInput")
            .field("name", &self.name)
            .field(
                "value",
                &format_args!("<{} chars>", self.value.chars().count()),
            )
            .finish()
    }
}

/// The MCP tool behind a name the model called, for the approval card (AI-31).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpToolInfo {
    pub server_id: String,
    pub server_name: String,
    pub always_ask: bool,
    pub tool: McpToolView,
}

/// What the approval card of a `call_mcp_tool` call needs (AI-45).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpProxyInfo {
    /// The tool the call names, as `mcp_tool_info` gives a direct call's; null when the
    /// conversation has no tool of that name or its server was deleted.
    pub tool: Option<McpToolInfo>,
    /// False when the call cannot run: it then gets its error result without asking.
    pub runs: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct McpImportServer {
    pub name: String,
    pub transport: McpTransportView,
    /// A saved server has this name; the import adds a numeric suffix.
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct McpImportSkipped {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct McpImportPreview {
    pub servers: Vec<McpImportServer>,
    /// Entries that cannot be imported, such as an `sse` server, with the reason.
    pub skipped: Vec<McpImportSkipped>,
}
