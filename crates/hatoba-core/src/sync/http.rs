//! Shared HTTP plumbing for the Worker and D1 backends.
//!
//! TLS is rustls with the `ring` provider (the same crypto backend `russh` uses), installed once
//! per process. Neither backend ever logs a URL query, header or body.

use std::sync::{Once, PoisonError, RwLock};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{HeaderMap, RETRY_AFTER};
use serde::Deserialize;

use crate::error::{Error, Result};

/// Per-request timeout (spec: 15 s).
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Responses larger than this are refused (a hostile server must not exhaust memory).
pub(crate) const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

static INSTALL_PROVIDER: Once = Once::new();

/// The proxy that clients built from now on use; `None` uses the system proxy.
static PROXY: RwLock<Option<reqwest::Proxy>> = RwLock::new(None);

/// Installs the process-wide rustls crypto provider (ring). Idempotent; harmless if another
/// component already installed one.
pub(crate) fn ensure_crypto_provider() {
    INSTALL_PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Sets the proxy of sync and deploy requests; `None` uses the system proxy. Backends built
/// earlier keep the proxy they were built with.
pub fn set_proxy(proxy: Option<reqwest::Proxy>) {
    *PROXY.write().unwrap_or_else(PoisonError::into_inner) = proxy;
}

/// Builds the shared client, through the proxy of [`set_proxy`]. Loopback targets bypass any
/// proxy.
pub(crate) fn build_client(loopback: bool) -> Result<reqwest::Client> {
    ensure_crypto_provider();
    let mut builder = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        // Never follow redirects: they could carry the bearer token to another origin.
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("hatoba-core/", env!("CARGO_PKG_VERSION")));
    if loopback {
        builder = builder.no_proxy();
    } else if let Some(proxy) = PROXY.read().unwrap_or_else(PoisonError::into_inner).clone() {
        builder = builder.proxy(proxy);
    }
    builder
        .build()
        .map_err(|_| Error::Server("could not build the HTTP client".into()))
}

/// Maps a transport-level failure (no HTTP response) to an error.
pub(crate) fn map_transport(err: &reqwest::Error) -> Error {
    if err.is_timeout() || err.is_connect() || err.is_request() {
        Error::Offline
    } else if err.is_builder() || err.is_redirect() {
        Error::Server("invalid request".into())
    } else {
        // Body read failures, TLS errors surfaced late, etc.: treat as unreachable.
        Error::Offline
    }
}

/// Reads a response body with a size cap.
pub(crate) async fn read_body(
    mut resp: reqwest::Response,
) -> Result<(StatusCode, HeaderMap, Vec<u8>)> {
    let status = resp.status();
    let headers = resp.headers().clone();
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| map_transport(&e))? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(Error::Protocol("response too large".into()));
        }
        body.extend_from_slice(&chunk);
    }
    Ok((status, headers, body))
}

/// `{ "error": "<code>", "message"?: "…" }` as the Worker reports failures.
#[derive(Deserialize)]
struct ErrorBody {
    error: Option<String>,
}

/// The short machine code from an error body, sanitised so it is safe to log and display.
pub(crate) fn error_code(body: &[u8]) -> Option<String> {
    let parsed: ErrorBody = serde_json::from_slice(body).ok()?;
    let code = parsed.error?;
    let clean: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ' '))
        .take(64)
        .collect();
    (!clean.is_empty()).then_some(clean)
}

/// `Retry-After` in whole seconds, if present and numeric.
pub(crate) fn retry_after(headers: &HeaderMap) -> Option<u64> {
    headers.get(RETRY_AFTER)?.to_str().ok()?.trim().parse().ok()
}

/// Whether a host is loopback (development servers).
pub(crate) fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_sanitised() {
        assert_eq!(
            error_code(br#"{"error":"not_initialized","message":"x"}"#).as_deref(),
            Some("not_initialized")
        );
        assert_eq!(error_code(br#"{"error":"a<b>\n"}"#).as_deref(), Some("ab"));
        assert_eq!(error_code(b"<html>").as_deref(), None);
        assert_eq!(error_code(br#"{"error":""}"#).as_deref(), None);
        let long = format!(r#"{{"error":"{}"}}"#, "x".repeat(500));
        assert_eq!(error_code(long.as_bytes()).unwrap().len(), 64);
    }

    #[test]
    fn loopback_detection() {
        for h in ["localhost", "127.0.0.1", "[::1]"] {
            assert!(is_loopback_host(h));
        }
        assert!(!is_loopback_host("example.com"));
        assert!(!is_loopback_host("127.0.0.1.evil.com"));
    }
}
