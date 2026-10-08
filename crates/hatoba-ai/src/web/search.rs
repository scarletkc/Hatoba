//! `web_search` (AI-14): Brave Search API, Tavily or a SearXNG instance.

use std::fmt;
use std::time::Duration;

use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use super::text::plain_text;
use crate::error::AiError;
use crate::net;
use crate::provider::validate_base_url;

/// The search services Hatoba supports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    /// Brave Search API (`X-Subscription-Token`).
    #[default]
    Brave,
    /// Tavily (`Authorization: Bearer`).
    Tavily,
    /// A SearXNG instance with the JSON format enabled.
    Searxng,
}

/// The chosen search provider. The key is wiped when dropped and never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct SearchConfig {
    /// Which service.
    pub kind: SearchKind,
    /// The SearXNG instance URL (ignored for Brave and Tavily).
    pub base_url: Option<String>,
    /// API key (Brave, Tavily); SearXNG needs none.
    pub api_key: Zeroizing<String>,
}

impl fmt::Debug for SearchConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SearchConfig")
            .field("kind", &self.kind)
            .field("base_url", &self.base_url)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "<empty>"
                } else {
                    "<redacted>"
                },
            )
            .finish()
    }
}

/// One search result.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    /// Page title, as plain text.
    pub title: String,
    /// Page URL.
    pub url: String,
    /// Snippet, as plain text.
    pub snippet: String,
}

/// Results returned at most.
const MAX_RESULTS: usize = 10;
/// Longest snippet kept, in characters.
const MAX_SNIPPET_CHARS: usize = 1_000;
/// Overall limit of a search.
const SEARCH_TIMEOUT: Duration = Duration::from_secs(30);
const BRAVE_ENDPOINT: &str = "https://api.search.brave.com/res/v1/web/search";
const TAVILY_ENDPOINT: &str = "https://api.tavily.com/search";

/// The fixed service endpoints (replaced in tests).
pub(crate) struct Endpoints {
    pub brave: String,
    pub tavily: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            brave: BRAVE_ENDPOINT.into(),
            tavily: TAVILY_ENDPOINT.into(),
        }
    }
}

/// Sends `query` to the configured service and returns up to 10 results (AI-14).
pub async fn web_search(
    http: &reqwest::Client,
    cfg: &SearchConfig,
    query: &str,
    cancel: &CancellationToken,
) -> Result<Vec<SearchResult>, AiError> {
    search_at(http, cfg, query, cancel, &Endpoints::default()).await
}

pub(crate) async fn search_at(
    http: &reqwest::Client,
    cfg: &SearchConfig,
    query: &str,
    cancel: &CancellationToken,
    endpoints: &Endpoints,
) -> Result<Vec<SearchResult>, AiError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(AiError::Config("the search query is empty".into()));
    }
    tokio::time::timeout(SEARCH_TIMEOUT, search(http, cfg, query, cancel, endpoints))
        .await
        .unwrap_or_else(|_| Err(AiError::Network("the search timed out".into())))
}

fn key_header(value: &str) -> Result<HeaderValue, AiError> {
    let mut value = HeaderValue::from_str(value).map_err(|_| {
        AiError::Config("the API key contains characters an HTTP header cannot carry".into())
    })?;
    value.set_sensitive(true);
    Ok(value)
}

fn require_key(cfg: &SearchConfig) -> Result<&str, AiError> {
    let key = cfg.api_key.trim();
    if key.is_empty() {
        Err(AiError::Config("the search provider has no API key".into()))
    } else {
        Ok(key)
    }
}

async fn search(
    http: &reqwest::Client,
    cfg: &SearchConfig,
    query: &str,
    cancel: &CancellationToken,
    endpoints: &Endpoints,
) -> Result<Vec<SearchResult>, AiError> {
    let (request, list_pointer, snippet_key) = match cfg.kind {
        SearchKind::Brave => {
            let key = key_header(require_key(cfg)?)?;
            let mut url = url::Url::parse(&endpoints.brave)
                .map_err(|_| AiError::InvalidUrl("the search endpoint is invalid".into()))?;
            url.query_pairs_mut()
                .append_pair("q", query)
                .append_pair("count", &MAX_RESULTS.to_string());
            (
                http.get(url)
                    .header(ACCEPT, "application/json")
                    .header(HeaderName::from_static("x-subscription-token"), key),
                "/web/results",
                "description",
            )
        }
        SearchKind::Tavily => {
            let key = key_header(&format!("Bearer {}", require_key(cfg)?))?;
            (
                http.post(&endpoints.tavily)
                    .header(AUTHORIZATION, key)
                    .json(&json!({"query": query, "max_results": MAX_RESULTS})),
                "/results",
                "content",
            )
        }
        SearchKind::Searxng => {
            let base = cfg
                .base_url
                .as_deref()
                .filter(|b| !b.trim().is_empty())
                .ok_or_else(|| AiError::Config("the SearXNG instance URL is missing".into()))?;
            let base = tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(AiError::Cancelled),
                base = validate_base_url(base) => base?,
            };
            let mut url = net::endpoint(&base, "/search");
            url.query_pairs_mut()
                .append_pair("q", query)
                .append_pair("format", "json");
            (
                http.get(url).header(ACCEPT, "application/json"),
                "/results",
                "content",
            )
        }
    };
    let response = net::send(request, cancel).await?;
    if !response.status().is_success() {
        return Err(net::http_error(response, cancel).await);
    }
    let body = net::read_json(response, cancel).await?;
    Ok(parse_results(&body, list_pointer, snippet_key))
}

fn parse_results(body: &Value, list_pointer: &str, snippet_key: &str) -> Vec<SearchResult> {
    let Some(items) = body.pointer(list_pointer).and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let url = item.get("url").and_then(Value::as_str)?.trim();
            if url.is_empty() {
                return None;
            }
            let text = |key: &str| plain_text(item.get(key).and_then(Value::as_str).unwrap_or(""));
            let mut snippet = text(snippet_key);
            if let Some((cut, _)) = snippet.char_indices().nth(MAX_SNIPPET_CHARS) {
                snippet.truncate(cut);
                snippet.push('…');
            }
            Some(SearchResult {
                title: text("title"),
                url: url.to_owned(),
                snippet,
            })
        })
        .take(MAX_RESULTS)
        .collect()
}

/// The `web_search` result text: numbered results with title, URL and snippet, or a line
/// saying there were none.
#[must_use]
pub fn format_search_results(results: &[SearchResult]) -> String {
    if results.is_empty() {
        return "No results.".to_owned();
    }
    let mut out = String::new();
    for (i, r) in results.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let title = if r.title.is_empty() {
            "(no title)"
        } else {
            &r.title
        };
        out.push_str(&format!("{}. {title}\n   {}\n", i + 1, r.url));
        if !r.snippet.is_empty() {
            out.push_str(&format!("   {}\n", r.snippet));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::provider::http_client;

    fn cfg(kind: SearchKind, key: &str, base_url: Option<String>) -> SearchConfig {
        SearchConfig {
            kind,
            base_url,
            api_key: Zeroizing::new(key.into()),
        }
    }

    fn endpoints(server: &MockServer) -> Endpoints {
        Endpoints {
            brave: format!("{}/res/v1/web/search", server.uri()),
            tavily: format!("{}/search", server.uri()),
        }
    }

    #[tokio::test]
    async fn brave_sends_the_token_and_cleans_snippets() {
        let server = MockServer::start().await;
        let results: Vec<Value> = (0..15)
            .map(|i| {
                json!({
                    "title": format!("Result &amp; {i}"),
                    "url": format!("https://example.com/{i}"),
                    "description": "Use <strong>systemctl</strong>&nbsp;restart&#x21;"
                })
            })
            .collect();
        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .and(query_param("q", "restart nginx"))
            .and(query_param("count", "10"))
            .and(header("x-subscription-token", "brave-key"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"web": {"results": results}})),
            )
            .mount(&server)
            .await;
        let found = search_at(
            &http_client(),
            &cfg(SearchKind::Brave, "brave-key", None),
            " restart nginx ",
            &CancellationToken::new(),
            &endpoints(&server),
        )
        .await
        .unwrap();
        assert_eq!(found.len(), 10);
        assert_eq!(
            found[0],
            SearchResult {
                title: "Result & 0".into(),
                url: "https://example.com/0".into(),
                snippet: "Use systemctl restart!".into(),
            }
        );
    }

    #[tokio::test]
    async fn tavily_posts_the_query_with_a_bearer_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/search"))
            .and(header("authorization", "Bearer tvly-key"))
            .and(body_json(json!({"query": "rust", "max_results": 10})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "results": [{"title": "Rust", "url": "https://rust-lang.org", "content": "A language"}]
            })))
            .mount(&server)
            .await;
        let found = search_at(
            &http_client(),
            &cfg(SearchKind::Tavily, "tvly-key", None),
            "rust",
            &CancellationToken::new(),
            &endpoints(&server),
        )
        .await
        .unwrap();
        assert_eq!(found[0].snippet, "A language");
    }

    #[tokio::test]
    async fn searxng_uses_the_instance_url() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/searx/search"))
            .and(query_param("q", "zfs"))
            .and(query_param("format", "json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "results": [
                    {"title": "ZFS", "url": "https://openzfs.org", "content": "File system"},
                    {"title": "no url"}
                ]
            })))
            .mount(&server)
            .await;
        let found = web_search(
            &http_client(),
            &cfg(
                SearchKind::Searxng,
                "",
                Some(format!("{}/searx/", server.uri())),
            ),
            "zfs",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].url, "https://openzfs.org");
    }

    #[tokio::test]
    async fn errors_and_missing_configuration() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(429)
                    .set_body_json(json!({"error": {"message": "Rate limit exceeded"}})),
            )
            .mount(&server)
            .await;
        let http = http_client();
        let cancel = CancellationToken::new();
        let err = search_at(
            &http,
            &cfg(SearchKind::Brave, "k", None),
            "q",
            &cancel,
            &endpoints(&server),
        )
        .await
        .unwrap_err();
        assert_eq!(
            err,
            AiError::Http {
                status: 429,
                message: "Rate limit exceeded".into()
            }
        );
        for config in [
            cfg(SearchKind::Brave, "", None),
            cfg(SearchKind::Tavily, " ", None),
            cfg(SearchKind::Searxng, "", None),
        ] {
            assert!(matches!(
                search_at(&http, &config, "q", &cancel, &endpoints(&server)).await,
                Err(AiError::Config(_))
            ));
        }
        assert!(matches!(
            search_at(
                &http,
                &cfg(SearchKind::Brave, "k", None),
                "  ",
                &cancel,
                &endpoints(&server)
            )
            .await,
            Err(AiError::Config(_))
        ));
        let public_http = cfg(SearchKind::Searxng, "", Some("http://8.8.8.8".into()));
        assert!(matches!(
            web_search(&http, &public_http, "q", &cancel).await,
            Err(AiError::InvalidUrl(_))
        ));
    }

    #[test]
    fn debug_never_prints_the_key() {
        let config = cfg(SearchKind::Tavily, "tvly-secret-key", None);
        let debug = format!("{config:?}");
        assert!(!debug.contains("tvly-secret-key"));
        assert!(debug.contains("<redacted>"));
        assert_eq!(
            serde_json::to_string(&SearchKind::Searxng).unwrap(),
            "\"searxng\""
        );
    }

    #[test]
    fn results_are_formatted() {
        assert_eq!(format_search_results(&[]), "No results.");
        let text = format_search_results(&[
            SearchResult {
                title: "A".into(),
                url: "https://a".into(),
                snippet: "about a".into(),
            },
            SearchResult {
                title: String::new(),
                url: "https://b".into(),
                snippet: String::new(),
            },
        ]);
        assert_eq!(
            text,
            "1. A\n   https://a\n   about a\n\n2. (no title)\n   https://b\n"
        );
    }
}
