//! Provider configuration, base URL validation (AI-02) and the shared HTTP client.

use std::fmt;
use std::time::Duration;

use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use url::Url;
use zeroize::Zeroizing;

use crate::chatgpt::{self, ChatgptAuth};
use crate::error::AiError;
use crate::net;

/// The wire protocol a provider speaks (spec §13.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// OpenAI Chat Completions: `POST {base_url}/chat/completions`.
    #[default]
    ChatCompletions,
    /// Anthropic Messages: `POST {base_url}/v1/messages`.
    Anthropic,
    /// OpenAI Responses: `POST {base_url}/responses`, stateless (`store: false`).
    Responses,
    /// TypeSafe's System One decision format: `POST {base_url}/systemone`, or the model's own URL
    /// on Workers AI (see [`decision`](crate::decision)). Such a provider answers decision
    /// questions for the judge of Auto mode (AI-47) and cannot chat.
    SystemOne,
}

/// Which header carries the key for the `anthropic` protocol (Chat Completions, Responses and
/// System One always use `Authorization: Bearer`).
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
    /// Signed in with a ChatGPT account instead of a key (AI-52): `responses` at
    /// [`chatgpt::BASE_URL`], with an access token of the account's session on this device.
    /// Its requests always stream and group function tools in [`chatgpt::NAMESPACE`].
    pub chatgpt: Option<ChatgptAuth>,
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
            .field("chatgpt", &self.chatgpt.as_ref().map(|c| &c.id))
            .finish()
    }
}

/// How often a ChatGPT request refused with 503 before it streams is sent again, and the wait
/// before the first retry (doubled for the next).
const CHATGPT_503_RETRIES: u32 = 2;
const CHATGPT_503_WAIT: Duration = Duration::from_secs(1);

impl ProviderConfig {
    /// Whether the provider signs in with a ChatGPT account (AI-52).
    #[must_use]
    pub fn is_chatgpt(&self) -> bool {
        self.chatgpt.is_some()
    }

    /// Sends the request `build` makes from the authentication headers, and returns its response,
    /// successful or not. A ChatGPT provider's headers carry an access token of its session,
    /// refreshed first when due; a 401 refreshes the session once and sends the request again,
    /// and a 503 is sent again twice, after 1 and 2 seconds.
    pub(crate) async fn send(
        &self,
        cancel: &CancellationToken,
        build: impl Fn(HeaderMap) -> reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, AiError> {
        let Some(auth) = &self.chatgpt else {
            return net::send(build(self.headers()?), cancel).await;
        };
        let mut token = auth.access_token(None, cancel).await?;
        let mut refreshed = false;
        let mut retries = 0;
        loop {
            let response = net::send(build(bearer(&token)?), cancel).await?;
            match response.status().as_u16() {
                401 if !refreshed => {
                    refreshed = true;
                    token = auth.access_token(Some(&token), cancel).await?;
                }
                503 if retries < CHATGPT_503_RETRIES => {
                    let wait = CHATGPT_503_WAIT * 2u32.pow(retries);
                    retries += 1;
                    tokio::select! {
                        biased;
                        () = cancel.cancelled() => return Err(AiError::Cancelled),
                        () = tokio::time::sleep(wait) => {}
                    }
                }
                _ => return Ok(response),
            }
        }
    }

    /// The error of a failed response: the provider's message, and on the ChatGPT route a message
    /// that says what to do and the request ID.
    pub(crate) async fn error(
        &self,
        response: reqwest::Response,
        cancel: &CancellationToken,
    ) -> AiError {
        let failure = match net::read_http_error(response, cancel).await {
            Ok(failure) => failure,
            Err(err) => return err,
        };
        if !self.is_chatgpt() {
            return failure.into_error();
        }
        let request_id = failure.request_id.clone();
        let known = failure
            .code
            .as_deref()
            .is_some_and(|code| chatgpt::explain(code, None).is_some());
        let mut err = failure.into_error();
        if let AiError::Http { status, message } = &mut err {
            if !known && let Some(text) = chatgpt::admission_message(*status, message) {
                *message = text;
            }
            if let Some(id) = request_id {
                message.push_str(&format!(" (request ID {id})"));
            }
        }
        err
    }
}

/// `Authorization: Bearer <token>`, marked sensitive.
fn bearer(token: &str) -> Result<HeaderMap, AiError> {
    let mut value = HeaderValue::from_str(&Zeroizing::new(format!("Bearer {}", token.trim())))
        .map_err(|_| AiError::Config("the access token cannot be sent in a header".into()))?;
    value.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, value);
    Ok(headers)
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

    /// The value of Anthropic's `output_config.effort`, of Chat Completions' `reasoning_effort`
    /// and of Responses' `reasoning.effort`.
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

/// Checks a provider, SearXNG or MCP base URL (AI-02) and returns it parsed: `http` or `https`,
/// for any host. URLs with credentials, without a host, or with another scheme are refused.
pub fn validate_base_url(base_url: &str) -> Result<Url, AiError> {
    let mut url = Url::parse(base_url.trim())
        .map_err(|_| AiError::InvalidUrl("the base URL is not a valid URL".into()))?;
    url.set_fragment(None);
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AiError::InvalidUrl(
            "the base URL must not contain a user name or password".into(),
        ));
    }
    if url.host().is_none() {
        return Err(AiError::InvalidUrl("the base URL has no host".into()));
    }
    match url.scheme() {
        "https" | "http" => Ok(url),
        _ => Err(AiError::InvalidUrl(
            "the base URL must start with https:// or http://".into(),
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

/// The builder behind [`http_client`]: rustls with the ring provider, connect timeout 15 s, read
/// (idle) timeout 5 minutes and no overall timeout, User-Agent `Hatoba/<version>`, and redirects
/// followed only within the same origin.
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
/// runtime it is first used on. Every request to a provider, SearXNG or MCP server uses it.
#[must_use]
pub fn http_client() -> reqwest::Client {
    client_builder()
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
            chatgpt: None,
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

        // Responses: a bearer token whatever the auth header says.
        let h = config(Protocol::Responses, "k5", AuthHeader::XApiKey)
            .headers()
            .unwrap();
        assert_eq!(h[AUTHORIZATION], "Bearer k5");
        assert_eq!(h.len(), 1);

        // System One: always a bearer token, and nothing when the key is empty.
        let h = config(Protocol::SystemOne, "k4", AuthHeader::XApiKey)
            .headers()
            .unwrap();
        assert_eq!(h[AUTHORIZATION], "Bearer k4");
        assert_eq!(h.len(), 1);
        assert!(h[AUTHORIZATION].is_sensitive());
        let h = config(Protocol::SystemOne, "", AuthHeader::Authorization)
            .headers()
            .unwrap();
        assert!(h.is_empty());

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
            serde_json::to_string(&Protocol::Responses).unwrap(),
            "\"responses\""
        );
        assert_eq!(
            serde_json::to_string(&Protocol::SystemOne).unwrap(),
            "\"system_one\""
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

    #[test]
    fn base_urls_take_http_and_https_for_any_host() {
        for ok in [
            "https://api.openai.com/v1",
            "https://api.deepseek.com/anthropic/",
            "http://localhost:11434/v1",
            "http://127.0.0.1:8080/v1",
            "http://192.168.1.20:1234/v1",
            "http://[::1]:8000",
            "http://8.8.8.8/v1",
            "http://[2001:4860:4860::8888]/v1",
            "http://llm.example.com/v1",
            " https://api.anthropic.com ",
        ] {
            assert!(validate_base_url(ok).is_ok(), "{ok} must be accepted");
        }
        for bad in [
            "ftp://example.com",
            "file:///etc/passwd",
            "not a url",
            "https://user:pass@example.com",
            "http://user@example.com",
            "",
        ] {
            assert!(
                matches!(validate_base_url(bad), Err(AiError::InvalidUrl(_))),
                "{bad} must be refused"
            );
        }
        let url = validate_base_url("https://example.com/v1#frag").unwrap();
        assert_eq!(url.fragment(), None);
    }
}
