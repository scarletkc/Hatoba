//! MCP server commands (spec §13.9): the servers of Settings → AI (AI-29), their state on this
//! device (enabled, Always allow, AI-31), their live status (AI-32), the tool behind a name the
//! model called, and import and export in the `mcpServers` format (AI-33). The connections run in
//! [`crate::mcp::McpManager`].
//!
//! Environment and header values go one way, WebView → Rust, like API keys: views carry only
//! their names, and an input value of `null` keeps the saved one (AI-29). SEC-04: they, the
//! pasted import text and the servers' messages are never logged.

use std::collections::HashSet;

use hatoba_ai::AiError;
use hatoba_ai::mcp::McpError;
use hatoba_ai::mcp::config::{ImportReport, export_json, parse_import};
use hatoba_ai::provider::validate_base_url;
use hatoba_core::Vault;
use hatoba_core::model::{Item, McpServer, McpTransport};
use reqwest::header::{HeaderName, HeaderValue};
use tauri::{AppHandle, State};
use tauri_specta::Event;
use zeroize::Zeroizing;

use crate::dto::{
    McpImportPreview, McpImportServer, McpImportSkipped, McpSecretInput, McpServerInput,
    McpServerStatus, McpServerView, McpToolInfo, McpTransportInput,
};
use crate::error::{AppError, AppResult};
use crate::mcp::{
    DeviceState, McpEvents, McpManager, config_view, core_transport, tool_view, transport_config,
    transport_view,
};
use crate::state::AppState;
use crate::sync;

/// The longest server name, in characters.
const NAME_MAX: usize = 64;

/// Headers the Streamable HTTP transport or the HTTP stack sets itself (as `hatoba_ai::mcp`
/// refuses them when connecting).
const RESERVED_HEADERS: [&str; 9] = [
    "accept",
    "connection",
    "content-length",
    "content-type",
    "host",
    "last-event-id",
    "mcp-protocol-version",
    "mcp-session-id",
    "transfer-encoding",
];

/// Status changes as the `ai://mcp-status` event.
pub struct AppMcpEvents(pub AppHandle);

impl McpEvents for AppMcpEvents {
    fn status(&self, status: McpServerStatus) {
        let _ = status.emit(&self.0);
    }
}

/// A message of the MCP crate (`the url is missing`) as a sentence (`The url is missing.`).
fn sentence(text: &str) -> String {
    let mut chars = text.trim().chars();
    let mut out: String = chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    if !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

// ───────────────────────── views ─────────────────────────

fn find_server(v: &Vault, id: &str) -> AppResult<McpServer> {
    v.get(id)
        .and_then(Item::as_mcp_server)
        .cloned()
        .ok_or_else(|| AppError::not_found("MCP server"))
}

pub(crate) fn server_view(id: &str, server: &McpServer, device: &DeviceState) -> McpServerView {
    let here = device.server(id);
    McpServerView {
        id: id.to_owned(),
        name: server.name.clone(),
        transport: transport_view(&server.transport),
        always_ask: server.always_ask,
        enabled: device.enabled(id, &server.transport),
        always_allow: here.always_allow,
        always_allow_tools: here.always_allow_tools,
        updated_at: server.updated_at,
    }
}

pub(crate) fn servers_list(v: &Vault) -> Vec<McpServerView> {
    let device = DeviceState::load(v);
    let mut list: Vec<McpServerView> = v
        .mcp_servers()
        .iter()
        .map(|(id, s)| server_view(id, s, &device))
        .collect();
    list.sort_by_cached_key(|s| (s.name.to_lowercase(), s.id.clone()));
    list
}

// ───────────────────────── saving (AI-29) ─────────────────────────

/// Whether another server has this name, ignoring case.
fn name_taken(v: &Vault, name: &str, except: Option<&str>) -> bool {
    let name = name.to_lowercase();
    v.mcp_servers()
        .iter()
        .any(|(id, s)| Some(id.as_str()) != except && s.name.trim().to_lowercase() == name)
}

fn checked_name(v: &Vault, name: &str, except: Option<&str>) -> AppResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::invalid("name", "A name is required."));
    }
    if name.chars().count() > NAME_MAX {
        return Err(AppError::invalid(
            "name",
            format!("The name can have at most {NAME_MAX} characters."),
        ));
    }
    if name_taken(v, name, except) {
        return Err(AppError::invalid(
            "name",
            format!("A server named \"{name}\" already exists."),
        ));
    }
    Ok(name.to_owned())
}

/// The secrets of a form, checked by `check(name)`, with `null` values taken from `saved`.
fn secrets(
    field: &str,
    inputs: &[McpSecretInput],
    saved: Option<&std::collections::BTreeMap<String, Zeroizing<String>>>,
    check: impl Fn(&str, &str) -> Result<(), String>,
) -> AppResult<std::collections::BTreeMap<String, Zeroizing<String>>> {
    let mut out = std::collections::BTreeMap::new();
    let mut seen = HashSet::new();
    for input in inputs {
        let key = input.key.trim();
        if key.is_empty() {
            return Err(AppError::invalid(field, "Every entry needs a name."));
        }
        if !seen.insert(key.to_lowercase()) {
            return Err(AppError::invalid(field, format!("{key} is listed twice.")));
        }
        let value = match (&input.value, saved.and_then(|s| s.get(key))) {
            (Some(value), _) => Zeroizing::new(value.clone()),
            (None, Some(saved)) => saved.clone(),
            (None, None) => {
                return Err(AppError::invalid(
                    field,
                    format!("Enter a value for {key}."),
                ));
            }
        };
        check(key, &value).map_err(|message| AppError::invalid(field, message))?;
        out.insert(key.to_owned(), value);
    }
    Ok(out)
}

fn check_env(key: &str, value: &str) -> Result<(), String> {
    if key.contains(['=', '\0']) {
        return Err(format!("\"{key}\" is not a valid variable name."));
    }
    if value.contains('\0') {
        return Err(format!("The value of {key} is not valid."));
    }
    Ok(())
}

fn check_header(key: &str, value: &str) -> Result<(), String> {
    if HeaderName::from_bytes(key.as_bytes()).is_err() {
        return Err(format!("\"{key}\" is not a valid header name."));
    }
    if RESERVED_HEADERS.contains(&key.to_ascii_lowercase().as_str()) {
        return Err(format!("{key} is set by Hatoba and cannot be changed."));
    }
    if HeaderValue::from_str(value).is_err() {
        return Err(format!("The value of {key} is not a valid header value."));
    }
    Ok(())
}

/// The transport to store from the form. `url` is the checked URL of an `http` server. A
/// `null` value keeps the saved value of that key, only from a saved transport of the same kind.
pub(crate) fn transport_from_input(
    input: &McpTransportInput,
    saved: Option<&McpTransport>,
    url: Option<String>,
) -> AppResult<McpTransport> {
    match input {
        McpTransportInput::Stdio { command, args, env } => {
            let command = command.trim();
            if command.is_empty() {
                return Err(AppError::invalid("command", "A command is required."));
            }
            if command.contains('\0') || args.iter().any(|a| a.contains('\0')) {
                return Err(AppError::invalid(
                    "command",
                    "The command line is not valid.",
                ));
            }
            let saved = match saved {
                Some(McpTransport::Stdio { env, .. }) => Some(env),
                _ => None,
            };
            Ok(McpTransport::Stdio {
                command: command.to_owned(),
                args: args.clone(),
                env: secrets("env", env, saved, check_env)?,
            })
        }
        McpTransportInput::Http { headers, .. } => {
            let saved = match saved {
                Some(McpTransport::Http { headers, .. }) => Some(headers),
                _ => None,
            };
            Ok(McpTransport::Http {
                url: url.ok_or_else(|| AppError::invalid("url", "A URL is required."))?,
                headers: secrets("headers", headers, saved, check_header)?,
            })
        }
    }
}

/// AI-02's rules for an `http` server's URL: `https`, or `http` for a local address.
async fn checked_url(input: &McpTransportInput) -> AppResult<Option<String>> {
    let McpTransportInput::Http { url, .. } = input else {
        return Ok(None);
    };
    let url = url.trim();
    if url.is_empty() {
        return Err(AppError::invalid("url", "A URL is required."));
    }
    validate_base_url(url).await.map_err(|e| {
        let message = match &e {
            AiError::InvalidUrl(m) | AiError::Network(m) => {
                m.replacen("the base URL", "the URL", 1)
            }
            other => other.to_string(),
        };
        AppError::invalid("url", sentence(&message))
    })?;
    Ok(Some(url.to_owned()))
}

/// What a save did, for the commands.
pub(crate) struct Saved {
    pub view: McpServerView,
    /// The name or the transport changed, so a running server restarts.
    pub changed: bool,
    pub server: McpServer,
}

/// Stores a server from the form; `url` is checked already. A server created here is enabled on
/// this device (AI-29); one edited here keeps what this device chose for it.
pub(crate) fn save_server(
    v: &mut Vault,
    input: &McpServerInput,
    url: Option<String>,
) -> AppResult<Saved> {
    let name = checked_name(v, &input.name, input.id.as_deref())?;
    let existing = match &input.id {
        Some(id) => Some(find_server(v, id)?),
        None => None,
    };
    let transport = transport_from_input(
        &input.transport,
        existing.as_ref().map(|s| &s.transport),
        url,
    )?;
    let changed = existing
        .as_ref()
        .is_none_or(|s| s.name != name || s.transport != transport);
    // Read before the new transport is stored, which the entry was not chosen for yet.
    let mut device = DeviceState::load(v);
    let id = v.put(
        input.id.as_deref(),
        Item::McpServer(McpServer {
            name,
            transport,
            always_ask: input.always_ask,
            updated_at: 0,
        }),
    )?;
    let server = find_server(v, &id)?;
    let before = device.clone();
    if input.id.is_none() {
        device.server_mut(&id, &server.transport).enabled = Some(true);
    } else {
        device.edited(&id, &server.transport);
    }
    if device != before {
        device.save(v)?;
    }
    Ok(Saved {
        view: server_view(&id, &server, &device),
        changed,
        server,
    })
}

pub(crate) fn delete_server(v: &mut Vault, id: &str) -> AppResult<()> {
    find_server(v, id)?;
    v.delete(id)?;
    let mut device = DeviceState::load(v);
    device.remove(id);
    device.save(v)
}

pub(crate) fn set_enabled(v: &mut Vault, id: &str, enabled: bool) -> AppResult<()> {
    let server = find_server(v, id)?;
    let mut device = DeviceState::load(v);
    device.server_mut(id, &server.transport).enabled = Some(enabled);
    device.save(v)
}

/// AI-31 Always allow on this device: one tool (the server's own name) or the whole server.
pub(crate) fn set_always_allow(
    v: &mut Vault,
    id: &str,
    tool: Option<&str>,
    allow: bool,
) -> AppResult<()> {
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
pub(crate) fn tool_info(
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

// ───────────────────────── import and export (AI-33) ─────────────────────────

fn parse(json: &str) -> AppResult<ImportReport> {
    parse_import(json).map_err(|e| match e {
        McpError::InvalidConfig(message) => AppError::invalid("json", sentence(&message)),
        other => AppError::invalid("json", sentence(&other.to_string())),
    })
}

pub(crate) fn import_preview(v: &Vault, report: &ImportReport) -> McpImportPreview {
    McpImportPreview {
        servers: report
            .servers
            .iter()
            .map(|s| McpImportServer {
                name: s.name.clone(),
                transport: config_view(&s.transport),
                exists: name_taken(v, s.name.trim(), None),
            })
            .collect(),
        skipped: report
            .skipped
            .iter()
            .map(|(name, reason)| McpImportSkipped {
                name: name.clone(),
                reason: sentence(reason),
            })
            .collect(),
    }
}

/// A name no server has: `name`, else `name-2`, `name-3`, … ("importing adds another").
fn free_name(v: &Vault, name: &str) -> String {
    let base: String = name.trim().chars().take(NAME_MAX).collect();
    let base = if base.is_empty() {
        "server".to_owned()
    } else {
        base
    };
    if !name_taken(v, &base, None) {
        return base;
    }
    (2..)
        .map(|n| {
            let suffix = format!("-{n}");
            let keep = NAME_MAX - suffix.chars().count();
            format!("{}{suffix}", base.chars().take(keep).collect::<String>())
        })
        .find(|candidate| !name_taken(v, candidate, None))
        .expect("the suffixes are unbounded")
}

/// Adds every importable server, enabled on this device; a taken name gets a numeric suffix,
/// so an import never replaces a server.
pub(crate) fn import_servers(v: &mut Vault, report: ImportReport) -> AppResult<Vec<McpServerView>> {
    let mut device = DeviceState::load(v);
    let mut ids = Vec::with_capacity(report.servers.len());
    for server in report.servers {
        let name = free_name(v, &server.name);
        let transport = core_transport(server.transport);
        let id = v.put(
            None,
            Item::McpServer(McpServer {
                name,
                transport: transport.clone(),
                always_ask: false,
                updated_at: 0,
            }),
        )?;
        device.server_mut(&id, &transport).enabled = Some(true);
        ids.push(id);
    }
    device.save(v)?;
    ids.iter()
        .map(|id| Ok(server_view(id, &find_server(v, id)?, &device)))
        .collect()
}

/// `mcpServers` JSON of every server, with placeholders in place of the values (AI-33).
pub(crate) fn export(v: &Vault) -> String {
    let mut servers = v.mcp_servers();
    servers.sort_by_cached_key(|(id, s)| (s.name.to_lowercase(), id.clone()));
    let list: Vec<(String, hatoba_ai::mcp::McpTransportConfig)> = servers
        .iter()
        .map(|(_, s)| (s.name.clone(), transport_config(&s.transport)))
        .collect();
    export_json(&list)
}

// ───────────────────────── commands ─────────────────────────

#[tauri::command]
#[specta::specta]
pub async fn mcp_servers_list(state: State<'_, AppState>) -> AppResult<Vec<McpServerView>> {
    state.with_unlocked(|v| Ok(servers_list(v)))
}

/// Saves a server; a running one restarts when its name or transport changed.
#[tauri::command]
#[specta::specta]
pub async fn mcp_server_save(
    app: AppHandle,
    state: State<'_, AppState>,
    input: McpServerInput,
) -> AppResult<McpServerView> {
    let url = checked_url(&input.transport).await?;
    let saved = state.with_unlocked(|v| save_server(v, &input, url))?;
    let id = saved.view.id.clone();
    tracing::info!(server_id = %id, transport = saved.server.transport.kind_str(), "MCP server saved");
    sync::local_change(&app);
    if saved.changed {
        state
            .mcp
            .restart_if_active(
                &state.vault,
                &id,
                &saved.server.name,
                transport_config(&saved.server.transport),
                saved.view.enabled,
            )
            .await;
    }
    Ok(saved.view)
}

#[tauri::command]
#[specta::specta]
pub async fn mcp_server_delete(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    state.with_unlocked(|v| delete_server(v, &id))?;
    tracing::info!(server_id = %id, "MCP server deleted");
    sync::local_change(&app);
    state.mcp.forget(&state.vault, &id).await;
    Ok(())
}

/// On this device only (AI-29). Disabling stops a running server.
#[tauri::command]
#[specta::specta]
pub async fn mcp_server_set_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> AppResult<()> {
    state.with_unlocked(|v| set_enabled(v, &id, enabled))?;
    if !enabled {
        state.mcp.stop(&state.vault, &id).await;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn mcp_server_status(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<McpServerStatus> {
    let device = state.with_unlocked(|v| {
        find_server(v, &id)?;
        Ok(DeviceState::load(v))
    })?;
    Ok(state.mcp.status(&device, &id))
}

/// Starts (or restarts) the server now and lists its tools.
#[tauri::command]
#[specta::specta]
pub async fn mcp_server_start(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<McpServerStatus> {
    let (name, config) = state.with_unlocked(|v| {
        let server = find_server(v, &id)?;
        Ok((server.name, transport_config(&server.transport)))
    })?;
    Ok(state.mcp.start_now(&state.vault, &id, &name, config).await)
}

#[tauri::command]
#[specta::specta]
pub async fn mcp_server_stop(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.with_unlocked(|v| find_server(v, &id).map(drop))?;
    state.mcp.stop(&state.vault, &id).await;
    Ok(())
}

/// AI-31 Always allow on this device: one tool (the server's own name), or every tool with `tool`
/// null.
#[tauri::command]
#[specta::specta]
pub async fn mcp_set_always_allow(
    state: State<'_, AppState>,
    server_id: String,
    tool: Option<String>,
    allow: bool,
) -> AppResult<()> {
    state.with_unlocked(|v| set_always_allow(v, &server_id, tool.as_deref(), allow))?;
    state.mcp.emit(&state.vault, &server_id);
    Ok(())
}

/// The MCP tool behind a name the model called in the conversation: from the offer of the
/// conversation's latest request, as `ai_tool_run` resolves it, else from the running servers.
/// Null when neither has it, or its server was deleted; the panel then asks in either mode.
#[tauri::command]
#[specta::specta]
pub async fn mcp_tool_info(
    state: State<'_, AppState>,
    conversation_id: String,
    name: String,
) -> AppResult<Option<McpToolInfo>> {
    state.with_unlocked(|v| Ok(tool_info(v, &state.mcp, &conversation_id, &name)))
}

/// AI-33: what pasted `mcpServers` / VS Code `servers` JSON would add.
#[tauri::command]
#[specta::specta]
pub async fn mcp_import_preview(
    state: State<'_, AppState>,
    json: String,
) -> AppResult<McpImportPreview> {
    let json = Zeroizing::new(json);
    let report = parse(&json)?;
    state.with_unlocked(|v| Ok(import_preview(v, &report)))
}

/// Adds every importable server; env and header values move into the vault.
#[tauri::command]
#[specta::specta]
pub async fn mcp_import(
    app: AppHandle,
    state: State<'_, AppState>,
    json: String,
) -> AppResult<Vec<McpServerView>> {
    let json = Zeroizing::new(json);
    let report = parse(&json)?;
    let views = state.with_unlocked(|v| import_servers(v, report))?;
    tracing::info!(count = views.len(), "MCP servers imported");
    sync::local_change(&app);
    Ok(views)
}

/// `mcpServers` JSON with placeholders in place of env and header values.
#[tauri::command]
#[specta::specta]
pub async fn mcp_export(state: State<'_, AppState>) -> AppResult<String> {
    state.with_unlocked(|v| Ok(export(v)))
}

#[cfg(test)]
mod tests;
