//! Built-in tool names (spec §13.4).

/// `read_terminal` (AI-11), run by the frontend.
pub const READ_TERMINAL: &str = "read_terminal";
/// `run_command` (AI-12), run in Rust on a new exec channel.
pub const RUN_COMMAND: &str = "run_command";
/// `send_input` (AI-13), run by the frontend.
pub const SEND_INPUT: &str = "send_input";
/// `read_file` (AI-38), run in Rust on a new exec channel.
pub const READ_FILE: &str = "read_file";
/// `edit_file` (AI-39), run in Rust on a new exec channel.
pub const EDIT_FILE: &str = "edit_file";
/// `write_file` (AI-40), run in Rust on a new exec channel.
pub const WRITE_FILE: &str = "write_file";
/// `grep` (AI-48), run in Rust on a new exec channel.
pub const GREP: &str = "grep";
/// `glob` (AI-49), run in Rust on a new exec channel.
pub const GLOB: &str = "glob";
/// `web_search` (AI-14).
pub const WEB_SEARCH: &str = "web_search";
/// `fetch_url` (AI-15).
pub const FETCH_URL: &str = "fetch_url";
/// `read_skill` (AI-28).
pub const READ_SKILL: &str = "read_skill";
/// `search_tools` (AI-45), offered in place of deferred MCP tools.
pub const SEARCH_TOOLS: &str = "search_tools";
/// `call_mcp_tool` (AI-45), which calls a deferred MCP tool.
pub const CALL_MCP_TOOL: &str = "call_mcp_tool";
/// `memory` (AI-53), run in Rust against the vault.
pub const MEMORY: &str = "memory";
