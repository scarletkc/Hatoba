use std::fmt;

use serde::{Deserialize, Serialize};
use specta::Type;
use zeroize::Zeroize;

// ───────────────────────── AI assistant (§13) ─────────────────────────
//
// API keys travel one way, WebView → Rust: views say only `has_api_key`, and the inputs that
// carry a key print it redacted and wipe it when dropped. Conversation content (messages, tool
// inputs and results) is never logged (SEC-04), so the inputs that carry it print only lengths.

/// A provider's protocol (spec §13.2). A `system_one` provider answers decision questions for
/// the judge of Auto mode (AI-47) and cannot chat.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiProtocol {
    ChatCompletions,
    Anthropic,
    Responses,
    SystemOne,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
pub enum AiAuthHeader {
    #[serde(rename = "x-api-key")]
    XApiKey,
    #[serde(rename = "authorization")]
    Authorization,
}

/// A thinking level (AI-05), lowest first. Where one is optional, `None` is Default.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AiEffort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

/// A model of a provider (AI-03). Token limits are `None` when unknown. A provider's view fills
/// what a row leaves unknown from models.dev, as requests do.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct AiModel {
    pub id: String,
    pub name: String,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    /// The thinking levels the model accepts, lowest first (AI-05); empty for none, `None` when
    /// unknown (the panel then offers Low to High).
    pub efforts: Option<Vec<AiEffort>>,
    /// Whether the model supports adaptive thinking (Anthropic's model list or models.dev),
    /// `None` when unknown.
    pub adaptive_thinking: Option<bool>,
}

/// AI-01. The API key never reaches the WebView: only whether one is saved.
#[derive(Debug, Clone, Serialize, Type)]
pub struct AiProviderView {
    pub id: String,
    pub name: String,
    pub protocol: AiProtocol,
    pub base_url: String,
    pub has_api_key: bool,
    /// Only used by `anthropic`.
    pub auth_header: AiAuthHeader,
    pub models: Vec<AiModel>,
    /// Set for a provider that signs in with a ChatGPT account instead of a key (AI-52).
    pub chatgpt: Option<AiChatgptView>,
    pub updated_at: i64,
}

/// AI-52: where a ChatGPT provider's sign-in stands on this device. Its tokens never reach the
/// WebView.
#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiChatgptStatus {
    /// Signed in, with plan use allowed.
    SignedIn,
    /// Signed in, but plan use was not allowed.
    PlanDisabled,
    /// The session ended (expired, revoked, or signed out in ChatGPT): sign in again.
    Ended,
    /// Not signed in on this device (signed out here, or signed in on another device only).
    SignedOut,
}

/// AI-52: the ChatGPT account of a provider.
#[derive(Debug, Clone, Serialize, Type)]
pub struct AiChatgptView {
    pub email: String,
    pub status: AiChatgptStatus,
}

/// AI-52: a finished sign-in.
#[derive(Debug, Clone, Serialize, Type)]
pub struct AiChatgptSignIn {
    pub provider: AiProviderView,
    /// The sign-in registered Hatoba for a new account, so the panel says once that requests
    /// now use the ChatGPT plan.
    pub first_sign_in: bool,
}

#[derive(Clone, Deserialize, Type)]
pub struct AiProviderInput {
    /// `None` creates a provider. With an id, a `None` key keeps (or tests with) the saved one.
    pub id: Option<String>,
    pub name: String,
    pub protocol: AiProtocol,
    pub base_url: String,
    /// `None` keeps the saved key (as HOST-08 does for passwords); `""` clears it.
    pub api_key: Option<String>,
    pub auth_header: AiAuthHeader,
    pub models: Vec<AiModel>,
}

impl fmt::Debug for AiProviderInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiProviderInput")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("protocol", &self.protocol)
            .field("base_url", &self.base_url)
            .field("api_key", &redacted(self.api_key.as_deref()))
            .field("auth_header", &self.auth_header)
            .field("models", &self.models)
            .finish()
    }
}

impl Drop for AiProviderInput {
    fn drop(&mut self) {
        self.api_key.zeroize();
    }
}

/// AI-04 Test Connection, also used for the search provider test.
#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiTestFailure {
    Auth,
    Network,
    UnknownModel,
    InvalidUrl,
    Other,
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiTestResult {
    pub ok: bool,
    pub failure: Option<AiTestFailure>,
    pub status: Option<u16>,
    /// The provider's own message, when it sent one.
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    Brave,
    Tavily,
    Searxng,
}

/// The backend of `web_search` (AI-14).
#[derive(Debug, Clone, Serialize, Type)]
pub struct SearchProviderView {
    pub id: String,
    pub kind: SearchKind,
    /// The instance URL for SearXNG, `None` otherwise.
    pub base_url: Option<String>,
    pub has_api_key: bool,
    pub updated_at: i64,
}

#[derive(Clone, Deserialize, Type)]
pub struct SearchProviderInput {
    pub id: Option<String>,
    pub kind: SearchKind,
    pub base_url: Option<String>,
    /// `None` keeps the saved key; `""` clears it.
    pub api_key: Option<String>,
}

impl fmt::Debug for SearchProviderInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SearchProviderInput")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("base_url", &self.base_url)
            .field("api_key", &redacted(self.api_key.as_deref()))
            .finish()
    }
}

impl Drop for SearchProviderInput {
    fn drop(&mut self) {
        self.api_key.zeroize();
    }
}

/// How an optional secret prints: whether it is there, never what it is.
pub(super) fn redacted(secret: Option<&str>) -> &'static str {
    match secret {
        None => "<keep>",
        Some("") => "<empty>",
        Some(_) => "<redacted>",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct AiModelRef {
    pub provider_id: String,
    pub model_id: String,
}

/// The synced `Settings.ai` (§5.1).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiSettingsView {
    pub default_model: Option<AiModelRef>,
    /// The thinking level new conversations start with (AI-05); `None` is Default.
    pub default_effort: Option<AiEffort>,
    pub search_provider_id: Option<String>,
    /// While no search provider is chosen, `web_search` uses the chat provider's own search
    /// (AI-14).
    pub native_search: bool,
    /// The built-in `hatoba` skill is offered (AI-34).
    pub builtin_skill_enabled: bool,
    /// Sent with every request, at most 32 KB of UTF-8 (AI-36).
    pub custom_instructions: String,
    /// Messages go with the attached terminal's screen unless the panel's + menu turns it off for
    /// the conversation (AI-46).
    pub send_terminal_screen: bool,
    /// The judge of Auto mode (AI-47): a model of a System One provider or of a provider that
    /// chats. Auto is unavailable until one is chosen.
    pub judge_model: Option<AiModelRef>,
    /// Conversations start with the memories, and the assistant saves them (AI-53).
    pub memory: bool,
    /// The AI panel's conversation and input text size in px, 11 to 24 (TERM-13).
    pub text_size: u32,
}

/// What `ai_settings_save` changes in `Settings.ai`, like [`SettingsPatch`]: a field left out keeps
/// its stored value. `null` clears the default model, the search provider and the judge, and sets
/// the default thinking level to Default.
#[derive(Debug, Clone, Default, Deserialize, Type)]
#[serde(default)]
pub struct AiSettingsPatch {
    #[specta(type = Option<AiModelRef>)]
    pub default_model: Clearable<AiModelRef>,
    #[specta(type = Option<AiEffort>)]
    pub default_effort: Clearable<AiEffort>,
    #[specta(type = Option<String>)]
    pub search_provider_id: Clearable<String>,
    pub native_search: Option<bool>,
    pub builtin_skill_enabled: Option<bool>,
    pub custom_instructions: Option<String>,
    pub send_terminal_screen: Option<bool>,
    #[specta(type = Option<AiModelRef>)]
    pub judge_model: Clearable<AiModelRef>,
    pub memory: Option<bool>,
    /// Kept within 11 to 24.
    pub text_size: Option<u32>,
}

/// A patch field that `null` clears.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Clearable<T> {
    /// Left out: the stored value stays.
    #[default]
    Keep,
    Clear,
    Set(T),
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Clearable<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Option::<T>::deserialize(deserializer)?.map_or(Self::Clear, Self::Set))
    }
}

/// A conversation's permission mode on this device (AI-16, spec §13.5), the strictest first.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiPermissionMode {
    Manual,
    /// AI-47: a call that manual approval would ask about goes to the judge first
    /// (`ai_judge`), which runs it without the approval card or lets the card ask.
    Auto,
    Bypass,
}

/// What the judge of Auto mode decided about a call (AI-47). It never rejects a call.
#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiJudgeDecision {
    /// The call runs without the approval card.
    Run,
    /// The approval card asks, as in manual approval.
    Ask,
}

/// The judge's answer to one of its questions about a call (AI-47).
#[derive(Debug, Clone, Serialize, Type, PartialEq)]
pub struct AiJudgeAnswer {
    /// The question's id, such as `requested` or `destroys_data`.
    pub question: String,
    /// The probability of yes, from 0 to 1. Always finite, so never `null` in JSON, which the
    /// generated type of a float would allow.
    #[specta(type = specta_typescript::Number)]
    pub probability: f64,
}

/// The verdict of the judge of Auto mode on a call (AI-47): from `ai_judge`, and stored with the
/// result of a call Auto decided, so history shows why it ran without the card or what concerned
/// the judge. It never reaches the model.
#[derive(Debug, Clone, Serialize, Type, PartialEq)]
pub struct AiAutoVerdict {
    /// The provider item id of the judge; empty when no judge was chosen.
    pub provider_id: String,
    /// The judge's model id; empty when no judge was chosen.
    pub model_id: String,
    pub decision: AiJudgeDecision,
    /// The answers in the order the judge asks its questions, then any others by id; empty when
    /// the judge failed.
    pub answers: Vec<AiJudgeAnswer>,
    /// The ids of the questions whose answers kept the call from running, in the same order.
    pub concerns: Vec<String>,
    /// Why the judge gave no usable answer: `unavailable` (no judge, or its provider or model is
    /// gone), `too_long`, `timeout`, `unreachable`, `refused`, `malformed` or `cancelled`, or a
    /// kind a newer version added. Then the decision is to ask.
    pub failure: Option<String>,
}

/// The judge's verdict on one sample call of **Test** beside the judge in Settings → AI
/// (`ai_judge_test`, AI-47).
#[derive(Debug, Clone, Serialize, Type, PartialEq)]
pub struct AiJudgeSampleOutcome {
    /// The sample's stable name, such as `restart_requested`; a newer version may add others.
    pub name: String,
    /// The decision a good judge gives the sample.
    pub expected: AiJudgeDecision,
    /// What the judge decided, with its answers in the order it asks its questions.
    pub verdict: AiAutoVerdict,
}

/// What an AI approval saved on this device lets run without asking (AI-19).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiAllowanceKind {
    /// `run_command` calls whose command starts with `value`, on one host.
    Command,
    /// Every call of the tool `value` (`edit_file`, `write_file` or `send_input`), on one host.
    Tool,
    /// `fetch_url` calls of pages on the host name `value`, on any tab.
    Domain,
}

/// An AI approval saved on this device (AI-19): never synced, kept until the user removes it.
/// A command or tool approval names its host by `host_id`, or by `target` for a quick-connect tab
/// or this computer's local tabs.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct AiAllowance {
    pub kind: AiAllowanceKind,
    /// The saved host the approval is for.
    pub host_id: Option<String>,
    /// The quick-connect target (`user@host:port`) the approval is for, without a saved host, or
    /// `local` for this computer's local tabs (§7.5). Approvals stay on the device, so `local`
    /// always means the computer that keeps them.
    pub target: Option<String>,
    /// The command prefix, the tool's name, or the domain.
    pub value: String,
}
