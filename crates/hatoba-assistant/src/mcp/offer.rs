use std::time::Duration;

use hatoba_ai::chat::ToolDef;
use hatoba_ai::mcp::deferred::ListedTool;
use hatoba_ai::mcp::{self, McpConnection, McpPrompt, McpResource, McpTool, McpTransportConfig};
use hatoba_ai::provider::Protocol;
use hatoba_core::Vault;

use super::device::{DeviceState, error_kind, transport_config};
use crate::dto::{
    McpPromptArgumentView, McpPromptView, McpResourceView, McpToolAnnotations, McpToolView,
};

/// How long getting a prompt or reading a resource for the panel's menus may take (AI-50, AI-51).
pub(super) const FETCH_TIMEOUT: Duration = Duration::from_secs(60);
/// The most text a prompt or resource attaches: what a whole message may attach (AI-35,
/// `MESSAGE_MAX_BYTES` in `attachments.ts`). A larger one is refused here rather than sent to the
/// WebView to be refused there.
pub(super) const MAX_FETCH_BYTES: usize = 512 * 1024;

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
    pub(super) tools: Vec<OfferedTool>,
}

impl Offer {
    /// Names every tool of `servers`, which are `(id, name, tools)` sorted by name, then id.
    pub(super) fn new(servers: &[(String, String, Vec<McpTool>)]) -> Self {
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

    /// The tools as the deferred list and `search_tools` see them (AI-45), in the offer's order.
    pub fn listed(&self) -> Vec<ListedTool<'_>> {
        self.tools
            .iter()
            .map(|t| ListedTool {
                name: &t.name,
                server_id: &t.server_id,
                server: &t.server_name,
                tool: &t.tool,
            })
            .collect()
    }

    /// The servers the tools come from, by name, as a response records them and the
    /// `mcp_change` note names them (AI-30).
    pub fn server_names(&self) -> Vec<String> {
        mcp::change::server_names(self.tools.iter().map(|t| t.server_name.as_str()))
    }

    pub(super) fn find(&self, name: &str) -> Option<&OfferedTool> {
        self.tools.iter().find(|t| t.name == name)
    }
}

/// The servers a conversation's requests offer the tools of, read from an unlocked vault.
pub(super) struct Wanted {
    /// `(id, name, config)` of every server enabled on this device and not switched off for the
    /// conversation, with or without a terminal tab (AI-09), sorted by name, then id.
    pub(super) wanted: Vec<(String, String, McpTransportConfig)>,
    /// Every server's id.
    pub(super) live: Vec<String>,
    /// The servers off on this device.
    pub(super) off: Vec<String>,
}

impl Wanted {
    pub(super) fn read(v: &Vault, disabled: &[String]) -> Self {
        let device = DeviceState::load(v);
        let servers = v.mcp_servers();
        let live: Vec<String> = servers.iter().map(|(id, _)| id.clone()).collect();
        let mut wanted: Vec<(String, String, McpTransportConfig)> = Vec::new();
        let mut off = Vec::new();
        for (id, s) in servers {
            if !device.enabled(&id, &s.transport) {
                off.push(id);
            } else if !disabled.contains(&id) {
                let config = transport_config(&s.transport);
                wanted.push((id, s.name, config));
            }
        }
        wanted.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        Self { wanted, live, off }
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

/// The view of a prompt (AI-50).
pub(super) fn prompt_view(prompt: &McpPrompt) -> McpPromptView {
    McpPromptView {
        name: prompt.name.clone(),
        title: prompt.title.clone(),
        description: prompt.description.clone(),
        arguments: prompt
            .arguments
            .iter()
            .map(|a| McpPromptArgumentView {
                name: a.name.clone(),
                description: a.description.clone(),
                required: a.required,
            })
            .collect(),
    }
}

/// The view of a resource (AI-51).
pub(super) fn resource_view(resource: &McpResource) -> McpResourceView {
    McpResourceView {
        uri: resource.uri.clone(),
        name: resource.name.clone(),
        title: resource.title.clone(),
        description: resource.description.clone(),
        mime_type: resource.mime_type.clone(),
    }
}

/// The prompts and resources of a server, each listed when asked for (AI-50, AI-51). `None` for a
/// list not asked for, or one whose listing failed: a server whose prompts or resources cannot be
/// listed still serves its tools, so the failure is only logged, and the next menu opening lists
/// them again.
pub(super) async fn list_extras(
    id: &str,
    conn: &McpConnection,
    prompts: bool,
    resources: bool,
) -> (Option<Vec<McpPrompt>>, Option<Vec<McpResource>>) {
    let prompts = async {
        if !prompts {
            return None;
        }
        conn.list_prompts()
            .await
            .inspect_err(|e| {
                tracing::warn!(
                    server_id = id,
                    kind = error_kind(e),
                    "MCP prompt listing failed"
                );
            })
            .ok()
    };
    let resources = async {
        if !resources {
            return None;
        }
        conn.list_resources()
            .await
            .inspect_err(|e| {
                tracing::warn!(
                    server_id = id,
                    kind = error_kind(e),
                    "MCP resource listing failed"
                );
            })
            .ok()
    };
    futures::join!(prompts, resources)
}

/// The text of a prompt or resource (`what`) of the server `server`, unless it is over
/// [`MAX_FETCH_BYTES`], more than a message attaches (AI-35).
pub(super) fn fits_message(text: String, server: &str, what: &str) -> Result<String, String> {
    if text.len() > MAX_FETCH_BYTES {
        return Err(format!(
            "the {what} of the MCP server \"{server}\" is over {} KB, more than a message can attach",
            MAX_FETCH_BYTES / 1024
        ));
    }
    Ok(text)
}
