use std::fmt;

use serde::{Deserialize, Serialize};
use specta::Type;

use super::{AiAutoVerdict, AiEffort, AiPermissionMode, LocalShellKind, QuickTarget};

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

// ───────────────────────── AI assistant: history search (AI-24) ─────────────────────────

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiSearchHit {
    pub conversation_id: String,
    /// The first matching entry, or `None` when only the title matched.
    pub entry_id: Option<String>,
    /// Text around the first match.
    pub snippet: String,
}
