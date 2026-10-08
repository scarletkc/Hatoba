//! Model provider adapters, web tools and the MCP client for the Hatoba AI assistant (spec §13).
//!
//! Depends on neither Tauri nor `hatoba-ssh` (spec §13.1): the desktop shell owns the turn loop,
//! the vault and the terminal, and calls into this crate to build requests from stored entries,
//! stream responses and run the web tools.
//!
//! - [`entry`]: the stored conversation entry JSON (§13.7).
//! - [`provider`]: provider configuration, base URL validation (AI-02) and the shared HTTP client.
//! - [`chat`]: the two protocol adapters (Chat Completions and Anthropic Messages).
//! - [`models`]: model listing (AI-03) and Test Connection (AI-04).
//! - [`tools`]: built-in tool definitions, the system prompt and tool result helpers (§13.4).
//! - [`web`]: `web_search` (AI-14) and `fetch_url` (AI-15).
//!
//! Nothing here logs an API key, a request or response body, or conversation content (SEC-04).

#![forbid(unsafe_code)]

pub mod chat;
pub mod entry;
pub mod error;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod models;
mod net;
pub mod provider;
#[cfg(feature = "skills")]
pub mod skills;
pub mod tools;
pub mod web;

pub use error::AiError;
