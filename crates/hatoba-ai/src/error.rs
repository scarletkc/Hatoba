//! Errors returned by the protocol adapters and the web tools.
//!
//! Messages never carry an API key or request content (SEC-04): an HTTP error carries the
//! provider's own error message, and network errors describe the failure without the URL, so
//! the desktop shell can show them and hand them to the model as tool results.

/// Everything that can go wrong talking to a provider, a search service or a fetched page.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AiError {
    /// The URL is malformed or not allowed: a base URL that is neither HTTPS nor a local
    /// address (AI-02), or a `fetch_url` URL that is not `http`/`https` (AI-15).
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    /// The configuration cannot be used, e.g. an API key with characters an HTTP header cannot
    /// carry. The message never contains the key.
    #[error("invalid configuration: {0}")]
    Config(String),
    /// A non-2xx response, or an error event in the middle of a stream. `message` is the
    /// provider's error message (or a short excerpt of the response body).
    #[error("HTTP {status}: {message}")]
    Http {
        /// HTTP status, or the status implied by a mid-stream error event.
        status: u16,
        /// The provider's message.
        message: String,
    },
    /// Connection, DNS, TLS or timeout failure: no usable response.
    #[error("network error: {0}")]
    Network(String),
    /// The response could not be understood (malformed stream, unexpected JSON).
    #[error("unexpected response: {0}")]
    Protocol(String),
    /// `fetch_url` refused the address: loopback, private, link-local and similar ranges (AI-15).
    #[error("blocked: {0}")]
    Blocked(String),
    /// `fetch_url` refused the content type: only text, HTML, JSON and XML are fetched (AI-15).
    #[error("unsupported content type: {0}")]
    UnsupportedContentType(String),
    /// The caller's cancellation token fired.
    #[error("cancelled")]
    Cancelled,
}

impl AiError {
    /// The HTTP status of an [`AiError::Http`].
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Http { status, .. } => Some(*status),
            _ => None,
        }
    }
}
