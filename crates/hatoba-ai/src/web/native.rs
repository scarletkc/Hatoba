//! `web_search` (AI-14) through the search the chat provider runs itself, which it uses while no
//! search service is chosen. The model that made the call is asked, in a request of its own, to
//! search for the query, and what it found comes back in one shape whichever provider searched:
//! the pages its answer cites, then the other pages it found, each with a title and URL and a
//! snippet where the provider gives one, and the model's account of what they say.
//!
//! - Anthropic Messages, any provider of that protocol: the `web_search_20250305` server tool,
//!   at most 5 searches, a paused turn (`pause_turn`) continued up to 3 times.
//! - OpenRouter: the `openrouter:web_search` server tool on Chat Completions, which searches with
//!   the model's own provider where it has a search and with Exa elsewhere.
//! - OpenAI and xAI: the Responses API (`POST {base_url}/responses`) with the `web_search` tool.
//!   A ChatGPT provider (AI-52) is OpenAI too; its request streams, as that route requires.
//! - Gemini: `generateContent` with the `google_search` tool, the key in `x-goog-api-key`.
//! - Any other Chat Completions or Responses provider, which may relay OpenAI: tried as OpenAI
//!   is.
//!   [`turned_away`] tells an error that says it has no such search from one that may pass.
//!
//! A response in which no search ran is an error, so an answer from the model's memory never
//! passes for search results.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use url::Url;

use super::search::{
    MAX_RESULTS, SearchResult, cut_snippet, format_search_results, key_header, send_retrying,
};
use crate::error::AiError;
use crate::provider::{ModelSpec, Protocol, ProviderConfig, validate_base_url};
use crate::{chat, net};

/// How a chat provider searches the web itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NativeSearch {
    /// Anthropic's `web_search` server tool.
    Anthropic,
    /// OpenRouter's `openrouter:web_search` server tool.
    OpenRouter,
    /// OpenAI's Responses API with the `web_search` tool.
    OpenAi,
    /// xAI's Responses API with the `web_search` tool.
    Xai,
    /// Gemini's `generateContent` with the `google_search` tool.
    Gemini,
    /// Another Chat Completions provider, such as a relay of OpenAI's API, tried with the
    /// Responses API's `web_search` tool as OpenAI is.
    Other,
}

impl NativeSearch {
    /// How `provider` searches the web itself: every Anthropic Messages provider with Anthropic's
    /// server tool, and a Chat Completions or Responses provider by its base URL's host,
    /// [`Self::Other`] when the host is none Hatoba knows. A System One provider (AI-47) cannot
    /// chat, so it has none.
    #[must_use]
    pub fn of(provider: &ProviderConfig) -> Option<Self> {
        match provider.protocol {
            Protocol::Anthropic => Some(Self::Anthropic),
            Protocol::SystemOne => None,
            Protocol::ChatCompletions | Protocol::Responses => {
                let host = validate_base_url(&provider.base_url)
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
                    .unwrap_or_default();
                Some(match host.trim_end_matches('.') {
                    "api.openai.com" => Self::OpenAi,
                    "api.x.ai" => Self::Xai,
                    "generativelanguage.googleapis.com" => Self::Gemini,
                    "openrouter.ai" => Self::OpenRouter,
                    host if host.ends_with(".openrouter.ai") => Self::OpenRouter,
                    _ => Self::Other,
                })
            }
        }
    }
}

/// Whether `err`, from a search of [`NativeSearch::Other`], says that the provider has no such
/// search rather than that this one failed: the Responses API refused the request (400, 404, 405,
/// 415, 422 or 501), or the answer was no search (not JSON, or no search ran in it).
#[must_use]
pub fn turned_away(err: &AiError) -> bool {
    match err {
        AiError::Http { status, .. } => matches!(status, 400 | 404 | 405 | 415 | 422 | 501),
        AiError::Protocol(_) => true,
        _ => false,
    }
}

/// What a search of the chat provider found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeResults {
    /// The pages the answer cites, in the order it first cites them, then the other pages the
    /// search found; at most 10.
    pub sources: Vec<SearchResult>,
    /// The model's account of what the results say.
    pub answer: String,
}

/// Longest a search may take: the model may search several times before it answers.
const NATIVE_TIMEOUT: Duration = Duration::from_secs(180);
/// Most searches of one call (Anthropic's `max_uses`).
const MAX_SEARCHES: u32 = 5;
/// `max_tokens` of the Anthropic request, or the model's output limit when that is lower.
const ANTHROPIC_MAX_TOKENS: u64 = 4_096;
/// How many times a paused Anthropic turn (`pause_turn`) is continued.
const MAX_CONTINUATIONS: usize = 3;
/// What the model that searches is told.
const SYSTEM: &str = "You search the web for another assistant, which reads your reply as the \
    result of its web_search tool. Search the web for the query you are given, then state what \
    the results say that bears on it, such as facts, versions, dates and commands, naming the \
    source of each. Say what the results do not cover instead of answering from memory.";

fn prompt(query: &str) -> String {
    format!("Search the web for: {query}")
}

/// Asks the model to search the web for `query` with its provider's own search (AI-14).
pub async fn native_search(
    http: &reqwest::Client,
    provider: &ProviderConfig,
    model: &ModelSpec,
    query: &str,
    cancel: &CancellationToken,
) -> Result<NativeResults, AiError> {
    let Some(kind) = NativeSearch::of(provider) else {
        return Err(AiError::Config(
            "a System One provider cannot search the web".into(),
        ));
    };
    search_with(http, kind, provider, model, query, cancel).await
}

pub(crate) async fn search_with(
    http: &reqwest::Client,
    kind: NativeSearch,
    provider: &ProviderConfig,
    model: &ModelSpec,
    query: &str,
    cancel: &CancellationToken,
) -> Result<NativeResults, AiError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(AiError::Config("the search query is empty".into()));
    }
    let base = validate_base_url(&provider.base_url)?;
    let search = async {
        let mut found = Found::default();
        match kind {
            NativeSearch::Anthropic => {
                anthropic(http, &base, provider, model, query, cancel, &mut found).await?;
            }
            NativeSearch::OpenRouter => {
                openrouter(http, &base, provider, model, query, cancel, &mut found).await?;
            }
            NativeSearch::OpenAi | NativeSearch::Xai | NativeSearch::Other => {
                let openai_options = kind != NativeSearch::Xai;
                responses(
                    http,
                    &base,
                    provider,
                    model,
                    query,
                    openai_options,
                    cancel,
                    &mut found,
                )
                .await?;
            }
            NativeSearch::Gemini => {
                gemini(http, &base, provider, model, query, cancel, &mut found).await?;
            }
        }
        found.finish()
    };
    tokio::time::timeout(NATIVE_TIMEOUT, search)
        .await
        .unwrap_or_else(|_| Err(AiError::Network("the search timed out".into())))
}

async fn post(
    http: &reqwest::Client,
    url: Url,
    headers: HeaderMap,
    body: &Value,
    cancel: &CancellationToken,
) -> Result<Value, AiError> {
    let response = send_retrying(http.post(url).headers(headers).json(body), cancel).await?;
    if !response.status().is_success() {
        return Err(net::http_error(response, cancel).await);
    }
    net::read_json(response, cancel).await
}

fn str_at<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn array_at<'a>(value: &'a Value, pointer: &str) -> &'a [Value] {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

async fn anthropic(
    http: &reqwest::Client,
    base: &Url,
    provider: &ProviderConfig,
    model: &ModelSpec,
    query: &str,
    cancel: &CancellationToken,
    found: &mut Found,
) -> Result<(), AiError> {
    let url = net::endpoint(base, "/v1/messages");
    let headers = provider.headers()?;
    let max_tokens = model
        .max_output_tokens
        .map_or(ANTHROPIC_MAX_TOKENS, |limit| {
            limit.clamp(1, ANTHROPIC_MAX_TOKENS)
        });
    let mut messages = vec![json!({"role": "user", "content": prompt(query)})];
    for _ in 0..=MAX_CONTINUATIONS {
        let body = json!({
            "model": model.id,
            "max_tokens": max_tokens,
            "system": SYSTEM,
            "messages": messages,
            "tools": [{"type": "web_search_20250305", "name": "web_search", "max_uses": MAX_SEARCHES}],
        });
        let response = post(http, url.clone(), headers.clone(), &body, cancel).await?;
        let content = array_at(&response, "/content");
        found.read_anthropic(content);
        if str_at(&response, "stop_reason") != "pause_turn" {
            break;
        }
        messages.push(json!({"role": "assistant", "content": content}));
    }
    Ok(())
}

async fn openrouter(
    http: &reqwest::Client,
    base: &Url,
    provider: &ProviderConfig,
    model: &ModelSpec,
    query: &str,
    cancel: &CancellationToken,
    found: &mut Found,
) -> Result<(), AiError> {
    let body = json!({
        "model": model.id,
        "messages": [
            {"role": "system", "content": SYSTEM},
            {"role": "user", "content": prompt(query)},
        ],
        "tools": [{"type": "openrouter:web_search", "parameters": {"max_results": MAX_RESULTS}}],
        "stream": false,
    });
    let url = net::endpoint(base, "/chat/completions");
    let response = post(http, url, provider.headers()?, &body, cancel).await?;
    let message = response
        .pointer("/choices/0/message")
        .unwrap_or(&Value::Null);
    match message.get("content") {
        Some(Value::String(text)) => found.answer.push_str(text),
        Some(Value::Array(parts)) => {
            for part in parts {
                found.answer.push_str(str_at(part, "text"));
            }
        }
        _ => {}
    }
    for annotation in array_at(message, "/annotations") {
        if str_at(annotation, "type") == "url_citation" {
            let citation = annotation.get("url_citation").unwrap_or(annotation);
            found.searched = true;
            found.cite(
                str_at(citation, "url"),
                str_at(citation, "title"),
                str_at(citation, "content"),
            );
        }
    }
    let requests = response
        .pointer("/usage/server_tool_use/web_search_requests")
        .and_then(Value::as_u64);
    if requests.is_some_and(|n| n > 0) {
        found.searched = true;
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "a search's request, with whether it takes OpenAI's options"
)]
async fn responses(
    http: &reqwest::Client,
    base: &Url,
    provider: &ProviderConfig,
    model: &ModelSpec,
    query: &str,
    openai_options: bool,
    cancel: &CancellationToken,
    found: &mut Found,
) -> Result<(), AiError> {
    let mut body = json!({
        "model": model.id,
        "instructions": SYSTEM,
        "input": prompt(query),
        "tools": [{"type": "web_search"}],
        "store": false,
    });
    if openai_options {
        // Every page the search read, not only the cited ones, and a search before the answer.
        body["include"] = json!(["web_search_call.action.sources"]);
        body["tool_choice"] = json!("required");
    }
    let url = net::endpoint(base, "/responses");
    let response = if provider.is_chatgpt() {
        // The ChatGPT route (AI-52) takes `input` as a list of items, and only streams.
        body["input"] = json!([{"type": "message", "role": "user", "content": prompt(query)}]);
        let Value::Object(body) = body else {
            unreachable!("the body is an object");
        };
        chat::responses_json(http, provider, &url, body, cancel).await?
    } else {
        post(http, url, provider.headers()?, &body, cancel).await?
    };
    for item in array_at(&response, "/output") {
        match str_at(item, "type") {
            "web_search_call" => {
                found.searched = true;
                for source in array_at(item, "/action/sources") {
                    found.page(str_at(source, "url"), str_at(source, "title"), "");
                }
            }
            "message" => {
                for part in array_at(item, "/content") {
                    if str_at(part, "type") != "output_text" {
                        continue;
                    }
                    found.answer.push_str(str_at(part, "text"));
                    for annotation in array_at(part, "/annotations") {
                        if str_at(annotation, "type") == "url_citation" {
                            found.cite(str_at(annotation, "url"), str_at(annotation, "title"), "");
                        }
                    }
                }
            }
            _ => {}
        }
    }
    // xAI also lists the pages it used, as URLs.
    for url in array_at(&response, "/citations") {
        if let Some(url) = url.as_str() {
            found.searched = true;
            found.page(url, "", "");
        }
    }
    Ok(())
}

async fn gemini(
    http: &reqwest::Client,
    base: &Url,
    provider: &ProviderConfig,
    model: &ModelSpec,
    query: &str,
    cancel: &CancellationToken,
    found: &mut Found,
) -> Result<(), AiError> {
    let key = provider.api_key.trim();
    if key.is_empty() {
        return Err(AiError::Config("the provider has no API key".into()));
    }
    let mut headers = HeaderMap::new();
    headers.insert(HeaderName::from_static("x-goog-api-key"), key_header(key)?);
    // The OpenAI-compatible base URL (`…/v1beta/openai`) names the host; the model's own method
    // is on the native API.
    let mut url = base.clone();
    url.set_query(None);
    url.set_fragment(None);
    let model_id = model.id.trim();
    let model_id = model_id.strip_prefix("models/").unwrap_or(model_id);
    url.path_segments_mut()
        .map_err(|()| AiError::InvalidUrl("the base URL has no host".into()))?
        .clear()
        .extend(["v1beta", "models", &format!("{model_id}:generateContent")]);
    let body = json!({
        "systemInstruction": {"parts": [{"text": SYSTEM}]},
        "contents": [{"role": "user", "parts": [{"text": prompt(query)}]}],
        "tools": [{"google_search": {}}],
    });
    let response = post(http, url, headers, &body, cancel).await?;
    let candidate = response.pointer("/candidates/0").unwrap_or(&Value::Null);
    for part in array_at(candidate, "/content/parts") {
        if part.get("thought").and_then(Value::as_bool) != Some(true) {
            found.answer.push_str(str_at(part, "text"));
        }
    }
    let metadata = candidate.get("groundingMetadata").unwrap_or(&Value::Null);
    if !array_at(metadata, "/webSearchQueries").is_empty() {
        found.searched = true;
    }
    let pages: Vec<Option<usize>> = array_at(metadata, "/groundingChunks")
        .iter()
        .map(|chunk| {
            let web = chunk.get("web")?;
            found.searched = true;
            found.page(str_at(web, "uri"), str_at(web, "title"), "")
        })
        .collect();
    for support in array_at(metadata, "/groundingSupports") {
        for index in array_at(support, "/groundingChunkIndices") {
            let page = index
                .as_u64()
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| pages.get(i).copied().flatten());
            if let Some(page) = page {
                found.cite_page(page);
            }
        }
    }
    Ok(())
}

/// What the responses of a search held.
#[derive(Debug, Default)]
struct Found {
    /// A search ran.
    searched: bool,
    /// Every page found or cited, in the order first seen.
    pages: Vec<SearchResult>,
    /// The cited pages, as indices into `pages`, in the order first cited.
    cited: Vec<usize>,
    /// The model's answer.
    answer: String,
    /// The error codes of searches that failed (Anthropic).
    errors: Vec<String>,
}

impl Found {
    /// Adds a page, or fills in the title and snippet of one already found. Returns its index,
    /// or `None` for a page without a URL.
    fn page(&mut self, url: &str, title: &str, snippet: &str) -> Option<usize> {
        let url = url.trim();
        if url.is_empty() {
            return None;
        }
        let index = self
            .pages
            .iter()
            .position(|p| p.url == url)
            .unwrap_or_else(|| {
                self.pages.push(SearchResult {
                    url: url.to_owned(),
                    ..SearchResult::default()
                });
                self.pages.len() - 1
            });
        let page = &mut self.pages[index];
        if page.title.is_empty() {
            title.trim().clone_into(&mut page.title);
        }
        if page.snippet.is_empty() {
            page.snippet = cut_snippet(snippet.split_whitespace().collect::<Vec<_>>().join(" "));
        }
        Some(index)
    }

    fn cite(&mut self, url: &str, title: &str, snippet: &str) {
        if let Some(index) = self.page(url, title, snippet) {
            self.cite_page(index);
        }
    }

    fn cite_page(&mut self, index: usize) {
        if !self.cited.contains(&index) {
            self.cited.push(index);
        }
    }

    /// The content blocks of an Anthropic response. Text before the first search only says that
    /// the model will search, so the answer starts after it; text blocks that a search separates
    /// are separate paragraphs.
    fn read_anthropic(&mut self, content: &[Value]) {
        let mut after_text = false;
        for block in content {
            match str_at(block, "type") {
                "server_tool_use" => {
                    self.searched |= str_at(block, "name") == "web_search";
                    after_text = false;
                }
                "web_search_tool_result" => {
                    self.searched = true;
                    after_text = false;
                    match block.get("content") {
                        Some(Value::Array(results)) => {
                            for result in results {
                                self.page(str_at(result, "url"), str_at(result, "title"), "");
                            }
                        }
                        Some(error) => {
                            let code = str_at(error, "error_code");
                            if !code.is_empty() {
                                self.errors.push(code.to_owned());
                            }
                        }
                        None => {}
                    }
                }
                "text" if self.searched => {
                    if !after_text && !self.answer.is_empty() {
                        self.answer.push_str("\n\n");
                    }
                    self.answer.push_str(str_at(block, "text"));
                    after_text = true;
                    for citation in array_at(block, "/citations") {
                        if str_at(citation, "type") == "web_search_result_location" {
                            self.cite(
                                str_at(citation, "url"),
                                str_at(citation, "title"),
                                str_at(citation, "cited_text"),
                            );
                        }
                    }
                }
                _ => after_text = false,
            }
        }
    }

    fn finish(self) -> Result<NativeResults, AiError> {
        if !self.searched {
            return Err(AiError::Protocol(
                "the model ran no web search; its provider may not offer one for it".into(),
            ));
        }
        if self.pages.is_empty() && !self.errors.is_empty() {
            return Err(AiError::Protocol(format!(
                "the web search failed ({})",
                self.errors.join(", ")
            )));
        }
        let mut order = self.cited.clone();
        order.extend((0..self.pages.len()).filter(|i| !self.cited.contains(i)));
        let mut pages: Vec<Option<SearchResult>> = self.pages.into_iter().map(Some).collect();
        let sources = order
            .into_iter()
            .filter_map(|i| pages[i].take())
            .take(MAX_RESULTS)
            .collect();
        Ok(NativeResults {
            sources,
            answer: self.answer.trim().to_owned(),
        })
    }
}

/// The `web_search` result text of a native search: the sources as
/// [`format_search_results`] numbers them, then the model's account of them.
#[must_use]
pub fn format_native_results(found: &NativeResults) -> String {
    match (found.sources.is_empty(), found.answer.is_empty()) {
        (true, true) => "No results.".to_owned(),
        (true, false) => found.answer.clone(),
        (false, true) => format_search_results(&found.sources),
        (false, false) => format!(
            "{}\nWhat the results say:\n{}\n",
            format_search_results(&found.sources),
            found.answer
        ),
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use zeroize::Zeroizing;

    use super::*;
    use crate::provider::{AuthHeader, http_client};

    fn provider(protocol: Protocol, base_url: &str) -> ProviderConfig {
        ProviderConfig {
            protocol,
            base_url: base_url.into(),
            api_key: Zeroizing::new("sk-test".into()),
            auth_header: AuthHeader::XApiKey,
            chatgpt: None,
        }
    }

    async fn run(
        kind: NativeSearch,
        protocol: Protocol,
        base_url: &str,
        model: &str,
    ) -> Result<NativeResults, AiError> {
        search_with(
            &http_client(),
            kind,
            &provider(protocol, base_url),
            &ModelSpec::new(model),
            " rust 2026 edition ",
            &CancellationToken::new(),
        )
        .await
    }

    #[test]
    fn each_provider_has_its_search() {
        let of = |protocol, url: &str| NativeSearch::of(&provider(protocol, url));
        use NativeSearch as N;
        use Protocol::{Anthropic, ChatCompletions, SystemOne};
        assert_eq!(
            of(Anthropic, "https://api.anthropic.com"),
            Some(N::Anthropic)
        );
        assert_eq!(
            of(Anthropic, "https://relay.example.com/claude"),
            Some(N::Anthropic)
        );
        assert_eq!(
            of(ChatCompletions, "https://openrouter.ai/api/v1"),
            Some(N::OpenRouter)
        );
        assert_eq!(
            of(ChatCompletions, "https://us.OpenRouter.ai/api/v1"),
            Some(N::OpenRouter)
        );
        assert_eq!(
            of(ChatCompletions, "https://api.openai.com/v1"),
            Some(N::OpenAi)
        );
        assert_eq!(of(ChatCompletions, "https://api.x.ai/v1"), Some(N::Xai));
        assert_eq!(
            of(
                ChatCompletions,
                "https://generativelanguage.googleapis.com/v1beta/openai/"
            ),
            Some(N::Gemini)
        );
        // Any other host is tried as OpenAI is.
        for url in [
            "https://api.deepseek.com/v1",
            "http://localhost:11434/v1",
            "https://openrouter.ai.example.com/v1",
            "not a url",
        ] {
            assert_eq!(of(ChatCompletions, url), Some(N::Other), "{url}");
        }
        assert_eq!(of(SystemOne, "https://openrouter.ai/api/v1"), None);
    }

    #[tokio::test]
    async fn other_providers_are_tried_as_openai_is() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/relay/v1/responses"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "tools": [{"type": "web_search"}],
                "tool_choice": "required",
                "include": ["web_search_call.action.sources"],
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output": [
                    {"type": "web_search_call", "status": "completed", "action": {"type": "search", "query": "q"}},
                    {"type": "message", "content": [{"type": "output_text", "text": "Found it.", "annotations": []}]}
                ]
            })))
            .mount(&server)
            .await;
        let found = run(
            NativeSearch::Other,
            Protocol::ChatCompletions,
            &format!("{}/relay/v1", server.uri()),
            "gpt-x",
        )
        .await
        .unwrap();
        assert_eq!(found.answer, "Found it.");
    }

    #[test]
    fn a_refused_request_or_no_search_turns_the_search_away() {
        let http = |status| AiError::Http {
            status,
            message: String::new(),
        };
        for status in [400, 404, 405, 415, 422, 501] {
            assert!(turned_away(&http(status)), "{status}");
        }
        assert!(turned_away(&AiError::Protocol(
            "the model ran no web search".into()
        )));
        for passing in [
            http(401),
            http(429),
            http(500),
            http(503),
            AiError::Network("timed out".into()),
            AiError::Cancelled,
        ] {
            assert!(!turned_away(&passing), "{passing}");
        }
    }

    #[tokio::test]
    async fn anthropic_continues_a_paused_turn_and_reads_citations() {
        let server = MockServer::start().await;
        let search = json!({"type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search", "input": {"query": "rust 2026 edition"}});
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "sk-test"))
            .and(body_partial_json(json!({
                "model": "claude-x",
                "max_tokens": 4096,
                "tools": [{"type": "web_search_20250305", "name": "web_search", "max_uses": 5}],
                "messages": [{"role": "user", "content": "Search the web for: rust 2026 edition"}],
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [
                    {"type": "text", "text": "I'll search for that."},
                    search,
                    {"type": "web_search_tool_result", "tool_use_id": "srvtoolu_1", "content": [
                        {"type": "web_search_result", "url": "https://blog.rust-lang.org/a", "title": "Rust Blog", "encrypted_content": "x"},
                        {"type": "web_search_result", "url": "https://doc.rust-lang.org/edition-guide", "title": "Edition Guide", "encrypted_content": "y"}
                    ]}
                ],
                "stop_reason": "pause_turn"
            })))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(body_partial_json(json!({"messages": [{}, {"role": "assistant"}]})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [
                    {"type": "text", "text": "Based on the results, "},
                    {"type": "text", "text": "the guide covers the edition.", "citations": [
                        {"type": "web_search_result_location", "url": "https://doc.rust-lang.org/edition-guide", "title": "Edition Guide", "cited_text": "The  Rust\n2026 edition…", "encrypted_index": "z"}
                    ]}
                ],
                "stop_reason": "end_turn"
            })))
            .mount(&server)
            .await;
        let found = run(
            NativeSearch::Anthropic,
            Protocol::Anthropic,
            &server.uri(),
            "claude-x",
        )
        .await
        .unwrap();
        assert_eq!(
            found.answer,
            "Based on the results, the guide covers the edition."
        );
        assert_eq!(
            found.sources,
            [
                SearchResult {
                    title: "Edition Guide".into(),
                    url: "https://doc.rust-lang.org/edition-guide".into(),
                    snippet: "The Rust 2026 edition…".into(),
                },
                SearchResult {
                    title: "Rust Blog".into(),
                    url: "https://blog.rust-lang.org/a".into(),
                    snippet: String::new(),
                },
            ]
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_response_without_a_search_or_with_failed_searches_is_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [{"type": "text", "text": "From memory: it came out in 2025."}],
                "stop_reason": "end_turn"
            })))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [
                    {"type": "server_tool_use", "id": "s", "name": "web_search", "input": {}},
                    {"type": "web_search_tool_result", "tool_use_id": "s", "content": {"type": "web_search_tool_result_error", "error_code": "too_many_requests"}},
                    {"type": "text", "text": "The search failed."}
                ],
                "stop_reason": "end_turn"
            })))
            .mount(&server)
            .await;
        let none = run(
            NativeSearch::Anthropic,
            Protocol::Anthropic,
            &server.uri(),
            "m",
        )
        .await;
        assert!(matches!(none, Err(AiError::Protocol(m)) if m.contains("ran no web search")));
        let failed = run(
            NativeSearch::Anthropic,
            Protocol::Anthropic,
            &server.uri(),
            "m",
        )
        .await;
        assert!(matches!(failed, Err(AiError::Protocol(m)) if m.contains("too_many_requests")));
    }

    #[tokio::test]
    async fn openrouter_reads_url_citations() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/chat/completions"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "anthropic/claude-x",
                "tools": [{"type": "openrouter:web_search"}],
                "stream": false,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {
                    "role": "assistant",
                    "content": "Rust 2026 is planned.",
                    "annotations": [{"type": "url_citation", "url_citation": {
                        "url": "https://blog.rust-lang.org/b", "title": "Plans", "content": "planned for 2026", "start_index": 0, "end_index": 4
                    }}]
                }}]
            })))
            .mount(&server)
            .await;
        let found = run(
            NativeSearch::OpenRouter,
            Protocol::ChatCompletions,
            &format!("{}/api/v1", server.uri()),
            "anthropic/claude-x",
        )
        .await
        .unwrap();
        assert_eq!(found.answer, "Rust 2026 is planned.");
        assert_eq!(found.sources[0].snippet, "planned for 2026");
    }

    #[tokio::test]
    async fn openrouter_without_citations_counts_its_searches() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"role": "assistant", "content": "Nothing on that."}}],
                "usage": {"server_tool_use": {"web_search_requests": 1}}
            })))
            .mount(&server)
            .await;
        let found = run(
            NativeSearch::OpenRouter,
            Protocol::ChatCompletions,
            &server.uri(),
            "m",
        )
        .await
        .unwrap();
        assert!(found.sources.is_empty());
        assert_eq!(format_native_results(&found), "Nothing on that.");
    }

    #[tokio::test]
    async fn openai_uses_the_responses_api() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "gpt-x",
                "tools": [{"type": "web_search"}],
                "tool_choice": "required",
                "include": ["web_search_call.action.sources"],
                "store": false,
                "input": "Search the web for: rust 2026 edition",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output": [
                    {"type": "reasoning", "id": "rs_1", "summary": []},
                    {"type": "web_search_call", "id": "ws_1", "status": "completed", "action": {
                        "type": "search", "query": "rust 2026 edition",
                        "sources": [{"type": "url", "url": "https://a.example"}, {"type": "url", "url": "https://b.example"}]
                    }},
                    {"type": "message", "role": "assistant", "content": [{
                        "type": "output_text", "text": "It ships in 2026.",
                        "annotations": [{"type": "url_citation", "url": "https://b.example", "title": "B", "start_index": 0, "end_index": 5}]
                    }]}
                ]
            })))
            .mount(&server)
            .await;
        let found = run(
            NativeSearch::OpenAi,
            Protocol::ChatCompletions,
            &format!("{}/v1", server.uri()),
            "gpt-x",
        )
        .await
        .unwrap();
        let urls: Vec<_> = found
            .sources
            .iter()
            .map(|s| (s.url.as_str(), s.title.as_str()))
            .collect();
        assert_eq!(
            urls,
            [("https://b.example", "B"), ("https://a.example", "")]
        );
        assert_eq!(found.answer, "It ships in 2026.");
    }

    #[tokio::test]
    async fn xai_sends_neither_include_nor_tool_choice() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output": [{"type": "message", "content": [{"type": "output_text", "text": "Yes.", "annotations": []}]}],
                "citations": ["https://x.example/post"]
            })))
            .mount(&server)
            .await;
        let found = run(
            NativeSearch::Xai,
            Protocol::ChatCompletions,
            &format!("{}/v1", server.uri()),
            "grok-x",
        )
        .await
        .unwrap();
        assert_eq!(found.sources[0].url, "https://x.example/post");
        let sent: Value = server.received_requests().await.unwrap()[0]
            .body_json()
            .unwrap();
        assert!(sent.get("include").is_none() && sent.get("tool_choice").is_none());
    }

    #[tokio::test]
    async fn gemini_grounds_with_google_search() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1beta/models/gemini-x:generateContent"))
            .and(header("x-goog-api-key", "sk-test"))
            .and(body_partial_json(json!({"tools": [{"google_search": {}}]})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{
                    "content": {"parts": [{"text": "thinking", "thought": true}, {"text": "Spain won."}]},
                    "groundingMetadata": {
                        "webSearchQueries": ["euro 2024 winner"],
                        "groundingChunks": [
                            {"web": {"uri": "https://vertexaisearch.example/1", "title": "uefa.com"}},
                            {"web": {"uri": "https://vertexaisearch.example/2", "title": "aljazeera.com"}}
                        ],
                        "groundingSupports": [{"segment": {"text": "Spain won."}, "groundingChunkIndices": [1, 0]}]
                    }
                }]
            })))
            .mount(&server)
            .await;
        let found = run(
            NativeSearch::Gemini,
            Protocol::ChatCompletions,
            &format!("{}/v1beta/openai/", server.uri()),
            "models/gemini-x",
        )
        .await
        .unwrap();
        assert_eq!(found.answer, "Spain won.");
        let titles: Vec<_> = found.sources.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["aljazeera.com", "uefa.com"]);
    }

    #[tokio::test]
    async fn provider_errors_and_unsupported_providers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "type": "error",
                "error": {"type": "invalid_request_error", "message": "web search is not enabled"}
            })))
            .mount(&server)
            .await;
        assert_eq!(
            run(
                NativeSearch::Anthropic,
                Protocol::Anthropic,
                &server.uri(),
                "m"
            )
            .await
            .unwrap_err(),
            AiError::Http {
                status: 400,
                message: "web search is not enabled".into()
            }
        );
        let http = http_client();
        let cancel = CancellationToken::new();
        let judge = provider(Protocol::SystemOne, &server.uri());
        assert!(matches!(
            native_search(&http, &judge, &ModelSpec::new("m"), "q", &cancel).await,
            Err(AiError::Config(_))
        ));
        let anthropic = provider(Protocol::Anthropic, &server.uri());
        assert!(matches!(
            native_search(&http, &anthropic, &ModelSpec::new("m"), "  ", &cancel).await,
            Err(AiError::Config(_))
        ));
    }

    #[test]
    fn results_are_formatted() {
        let source = SearchResult {
            title: "A".into(),
            url: "https://a".into(),
            snippet: String::new(),
        };
        let both = NativeResults {
            sources: vec![source.clone()],
            answer: "A says so.".into(),
        };
        assert_eq!(
            format_native_results(&both),
            "1. A\n   https://a\n\nWhat the results say:\nA says so.\n"
        );
        assert_eq!(
            format_native_results(&NativeResults {
                sources: vec![source],
                answer: String::new()
            }),
            "1. A\n   https://a\n"
        );
        assert_eq!(
            format_native_results(&NativeResults::default()),
            "No results."
        );
    }
}
