//! Shared HTTP plumbing: the rustls provider, address classification, cancellable sends,
//! capped body reads and error mapping.
//!
//! Nothing here logs a URL, a header or a body (SEC-04). Error messages describe the failure
//! without the URL, and HTTP errors carry the provider's own message.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Once};

use futures::future::BoxFuture;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::CONTENT_TYPE;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::error::AiError;

static INSTALL_PROVIDER: Once = Once::new();

/// Installs the process-wide rustls crypto provider (ring), like `hatoba-core`'s sync transport.
/// Idempotent; harmless if another component already installed one.
pub(crate) fn ensure_crypto_provider() {
    INSTALL_PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// User-Agent of every request this crate makes.
pub(crate) const USER_AGENT: &str = concat!("Hatoba/", env!("CARGO_PKG_VERSION"));

/// Error bodies larger than this are cut before the message is extracted.
const MAX_ERROR_BODY: usize = 64 * 1024;
/// Longest provider message passed on.
const MAX_MESSAGE_CHARS: usize = 500;

// ---------------------------------------------------------------------------------------------
// Address classification
// ---------------------------------------------------------------------------------------------

/// The IPv4 address an IPv6 address carries, for the forms a router or the OS may translate to
/// IPv4: IPv4-mapped (`::ffff:a.b.c.d`), IPv4-compatible (`::a.b.c.d`), NAT64 (`64:ff9b::/96`),
/// the local-use NAT64 prefix (`64:ff9b:1::/48`, RFC 8215) and 6to4 (`2002::/16`).
///
/// Operators carve longer prefixes out of the local-use range; the IPv4 address is read from the
/// last 32 bits, which is where it sits in the common `/96` layout.
fn embedded_v4(v6: &Ipv6Addr) -> Option<Ipv4Addr> {
    if let Some(v4) = v6.to_ipv4_mapped() {
        return Some(v4);
    }
    let s = v6.segments();
    let tail = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
    if s[..6] == [0; 6] && !v6.is_unspecified() && !v6.is_loopback() {
        return Some(tail);
    }
    if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] || s[..3] == [0x64, 0xff9b, 1] {
        return Some(tail);
    }
    if s[0] == 0x2002 {
        return Some(Ipv4Addr::new(
            (s[1] >> 8) as u8,
            s[1] as u8,
            (s[2] >> 8) as u8,
            s[2] as u8,
        ));
    }
    None
}

/// 100.64.0.0/10 (carrier-grade NAT, RFC 6598).
fn is_cgnat(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    o[0] == 100 && (o[1] & 0xc0) == 64
}

/// Whether a base URL may use plain `http` with this address (AI-02): loopback, unspecified,
/// RFC 1918, link-local, CGNAT, unique local IPv6 and IPv4-mapped forms of these.
pub(crate) fn is_local_network(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_unspecified()
                || v4.is_private()
                || v4.is_link_local()
                || is_cgnat(v4)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_local_network(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                || (first & 0xfe00) == 0xfc00 // fc00::/7 unique local
                || (first & 0xffc0) == 0xfe80 // fe80::/10 link-local
        }
    }
}

/// Whether `fetch_url` must refuse this address (AI-15): loopback, private, link-local,
/// unspecified, multicast, broadcast, reserved, CGNAT, site-local, and IPv6 forms that carry
/// such an IPv4 address.
///
/// 198.18.0.0/15 (benchmarking, RFC 2544) is deliberately allowed: fake-IP proxies in TUN mode
/// (Clash, sing-box) answer every lookup with an address from it. Under such a proxy this check
/// cannot protect the user, because the proxy connects to the name's real address itself.
pub(crate) fn is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            is_local_network(IpAddr::V4(v4))
                || o[0] == 0 // 0.0.0.0/8 "this network"
                || v4.is_multicast()
                || v4.is_broadcast()
                || o[0] >= 240 // 240.0.0.0/4 reserved
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = embedded_v4(&v6) {
                return is_blocked(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            is_local_network(IpAddr::V6(v6)) || v6.is_multicast() || (first & 0xffc0) == 0xfec0 // fec0::/10 site-local (deprecated)
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Guarded resolver
// ---------------------------------------------------------------------------------------------

/// Resolves a host name to its addresses.
pub(crate) type Lookup =
    Arc<dyn Fn(String) -> BoxFuture<'static, io::Result<Vec<IpAddr>>> + Send + Sync>;

/// The system resolver.
pub(crate) fn system_lookup() -> Lookup {
    Arc::new(|host: String| {
        Box::pin(async move {
            Ok(tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .map(|addr| addr.ip())
                .collect())
        })
    })
}

/// A resolver that refuses a name when any of its addresses is not allowed, and otherwise
/// returns exactly the addresses it checked (the connection is pinned to them, so the answer
/// cannot change between the check and the connection).
///
/// `refusal` builds the error, which [`network_error`] recognises in reqwest's error chain.
pub(crate) struct GuardedResolver {
    pub(crate) allowed: fn(IpAddr) -> bool,
    pub(crate) lookup: Lookup,
    pub(crate) refusal: fn() -> Box<dyn std::error::Error + Send + Sync>,
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        let allowed = self.allowed;
        let refusal = self.refusal;
        let lookup = Arc::clone(&self.lookup);
        Box::pin(async move {
            let addrs = lookup(host).await?;
            if addrs.is_empty() {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no addresses").into());
            }
            if addrs.iter().any(|ip| !allowed(*ip)) {
                return Err(refusal());
            }
            let addrs: Addrs = Box::new(addrs.into_iter().map(|ip| SocketAddr::new(ip, 0)));
            Ok(addrs)
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------------------------

/// Sends a request, giving up with [`AiError::Cancelled`] as soon as `cancel` fires (dropping
/// the connection).
pub(crate) async fn send(
    request: reqwest::RequestBuilder,
    cancel: &CancellationToken,
) -> Result<reqwest::Response, AiError> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(AiError::Cancelled),
        result = request.send() => result.map_err(|e| network_error(&e)),
    }
}

/// Reads at most `cap` bytes of a body. Returns the bytes and whether the body was longer.
pub(crate) async fn read_capped(
    mut response: reqwest::Response,
    cap: usize,
    cancel: &CancellationToken,
) -> Result<(Vec<u8>, bool), AiError> {
    let mut body = Vec::new();
    loop {
        let chunk = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(AiError::Cancelled),
            chunk = response.chunk() => chunk.map_err(|e| network_error(&e))?,
        };
        let Some(chunk) = chunk else {
            return Ok((body, false));
        };
        if body.len() + chunk.len() > cap {
            let room = cap - body.len();
            body.extend_from_slice(&chunk[..room]);
            return Ok((body, true));
        }
        body.extend_from_slice(&chunk);
    }
}

/// Reads a successful JSON response (capped at 16 MB).
pub(crate) async fn read_json(
    response: reqwest::Response,
    cancel: &CancellationToken,
) -> Result<Value, AiError> {
    let (body, truncated) = read_capped(response, 16 * 1024 * 1024, cancel).await?;
    if truncated {
        return Err(AiError::Protocol("the response is too large".into()));
    }
    serde_json::from_slice(&body)
        .map_err(|_| AiError::Protocol("the response is not valid JSON".into()))
}

/// Turns a non-2xx response into [`AiError::Http`] with the provider's message.
pub(crate) async fn http_error(response: reqwest::Response, cancel: &CancellationToken) -> AiError {
    let status = response.status();
    let is_json = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().contains("json"));
    let body = match read_capped(response, MAX_ERROR_BODY, cancel).await {
        Ok((body, _)) => body,
        Err(AiError::Cancelled) => return AiError::Cancelled,
        Err(_) => Vec::new(),
    };
    let message = error_message(&body)
        .or_else(|| (!is_json).then(|| excerpt(&body)).flatten())
        .unwrap_or_else(|| {
            status
                .canonical_reason()
                .unwrap_or("request failed")
                .to_owned()
        });
    AiError::Http {
        status: status.as_u16(),
        message,
    }
}

/// The provider's message from an error body: `error.message`, `message`, `error` (string),
/// `detail` (string), for an object or the first element of an array (Gemini wraps errors in
/// one).
pub(crate) fn error_message(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    message_of(&value)
}

/// [`error_message`] for an already parsed value (also used for mid-stream error events).
pub(crate) fn message_of(value: &Value) -> Option<String> {
    let value = match value {
        Value::Array(items) => items.first()?,
        other => other,
    };
    let candidates = [
        value.pointer("/error/message"),
        value.get("message"),
        value.get("error"),
        value.get("detail"),
        value.pointer("/detail/0/msg"),
    ];
    candidates
        .into_iter()
        .flatten()
        .find_map(|v| v.as_str().map(clean_message).filter(|m| !m.is_empty()))
}

/// A short single-line excerpt of a non-JSON error body (an HTML error page, plain text).
fn excerpt(body: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(body);
    let cleaned = clean_message(&text);
    (!cleaned.is_empty()).then_some(cleaned)
}

/// Collapses whitespace and control characters and caps the length, so a message is safe to
/// show on one line.
pub(crate) fn clean_message(message: &str) -> String {
    let mut out = String::new();
    let mut count = 0;
    for word in message.split(|c: char| c.is_whitespace() || c.is_control()) {
        if word.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
            count += 1;
        }
        for c in word.chars() {
            if count >= MAX_MESSAGE_CHARS {
                out.push('…');
                return out;
            }
            out.push(c);
            count += 1;
        }
    }
    out
}

/// Maps a transport-level failure (no usable HTTP response) to [`AiError::Network`], without
/// the URL. A refusal by `fetch_url`'s address guard becomes [`AiError::Blocked`], and one by the
/// guard of an `http` base URL becomes [`AiError::InvalidUrl`].
pub(crate) fn network_error(err: &reqwest::Error) -> AiError {
    use std::error::Error as _;

    let mut source = err.source();
    let mut innermost = None;
    while let Some(cause) = source {
        if cause.downcast_ref::<BlockedAddress>().is_some() {
            return AiError::Blocked(BLOCKED_MESSAGE.into());
        }
        if cause.downcast_ref::<NotLocalAddress>().is_some() {
            return AiError::InvalidUrl(NOT_LOCAL_MESSAGE.into());
        }
        innermost = Some(cause);
        source = cause.source();
    }
    let what = if err.is_timeout() {
        "the request timed out"
    } else if err.is_connect() {
        "could not connect to the server"
    } else if err.is_body() || err.is_decode() {
        "the connection broke while reading the response"
    } else if err.is_redirect() {
        "too many redirects"
    } else {
        "the request failed"
    };
    match innermost.map(|c| clean_message(&c.to_string())) {
        Some(cause) if !cause.is_empty() && !err.is_timeout() => {
            AiError::Network(format!("{what}: {cause}"))
        }
        _ => AiError::Network(what.into()),
    }
}

/// Message of a `fetch_url` refusal (the URL is left out: it is conversation content).
pub(crate) const BLOCKED_MESSAGE: &str = "the address is on a loopback, private, link-local or \
     otherwise non-public network, which fetch_url does not fetch";

/// The resolver's refusal, recognised in reqwest's error chain by [`network_error`].
#[derive(Debug)]
pub(crate) struct BlockedAddress;

impl std::fmt::Display for BlockedAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(BLOCKED_MESSAGE)
    }
}

impl std::error::Error for BlockedAddress {}

/// Builds the [`BlockedAddress`] refusal of `fetch_url`'s resolver.
pub(crate) fn refuse_blocked() -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(BlockedAddress)
}

/// Message for an `http` base URL whose host name resolves to a public address when connecting,
/// after it resolved to local addresses when the URL was checked (DNS rebinding, or a changed
/// network). The URL is left out.
pub(crate) const NOT_LOCAL_MESSAGE: &str = "the host name now resolves to a public address, and \
     http is allowed only for loopback and private network addresses; use https";

/// The refusal of the resolver that guards `http` base URLs, recognised by [`network_error`].
#[derive(Debug)]
pub(crate) struct NotLocalAddress;

impl std::fmt::Display for NotLocalAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(NOT_LOCAL_MESSAGE)
    }
}

impl std::error::Error for NotLocalAddress {}

/// Builds the [`NotLocalAddress`] refusal of the resolver that guards `http` base URLs.
pub(crate) fn refuse_not_local() -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(NotLocalAddress)
}

/// `{base}{suffix}` with the base path's trailing slashes removed; the query (e.g. Azure's
/// `api-version`) is kept and the fragment dropped.
pub(crate) fn endpoint(base: &url::Url, suffix: &str) -> url::Url {
    let mut url = base.clone();
    let path = format!("{}{suffix}", url.path().trim_end_matches('/'));
    url.set_path(&path);
    url.set_fragment(None);
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn local_network_addresses_for_http_base_urls() {
        for local in [
            "127.0.0.1",
            "127.8.9.10",
            "0.0.0.0",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.10",
            "169.254.1.1",
            "100.64.0.1",
            "100.127.255.254",
            "::1",
            "::",
            "fd12:3456::1",
            "fe80::1",
            "::ffff:192.168.1.1",
        ] {
            assert!(is_local_network(ip(local)), "{local} is local");
        }
        for public in [
            "8.8.8.8",
            "172.32.0.1",
            "100.128.0.1",
            "100.63.255.255",
            "1.1.1.1",
            "2001:4860:4860::8888",
            "::ffff:8.8.8.8",
            "224.0.0.1",
        ] {
            assert!(!is_local_network(ip(public)), "{public} is not local");
        }
    }

    #[test]
    fn blocked_addresses_for_fetch_url() {
        for blocked in [
            "127.0.0.1",
            "0.0.0.0",
            "0.1.2.3",
            "10.0.0.1",
            "172.20.1.1",
            "192.168.0.1",
            "169.254.169.254",
            "100.100.100.200",
            "224.0.0.251",
            "239.255.255.250",
            "255.255.255.255",
            "240.0.0.1",
            "::1",
            "::",
            "fc00::1",
            "fdff::1",
            "fe80::abcd",
            "fec0::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::127.0.0.1",
            "64:ff9b::a9fe:a9fe",
            // Local-use NAT64 (RFC 8215), the address in the last 32 bits.
            "64:ff9b:1::a9fe:a9fe",
            "64:ff9b:1::a00:1",
            "64:ff9b:1:1:2:3:c0a8:101",
            "2002:c0a8:0101::1",
        ] {
            assert!(is_blocked(ip(blocked)), "{blocked} must be blocked");
        }
        for allowed in [
            "8.8.8.8",
            "93.184.216.34",
            // 198.18.0.0/15 stays allowed on purpose. Fake-IP proxies in TUN mode (Clash,
            // sing-box) answer every lookup with an address from it, so blocking it would break
            // `fetch_url` for their users. For the same reason the resolver check cannot protect
            // them: the proxy connects to the name's real address itself, after the check.
            "198.18.0.1",
            "2606:4700::1111",
            "::ffff:1.1.1.1",
            "64:ff9b::808:808",
            "64:ff9b:1::808:808",
            "2002:0808:0808::1",
        ] {
            assert!(!is_blocked(ip(allowed)), "{allowed} must be allowed");
        }
    }

    #[test]
    fn provider_messages_are_extracted_and_cleaned() {
        let cases: [(&[u8], Option<&str>); 7] = [
            (
                br#"{"error":{"message":"Invalid API key","type":"auth"}}"#,
                Some("Invalid API key"),
            ),
            (
                br#"{"type":"error","error":{"type":"not_found_error","message":"model: x"}}"#,
                Some("model: x"),
            ),
            (
                br#"[{"error":{"code":400,"message":"bad\nrequest"}}]"#,
                Some("bad request"),
            ),
            (br#"{"message":"nope"}"#, Some("nope")),
            (
                br#"{"error":"stream_options unsupported"}"#,
                Some("stream_options unsupported"),
            ),
            (br#"{"detail":"Not Found"}"#, Some("Not Found")),
            (b"<html>oops</html>", None),
        ];
        for (body, expected) in cases {
            assert_eq!(error_message(body).as_deref(), expected);
        }
        let long = "x".repeat(2000);
        assert_eq!(clean_message(&long).chars().count(), MAX_MESSAGE_CHARS + 1);
        assert_eq!(clean_message("  a \t\r\n b\u{7}c "), "a b c");
    }

    #[test]
    fn endpoints_join_paths_and_keep_queries() {
        let base = url::Url::parse("https://api.openai.com/v1/").unwrap();
        assert_eq!(
            endpoint(&base, "/chat/completions").as_str(),
            "https://api.openai.com/v1/chat/completions"
        );
        let azure =
            url::Url::parse("https://x.openai.azure.com/openai/deployments/d?api-version=1#f")
                .unwrap();
        assert_eq!(
            endpoint(&azure, "/chat/completions").as_str(),
            "https://x.openai.azure.com/openai/deployments/d/chat/completions?api-version=1"
        );
        let root = url::Url::parse("https://api.anthropic.com").unwrap();
        assert_eq!(
            endpoint(&root, "/v1/messages").as_str(),
            "https://api.anthropic.com/v1/messages"
        );
    }
}
