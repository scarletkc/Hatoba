//! The AI assistant's data transfer objects (spec §10, §13): what the `ai_*`, skill and MCP
//! commands exchange with the WebView.
//!
//! The desktop app re-exports them from its own `dto` module, and tauri-specta exports them, with
//! their doc comments, to `apps/desktop/src/ipc/bindings.ts`, the frontend's only definition of
//! them. **No DTO carries a secret**; inputs may carry secrets one way (WebView → Rust), and they
//! are never echoed back.

use std::fmt;

use serde::{Deserialize, Serialize};
use specta::Type;
use zeroize::Zeroize;

pub use hatoba_ipc::dto::{LocalShellKind, QuickTarget};

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
fn redacted(secret: Option<&str>) -> &'static str {
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

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiConversationView {
    pub id: String,
    pub title: String,
    /// The host the conversation last worked on.
    pub host_id: Option<String>,
    /// The quick-connect target (HOST-12) it last worked on while `host_id` is `None` (AI-09),
    /// as the conversation records it: the address in lowercase and without brackets.
    pub quick_target: Option<QuickTarget>,
    /// AI-43: with neither, the name of the server a fork's copies came from, when the fork could
    /// not tell which it is; its next message on a tab of a host or target notes the move from it.
    pub origin: Option<String>,
    /// §7.5: with neither a host nor a target, the computer whose local terminal tab it last
    /// worked on, by its name (`AppInfo.device_name` on that computer).
    pub local: Option<String>,
    pub pinned: bool,
    /// `entry_id` where the context sent to the model starts (AI-21).
    pub context_start: Option<String>,
    /// The thinking level its last message was sent with (AI-05); `None` is Default.
    pub effort: Option<AiEffort>,
    pub created_at: i64,
    pub updated_at: i64,
    /// The newest entry or conversation change, for sorting history (AI-23).
    pub last_activity: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiFinish {
    Stop,
    ToolCalls,
    Length,
    Refused,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiToolStatus {
    Ok,
    Error,
    Rejected,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiToolCall {
    pub id: String,
    pub name: String,
    /// JSON text.
    pub arguments: String,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
pub struct AiUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated: bool,
}

/// What `ai_tool_run` stored: the call's result entry.
#[derive(Debug, Clone, Serialize, Type, PartialEq)]
pub struct AiToolRun {
    pub entry: AiEntryView,
    /// AI-41: keeping the snapshot of this call's change dropped older ones over the size limit,
    /// of this or another conversation, so the panel reads the file changes of its other
    /// conversations again.
    pub dropped_file_changes: bool,
}

/// A stored conversation entry (§13.7), without the provider's raw message.
#[derive(Debug, Clone, Serialize, Type, PartialEq)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum AiEntryView {
    User {
        entry_id: String,
        created_at: i64,
        text: String,
    },
    Assistant {
        entry_id: String,
        created_at: i64,
        provider_id: String,
        model_id: String,
        text: String,
        reasoning: Option<String>,
        tool_calls: Vec<AiToolCall>,
        finish: AiFinish,
        usage: Option<AiUsage>,
    },
    Tool {
        entry_id: String,
        created_at: i64,
        tool_call_id: String,
        status: AiToolStatus,
        content: String,
        /// What the judge of Auto mode decided about the call (AI-47), when Auto decided it.
        auto: Option<AiAutoVerdict>,
    },
    Summary {
        entry_id: String,
        created_at: i64,
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct AiConversationDetail {
    pub conversation: AiConversationView,
    pub entries: Vec<AiEntryView>,
    /// A turn of this conversation is running (its events go to the channel that started it).
    pub running: bool,
    /// The changes of its `edit_file` and `write_file` calls that this device can revert, or
    /// has reverted (AI-41).
    pub file_changes: Vec<AiFileChange>,
}

/// A change an `edit_file` or `write_file` call made, kept on this device so the user can revert
/// it (AI-41).
#[derive(Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiFileChange {
    /// The entry id of the response that made the call.
    pub response_id: String,
    pub tool_call_id: String,
    /// The server the call wrote on, as `user@host:port`.
    pub server: String,
    /// The file's path as the call resolved it.
    pub path: String,
    /// The call created the file, so a revert deletes it.
    pub created: bool,
    pub state: AiFileChangeState,
}

/// Where a file tool call's change stands on this device (AI-41).
#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiFileChangeState {
    /// The call wrote the file.
    Written,
    /// The write failed after it may have begun, or its end was never recorded: the file may be
    /// cut short. It can be reverted like a written one.
    Unfinished,
    /// The user reverted the change.
    Reverted,
    /// The 32 MB limit dropped what the file held before, so the change can no longer be
    /// reverted.
    Dropped,
}

impl fmt::Debug for AiFileChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiFileChange")
            .field("response_id", &self.response_id)
            .field("tool_call_id", &self.tool_call_id)
            .field("created", &self.created)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// Why a revert did not happen (AI-41), for the panel to say in the user's language.
#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiRevertFailure {
    /// The conversation's tab is not connected.
    Disconnected,
    /// The tab is connected to another server than the one the call wrote on; `detail` names it.
    OtherServer,
    /// The user the tab is logged in as cannot write or delete the file.
    PermissionDenied,
    /// The file changed since the call wrote it into something that is not a text file Hatoba
    /// can read (a directory, a binary file, a file over 1 MB): there is no diff to show.
    NotText,
    /// A symbolic link is in the place of the file the call created, so neither the link nor its
    /// target was deleted.
    SymbolicLink,
    /// Reading the file did not finish within 30 seconds; nothing was written.
    TimedOut,
    /// Writing or deleting failed after it may have begun, a timeout included: the file may be
    /// cut short, or deleted. `detail` says why, when there was an error.
    Unfinished,
    /// Anything else; `detail` says what.
    Failed,
}

/// The outcome of a revert (AI-41).
#[derive(Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AiFileRevert {
    /// The file holds what it held before the call again, or no longer exists when the call
    /// created it. `wrote`: the revert wrote or deleted it; otherwise it was so already, as after
    /// a revert whose result was lost.
    Reverted { wrote: bool },
    /// The file holds neither what the call wrote nor what it held before, so nothing was
    /// written: the diff from `current` to `before` is what Restore anyway would do. The texts
    /// show CRLF line breaks as `\n`.
    Changed {
        path: String,
        /// What the file holds now; `None` when it does not exist.
        current: Option<String>,
        /// What it held before the call; `None` when the call created it, so Restore anyway
        /// deletes it.
        before: Option<String>,
        /// A later call of this conversation wrote the file too, so reverting the later changes
        /// first, newest first, may bring it back to what this call wrote.
        later: bool,
    },
    Failed {
        reason: AiRevertFailure,
        detail: Option<String>,
    },
}

impl fmt::Debug for AiFileRevert {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reverted { wrote } => f.debug_struct("Reverted").field("wrote", wrote).finish(),
            Self::Changed { current, later, .. } => f
                .debug_struct("Changed")
                .field("exists", &current.is_some())
                .field("later", later)
                .finish_non_exhaustive(),
            Self::Failed { reason, .. } => f
                .debug_struct("Failed")
                .field("reason", reason)
                .finish_non_exhaustive(),
        }
    }
}

/// What a request is made with. The frontend owns the tab, so it says what the turn may act on.
#[derive(Debug, Clone, Deserialize, Type, PartialEq, Eq)]
pub struct AiTurnContext {
    pub provider_id: String,
    pub model_id: String,
    /// The thinking level (AI-05); `None` is Default. Requests send the highest level the model
    /// offers that is not above it, and the conversation keeps it for its next message.
    pub effort: Option<AiEffort>,
    /// The tab's host; the next message moves the conversation to it (AI-09).
    pub host_id: Option<String>,
    /// The tab's quick-connect target when it has no saved host (HOST-12): the next message
    /// moves the conversation off its saved host, and the request names the target instead.
    pub target: Option<QuickTarget>,
    /// A local terminal tab's computer and shell (§7.5), or on the home tab the computer a
    /// conversation recorded, while there is neither a host nor a target. The next message moves
    /// the conversation to that computer.
    pub local: Option<AiLocalTab>,
    /// A connected terminal tab is attached; `false` offers no terminal tools (AI-09).
    pub tab: bool,
    /// `host_id` names a Telnet host, and the tab is its Telnet session (TELNET-04): the
    /// terminal tools are only `read_terminal` and `send_input`.
    #[serde(default)]
    pub telnet: bool,
    /// The tab's SSH session while it is connected, whose server's identification string the
    /// system prompt states (§13.1).
    pub session_id: Option<String>,
    /// MCP servers switched off for this conversation (AI-30, P2).
    pub disabled_mcp_servers: Vec<String>,
    /// The conversation's permission mode on this device (AI-16). The frontend applies it to each
    /// call; a message sent in another mode than the model was last told of starts with a
    /// `permission_mode` note (§13.5).
    pub permission_mode: AiPermissionMode,
    /// The attached terminal's screen as the panel read it for this message (AI-46), while the
    /// tab is connected and **Send Terminal Screen** is on for the conversation. Only a stored
    /// message (and an idle side question) shows it, in a `terminal_state` note.
    pub screen: Option<AiScreen>,
}

/// A local terminal tab (§7.5) in a turn's context.
#[derive(Debug, Clone, Deserialize, Type, PartialEq, Eq)]
pub struct AiLocalTab {
    /// The computer's name: this one's (`AppInfo.device_name`) for a local tab, or the one a home
    /// tab conversation recorded.
    pub device: String,
    /// The tab's shell, for a local tab: its name as the shell picker shows it in English, and
    /// its kind, which decides how the tools run and what the system prompt says of them.
    pub shell: Option<String>,
    pub kind: Option<LocalShellKind>,
}

/// What a `terminal_state` note shows (AI-46, §13.3). Its `Debug` leaves the text out, like
/// every log (§13.1).
#[derive(Clone, Deserialize, Type, PartialEq, Eq)]
pub struct AiScreen {
    /// The screen's last lines, or the whole screen while `alternate`.
    pub text: String,
    /// A full-screen program (vim, less, …) uses the alternate screen.
    pub alternate: bool,
}

impl fmt::Debug for AiScreen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiScreen")
            .field(
                "text",
                &format_args!("<{} chars>", self.text.chars().count()),
            )
            .field("alternate", &self.alternate)
            .finish()
    }
}

#[derive(Clone, Deserialize, Type)]
pub struct AiSendInput {
    /// `None` starts a new conversation, stored with this first message (AI-07).
    pub conversation_id: Option<String>,
    pub text: String,
    pub context: AiTurnContext,
}

impl fmt::Debug for AiSendInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiSendInput")
            .field("conversation_id", &self.conversation_id)
            .field(
                "text",
                &format_args!("<{} chars>", self.text.chars().count()),
            )
            .field("context", &self.context)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct AiSendStarted {
    pub conversation: AiConversationView,
    pub user_entry: AiEntryView,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiTurnEndReason {
    Completed,
    Length,
    Refused,
    /// The response had neither text nor tool calls, such as a reply of reasoning only (§13.1).
    Empty,
    Stopped,
    Error,
}

/// Streamed on the channel of `ai_send` / `ai_retry` for the whole turn (§13.1).
#[derive(Debug, Clone, Serialize, Type, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AiTurnEvent {
    /// The turn of a new conversation is registered under this id, which the conversation is
    /// stored under with its message. Sent before the message waits for the MCP servers (AI-45),
    /// so a stop can name the turn before `ai_send` returns.
    Opened {
        conversation_id: String,
    },
    RequestStarted,
    Text {
        delta: String,
    },
    Reasoning {
        delta: String,
    },
    /// The model started thinking, also when no reasoning text comes with it (AI-06).
    Thinking,
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        estimated: bool,
    },
    /// An entry was stored: the assistant response, a tool result, or a cancelled result.
    Entry {
        entry: AiEntryView,
    },
    /// One response finished. With `tool_calls`, Rust waits for every call's result.
    Done {
        finish: AiFinish,
    },
    /// The tool of an `ai_tool_run` call finished, and closed its exec channel if it had one; its
    /// result is stored once every earlier call of the response has one (§13.1). The panel counts
    /// the channel as free from now on.
    ToolRan {
        tool_call_id: String,
    },
    Error {
        status: Option<u16>,
        message: String,
    },
    /// The provider refused the thinking level, so the request was sent again without it and the
    /// model answers at its default depth (AI-05).
    EffortIgnored,
    TurnEnded {
        reason: AiTurnEndReason,
    },
}

/// AI-42: a side question about a conversation, asked without stopping its turn.
#[derive(Clone, Deserialize, Type)]
pub struct AiSideQuestionInput {
    pub conversation_id: String,
    /// The panel's number for the question, which `ai_side_stop` names, so that a stop meant for
    /// it never stops a newer one.
    pub question_id: u32,
    /// The question, after the attachment blocks the input area adds (spec §13.3).
    pub text: String,
    /// What the next message would be sent with; a running turn's own context wins.
    pub context: AiTurnContext,
    /// The call whose approval card waits (AI-17), so its result in the request says so.
    pub approval_call_id: Option<String>,
}

impl fmt::Debug for AiSideQuestionInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiSideQuestionInput")
            .field("conversation_id", &self.conversation_id)
            .field("question_id", &self.question_id)
            .field(
                "text",
                &format_args!("<{} chars>", self.text.chars().count()),
            )
            .field("context", &self.context)
            .field("approval_call_id", &self.approval_call_id)
            .finish()
    }
}

/// Streamed on the channel of `ai_side_question` while the answer comes (AI-42).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AiSideEvent {
    Reasoning {
        delta: String,
    },
    Text {
        delta: String,
    },
    /// The answer called tools, so it was dropped and the question went again without tools:
    /// what streamed before is discarded.
    Restarted,
}

/// The answer to a side question (AI-42), which nothing stores.
#[derive(Debug, Clone, Serialize, Type)]
pub struct AiSideAnswer {
    pub text: String,
    pub finish: AiFinish,
}

/// A result the frontend produced: `read_terminal`, `send_input`, or a rejection (AI-17).
#[derive(Clone, Deserialize, Type)]
pub struct AiToolResultInput {
    pub status: AiToolStatus,
    pub content: String,
    /// AI-17 Edit: the arguments the user changed the call to, so the result tells the model.
    pub edited_arguments: Option<String>,
    /// AI-47: Auto decided the call (it ran on the judge's verdict, or its approval card showed
    /// one), so the result keeps the verdict.
    #[serde(default)]
    pub judged: bool,
}

impl fmt::Debug for AiToolResultInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiToolResultInput")
            .field("status", &self.status)
            .field(
                "content",
                &format_args!("<{} chars>", self.content.chars().count()),
            )
            .field("edited", &self.edited_arguments.is_some())
            .field("judged", &self.judged)
            .finish()
    }
}

/// The change an `edit_file` or `write_file` call would make, for its approval card (AI-39,
/// AI-40). The texts show CRLF line breaks as `\n`; `crlf` says the write keeps them as CRLF.
#[derive(Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiFilePreview {
    /// The path the call names.
    pub path: String,
    /// The file as Hatoba read it, or `None` when it does not exist (the call creates it) or
    /// `error` is set.
    pub before: Option<String>,
    /// The file after the call, or `None` when `error` is set.
    pub after: Option<String>,
    /// Why the call cannot apply: the error result running it would give the model.
    pub error: Option<String>,
    /// Most of the file's line breaks are CRLF, and the write keeps them.
    pub crlf: bool,
}

impl fmt::Debug for AiFilePreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let size = |t: &Option<String>| t.as_ref().map(|t| t.chars().count());
        f.debug_struct("AiFilePreview")
            .field("before_chars", &size(&self.before))
            .field("after_chars", &size(&self.after))
            .field("error", &self.error.is_some())
            .field("crlf", &self.crlf)
            .finish_non_exhaustive()
    }
}

// ───────────────────────── AI assistant: skills (§13.8) ─────────────────────────
//
// Skill text reaches the model as instructions; it is never logged (SEC-04), so the types that
// carry it print only sizes.

#[derive(Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct SkillFileView {
    /// Relative path with forward slashes, such as `references/nginx.md`. Never `SKILL.md`.
    pub path: String,
    pub content: String,
}

impl fmt::Debug for SkillFileView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SkillFileView")
            .field("path", &self.path)
            .field("content", &format_args!("<{} bytes>", self.content.len()))
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct SkillView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    /// Paths of the files besides `SKILL.md`, sorted.
    pub files: Vec<String>,
    pub updated_at: i64,
}

/// A memory of the AI assistant (AI-53): a global one, with neither `host_id` nor `local`, or a
/// host memory of a saved host or of a computer's local terminal.
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiMemoryView {
    pub id: String,
    pub name: String,
    pub content: String,
    pub host_id: Option<String>,
    /// The computer's name, while `host_id` is `None`.
    pub local: Option<String>,
    pub updated_at: i64,
}

/// A memory to save from Settings → AI (AI-53), in the scope `host_id` and `local` name, as in
/// [`AiMemoryView`].
#[derive(Debug, Clone, Deserialize, Type)]
pub struct AiMemoryInput {
    /// `None` creates a memory.
    pub id: Option<String>,
    pub name: String,
    pub content: String,
    pub host_id: Option<String>,
    pub local: Option<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SkillDetail {
    pub skill: SkillView,
    /// `SKILL.md` without its frontmatter.
    pub body: String,
    pub files: Vec<SkillFileView>,
    /// Other frontmatter fields, kept for export. They change nothing (AI-27).
    pub frontmatter_keys: Vec<String>,
}

/// The built-in `hatoba` skill (AI-34), read-only.
#[derive(Debug, Clone, Serialize, Type)]
pub struct BuiltinSkillView {
    pub name: String,
    pub description: String,
    /// `Settings.ai.builtin_skill_enabled`.
    pub enabled: bool,
    /// `SKILL.md` without its frontmatter, with the app's version filled in.
    pub body: String,
    /// The files besides `SKILL.md`, sorted by path.
    pub files: Vec<SkillFileView>,
}

#[derive(Clone, Deserialize, Type)]
pub struct SkillInput {
    /// `None` creates a skill.
    pub id: Option<String>,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub body: String,
    /// Every file besides `SKILL.md`; a saved file left out is deleted.
    pub files: Vec<SkillFileView>,
}

impl fmt::Debug for SkillInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SkillInput")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("enabled", &self.enabled)
            .field("body", &format_args!("<{} bytes>", self.body.len()))
            .field("files", &self.files)
            .finish()
    }
}

/// Why a skill cannot be imported or saved (AI-27).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SkillIssue {
    MissingSkillMd,
    InvalidFrontmatter {
        detail: String,
    },
    InvalidName {
        name: String,
    },
    MissingDescription,
    DescriptionTooLong {
        chars: u64,
    },
    FileTooLarge {
        path: String,
        size: u64,
    },
    UnsafePath {
        path: String,
    },
    TooManyFiles {
        count: u64,
    },
    /// More than 5 MB in total.
    TooLarge {
        bytes: u64,
    },
}

/// Why a file dropped on the AI panel is not attached (AI-35): the panel's own reasons.
#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DroppedFileRefusal {
    /// An image, which attachments do not take yet.
    Image,
    /// Over 256 KB.
    TooLarge,
    /// NUL bytes or invalid UTF-8.
    Binary,
    /// Not a file, gone, or not readable.
    Unreadable,
}

/// A text file dropped on the AI panel (AI-35), named by its base name only, never its path (a
/// path can name the local user).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DroppedFile {
    Ok {
        name: String,
        text: String,
    },
    Refused {
        name: String,
        reason: DroppedFileRefusal,
    },
}

/// One entry of a directory on the tab's host, for the panel's `@` menu (AI-51).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiHostEntry {
    pub name: String,
    /// A directory, or a symbolic link to one.
    pub dir: bool,
}

/// Why a path on the tab's host cannot be listed or attached (AI-51).
#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiHostRefusal {
    /// The tab is not connected.
    Disconnected,
    NotFound,
    /// Read: a directory. List: not a directory.
    WrongKind,
    /// A device, FIFO, socket or a symbolic link to nothing.
    NotRegular,
    PermissionDenied,
    /// Over 256 KB.
    TooLarge,
    /// NUL bytes or invalid UTF-8.
    Binary,
    /// The command failed or did not finish within 30 seconds.
    Failed,
}

/// A directory on the tab's host (AI-51): its entries, or why it cannot be listed.
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AiHostListing {
    Ok {
        /// The path as the host resolved it: absolute, with `~/` and a relative path taken from
        /// the home directory.
        path: String,
        /// Sorted by name.
        entries: Vec<AiHostEntry>,
        /// Entries were left out: the listing passed its size limit.
        cut: bool,
    },
    Refused {
        reason: AiHostRefusal,
    },
}

/// A text file on the tab's host for an `@` mention (AI-51), or why it cannot be attached.
#[derive(Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AiHostFile {
    Ok {
        /// The path as the host resolved it.
        path: String,
        text: String,
    },
    Refused {
        reason: AiHostRefusal,
    },
}

impl fmt::Debug for AiHostFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok { text, .. } => f
                .debug_struct("Ok")
                .field("text", &format_args!("<{} bytes>", text.len()))
                .finish_non_exhaustive(),
            Self::Refused { reason } => f.debug_struct("Refused").field("reason", reason).finish(),
        }
    }
}

/// An enabled skill the panel's `/` menu offers as a command (AI-50), the built-in one included.
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiSkillCommand {
    pub name: String,
    pub description: String,
}

/// What an import would save, shown before saving (AI-27). Importable when `issues` is empty.
#[derive(Debug, Clone, Serialize, Type)]
pub struct SkillImportPreview {
    /// `None` when `SKILL.md` is missing or unreadable.
    pub name: Option<String>,
    pub description: Option<String>,
    pub body: Option<String>,
    pub files: Vec<SkillFileView>,
    pub frontmatter_keys: Vec<String>,
    /// Files that are not UTF-8 text, which the import skips.
    pub skipped: Vec<String>,
    pub issues: Vec<SkillIssue>,
    /// A saved skill with the same name, which the user may replace (or rename the new one).
    pub existing_id: Option<String>,
    /// The name is the built-in skill's (AI-34): the skill can be imported only under another.
    pub reserved_name: bool,
}

// ───────────────────────── AI assistant: MCP servers (§13.9) ─────────────────────────
//
// Environment and header values travel one way, WebView → Rust, like API keys: views carry
// only their names, and the inputs that carry them print them redacted and wipe them when
// dropped (AI-29).

/// Environment and header values never reach the WebView, only their names (AI-29).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpTransportView {
    Stdio {
        command: String,
        args: Vec<String>,
        env_keys: Vec<String>,
    },
    Http {
        url: String,
        header_keys: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpServerView {
    pub id: String,
    pub name: String,
    pub transport: McpTransportView,
    /// Ask even in bypass mode. Synced (AI-31).
    pub always_ask: bool,
    /// Enabled on this device; a server from another device starts enabled only for `http`.
    pub enabled: bool,
    /// On this device, every tool of the server runs without asking in manual mode (AI-31).
    pub always_allow: bool,
    /// On this device, these tools (the server's own names) run without asking in manual mode.
    pub always_allow_tools: Vec<String>,
    pub updated_at: i64,
}

#[derive(Clone, Deserialize, Type)]
pub struct McpSecretInput {
    pub key: String,
    /// `None` keeps the saved value for this key.
    pub value: Option<String>,
}

impl fmt::Debug for McpSecretInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpSecretInput")
            .field("key", &self.key)
            .field("value", &redacted(self.value.as_deref()))
            .finish()
    }
}

impl Drop for McpSecretInput {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpTransportInput {
    Stdio {
        command: String,
        args: Vec<String>,
        env: Vec<McpSecretInput>,
    },
    Http {
        url: String,
        headers: Vec<McpSecretInput>,
    },
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct McpServerInput {
    /// `None` creates a server.
    pub id: Option<String>,
    pub name: String,
    pub transport: McpTransportInput,
    pub always_ask: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpServerState {
    Stopped,
    Starting,
    Running,
    Failed,
}

/// Shown on the approval card; never changes whether a call asks (AI-31).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq, Default)]
pub struct McpToolAnnotations {
    pub title: Option<String>,
    pub read_only_hint: Option<bool>,
    pub destructive_hint: Option<bool>,
    pub idempotent_hint: Option<bool>,
    pub open_world_hint: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpToolView {
    /// The name offered to the model, `mcp__<server>__<tool>` after cleaning (AI-30).
    pub name: String,
    /// The server's own tool name.
    pub tool: String,
    pub description: String,
    pub annotations: McpToolAnnotations,
    /// Always allow on this device (per tool, or because the whole server is).
    pub always_allow: bool,
}

/// A server's live state (AI-32), also pushed as `ai://mcp-status` on every change. The stderr
/// lines stay in memory and are never logged.
#[derive(Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpServerStatus {
    pub server_id: String,
    pub state: McpServerState,
    pub error: Option<String>,
    /// The last stderr lines of a `stdio` server, kept in memory only (AI-32).
    pub stderr: Vec<String>,
    /// The tools from the last successful listing.
    pub tools: Vec<McpToolView>,
    /// The prompts from the last successful listing, which the panel's `/` menu offers (AI-50).
    pub prompts: Vec<McpPromptView>,
    /// The resources from the last successful listing, which the panel's `@` menu offers (AI-51).
    pub resources: Vec<McpResourceView>,
}

impl fmt::Debug for McpServerStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpServerStatus")
            .field("server_id", &self.server_id)
            .field("state", &self.state)
            .field("error", &self.error.is_some())
            .field("stderr", &format_args!("<{} lines>", self.stderr.len()))
            .field("tools", &self.tools.len())
            .field("prompts", &self.prompts.len())
            .field("resources", &self.resources.len())
            .finish()
    }
}

/// A server's prompt (AI-50): the `/` menu offers it as `/mcp__<server>__<prompt>`, which the
/// panel names across the servers a conversation uses, as tool names are (AI-30).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpPromptView {
    /// The prompt's name on its server.
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    pub arguments: Vec<McpPromptArgumentView>,
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpPromptArgumentView {
    pub name: String,
    pub description: String,
    pub required: bool,
}

/// A server's resource (AI-51), which the `@` menu offers.
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpResourceView {
    pub uri: String,
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    pub mime_type: Option<String>,
}

/// A value for one argument of an MCP prompt (AI-50).
#[derive(Clone, Deserialize, Type)]
pub struct McpPromptArgumentInput {
    pub name: String,
    pub value: String,
}

impl fmt::Debug for McpPromptArgumentInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpPromptArgumentInput")
            .field("name", &self.name)
            .field(
                "value",
                &format_args!("<{} chars>", self.value.chars().count()),
            )
            .finish()
    }
}

/// The MCP tool behind a name the model called, for the approval card (AI-31).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpToolInfo {
    pub server_id: String,
    pub server_name: String,
    pub always_ask: bool,
    pub tool: McpToolView,
}

/// What the approval card of a `call_mcp_tool` call needs (AI-45).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpProxyInfo {
    /// The tool the call names, as `mcp_tool_info` gives a direct call's; null when the
    /// conversation has no tool of that name or its server was deleted.
    pub tool: Option<McpToolInfo>,
    /// False when the call cannot run: it then gets its error result without asking.
    pub runs: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct McpImportServer {
    pub name: String,
    pub transport: McpTransportView,
    /// A saved server has this name; the import adds a numeric suffix.
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct McpImportSkipped {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct McpImportPreview {
    pub servers: Vec<McpImportServer>,
    /// Entries that cannot be imported, such as an `sse` server, with the reason.
    pub skipped: Vec<McpImportSkipped>,
}

// ───────────────────────── AI assistant: history search (AI-24) ─────────────────────────

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiSearchHit {
    pub conversation_id: String,
    /// The first matching entry, or `None` when only the title matched.
    pub entry_id: Option<String>,
    /// Text around the first match.
    pub snippet: String,
}
