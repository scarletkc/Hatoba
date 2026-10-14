//! Model provider adapters, web tools and the MCP client for the Hatoba AI assistant (spec §13).
//!
//! Depends on neither Tauri nor `hatoba-ssh` (spec §13.1): the desktop shell owns the turn loop,
//! the vault and the terminal, and calls into this crate to build requests from stored entries,
//! stream responses, run the web tools, read skills and talk to MCP servers.
//!
//! - [`entry`]: the stored conversation entry JSON (§13.7).
//! - [`provider`]: provider configuration, base URL validation (AI-02) and the shared HTTP client.
//! - [`chat`]: the three protocol adapters (Chat Completions, Anthropic Messages and Responses).
//! - [`decision`]: decision questions in TypeSafe's System One format, answered by a System One
//!   endpoint or by a chat model in its place (AI-47).
//! - [`judge`]: the judge of Auto mode, which decides whether a call runs without the approval
//!   card (AI-47).
//! - [`models`]: model listing (AI-03) and Test Connection (AI-04).
//! - [`tools`]: built-in tool definitions, the system prompt and tool result helpers (§13.4).
//! - [`files`]: the text side of `read_file`, `edit_file` and `write_file` (AI-38…40).
//! - [`search`]: the text side of `grep` and `glob` (AI-48, AI-49).
//! - [`web`]: `web_search` (AI-14) and `fetch_url` (AI-15).
//! - [`skills`]: `SKILL.md`, folder and `.zip` import, `.zip` export (§13.8).
//! - [`mcp`]: the MCP client for stdio and Streamable HTTP servers, tool names and schemas, and
//!   `mcpServers` import and export (§13.9).
//!
//! Nothing here logs an API key, a request or response body, conversation content, an MCP
//! server's environment or header values, or its stderr (SEC-04).

#![forbid(unsafe_code)]

pub mod chat;
pub mod decision;
pub mod entry;
pub mod error;
pub mod files;
pub mod judge;
pub mod mcp;
pub mod models;
mod net;
pub mod provider;
pub mod search;
pub mod skills;
pub mod tools;
pub mod web;

pub use error::AiError;
