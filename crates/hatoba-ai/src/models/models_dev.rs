//! Well-known models (AI-03): the context window, output limit, thinking levels and adaptive
//! thinking of models whose provider's list does not say, as gateways such as New API and
//! OneAPI list only ids, from [models.dev](https://models.dev)'s data. Hatoba ships a snapshot of
//! it (`models-dev.json`, which `cargo run -p hatoba-ai --example models_dev_snapshot` refreshes)
//! and uses newer data the app fetched ([`fetch`]) once it is [`install`]ed.
//!
//! Only the models of the vendors' own providers are kept ([`SOURCES`]): a gateway's or a cloud's
//! entry for a model may name other levels. A model is matched by id, without a vendor prefix
//! (`anthropic/claude-opus-4.8`, `us.anthropic.claude-opus-4-8-v1:0`) and with suffixes such as a
//! date, `-thinking`, `:free` or `[1m]` ignored, but never across versions: `claude-opus-5-7` and
//! `gpt-5.9` are unknown until models.dev has them. Nor is a ChatGPT `-chat` model the model it
//! names (`gpt-5.2-chat-latest` does not reason as `gpt-5.2` does), and models.dev has none.
//!
//! The thinking levels are those of the vendor's own request format, so they apply only on that
//! protocol: Anthropic's levels on `anthropic`, DeepSeek's (the same in both of its formats) on
//! any, the others on Chat Completions and Responses. Elsewhere a gateway translates the levels
//! in its own way, so they stay unknown. models.dev does not say which models think adaptively:
//! a Claude model does when it takes the `max` level, as both came with Claude 4.6.

use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::sync::{Arc, LazyLock, PoisonError, RwLock};
use std::time::Duration;

use reqwest::header::{ACCEPT_ENCODING, CONTENT_ENCODING, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::error::AiError;
use crate::net;
use crate::provider::Effort::{self, Max};
use crate::provider::{ModelSpec, Protocol};

use super::ModelInfo;

/// models.dev's data: every provider's models, with their limits and reasoning options.
pub const MODELS_DEV_URL: &str = "https://models.dev/api.json";

/// The snapshot of models.dev's data that Hatoba ships, as [`ModelsDev::json`] writes it.
const SNAPSHOT: &str = include_str!("models-dev.json");

/// Overall limit of [`fetch`].
const FETCH_TIMEOUT: Duration = Duration::from_secs(60);
/// The most [`fetch`] reads, compressed (about 0.6 MB in 2026)…
const MAX_BODY: usize = 16 * 1024 * 1024;
/// …and decompressed (about 6 MB).
const MAX_DATA: usize = 64 * 1024 * 1024;

/// What models.dev says about a model on a provider's protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownModel {
    /// Context window in tokens: the input limit where models.dev gives one (GPT-5 takes 272,000
    /// of its 400,000 as input), as Anthropic's `max_input_tokens` is.
    pub context_window: Option<u64>,
    /// Output limit in tokens.
    pub max_output_tokens: Option<u64>,
    /// The thinking levels the model accepts on this protocol, lowest first; `None` when they
    /// are not known for it.
    pub efforts: Option<Vec<Effort>>,
    /// Whether the model supports adaptive thinking (Claude models on `anthropic`).
    pub adaptive_thinking: Option<bool>,
}

/// The request format whose thinking levels a provider's models list.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    /// Anthropic Messages: `output_config.effort`.
    Anthropic,
    /// OpenAI's Chat Completions and Responses: `reasoning_effort`, `reasoning.effort`.
    OpenAi,
    /// Both, with the same levels (DeepSeek's API).
    Any,
}

/// A vendor's own provider in models.dev's data.
struct Source {
    /// The provider's key.
    provider: &'static str,
    /// The lab of its own models in `canonical_model_id`; the models it serves of other labs are
    /// left out.
    lab: &'static str,
    format: Format,
    /// Whether a model whose thinking takes only a token budget takes Low to High as well:
    /// Google's OpenAI compatibility turns `reasoning_effort` into a budget for Gemini 2.5.
    budget_levels: bool,
}

const fn source(provider: &'static str, lab: &'static str, format: Format) -> Source {
    Source {
        provider,
        lab,
        format,
        budget_levels: false,
    }
}

/// The providers whose models Hatoba keeps, in the order a model id found twice is taken.
const SOURCES: &[Source] = &[
    source("anthropic", "anthropic", Format::Anthropic),
    source("openai", "openai", Format::OpenAi),
    Source {
        budget_levels: true,
        ..source("google", "google", Format::OpenAi)
    },
    source("deepseek", "deepseek", Format::Any),
    // Qwen on Alibaba Cloud Model Studio.
    source("alibaba", "alibaba", Format::OpenAi),
    source("xai", "xai", Format::OpenAi),
    source("mistral", "mistral", Format::OpenAi),
    source("moonshotai", "moonshotai", Format::OpenAi),
    source("zai", "zhipuai", Format::OpenAi),
];

/// models.dev's data, or the part of it Hatoba keeps: provider key to its models.
type Snapshot = BTreeMap<String, SnapshotProvider>;

#[derive(Serialize, Deserialize)]
struct SnapshotProvider {
    models: BTreeMap<String, SnapshotModel>,
}

#[derive(Serialize, Deserialize)]
struct SnapshotModel {
    limit: Limit,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reasoning: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reasoning_options: Option<Vec<ReasoningOption>>,
    /// `lab/model`, which [`keep`] reads and the snapshot leaves out.
    #[serde(default, skip_serializing)]
    canonical_model_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Limit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output: Option<u64>,
}

/// One way to control a model's reasoning: `{"type": "effort", "values": [...]}`, `toggle` or
/// `budget_tokens`.
#[derive(Serialize, Deserialize)]
struct ReasoningOption {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    values: Option<Vec<Option<String>>>,
}

/// The models of [`SOURCES`] in models.dev's `data`: those with a context window, without the
/// models a provider serves for another lab. An entry that does not parse is left out.
fn keep(data: &Value) -> Snapshot {
    let mut kept = Snapshot::new();
    for source in SOURCES {
        let Some(models) = data
            .get(source.provider)
            .and_then(|p| p.get("models"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        let models: BTreeMap<_, _> = models
            .iter()
            .filter_map(|(id, model)| {
                let model = SnapshotModel::deserialize(model).ok()?;
                let lab = model
                    .canonical_model_id
                    .as_deref()
                    .and_then(|id| id.split_once('/'))
                    .map(|(lab, _)| lab);
                let own = lab.is_none_or(|lab| lab == source.lab);
                let sized = model.limit.context.is_some_and(|n| n > 0);
                (own && sized).then(|| (id.clone(), model))
            })
            .collect();
        if !models.is_empty() {
            kept.insert(source.provider.to_owned(), SnapshotProvider { models });
        }
    }
    kept
}

/// The levels of `model`: none when it does not reason, those of its `effort` option otherwise
/// (`none`, `minimal` and `default` have no level here; Default leaves them to the model), and
/// `None` when models.dev does not say.
fn levels(model: &SnapshotModel, source: &Source) -> Option<Vec<Effort>> {
    if !model.reasoning? {
        return Some(Vec::new());
    }
    let options = model.reasoning_options.as_ref()?;
    let named: Vec<&str> = options
        .iter()
        .filter(|o| o.kind == "effort")
        .flat_map(|o| o.values.iter().flatten().flatten())
        .map(String::as_str)
        .collect();
    if named.is_empty() && source.budget_levels && options.iter().any(|o| o.kind == "budget_tokens")
    {
        return Some(Effort::UNKNOWN_MODEL.to_vec());
    }
    Some(
        Effort::ALL
            .into_iter()
            .filter(|e| named.contains(&e.as_str()))
            .collect(),
    )
}

#[derive(Clone)]
struct Entry {
    /// The normalized id, alone or followed by a suffix ([`matches`]).
    key: String,
    context_window: Option<u64>,
    max_output_tokens: Option<u64>,
    efforts: Option<Vec<Effort>>,
    adaptive_thinking: Option<bool>,
    format: Format,
}

/// The well-known models, from models.dev's data.
pub struct Catalog {
    entries: Vec<Entry>,
}

impl Catalog {
    fn new(snapshot: &Snapshot) -> Self {
        let mut seen = HashSet::new();
        let mut entries = Vec::new();
        for source in SOURCES {
            let Some(provider) = snapshot.get(source.provider) else {
                continue;
            };
            for (id, model) in &provider.models {
                let key = normalize(id);
                if key.is_empty() || !seen.insert(key.clone()) {
                    continue;
                }
                let efforts = levels(model, source);
                let adaptive_thinking = match source.format {
                    Format::Anthropic => efforts.as_ref().map(|levels| levels.contains(&Max)),
                    Format::OpenAi | Format::Any => None,
                };
                entries.push(Entry {
                    key,
                    context_window: model.limit.input.or(model.limit.context).filter(|n| *n > 0),
                    max_output_tokens: model.limit.output.filter(|n| *n > 0),
                    efforts,
                    adaptive_thinking,
                    format: source.format,
                });
            }
        }
        Self { entries }
    }

    /// How many models it knows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether it knows no model.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// This catalog, with the models of `older` it does not have: newer data wins, and a model
    /// models.dev has dropped since keeps the values it had, which do not change.
    #[must_use]
    pub fn over(mut self, older: &Catalog) -> Self {
        let keys: HashSet<String> = self.entries.iter().map(|e| e.key.clone()).collect();
        self.entries.extend(
            older
                .entries
                .iter()
                .filter(|e| !keys.contains(&e.key))
                .cloned(),
        );
        self
    }

    /// Whether it knows model `id`, on whichever protocol.
    #[must_use]
    pub fn knows(&self, id: &str) -> bool {
        let id = normalize(id);
        self.entries.iter().any(|e| matches(&id, &e.key))
    }

    /// What it says about model `id` on `protocol`, by the longest key that matches. `None` for a
    /// model it does not know, and on System One (AI-47), whose models only decide.
    #[must_use]
    pub fn lookup(&self, id: &str, protocol: Protocol) -> Option<KnownModel> {
        let format = match protocol {
            Protocol::Anthropic => Format::Anthropic,
            Protocol::ChatCompletions | Protocol::Responses => Format::OpenAi,
            Protocol::SystemOne => return None,
        };
        let id = normalize(id);
        let entry = self
            .entries
            .iter()
            .filter(|e| matches(&id, &e.key))
            .max_by_key(|e| e.key.len())?;
        let native = entry.format == format || entry.format == Format::Any;
        Some(KnownModel {
            context_window: entry.context_window,
            max_output_tokens: entry.max_output_tokens,
            efforts: entry.efforts.clone().filter(|_| native),
            adaptive_thinking: entry.adaptive_thinking.filter(|_| native),
        })
    }
}

/// models.dev's data as Hatoba keeps it.
pub struct ModelsDev {
    pub catalog: Catalog,
    /// The data the catalog was built from, as the snapshot and the app's cache keep it.
    pub json: String,
}

impl ModelsDev {
    /// Reads models.dev's `api.json`, or the JSON of an earlier [`ModelsDev::json`]. Fails when
    /// it is not JSON or has no Anthropic or OpenAI models, so a broken download never replaces
    /// good data.
    pub fn parse(text: &[u8]) -> Result<Self, AiError> {
        let data: Value = serde_json::from_slice(text)
            .map_err(|_| AiError::Protocol("models.dev's data is not valid JSON".into()))?;
        let snapshot = keep(&data);
        if !["anthropic", "openai"]
            .iter()
            .all(|p| snapshot.contains_key(*p))
        {
            return Err(AiError::Protocol(
                "models.dev's data has no Anthropic or OpenAI models".into(),
            ));
        }
        let json = serde_json::to_string_pretty(&snapshot)
            .map_err(|e| AiError::Protocol(format!("models.dev's data: {e}")))?;
        Ok(Self {
            catalog: Catalog::new(&snapshot),
            json,
        })
    }
}

static BUNDLED: LazyLock<Arc<Catalog>> = LazyLock::new(|| {
    Arc::new(
        ModelsDev::parse(SNAPSHOT.as_bytes())
            .expect("the bundled snapshot of models.dev's data must parse")
            .catalog,
    )
});

static INSTALLED: RwLock<Option<Arc<Catalog>>> = RwLock::new(None);

/// The catalog [`known_model`] reads: the last one installed, else the bundled snapshot's.
fn current() -> Arc<Catalog> {
    INSTALLED
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .unwrap_or_else(|| BUNDLED.clone())
}

/// Makes `catalog`, over the data in use ([`Catalog::over`]), the one [`known_model`] reads:
/// fetched data adds to what the snapshot and earlier fetches knew, and overrides it.
pub fn install(catalog: Catalog) {
    let mut installed = INSTALLED.write().unwrap_or_else(PoisonError::into_inner);
    let in_use = installed.clone().unwrap_or_else(|| BUNDLED.clone());
    *installed = Some(Arc::new(catalog.over(&in_use)));
}

/// What models.dev says about model `id` on `protocol` ([`Catalog::lookup`]), from the installed
/// data or the bundled snapshot.
#[must_use]
pub fn known_model(id: &str, protocol: Protocol) -> Option<KnownModel> {
    current().lookup(id, protocol)
}

/// Whether the installed data or the bundled snapshot knows model `id` ([`Catalog::knows`]).
#[must_use]
pub fn knows(id: &str) -> bool {
    current().knows(id)
}

/// Fetches models.dev's data ([`MODELS_DEV_URL`]), gzip-compressed, and keeps what
/// [`ModelsDev::parse`] keeps. Gives up after 60 s, and as soon as `cancel` fires.
pub async fn fetch(
    http: &reqwest::Client,
    cancel: &CancellationToken,
) -> Result<ModelsDev, AiError> {
    let url = Url::parse(MODELS_DEV_URL).expect("models.dev's URL parses");
    fetch_from(http, url, cancel).await
}

pub(crate) async fn fetch_from(
    http: &reqwest::Client,
    url: Url,
    cancel: &CancellationToken,
) -> Result<ModelsDev, AiError> {
    tokio::time::timeout(FETCH_TIMEOUT, async {
        let request = http
            .get(url)
            .header(ACCEPT_ENCODING, HeaderValue::from_static("gzip"));
        let response = net::send(request, cancel).await?;
        if !response.status().is_success() {
            return Err(net::http_error(response, cancel).await);
        }
        let gzip = response
            .headers()
            .get(CONTENT_ENCODING)
            .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"gzip"));
        let (body, truncated) = net::read_capped(response, MAX_BODY, cancel).await?;
        if truncated {
            return Err(AiError::Protocol("models.dev's data is too large".into()));
        }
        // About 6 MB of JSON: parsed off the async threads.
        tokio::task::spawn_blocking(move || {
            let body = if gzip { gunzip(&body)? } else { body };
            ModelsDev::parse(&body)
        })
        .await
        .map_err(|e| AiError::Protocol(format!("reading models.dev's data failed: {e}")))?
    })
    .await
    .unwrap_or_else(|_| Err(AiError::Network("the request timed out".into())))
}

fn gunzip(body: &[u8]) -> Result<Vec<u8>, AiError> {
    let mut data = Vec::new();
    flate2::read::GzDecoder::new(body)
        .take(MAX_DATA as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|_| AiError::Protocol("models.dev's data is not valid gzip".into()))?;
    if data.len() > MAX_DATA {
        return Err(AiError::Protocol("models.dev's data is too large".into()));
    }
    Ok(data)
}

/// The id as the catalog keys it: lowercase, without the vendor or path before the last `/`
/// (`anthropic/…`, `models/…`, `Qwen/…`), a Claude id from `claude-` on (Bedrock's
/// `us.anthropic.claude-…`), and with a Claude version's dots as dashes (OpenRouter's
/// `claude-opus-4.8`).
fn normalize(id: &str) -> String {
    let id = id.trim().to_ascii_lowercase();
    let id = id.rsplit('/').next().unwrap_or_default();
    match id.find("claude-") {
        Some(at) => id[at..].replace('.', "-"),
        None => id.to_owned(),
    }
}

/// Whether the normalized `id` is `key`, or `key` and a suffix: a date (`-20250929`,
/// `-2024-08-06`, `@20251101`), a variant (`-thinking`, `-preview`, `:free`, `[1m]`, `-v1:0`) or a
/// size (`-120b`, `:20b`). A short number after a dash continues the version instead
/// (`claude-opus-5` is not `claude-opus-5-5`), as does a dot (`gpt-5` is not `gpt-5.1`), and
/// `-chat` names another model (`gpt-5-chat-latest` is not `gpt-5`).
fn matches(id: &str, key: &str) -> bool {
    let Some(rest) = id.strip_prefix(key) else {
        return false;
    };
    match rest.chars().next() {
        None | Some(':' | '@' | '[') => true,
        Some('-') => {
            let rest = &rest[1..];
            if rest.starts_with("chat") {
                return false;
            }
            let digits = rest.chars().take_while(char::is_ascii_digit).count();
            let version = (1..=2).contains(&digits)
                && !rest[digits..].starts_with(|c: char| c.is_ascii_alphanumeric());
            !version
        }
        _ => false,
    }
}

/// Fills what is unknown of a model from models.dev: what the list or the user said stays.
fn fill(
    known: KnownModel,
    context_window: &mut Option<u64>,
    max_output_tokens: &mut Option<u64>,
    efforts: &mut Option<Vec<Effort>>,
    adaptive_thinking: &mut Option<bool>,
) {
    if context_window.is_none() {
        *context_window = known.context_window;
    }
    if max_output_tokens.is_none() {
        *max_output_tokens = known.max_output_tokens;
    }
    if efforts.is_none() {
        *efforts = known.efforts;
    }
    if adaptive_thinking.is_none() {
        *adaptive_thinking = known.adaptive_thinking;
    }
}

impl ModelInfo {
    /// Fills the context window, output limit, thinking levels and adaptive thinking the list
    /// left out from models.dev ([`known_model`]), for a provider of `protocol`.
    pub fn fill_known(&mut self, protocol: Protocol) {
        if let Some(known) = known_model(&self.id, protocol) {
            fill(
                known,
                &mut self.context_window,
                &mut self.max_output_tokens,
                &mut self.efforts,
                &mut self.adaptive_thinking,
            );
        }
    }
}

impl ModelSpec {
    /// Fills what is unknown of the model from models.dev ([`known_model`]), for a provider of
    /// `protocol`: what the model list or the user set stays.
    pub fn fill_known(&mut self, protocol: Protocol) {
        if let Some(known) = known_model(&self.id, protocol) {
            fill(
                known,
                &mut self.context_window,
                &mut self.max_output_tokens,
                &mut self.efforts,
                &mut self.adaptive_thinking,
            );
        }
    }
}
