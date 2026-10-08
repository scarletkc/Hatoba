//! MCP client (spec §13.9): tools only, over stdio child processes and Streamable HTTP.
//!
//! Hatoba is an MCP client for tools only: it declares no client capability (no sampling,
//! elicitation or roots) and uses no server prompts or resources.
//!
//! - [`McpConnection`]: one session with one server: start the child or open the HTTP session,
//!   list and call its tools, and shut it down (AI-32).
//! - [`tool_names`] and [`clean_schema`]: what a request offers the model (AI-30).
//! - [`config`]: import and export in the `mcpServers` format (AI-33).
//!
//! Nothing here logs an environment or header value, a tool's arguments or result, or a server's
//! stderr (SEC-04), and error messages carry none of them. The `rmcp` SDK itself logs protocol
//! messages and session ids through `tracing` (at `info` and below), so the application's log
//! filter must silence the `rmcp` target.

use std::fmt;

use zeroize::Zeroizing;

mod client;
pub mod config;
mod names;
mod schema;
mod stdio;
#[cfg(test)]
mod tests;

pub use client::McpConnection;
pub use names::{MAX_TOOL_NAME_LEN, tool_names};
pub use schema::clean_schema;
pub use stdio::{STDERR_LINES, StderrTail};

/// How to reach an MCP server. The environment and header values are secrets: they are wiped when
/// dropped, never printed by `Debug`, and only the child process or the server receives them.
/// `Debug` does not print the URL beyond its scheme, host and port (hosted servers often carry a
/// token in the path or query), nor the arguments (only how many there are).
#[derive(Clone, PartialEq, Eq)]
pub enum McpTransportConfig {
    /// A child process started on this device, speaking JSON-RPC over its stdin and stdout.
    Stdio {
        /// Executable, looked up on `PATH` like a shell does (`PATHEXT` on Windows).
        command: String,
        /// Arguments.
        args: Vec<String>,
        /// Environment variables added to the inherited environment (values are secrets). A
        /// `PATH` here is also the one the command is looked up on.
        env: Vec<(String, Zeroizing<String>)>,
    },
    /// A Streamable HTTP endpoint.
    Http {
        /// Endpoint URL: `https`, or `http` for loopback and private network addresses.
        url: String,
        /// Headers sent with every request (values are secrets).
        headers: Vec<(String, Zeroizing<String>)>,
    },
}

impl McpTransportConfig {
    /// `"stdio"` or `"http"`, as the vault item's `kind`.
    #[must_use]
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Stdio { .. } => "stdio",
            Self::Http { .. } => "http",
        }
    }
}

impl fmt::Debug for McpTransportConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Names only: the values are what is secret.
        fn redacted(pairs: &[(String, Zeroizing<String>)]) -> Vec<(&str, &'static str)> {
            pairs
                .iter()
                .map(|(name, _)| (name.as_str(), "<redacted>"))
                .collect()
        }
        match self {
            Self::Stdio { command, args, env } => f
                .debug_struct("Stdio")
                .field("command", command)
                .field("arg_count", &args.len())
                .field("env", &redacted(env))
                .finish(),
            Self::Http { url, headers } => f
                .debug_struct("Http")
                .field("origin", &origin_of(url))
                .field("headers", &redacted(headers))
                .finish(),
        }
    }
}

/// `scheme://host[:port]` of a server URL: no user name, password, path, query or fragment.
fn origin_of(url: &str) -> String {
    let Ok(url) = url::Url::parse(url.trim()) else {
        return "<invalid URL>".into();
    };
    let Some(host) = url.host_str() else {
        return "<no host>".into();
    };
    match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    }
}

/// A tool's annotations as the server describes them. They are hints for display only and never
/// change whether a call asks for approval (AI-31).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolAnnotations {
    /// Display title: the tool's `title`, else `annotations.title`.
    pub title: Option<String>,
    /// `readOnlyHint`: the tool does not modify its environment.
    pub read_only_hint: Option<bool>,
    /// `destructiveHint`: the tool may delete or overwrite.
    pub destructive_hint: Option<bool>,
    /// `idempotentHint`: repeating a call has no further effect.
    pub idempotent_hint: Option<bool>,
    /// `openWorldHint`: the tool reaches outside entities, such as the web.
    pub open_world_hint: Option<bool>,
}

/// A tool as `tools/list` returned it.
#[derive(Clone, Debug, PartialEq)]
pub struct McpTool {
    /// The tool's name on its server (before [`tool_names`] maps it for the model).
    pub name: String,
    /// The server's description, empty when it gave none. It reaches the model as instructions,
    /// so settings show it in full (§4.4).
    pub description: String,
    /// The input schema as the server sent it; [`clean_schema`] makes it acceptable to a provider.
    pub input_schema: serde_json::Value,
    /// Display hints.
    pub annotations: ToolAnnotations,
}

/// What a `tools/call` returned, reduced to text (AI-30).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpToolResult {
    /// The server's `isError`: the tool ran and failed.
    pub is_error: bool,
    /// Text blocks (and the text of embedded text resources) joined with newlines; every other
    /// block is one line naming its type, such as `[image content omitted]`. When the result has
    /// no content blocks, its `structuredContent` as JSON.
    pub content: String,
}

/// Everything that can go wrong with an MCP server. Messages never contain an environment or
/// header value, a URL, a tool's arguments or result, or stderr; the stderr tail is available
/// separately ([`McpConnection::stderr_tail`], [`StderrTail`]).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum McpError {
    /// The child process could not be started, or it exited before it finished initializing.
    #[error("the server could not start: {0}")]
    Spawn(String),
    /// The command was not found on `PATH` (the command as configured).
    #[error("command not found: {0}")]
    CommandNotFound(String),
    /// The HTTP server could not be reached or refused the connection or the credentials.
    #[error("could not connect to the server: {0}")]
    Connect(String),
    /// The server answered something this client cannot use.
    #[error("unexpected answer from the server: {0}")]
    Protocol(String),
    /// The server answered with a JSON-RPC error (its message and code).
    #[error("the server returned an error: {0}")]
    Server(String),
    /// The session is closed: shut down, the child exited, or the transport broke.
    #[error("the server connection is closed")]
    Closed,
    /// The server did not answer in time.
    #[error("the server did not answer in time")]
    Timeout,
    /// The caller's cancellation token fired.
    #[error("cancelled")]
    Cancelled,
    /// The server URL is malformed or not allowed (`http` only for local addresses).
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    /// The configuration cannot be used: an empty command, a header or variable name or value that
    /// cannot be sent, a header the transport sets itself, or an import that is not a server list.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
}
