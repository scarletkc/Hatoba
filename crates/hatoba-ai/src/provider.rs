//! Provider configuration, base URL validation (AI-02) and the shared HTTP client.

use std::fmt;
use std::time::Duration;

use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use url::{Host, Url};
use zeroize::Zeroizing;

use crate::error::AiError;
use crate::net::{self, is_local_network};

/// The wire protocol a provider speaks (spec §13.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// OpenAI Chat Completions: `POST {base_url}/chat/completions`.
    #[default]
    ChatCompletions,
    /// Anthropic Messages: `POST {base_url}/v1/messages`.
    Anthropic,
}

/// Which header carries the key for the `anthropic` protocol (Chat Completions always uses
/// `Authorization: Bearer`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuthHeader {
    /// `x-api-key: <key>`.
    #[default]
    #[serde(rename = "x-api-key")]
    XApiKey,
    /// `Authorization: Bearer <key>`.
    #[serde(rename = "authorization")]
    Authorization,
}

/// What a request needs to reach a provider. The key is wiped when dropped and never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderConfig {
    /// The provider's protocol.
    pub protocol: Protocol,
    /// Base URL as entered, e.g. `https://api.openai.com/v1` or `https://api.anthropic.com`.
    pub base_url: String,
    /// API key; may be empty for local servers (no auth header is sent then).
    pub api_key: Zeroizing<String>,
    /// Header for the key (`anthropic` only).
    pub auth_header: AuthHeader,
}

impl fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("protocol", &self.protocol)
            .field("base_url", &self.base_url)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "<empty>"
                } else {
                    "<redacted>"
                },
            )
            .field("auth_header", &self.auth_header)
            .finish()
    }
}

impl ProviderConfig {
    /// The authentication and version headers of every request to this provider. The key's
    /// header value is marked sensitive so the HTTP stack never prints it.
    pub(crate) fn headers(&self) -> Result<HeaderMap, AiError> {
        let mut headers = HeaderMap::new();
        if self.protocol == Protocol::Anthropic {
            headers.insert(
                HeaderName::from_static("anthropic-version"),
                HeaderValue::from_static("2023-06-01"),
            );
        }
        if self.api_key.is_empty() {
            return Ok(headers);
        }
        let (name, value) = match (self.protocol, self.auth_header) {
            (Protocol::Anthropic, AuthHeader::XApiKey) => (
                HeaderName::from_static("x-api-key"),
                Zeroizing::new(self.api_key.trim().to_owned()),
            ),
            _ => (
                AUTHORIZATION,
                Zeroizing::new(format!("Bearer {}", self.api_key.trim())),
            ),
        };
        let mut value = HeaderValue::from_str(&value).map_err(|_| {
            AiError::Config("the API key contains characters an HTTP header cannot carry".into())
        })?;
        value.set_sensitive(true);
        headers.insert(name, value);
        Ok(headers)
    }
}

/// A model as a request needs it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelSpec {
    /// The provider's model id.
    pub id: String,
    /// Context window in tokens, when known.
    pub context_window: Option<u64>,
    /// Output limit in tokens, when known. Anthropic requests send it as `max_tokens`
    /// (16,000 when unknown).
    pub max_output_tokens: Option<u64>,
}

impl ModelSpec {
    /// A model with only an id.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ..Self::default()
        }
    }
}

/// Checks a provider or SearXNG base URL (AI-02) and returns it parsed.
///
/// `https` is always allowed. `http` only when the host is a loopback or private network
/// address (RFC 1918, unique local, link-local, CGNAT 100.64/10): IP literals are checked
/// directly, `localhost` is allowed, and other host names are resolved and every address must be
/// local. URLs with credentials, without a host, or with another scheme are refused.
pub async fn validate_base_url(base_url: &str) -> Result<Url, AiError> {
    let mut url = Url::parse(base_url.trim())
        .map_err(|_| AiError::InvalidUrl("the base URL is not a valid URL".into()))?;
    url.set_fragment(None);
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AiError::InvalidUrl(
            "the base URL must not contain a user name or password".into(),
        ));
    }
    let host = url
        .host()
        .ok_or_else(|| AiError::InvalidUrl("the base URL has no host".into()))?;
    match url.scheme() {
        "https" => Ok(url),
        "http" => {
            let local = match host {
                Host::Ipv4(ip) => is_local_network(ip.into()),
                Host::Ipv6(ip) => is_local_network(ip.into()),
                Host::Domain(name) if name.eq_ignore_ascii_case("localhost") => true,
                Host::Domain(name) => {
                    let port = url.port_or_known_default().unwrap_or(80);
                    let addrs: Vec<_> = tokio::net::lookup_host((name, port))
                        .await
                        .map_err(|_| AiError::Network("could not resolve the host".into()))?
                        .collect();
                    !addrs.is_empty() && addrs.iter().all(|a| is_local_network(a.ip()))
                }
            };
            if local {
                Ok(url)
            } else {
                Err(AiError::InvalidUrl(
                    "http is allowed only for loopback and private network addresses; use https"
                        .into(),
                ))
            }
        }
        _ => Err(AiError::InvalidUrl(
            "the base URL must start with https:// (or http:// for a local server)".into(),
        )),
    }
}

/// Connect timeout of provider and search requests.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Longest silence while reading a response. Streams have no overall timeout: a long answer can
/// take many minutes, but a provider that sends nothing for this long is gone.
const READ_TIMEOUT: Duration = Duration::from_secs(300);
/// Redirects followed, and only within the same origin so a key never reaches another host.
const MAX_REDIRECTS: usize = 5;

/// The client for provider and search requests: rustls with the ring provider, connect timeout
/// 15 s, read (idle) timeout 5 minutes and no overall timeout, User-Agent `Hatoba/<version>`,
/// the system proxy, and redirects followed only within the same origin.
///
/// Build it once and share it (cloning is cheap); it holds a connection pool bound to the tokio
/// runtime it is first used on.
#[must_use]
pub fn http_client() -> reqwest::Client {
    net::ensure_crypto_provider();
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .user_agent(net::USER_AGENT)
        .referer(false)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let previous = attempt.previous();
            if previous.len() > MAX_REDIRECTS {
                return attempt.error("too many redirects");
            }
            let same_origin = previous
                .first()
                .is_some_and(|first| first.origin() == attempt.url().origin());
            if same_origin {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        // Only fails when no TLS provider is available, which `ensure_crypto_provider` rules out.
        .expect("the HTTP client must build with the ring provider installed")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(protocol: Protocol, key: &str, auth_header: AuthHeader) -> ProviderConfig {
        ProviderConfig {
            protocol,
            base_url: "https://example.com".into(),
            api_key: Zeroizing::new(key.into()),
            auth_header,
        }
    }

    #[test]
    fn debug_never_prints_the_key() {
        let p = config(
            Protocol::Anthropic,
            "sk-ant-secret-123",
            AuthHeader::XApiKey,
        );
        let debug = format!("{p:?}");
        assert!(!debug.contains("sk-ant-secret-123"), "{debug}");
        assert!(debug.contains("<redacted>"));
        let headers = p.headers().unwrap();
        assert!(!format!("{headers:?}").contains("sk-ant-secret-123"));
        assert!(headers["x-api-key"].is_sensitive());
    }

    #[test]
    fn auth_headers_follow_protocol_and_setting() {
        let h = config(Protocol::ChatCompletions, "k1", AuthHeader::XApiKey)
            .headers()
            .unwrap();
        assert_eq!(h[AUTHORIZATION], "Bearer k1");
        assert!(h.get("anthropic-version").is_none());

        let h = config(Protocol::Anthropic, "k2", AuthHeader::XApiKey)
            .headers()
            .unwrap();
        assert_eq!(h["x-api-key"], "k2");
        assert_eq!(h["anthropic-version"], "2023-06-01");
        assert!(h.get(AUTHORIZATION).is_none());

        let h = config(Protocol::Anthropic, "k3", AuthHeader::Authorization)
            .headers()
            .unwrap();
        assert_eq!(h[AUTHORIZATION], "Bearer k3");
        assert!(h.get("x-api-key").is_none());

        let h = config(Protocol::ChatCompletions, "", AuthHeader::XApiKey)
            .headers()
            .unwrap();
        assert!(h.is_empty(), "an empty key sends no auth header");

        let err = config(Protocol::ChatCompletions, "bad\nkey", AuthHeader::XApiKey)
            .headers()
            .unwrap_err();
        assert!(!err.to_string().contains("bad"));
    }

    #[test]
    fn protocol_and_header_serde_names() {
        assert_eq!(
            serde_json::to_string(&Protocol::ChatCompletions).unwrap(),
            "\"chat_completions\""
        );
        assert_eq!(
            serde_json::to_string(&Protocol::Anthropic).unwrap(),
            "\"anthropic\""
        );
        assert_eq!(
            serde_json::to_string(&AuthHeader::XApiKey).unwrap(),
            "\"x-api-key\""
        );
        assert_eq!(
            serde_json::to_string(&AuthHeader::Authorization).unwrap(),
            "\"authorization\""
        );
    }

    #[tokio::test]
    async fn base_urls_need_https_unless_local() {
        for ok in [
            "https://api.openai.com/v1",
            "https://api.deepseek.com/anthropic/",
            "http://localhost:11434/v1",
            "http://LOCALHOST:1234",
            "http://127.0.0.1:8080/v1",
            "http://192.168.1.20:1234/v1",
            "http://10.0.0.5",
            "http://100.101.102.103:11434/v1",
            "http://[::1]:8000",
            "http://[fd00::5]/v1",
            " https://api.anthropic.com ",
        ] {
            assert!(validate_base_url(ok).await.is_ok(), "{ok} must be accepted");
        }
        for bad in [
            "http://8.8.8.8/v1",
            "http://[2001:4860:4860::8888]/v1",
            "ftp://example.com",
            "file:///etc/passwd",
            "not a url",
            "https://user:pass@example.com",
            "",
        ] {
            assert!(
                matches!(validate_base_url(bad).await, Err(AiError::InvalidUrl(_))),
                "{bad} must be refused"
            );
        }
        let url = validate_base_url("https://example.com/v1#frag")
            .await
            .unwrap();
        assert_eq!(url.fragment(), None);
    }
}
