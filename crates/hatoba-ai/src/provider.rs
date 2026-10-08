//! Provider configuration, base URL validation (AI-02) and the shared HTTP client.

use std::borrow::Cow;
use std::fmt;
use std::time::Duration;

use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use url::{Host, Url};
use zeroize::Zeroizing;

use crate::error::AiError;
use crate::net::{
    self, GuardedResolver, Lookup, is_local_network, refuse_not_local, system_lookup,
};

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

/// A thinking level (AI-05), lowest first. A request without one uses the provider's default
/// depth, which the panel calls Default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    /// `low`.
    Low,
    /// `medium`.
    Medium,
    /// `high`.
    High,
    /// `xhigh` (Extra High).
    Xhigh,
    /// `max`.
    Max,
}

impl Effort {
    /// Every level, lowest first.
    pub const ALL: [Self; 5] = [Self::Low, Self::Medium, Self::High, Self::Xhigh, Self::Max];

    /// The levels a model offers while what it supports is unknown.
    pub const UNKNOWN_MODEL: [Self; 3] = [Self::Low, Self::Medium, Self::High];

    /// The value of Anthropic's `output_config.effort` and of Chat Completions'
    /// `reasoning_effort`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
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
    /// (16,000 when unknown, at most 128,000).
    pub max_output_tokens: Option<u64>,
    /// The thinking levels the model accepts, lowest first, when known (from Anthropic's model
    /// list, or set by the user); empty when it accepts none. Unknown means
    /// [`Effort::UNKNOWN_MODEL`].
    pub efforts: Option<Vec<Effort>>,
    /// Whether the model supports adaptive thinking (Anthropic's model list), when known.
    pub adaptive_thinking: Option<bool>,
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

    /// The thinking levels the model offers: the known ones, or [`Effort::UNKNOWN_MODEL`].
    #[must_use]
    pub fn efforts(&self) -> &[Effort] {
        self.efforts.as_deref().unwrap_or(&Effort::UNKNOWN_MODEL)
    }

    /// The level a request sends for `chosen`: the highest level the model offers that is not
    /// above it, or `None` (Default) when there is none or nothing was chosen. So Max becomes
    /// High on a model that offers Low to High, and any level becomes Default on a model that
    /// offers none.
    #[must_use]
    pub fn effort_for(&self, chosen: Option<Effort>) -> Option<Effort> {
        let chosen = chosen?;
        self.efforts()
            .iter()
            .copied()
            .filter(|e| *e <= chosen)
            .max()
    }
}

/// Checks a provider or SearXNG base URL (AI-02) and returns it parsed.
///
/// `https` is always allowed. `http` only when the host is a loopback or private network
/// address (RFC 1918, unique local, link-local, CGNAT 100.64/10): IP literals are checked
/// directly, `localhost` is allowed, and other host names are resolved and every address must be
/// local. URLs with credentials, without a host, or with another scheme are refused.
///
/// The lookup here happens before the request, which resolves the name again. Requests to an
/// `http` base URL with a host name therefore go through [`client_for`], whose resolver applies the
/// same rule at connect time, so a name that starts resolving to a public address (DNS rebinding,
/// a changed network) never receives the API key in clear text.
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

/// The builder behind [`http_client`] and [`client_for`]'s guarded client: rustls with the ring
/// provider, connect timeout 15 s, read (idle) timeout 5 minutes and no overall timeout,
/// User-Agent `Hatoba/<version>`, and redirects followed only within the same origin.
fn client_builder() -> reqwest::ClientBuilder {
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
}

/// The client for provider and search requests: rustls with the ring provider, connect timeout
/// 15 s, read (idle) timeout 5 minutes and no overall timeout, User-Agent `Hatoba/<version>`,
/// the system proxy, and redirects followed only within the same origin.
///
/// Build it once and share it (cloning is cheap); it holds a connection pool bound to the tokio
/// runtime it is first used on. Requests to a validated base URL go through [`client_for`], which
/// returns this client unless the URL is an `http` one with a host name.
#[must_use]
pub fn http_client() -> reqwest::Client {
    client_builder()
        .build()
        // Only fails when no TLS provider is available, which `ensure_crypto_provider` rules out.
        .expect("the HTTP client must build with the ring provider installed")
}

/// The client to send requests to the validated base URL `base` with: `http` itself, except for an
/// `http` URL whose host is a name (an IP literal needs no lookup, and `https` is not limited to
/// local addresses). That gets a client whose resolver refuses a name with any address that is not
/// loopback, private, link-local or CGNAT, and connects to exactly the addresses it checked, so
/// the rule [`validate_base_url`] applied still holds when the connection is made. A refusal is
/// [`AiError::InvalidUrl`].
///
/// This client ignores the system proxy, because a proxy resolves names itself and would defeat
/// the check; validation already needs the name to resolve on this machine. It is built per use
/// and has no connection pool worth sharing: base URLs of this kind are LAN servers.
pub(crate) fn client_for<'a>(http: &'a reqwest::Client, base: &Url) -> Cow<'a, reqwest::Client> {
    if needs_local_guard(base) {
        Cow::Owned(local_http_client(system_lookup()))
    } else {
        Cow::Borrowed(http)
    }
}

/// Whether requests to `base` must be bound to local addresses at connect time.
fn needs_local_guard(base: &Url) -> bool {
    base.scheme() == "http" && matches!(base.host(), Some(Host::Domain(_)))
}

/// [`client_for`]'s guarded client, resolving names with `lookup`.
fn local_http_client(lookup: Lookup) -> reqwest::Client {
    client_builder()
        .no_proxy()
        .dns_resolver(GuardedResolver {
            allowed: is_local_network,
            lookup,
            refusal: refuse_not_local,
        })
        .build()
        .expect("the HTTP client must build with the ring provider installed")
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio_util::sync::CancellationToken;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

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
    fn a_chosen_level_becomes_the_nearest_lower_one_the_model_offers() {
        use Effort::{High, Low, Max, Medium, Xhigh};
        let with = |efforts: Option<Vec<Effort>>| ModelSpec {
            efforts,
            ..ModelSpec::new("m")
        };
        // Unknown: Low to High.
        let unknown = with(None);
        assert_eq!(unknown.efforts(), &[Low, Medium, High]);
        assert_eq!(unknown.effort_for(None), None);
        assert_eq!(unknown.effort_for(Some(Medium)), Some(Medium));
        assert_eq!(unknown.effort_for(Some(Max)), Some(High));
        // Opus 4.6 has Max but no Extra High.
        let opus_4_6 = with(Some(vec![Low, Medium, High, Max]));
        assert_eq!(opus_4_6.effort_for(Some(Xhigh)), Some(High));
        assert_eq!(opus_4_6.effort_for(Some(Max)), Some(Max));
        // None offered, or none at or below the choice: Default.
        assert_eq!(with(Some(vec![])).effort_for(Some(High)), None);
        assert_eq!(with(Some(vec![High, Max])).effort_for(Some(Low)), None);
        assert_eq!(
            Effort::ALL.map(Effort::as_str),
            ["low", "medium", "high", "xhigh", "max"]
        );
        assert_eq!(serde_json::to_string(&Xhigh).unwrap(), "\"xhigh\"");
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

    #[test]
    fn only_http_urls_with_a_host_name_get_the_guarded_client() {
        let http = http_client();
        for (url, guarded) in [
            ("https://api.openai.com/v1", false),
            ("https://nas.local:8443/v1", false),
            ("http://nas.local:11434/v1", true),
            ("http://localhost:11434/v1", true),
            ("http://LOCALHOST:1234", true),
            // IP literals need no lookup, and validation checked them already.
            ("http://127.0.0.1:11434", false),
            ("http://127.1", false),
            ("http://192.168.1.20/v1", false),
            ("http://[::1]:8000", false),
            ("http://[fd00::5]/v1", false),
        ] {
            let url = Url::parse(url).unwrap();
            assert_eq!(needs_local_guard(&url), guarded, "{url}");
            assert_eq!(
                matches!(client_for(&http, &url), Cow::Owned(_)),
                guarded,
                "{url}"
            );
        }
    }

    fn lookup_of(
        answer: impl Fn(&str) -> std::io::Result<Vec<IpAddr>> + Send + Sync + 'static,
    ) -> Lookup {
        Arc::new(move |host: String| {
            let answer = answer(&host);
            Box::pin(async move { answer })
        })
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[tokio::test]
    async fn http_hosts_are_held_to_local_addresses_when_connecting() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;
        let port = server.address().port();
        let lookup = lookup_of(|host| match host {
            // A LAN name that resolves to a local address (the mock server is on loopback).
            "nas.local" => Ok(vec![ip("127.0.0.1")]),
            // A name that now resolves to a public address: DNS rebinding.
            "rebind.test" => Ok(vec![ip("93.184.216.34")]),
            "rebind6.test" => Ok(vec![ip("2606:4700::1111")]),
            // One local and one public address is refused as well.
            "mixed.test" => Ok(vec![ip("127.0.0.1"), ip("8.8.8.8")]),
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "unknown test host",
            )),
        });
        let client = local_http_client(lookup);
        let cancel = CancellationToken::new();
        let get = |host: &str| client.get(format!("http://{host}:{port}/models"));

        let response = net::send(get("nas.local"), &cancel).await.unwrap();
        assert_eq!(response.status(), 200);
        for host in ["rebind.test", "rebind6.test", "mixed.test"] {
            let err = net::send(get(host), &cancel).await.unwrap_err();
            assert_eq!(
                err,
                AiError::InvalidUrl(net::NOT_LOCAL_MESSAGE.into()),
                "{host}"
            );
        }
        let err = net::send(get("gone.test"), &cancel).await.unwrap_err();
        assert!(matches!(err, AiError::Network(_)), "{err:?}");
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            1,
            "only the request to the local address went out"
        );
    }

    #[tokio::test]
    async fn a_name_that_changes_its_answer_after_validation_is_refused() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let url = format!("http://llm.test:{}/v1/models", server.address().port());
        // The first lookup (what validation sees) is local, every later one is public.
        let lookups = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&lookups);
        let lookup = lookup_of(move |_| {
            Ok(vec![if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                ip("127.0.0.1")
            } else {
                ip("203.0.113.9")
            }])
        });
        let cancel = CancellationToken::new();

        let first = local_http_client(Arc::clone(&lookup));
        assert!(net::send(first.get(&url), &cancel).await.is_ok());
        let second = local_http_client(lookup);
        let err = net::send(second.get(&url), &cancel).await.unwrap_err();
        assert!(matches!(err, AiError::InvalidUrl(_)), "{err:?}");
        assert_eq!(lookups.load(Ordering::SeqCst), 2);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}
