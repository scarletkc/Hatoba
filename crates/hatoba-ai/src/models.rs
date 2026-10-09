//! Model listing (AI-03) and Test Connection (AI-04).

use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::error::AiError;
use crate::net;
use crate::provider::{Effort, Protocol, ProviderConfig, client_for, validate_base_url};

/// A model from the provider's list.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    /// The model id requests use.
    pub id: String,
    /// Display name: Anthropic's `display_name`, the list's `name` when it has one, else the id.
    pub name: String,
    /// Context window in tokens, when the list says.
    pub context_window: Option<u64>,
    /// Output limit in tokens, when the list says.
    pub max_output_tokens: Option<u64>,
    /// The thinking levels the model accepts, lowest first, when the list says (Anthropic's
    /// `capabilities.effort`); empty when it accepts none.
    pub efforts: Option<Vec<Effort>>,
    /// Whether the model supports adaptive thinking, when the list says (Anthropic's
    /// `capabilities.thinking`).
    pub adaptive_thinking: Option<bool>,
}

/// `{"supported": bool}` at `value`.
fn supported(value: Option<&Value>) -> Option<bool> {
    value?.get("supported")?.as_bool()
}

/// The thinking levels in an Anthropic model's `capabilities`: `effort.supported` false is none,
/// otherwise every level whose `supported` is true. `None` when the list does not say (no
/// `capabilities.effort`, or no level in it).
fn capability_efforts(capabilities: &Value) -> Option<Vec<Effort>> {
    let effort = capabilities.get("effort")?;
    if supported(Some(effort)) == Some(false) {
        return Some(Vec::new());
    }
    let mut known = false;
    let mut levels = Vec::new();
    for level in Effort::ALL {
        match supported(effort.get(level.as_str())) {
            Some(true) => {
                known = true;
                levels.push(level);
            }
            Some(false) => known = true,
            None => {}
        }
    }
    known.then_some(levels)
}

/// Whether an Anthropic model's `capabilities` say it supports adaptive thinking: `false` when
/// `thinking.supported` is false, else `thinking.types.adaptive.supported`.
fn capability_adaptive(capabilities: &Value) -> Option<bool> {
    let thinking = capabilities.get("thinking")?;
    if supported(Some(thinking)) == Some(false) {
        return Some(false);
    }
    supported(thinking.pointer("/types/adaptive"))
}

/// Overall limit of a model list request and of Test Connection.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Anthropic list pages followed at most (1,000 models each).
const MAX_PAGES: usize = 20;

/// Field names servers use for the context window and the output limit (OpenRouter, vLLM, Groq,
/// LM Studio, Gemini and others).
const CONTEXT_KEYS: &[&str] = &[
    "context_window",
    "context_length",
    "max_context_length",
    "max_model_len",
    "max_input_tokens",
    "input_token_limit",
];
const OUTPUT_KEYS: &[&str] = &[
    "max_output_tokens",
    "max_completion_tokens",
    "output_token_limit",
    "max_tokens",
];

fn first_u64(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|k| value.get(*k).and_then(Value::as_u64))
        .filter(|n| *n > 0)
}

/// One entry of a Chat Completions `/models` list.
fn chat_completions_model(item: &Value) -> Option<ModelInfo> {
    let id = item.get("id").and_then(Value::as_str)?.trim();
    if id.is_empty() {
        return None;
    }
    let name = ["display_name", "name"]
        .iter()
        .find_map(|k| item.get(*k).and_then(Value::as_str))
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(id);
    Some(ModelInfo {
        id: id.to_owned(),
        name: name.to_owned(),
        context_window: first_u64(item, CONTEXT_KEYS),
        max_output_tokens: first_u64(item, OUTPUT_KEYS).or_else(|| {
            item.get("top_provider")
                .and_then(|p| first_u64(p, &["max_completion_tokens"]))
        }),
        efforts: None,
        adaptive_thinking: None,
    })
}

/// One entry of Anthropic's `/v1/models` list, with the thinking levels and adaptive thinking
/// from its `capabilities` when it has them (Anthropic-compatible vendors often do not).
fn anthropic_model(item: &Value) -> Option<ModelInfo> {
    let id = item.get("id").and_then(Value::as_str)?;
    let name = item
        .get("display_name")
        .and_then(Value::as_str)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or(id);
    let capabilities = item.get("capabilities").filter(|c| c.is_object());
    Some(ModelInfo {
        id: id.to_owned(),
        name: name.trim().to_owned(),
        context_window: first_u64(item, &["max_input_tokens"]),
        max_output_tokens: first_u64(item, &["max_tokens"]),
        efforts: capabilities.and_then(capability_efforts),
        adaptive_thinking: capabilities.and_then(capability_adaptive),
    })
}

/// Fetches the provider's model list: `GET {base}/models` (`data[]`) for Chat Completions, or
/// `GET {base}/v1/models` for Anthropic, following `has_more`/`last_id` pages with `after_id`.
/// Duplicate ids are dropped; the provider's order is kept.
///
/// Gives up after 60 s, and with [`AiError::Cancelled`] as soon as `cancel` fires (the connection
/// is dropped, so a lock does not leave the key in use).
pub async fn list_models(
    http: &reqwest::Client,
    provider: &ProviderConfig,
    cancel: &CancellationToken,
) -> Result<Vec<ModelInfo>, AiError> {
    tokio::time::timeout(REQUEST_TIMEOUT, list_models_inner(http, provider, cancel))
        .await
        .unwrap_or_else(|_| Err(AiError::Network("the request timed out".into())))
}

/// [`validate_base_url`], giving up as soon as `cancel` fires.
async fn checked_base(
    provider: &ProviderConfig,
    cancel: &CancellationToken,
) -> Result<url::Url, AiError> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(AiError::Cancelled),
        base = validate_base_url(&provider.base_url) => base,
    }
}

async fn list_models_inner(
    http: &reqwest::Client,
    provider: &ProviderConfig,
    cancel: &CancellationToken,
) -> Result<Vec<ModelInfo>, AiError> {
    let base = checked_base(provider, cancel).await?;
    let headers = provider.headers()?;
    let http = client_for(http, &base);
    let mut models: Vec<ModelInfo> = Vec::new();
    let push = |model: ModelInfo, models: &mut Vec<ModelInfo>| {
        if !models.iter().any(|m| m.id == model.id) {
            models.push(model);
        }
    };
    match provider.protocol {
        Protocol::ChatCompletions => {
            let url = net::endpoint(&base, "/models");
            let response = net::send(http.get(url).headers(headers), cancel).await?;
            if !response.status().is_success() {
                return Err(net::http_error(response, cancel).await);
            }
            let body = net::read_json(response, cancel).await?;
            let items = body
                .get("data")
                .or_else(|| body.get("models"))
                .unwrap_or(&body)
                .as_array()
                .ok_or_else(|| AiError::Protocol("the model list has no `data` array".into()))?;
            for model in items.iter().filter_map(chat_completions_model) {
                push(model, &mut models);
            }
        }
        Protocol::Anthropic => {
            let mut after: Option<String> = None;
            for _ in 0..MAX_PAGES {
                let mut url = net::endpoint(&base, "/v1/models");
                url.query_pairs_mut().append_pair("limit", "1000");
                if let Some(after) = &after {
                    url.query_pairs_mut().append_pair("after_id", after);
                }
                let response = net::send(http.get(url).headers(headers.clone()), cancel).await?;
                if !response.status().is_success() {
                    return Err(net::http_error(response, cancel).await);
                }
                let body = net::read_json(response, cancel).await?;
                let items = body.get("data").and_then(Value::as_array).ok_or_else(|| {
                    AiError::Protocol("the model list has no `data` array".into())
                })?;
                for model in items.iter().filter_map(anthropic_model) {
                    push(model, &mut models);
                }
                let has_more = body.get("has_more").and_then(Value::as_bool) == Some(true);
                let last = body
                    .get("last_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                match last {
                    Some(last) if has_more && after.as_deref() != Some(last.as_str()) => {
                        after = Some(last);
                    }
                    _ => break,
                }
            }
        }
    }
    Ok(models)
}

/// Why Test Connection failed (AI-04).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestFailure {
    /// 401/403, or a key that cannot be sent.
    Auth,
    /// No response: DNS, connection, TLS, timeout.
    Network,
    /// The provider does not know the model.
    UnknownModel,
    /// The base URL was refused (AI-02).
    InvalidUrl,
    /// Anything else; `status` and `message` say more.
    Other,
}

/// The result of Test Connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestOutcome {
    /// The provider answered successfully.
    pub ok: bool,
    /// Why not, when `ok` is false.
    pub failure: Option<TestFailure>,
    /// HTTP status of a failed request, when there was a response.
    pub status: Option<u16>,
    /// The provider's error message or a description of the failure.
    pub message: Option<String>,
}

impl TestOutcome {
    fn ok() -> Self {
        Self {
            ok: true,
            failure: None,
            status: None,
            message: None,
        }
    }

    fn failed(failure: TestFailure, err: &AiError) -> Self {
        let message = match err {
            AiError::Http { message, .. }
            | AiError::InvalidUrl(message)
            | AiError::Config(message)
            | AiError::Network(message)
            | AiError::Protocol(message) => message.clone(),
            other => other.to_string(),
        };
        Self {
            ok: false,
            failure: Some(failure),
            status: err.status(),
            message: Some(message),
        }
    }
}

/// Whether an error message says the model does not exist (or is not available).
fn says_unknown_model(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("model")
        && [
            "not exist",
            "does not exist",
            "not found",
            "unknown",
            "invalid",
            "no such",
            "not supported",
            "unsupported",
            "not available",
            "no access",
        ]
        .iter()
        .any(|phrase| m.contains(phrase))
}

/// Classifies a failed test (AI-04).
pub(crate) fn classify(err: &AiError, with_model: bool) -> TestOutcome {
    let failure = match err {
        AiError::InvalidUrl(_) => TestFailure::InvalidUrl,
        AiError::Config(_) => TestFailure::Auth,
        AiError::Network(_) => TestFailure::Network,
        AiError::Http {
            status: 401 | 403, ..
        } => TestFailure::Auth,
        // A 404 that does not mention a model is usually a wrong base URL path.
        AiError::Http {
            status: 404,
            message,
        } if with_model && message.to_ascii_lowercase().contains("model") => {
            TestFailure::UnknownModel
        }
        AiError::Http {
            status: 400 | 422,
            message,
        } if with_model && says_unknown_model(message) => TestFailure::UnknownModel,
        _ => TestFailure::Other,
    };
    TestOutcome::failed(failure, err)
}

/// Test Connection (AI-04): with a model id, a minimal non-streaming request to that model
/// (a few tokens at most where the protocol allows a limit); without one, the model list.
/// Gives up after 60 s, and as soon as `cancel` fires with a failed outcome whose message is
/// `cancelled` (the connection is dropped, so a lock does not leave the key in use).
pub async fn test_connection(
    http: &reqwest::Client,
    provider: &ProviderConfig,
    model_id: Option<&str>,
    cancel: &CancellationToken,
) -> TestOutcome {
    let model_id = model_id.map(str::trim).filter(|m| !m.is_empty());
    let result = tokio::time::timeout(REQUEST_TIMEOUT, async {
        match model_id {
            Some(model) => probe_model(http, provider, model, cancel).await,
            None => list_models_inner(http, provider, cancel).await.map(drop),
        }
    })
    .await
    .unwrap_or_else(|_| Err(AiError::Network("the request timed out".into())));
    match result {
        Ok(()) => TestOutcome::ok(),
        Err(err) => classify(&err, model_id.is_some()),
    }
}

async fn probe_model(
    http: &reqwest::Client,
    provider: &ProviderConfig,
    model: &str,
    cancel: &CancellationToken,
) -> Result<(), AiError> {
    let base = checked_base(provider, cancel).await?;
    let headers = provider.headers()?;
    let http = client_for(http, &base);
    let messages = json!([{"role": "user", "content": "Hi"}]);
    let (url, body) = match provider.protocol {
        // No token limit: OpenAI's newer models refuse `max_tokens`, and the field name differs
        // between servers. A reply to "Hi" is short anyway.
        Protocol::ChatCompletions => (
            net::endpoint(&base, "/chat/completions"),
            json!({"model": model, "messages": messages}),
        ),
        Protocol::Anthropic => (
            net::endpoint(&base, "/v1/messages"),
            json!({"model": model, "max_tokens": 16, "messages": messages}),
        ),
    };
    let request = http
        .post(url)
        .headers(headers)
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(body.to_string());
    let response = net::send(request, cancel).await?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(net::http_error(response, cancel).await)
    }
}

#[cfg(test)]
mod tests;
