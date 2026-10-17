//! Well-known models (AI-03): the context window, output limit, thinking levels and adaptive
//! thinking of models whose provider's list does not say, as gateways such as New API and
//! OneAPI list only ids. Matched by id, without a vendor prefix (`anthropic/claude-opus-4.8`,
//! `us.anthropic.claude-opus-4-8-v1:0`) and with suffixes such as a date, `-thinking`, `:free` or
//! `[1m]` ignored, but never across versions: `claude-opus-5-7` and `gpt-5.9` are not in the
//! table until they are added.
//!
//! The values are the vendors' own (Anthropic's models overview and Models API, OpenAI's model
//! pages, Google's Gemini models and OpenAI compatibility pages, DeepSeek's and Alibaba Cloud
//! Model Studio's model lists). The thinking levels are those of the vendor's own request format,
//! so they apply only on that protocol: Anthropic's levels on `anthropic`, DeepSeek's (the same in
//! both of its formats) on any, the others on Chat Completions and Responses. Elsewhere a gateway
//! translates the levels in its own way, so they stay unknown.

use crate::provider::Effort::{self, High, Low, Max, Medium, Xhigh};
use crate::provider::{ModelSpec, Protocol};

use super::ModelInfo;

/// What the table says about a model on a provider's protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KnownModel {
    /// Context window in tokens.
    pub context_window: u64,
    /// Output limit in tokens.
    pub max_output_tokens: u64,
    /// The thinking levels the model accepts on this protocol, lowest first; `None` when they
    /// are not known for it.
    pub efforts: Option<&'static [Effort]>,
    /// Whether the model supports adaptive thinking (Claude models on `anthropic`).
    pub adaptive_thinking: Option<bool>,
}

/// The request format whose thinking levels an entry lists.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    /// Anthropic Messages: `output_config.effort`.
    Anthropic,
    /// OpenAI's Chat Completions and Responses: `reasoning_effort`, `reasoning.effort`.
    OpenAi,
    /// Both, with the same levels (DeepSeek's API).
    Any,
}

struct Entry {
    /// The normalized id, alone or followed by a suffix ([`matches`]).
    key: &'static str,
    context_window: u64,
    max_output_tokens: u64,
    efforts: &'static [Effort],
    adaptive_thinking: Option<bool>,
    format: Format,
}

const NONE: &[Effort] = &[];
const TO_HIGH: &[Effort] = &[Low, Medium, High];
const TO_XHIGH: &[Effort] = &[Low, Medium, High, Xhigh];
const NO_XHIGH: &[Effort] = &[Low, Medium, High, Max];
/// OpenAI's Pro models.
const PRO: &[Effort] = &[Medium, High, Xhigh];
const DEEPSEEK: &[Effort] = &[Low, High, Max];
const QWEN: &[Effort] = &[Low, Medium, Xhigh];
const ALL: &[Effort] = &Effort::ALL;

const fn entry(
    format: Format,
    key: &'static str,
    context_window: u64,
    max_output_tokens: u64,
    efforts: &'static [Effort],
    adaptive_thinking: Option<bool>,
) -> Entry {
    Entry {
        key,
        context_window,
        max_output_tokens,
        efforts,
        adaptive_thinking,
        format,
    }
}

/// A Claude model, whose levels are in Anthropic's format.
const fn claude(
    key: &'static str,
    context: u64,
    output: u64,
    efforts: &'static [Effort],
    adaptive: bool,
) -> Entry {
    entry(
        Format::Anthropic,
        key,
        context,
        output,
        efforts,
        Some(adaptive),
    )
}

/// A model whose levels are in the format of Chat Completions and Responses.
const fn chat(key: &'static str, context: u64, output: u64, efforts: &'static [Effort]) -> Entry {
    entry(Format::OpenAi, key, context, output, efforts, None)
}

/// A model that takes the same levels in both formats.
const fn any(key: &'static str, context: u64, output: u64, efforts: &'static [Effort]) -> Entry {
    entry(Format::Any, key, context, output, efforts, None)
}

const K: u64 = 1_000;
const M: u64 = 1_000_000;
/// Gemini's input limit.
const GEMINI_CONTEXT: u64 = 1_048_576;

#[rustfmt::skip]
const MODELS: &[Entry] = &[
    // Anthropic. 4.6 and later think adaptively; Opus 4.5 takes an effort without it.
    claude("claude-fable-5-1", M, 128 * K, ALL, true),
    claude("claude-fable-5", M, 128 * K, ALL, true),
    claude("claude-mythos-5-1", M, 128 * K, ALL, true),
    claude("claude-mythos-5", M, 128 * K, ALL, true),
    claude("claude-opus-5-5", M, 128 * K, ALL, true),
    claude("claude-opus-5", M, 128 * K, ALL, true),
    claude("claude-sonnet-5-5", M, 128 * K, ALL, true),
    claude("claude-sonnet-5", M, 128 * K, ALL, true),
    claude("claude-haiku-5-5", M, 128 * K, ALL, true),
    claude("claude-opus-4-8", M, 128 * K, ALL, true),
    claude("claude-opus-4-7", M, 128 * K, ALL, true),
    claude("claude-opus-4-6", M, 128 * K, NO_XHIGH, true),
    claude("claude-sonnet-4-6", M, 128 * K, NO_XHIGH, true),
    claude("claude-opus-4-5", 200 * K, 64 * K, TO_HIGH, false),
    claude("claude-sonnet-4-5", 200 * K, 64 * K, NONE, false),
    claude("claude-haiku-4-5", 200 * K, 64 * K, NONE, false),
    claude("claude-opus-4-1", 200 * K, 32 * K, NONE, false),
    claude("claude-opus-4-0", 200 * K, 32 * K, NONE, false),
    claude("claude-opus-4", 200 * K, 32 * K, NONE, false),
    claude("claude-sonnet-4-0", 200 * K, 64 * K, NONE, false),
    claude("claude-sonnet-4", 200 * K, 64 * K, NONE, false),
    // OpenAI. `none` and `minimal` have no level here; Default leaves them to the model.
    chat("gpt-6-astra", 1_050 * K, 128 * K, ALL),
    chat("gpt-6.1-sol", 1_050 * K, 128 * K, ALL),
    chat("gpt-6-sol", 1_050 * K, 128 * K, ALL),
    chat("gpt-6-luna", 1_050 * K, 128 * K, ALL),
    chat("gpt-5.6", 1_050 * K, 128 * K, ALL),
    chat("gpt-5.5", 1_050 * K, 128 * K, TO_XHIGH),
    chat("gpt-5.5-pro", 1_050 * K, 128 * K, PRO),
    chat("gpt-5.4", 1_050 * K, 128 * K, TO_XHIGH),
    chat("gpt-5.4-pro", 1_050 * K, 128 * K, PRO),
    chat("gpt-5.4-mini", 400 * K, 128 * K, TO_XHIGH),
    chat("gpt-5.4-nano", 400 * K, 128 * K, TO_XHIGH),
    chat("gpt-5.3-codex", 400 * K, 128 * K, TO_XHIGH),
    chat("gpt-5.2", 400 * K, 128 * K, TO_XHIGH),
    chat("gpt-5.2-pro", 400 * K, 128 * K, PRO),
    chat("gpt-5.1", 400 * K, 128 * K, TO_HIGH),
    chat("gpt-5", 400 * K, 128 * K, TO_HIGH),
    chat("gpt-5-pro", 400 * K, 272 * K, &[High]),
    // ChatGPT's chat models do not reason.
    chat("gpt-5.3-chat", 128 * K, 16_384, NONE),
    chat("gpt-5.2-chat", 128 * K, 16_384, NONE),
    chat("gpt-5.1-chat", 128 * K, 16_384, NONE),
    chat("gpt-5-chat", 128 * K, 16_384, NONE),
    chat("o3", 200 * K, 100 * K, TO_HIGH),
    chat("o4-mini", 200 * K, 100 * K, TO_HIGH),
    chat("o1", 200 * K, 100 * K, TO_HIGH),
    chat("gpt-oss", 131_072, 131_072, TO_HIGH),
    chat("gpt-4.1", 1_047_576, 32_768, NONE),
    chat("gpt-4o", 128 * K, 16_384, NONE),
    // Google, through its OpenAI compatibility, where `reasoning_effort` takes low to high.
    chat("gemini-4-argon", GEMINI_CONTEXT, 65_536, TO_HIGH),
    chat("gemini-3.8-flash", GEMINI_CONTEXT, 65_536, TO_HIGH),
    chat("gemini-3.7-flash", GEMINI_CONTEXT, 65_536, TO_HIGH),
    chat("gemini-3.6-flash", GEMINI_CONTEXT, 65_536, TO_HIGH),
    chat("gemini-3.5-flash", GEMINI_CONTEXT, 65_536, TO_HIGH),
    chat("gemini-3.5-flash-lite", GEMINI_CONTEXT, 65_536, TO_HIGH),
    chat("gemini-3.1-pro", GEMINI_CONTEXT, 65_536, TO_HIGH),
    chat("gemini-3.1-flash-lite", GEMINI_CONTEXT, 65_536, TO_HIGH),
    chat("gemini-3-pro", GEMINI_CONTEXT, 65_536, TO_HIGH),
    chat("gemini-3-flash", GEMINI_CONTEXT, 65_536, TO_HIGH),
    chat("gemini-2.5-pro", GEMINI_CONTEXT, 65_536, TO_HIGH),
    chat("gemini-2.5-flash", GEMINI_CONTEXT, 65_536, TO_HIGH),
    chat("gemini-2.5-flash-lite", GEMINI_CONTEXT, 65_536, TO_HIGH),
    chat("gemini-pro-latest", GEMINI_CONTEXT, 65_536, TO_HIGH),
    chat("gemini-flash-latest", GEMINI_CONTEXT, 65_536, TO_HIGH),
    chat("gemini-flash-lite-latest", GEMINI_CONTEXT, 65_536, TO_HIGH),
    // DeepSeek takes low, high and max in both of its formats (medium and xhigh are high there).
    any("deepseek-flash", 1_048_576, 393_216, DEEPSEEK),
    any("deepseek-v4", 1_048_576, 393_216, DEEPSEEK),
    any("deepseek-v4.1", 1_048_576, 393_216, DEEPSEEK),
    // Qwen on Alibaba Cloud Model Studio: only the 3.8 models take `reasoning_effort` (low,
    // medium and xhigh); the others turn thinking on and off with `enable_thinking`.
    chat("qwen3.8-max", M, 131_072, QWEN),
    chat("qwen3.8-flash", M, 131_072, QWEN),
    chat("qwen3.8-omni-flash", M, 131_072, QWEN),
    chat("qwen3.8-27b", M, 131_072, QWEN),
    chat("qwen3.8-2.4t-a95b", M, 131_072, QWEN),
    chat("qwen3.7-max", M, 131_072, NONE),
    chat("qwen3.7-plus", M, 131_072, NONE),
    chat("qwen3.7-flash", M, 131_072, NONE),
    chat("qwen3.6-plus", M, 65_536, NONE),
    chat("qwen3.6-flash", M, 65_536, NONE),
    chat("qwen3.6-35b-a3b", 262_144, 65_536, NONE),
    chat("qwen3.5-plus", M, 65_536, NONE),
    chat("qwen3.5-flash", M, 65_536, NONE),
    chat("qwen3.5-397b-a17b", 262_144, 65_536, NONE),
    chat("qwen3.5-122b-a10b", 262_144, 65_536, NONE),
    chat("qwen3.5-35b-a3b", 262_144, 65_536, NONE),
    chat("qwen3.5-27b", 262_144, 65_536, NONE),
    chat("qwen-plus", M, 32_768, NONE),
    chat("qwen-plus-2025-07-14", 131_072, 16_384, NONE),
    chat("qwen-plus-2025-04-28", 131_072, 16_384, NONE),
    chat("qwen-plus-2025-01-25", 131_072, 8_192, NONE),
    chat("qwen-flash", M, 32_768, NONE),
    chat("qwen3-coder-flash", M, 65_536, NONE),
];

/// The id as the table names it: lowercase, without the vendor or path before the last `/`
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
/// (`claude-opus-5` is not `claude-opus-5-5`), as does a dot (`gpt-5` is not `gpt-5.1`).
fn matches(id: &str, key: &str) -> bool {
    let Some(rest) = id.strip_prefix(key) else {
        return false;
    };
    match rest.chars().next() {
        None | Some(':' | '@' | '[') => true,
        Some('-') => {
            let rest = &rest[1..];
            let digits = rest.chars().take_while(char::is_ascii_digit).count();
            let version = (1..=2).contains(&digits)
                && !rest[digits..].starts_with(|c: char| c.is_ascii_alphanumeric());
            !version
        }
        _ => false,
    }
}

/// What the table says about model `id` on `protocol`, by the longest key that matches. `None`
/// for a model the table does not know, and on System One (AI-47), whose models only decide.
#[must_use]
pub fn known_model(id: &str, protocol: Protocol) -> Option<KnownModel> {
    let format = match protocol {
        Protocol::Anthropic => Format::Anthropic,
        Protocol::ChatCompletions | Protocol::Responses => Format::OpenAi,
        Protocol::SystemOne => return None,
    };
    let id = normalize(id);
    let entry = MODELS
        .iter()
        .filter(|e| matches(&id, e.key))
        .max_by_key(|e| e.key.len())?;
    let native = entry.format == format || entry.format == Format::Any;
    Some(KnownModel {
        context_window: entry.context_window,
        max_output_tokens: entry.max_output_tokens,
        efforts: native.then_some(entry.efforts),
        adaptive_thinking: entry.adaptive_thinking.filter(|_| native),
    })
}

/// Fills what is unknown of a model from the table: what the list or the user said stays.
fn fill(
    known: KnownModel,
    context_window: &mut Option<u64>,
    max_output_tokens: &mut Option<u64>,
    efforts: &mut Option<Vec<Effort>>,
    adaptive_thinking: &mut Option<bool>,
) {
    context_window.get_or_insert(known.context_window);
    max_output_tokens.get_or_insert(known.max_output_tokens);
    if efforts.is_none() {
        *efforts = known.efforts.map(<[Effort]>::to_vec);
    }
    if adaptive_thinking.is_none() {
        *adaptive_thinking = known.adaptive_thinking;
    }
}

impl ModelInfo {
    /// Fills the context window, output limit, thinking levels and adaptive thinking the list
    /// left out from the table of well-known models, for a provider of `protocol`.
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
    /// Fills what is unknown of the model from the table of well-known models, for a provider of
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
