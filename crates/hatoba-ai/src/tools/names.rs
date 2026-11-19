//! Built-in tool names and registry (spec §13.4).

use crate::judge::CallKind;

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

/// Every built-in tool Hatoba offers or handles (spec §13.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinTool {
    ReadTerminal,
    RunCommand,
    SendInput,
    ReadFile,
    EditFile,
    WriteFile,
    Grep,
    Glob,
    WebSearch,
    FetchUrl,
    ReadSkill,
    SearchTools,
    CallMcpTool,
    Memory,
}

impl BuiltinTool {
    #[cfg(test)]
    const ALL: [Self; 14] = [
        Self::ReadTerminal,
        Self::RunCommand,
        Self::SendInput,
        Self::ReadFile,
        Self::EditFile,
        Self::WriteFile,
        Self::Grep,
        Self::Glob,
        Self::WebSearch,
        Self::FetchUrl,
        Self::ReadSkill,
        Self::SearchTools,
        Self::CallMcpTool,
        Self::Memory,
    ];

    /// The tool's name as offered to the model.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ReadTerminal => READ_TERMINAL,
            Self::RunCommand => RUN_COMMAND,
            Self::SendInput => SEND_INPUT,
            Self::ReadFile => READ_FILE,
            Self::EditFile => EDIT_FILE,
            Self::WriteFile => WRITE_FILE,
            Self::Grep => GREP,
            Self::Glob => GLOB,
            Self::WebSearch => WEB_SEARCH,
            Self::FetchUrl => FETCH_URL,
            Self::ReadSkill => READ_SKILL,
            Self::SearchTools => SEARCH_TOOLS,
            Self::CallMcpTool => CALL_MCP_TOOL,
            Self::Memory => MEMORY,
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            READ_TERMINAL => Self::ReadTerminal,
            RUN_COMMAND => Self::RunCommand,
            SEND_INPUT => Self::SendInput,
            READ_FILE => Self::ReadFile,
            EDIT_FILE => Self::EditFile,
            WRITE_FILE => Self::WriteFile,
            GREP => Self::Grep,
            GLOB => Self::Glob,
            WEB_SEARCH => Self::WebSearch,
            FETCH_URL => Self::FetchUrl,
            READ_SKILL => Self::ReadSkill,
            SEARCH_TOOLS => Self::SearchTools,
            CALL_MCP_TOOL => Self::CallMcpTool,
            MEMORY => Self::Memory,
            _ => return None,
        })
    }

    /// The Auto mode judge's [`CallKind`] for this tool (AI-47).
    #[must_use]
    pub(crate) const fn call_kind(self) -> CallKind {
        match self {
            Self::EditFile | Self::WriteFile => CallKind::File,
            Self::FetchUrl => CallKind::Fetch,
            Self::ReadTerminal
            | Self::RunCommand
            | Self::SendInput
            | Self::ReadFile
            | Self::Grep
            | Self::Glob
            | Self::WebSearch
            | Self::ReadSkill
            | Self::SearchTools
            | Self::CallMcpTool
            | Self::Memory => CallKind::Command,
        }
    }

    /// What this tool does, for the judge of Auto mode (AI-47); `None` for tools the judge never
    /// sees (read-only tools and deferred-MCP wrappers).
    #[must_use]
    pub(crate) const fn judge_description(self) -> Option<&'static str> {
        Some(match self {
            Self::RunCommand => {
                "Runs `command` in a shell on `host`, apart from the terminal, and returns its \
                 output. It starts in `workdir` when given, else in the user's home directory."
            }
            Self::SendInput => {
                "Types `text` into a terminal connected to `host`, as the keyboard does, then \
                 presses each of `keys` in order; a key that is a single character types it. The \
                 program reading the terminal, which `screen_tail` shows, decides what that does: \
                 at a shell prompt, text sent with Enter runs as a command line."
            }
            Self::EditFile => {
                "Replaces `old_string` with `new_string` in the text file at `path` on `host`, \
                 every occurrence when `replace_all` is true."
            }
            Self::WriteFile => {
                "Creates the text file at `path` on `host` with `content`, or replaces all of its \
                 content."
            }
            Self::FetchUrl => {
                "Fetches `url` with GET from the user's own computer, not from `host`, without \
                 cookies or credentials, and returns the page as text."
            }
            Self::ReadTerminal
            | Self::ReadFile
            | Self::Grep
            | Self::Glob
            | Self::WebSearch
            | Self::ReadSkill
            | Self::SearchTools
            | Self::CallMcpTool
            | Self::Memory => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::defs::{ToolPlace, ToolSet, builtin_tools, deferred_mcp_tools};
    use super::*;

    #[test]
    fn every_builtin_tool_round_trips_through_from_name() {
        for place in [
            ToolPlace::Ssh,
            ToolPlace::Local,
            ToolPlace::LocalWindows,
            ToolPlace::Telnet,
        ] {
            let defs = builtin_tools(&ToolSet {
                terminal: true,
                web_search: true,
                read_skill: true,
                memory: true,
                place,
            });
            for def in defs.into_iter().chain(deferred_mcp_tools()) {
                let parsed = BuiltinTool::from_name(&def.name)
                    .unwrap_or_else(|| panic!("missing BuiltinTool for {}", def.name));
                assert_eq!(parsed.name(), def.name);
            }
        }
        for tool in BuiltinTool::ALL {
            assert_eq!(BuiltinTool::from_name(tool.name()), Some(tool));
        }
        assert_eq!(BuiltinTool::from_name("mcp__server__tool"), None);
        assert_eq!(BuiltinTool::from_name("unknown"), None);
    }
}
