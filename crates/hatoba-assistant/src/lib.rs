//! The Hatoba AI assistant (spec §13), independent of Tauri: the runtime that runs turns, tools and
//! conversations ([`ai`]), the MCP servers whose tools requests offer ([`mcp`]), the skills
//! ([`skills`]) and settings ([`settings`]) of Settings → AI, the files dropped on the window
//! ([`dropped`]), and the DTOs the desktop app's `ai_*`, skill and MCP commands exchange with the
//! WebView ([`dto`]).
//!
//! The desktop app keeps the commands, and gives the runtime what it needs through small traits:
//! where a turn's events go ([`ai::EventSink`]), the sync trigger and the tabs' sessions
//! ([`ai::AiEnv`]), where MCP status changes go ([`mcp::McpEvents`]), and its async runtime
//! ([`runtime::set`]). So the turn and MCP tests run without Tauri.

pub mod ai;
pub mod dropped;
pub mod dto;
pub mod mcp;
pub mod runtime;
pub mod settings;
pub mod skills;

use hatoba_ipc::error;

/// Now, in Unix milliseconds.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}
