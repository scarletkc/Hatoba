//! The AI assistant's runtime (spec §13.1): running turns, the tools that run in Rust, and the
//! conversation operations behind the `ai_*` commands.
//!
//! A turn is a task that builds a request from the stored conversation, streams the response to
//! the turn's channel, stores the assistant entry and, when the response has tool calls, waits
//! until every call has a stored result before it sends the next request. The frontend runs
//! `read_terminal` and `send_input` and reports with `ai_tool_result`; `ai_tool_run` runs the
//! other tools here. The frontend may run several calls of a response at once, and their results
//! are stored in the order of the calls. What is stored is exactly what the model receives
//! (§13.7).
//!
//! Before each request the turn asks the [`McpManager`] for the MCP tools to offer (AI-30),
//! which starts the servers that need it, and compacts the context first when the request would
//! pass 90% of the model's context window (AI-22). A new message waits for those servers before it
//! is stored: from the first message whose MCP tool definitions come to a tenth of the window, the
//! conversation's requests offer `search_tools` and `call_mcp_tool` in their place, and its
//! messages list the tools in an `mcp_tools` note, so the tool definitions and the system prompt
//! stay the same whatever the servers do (AI-45). A Compact request (AI-21) is the request a
//! turn would send next with the instruction after it, built by the same function, so it reuses
//! the prefix the provider cached. A side question (AI-42) sends the same request with the
//! question after it, and stores nothing. History search (AI-24), edit and resend (AI-26) and
//! forking (AI-43) are here too.
//!
//! Everything works on a [`SharedVault`] and two small traits, [`EventSink`] (where a turn's
//! events go) and [`AiEnv`] (the sync trigger and the tab's SSH session), so the turn tests run
//! without Tauri.
//!
//! Locks: the vault mutex is taken before the manager's own maps, never while holding one. A
//! turn checks its cancellation token under the vault lock before every write, and every stop
//! cancels under the vault lock, so nothing a stopped turn does reaches the vault or its channel
//! after the stop. [`AiEnv::changed`] runs only after the vault guard is dropped, because the
//! sync status it emits reads the vault.
//!
//! SEC-04: logs carry conversation ids, statuses and error kinds, never messages, tool inputs or
//! results, provider messages or keys.

use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use hatoba_ai::AiError;
use hatoba_ai::chat::{ChatRequest, StreamEvent, ToolDef, complete, stream_chat};
use hatoba_ai::entry::{
    AiEntry, AssistantEntry, AutoDecision, AutoVerdict, EntryBody, EntryPlace, EntryTarget, Finish,
    ToolCall, ToolStatus, Usage, fix_up_missing_results, pair_results,
};
use hatoba_ai::judge::{self, JudgeCall, JudgeFailure, JudgeState, Verdict};
use hatoba_ai::mcp::deferred;
use hatoba_ai::models::known_model;
use hatoba_ai::provider::{AuthHeader, Effort, ModelSpec, Protocol, ProviderConfig};
use hatoba_ai::skills::{BUILTIN_NAME, builtin_description, builtin_skill};
use hatoba_ai::tools::{
    self, CommandStop, FetchUrlArgs, LocalPrompt, MemoryArgs, PermissionMode, PromptContext,
    PromptModel, PromptTools, ReadSkillArgs, RunCommandArgs, StoppedCommand, ToolPlace, ToolSet,
    WebSearchArgs, escape_closing_tag,
};
use hatoba_ai::web::{self, NativeSearch, SearchConfig, SearchKind as WebSearchKind};
use hatoba_core::model::{
    AiAuthHeader, AiConversation, AiEffort as CoreEffort, AiModel as CoreModel,
    AiModelRef as CoreModelRef, AiProtocol as CoreProtocol, AiProvider, AiQuickTarget, Item,
    MAX_CUSTOM_INSTRUCTIONS_BYTES, MAX_HOST_AI_NOTES_BYTES, SearchKind as CoreSearchKind,
    SearchProvider,
};
use hatoba_core::sync::SharedVault;
use hatoba_core::{SnapshotKey, SnapshotState, Vault};
use hatoba_ssh::remote_command;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::dto::{
    AiAutoVerdict, AiConversationDetail, AiConversationView, AiEffort, AiEntryView, AiFileChange,
    AiFileChangeState, AiFilePreview, AiFileRevert, AiFinish, AiJudgeAnswer, AiJudgeDecision,
    AiJudgeSampleOutcome, AiLocalTab, AiModelRef, AiPermissionMode, AiSearchHit, AiSendInput,
    AiSendStarted, AiSideAnswer, AiSideEvent, AiSideQuestionInput, AiSkillCommand, AiToolCall,
    AiToolResultInput, AiToolRun, AiToolStatus, AiTurnContext, AiTurnEndReason, AiTurnEvent,
    AiUsage, LocalShellKind, QuickTarget,
};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::mcp::{McpManager, Offer, OfferedTool};
use crate::state::now_ms;
use files::FileJob;
use search::SearchJob;

/// The line a result starts with when the user edited the call before it ran (AI-17).
const EDIT_NOTE: &str = "The user edited the arguments before the call ran; it ran with";

/// What every MCP tool name starts with (AI-30).
const MCP_PREFIX: &str = "mcp__";

/// How long a stop waits for the commands of `run_command` calls to end and their calls to store
/// results that say so (AI-18): the time `remote_command::run` takes at most after a stop, and a
/// moment to store the result.
const COMMAND_STOP_WAIT: Duration =
    remote_command::STOP_WAIT.saturating_add(Duration::from_secs(1));

/// The `run_command` result while the tab has no live session (AI-08).
const DISCONNECTED: &str = "The terminal tab is disconnected, so the command did not run. Ask \
     the user to reconnect the tab.";

/// The final instruction of a Compact request (AI-21). The request offers the conversation's
/// tools, so it first asks for text only. The summary leaves out what the assistant can use:
/// every request states that in its system prompt, and a summary written by the fallback without
/// tools would otherwise tell the requests after it that there are none. It leaves out the memories
/// too (AI-53): the summary is stored with them as they are then, and a copy in the summary would
/// outlive an edit of one.
const COMPACT_INSTRUCTION: &str = "Respond with text only. Do not call any tools.\n\n\
     Summarize the conversation so far so that the summary can replace it as the context of the \
     rest of this conversation. Keep everything later work needs: the user's goals and requests, \
     the hosts and systems involved, the commands that were run and their outcomes, files and \
     settings that were changed, decisions made, and open problems and next steps. Leave out what \
     no longer matters, and leave out what the assistant can use, such as its tools, skills and \
     terminal connection: every request states that in its system prompt, and it can change \
     between requests. The memories in a <memory> note are stored with the summary as they are \
     then, so leave out what the note says. Write the summary in the user's language and reply \
     with the summary only.";

/// The instruction of a Compact request between tool calls (AI-22), after which the turn goes on
/// from the summary alone: it asks for the task in progress and its next step as well.
const COMPACT_MID_TURN_INSTRUCTION: &str = "Respond with text only. Do not call any tools.\n\n\
     The context is nearly full in the middle of a task. Summarize the conversation so far so \
     that the summary can replace it and the task can go on from it. Keep the user's goals and \
     requests, the hosts and systems involved, the task in progress and the step it has reached, \
     what the latest tool results showed, the commands that were run and their outcomes, files \
     and settings that were changed, decisions made, and the next step. Leave out what no longer \
     matters, and leave out what the assistant can use, such as its tools, skills and terminal \
     connection: every request states that in its system prompt, and it can change between \
     requests. The memories in a <memory> note are stored with the summary as they are then, so \
     leave out what the note says. Write the summary in the user's language and reply with the \
     summary only.";

/// The most output tokens a Compact request asks for (Anthropic's `max_tokens`, AI-21).
const COMPACT_MAX_TOKENS: u64 = 16_000;

/// The most output tokens a Compact request asks for, in hundredths of the context window: what
/// a context just under the AI-22 threshold leaves free, less a hundredth for the instruction and
/// the error of the estimate, so the request stays inside the window.
const COMPACT_OUTPUT_PERCENT: u64 = 9;

/// What comes before a side question (AI-42), in the last user message of its request. The
/// request offers the conversation's tools, so it first asks for text only, then for an answer
/// that leaves the conversation as it is: a turn may be running, and nothing keeps the answer.
const SIDE_QUESTION_INSTRUCTION: &str = "Respond with text only. Do not call any tools.\n\n\
     The user asks the side question below while this conversation goes on. Answer it from the \
     conversation so far, briefly and in the user's language. The answer is shown to the user \
     once and is not kept in the conversation, and any task in progress goes on without it, so \
     do not carry on with the task or say that you will do something.";

/// The result a side question's request gives the call whose approval card waits (AI-42).
const SIDE_AWAITING_APPROVAL: &str =
    "No result yet: this call is waiting for the user's approval and has not run.";

/// The result a side question's request gives each other call of the running turn without one
/// (AI-42).
const SIDE_PENDING: &str = "No result yet: this call is still running, or has not started.";

/// A skill's main file (§13.8).
const SKILL_MD: &str = "SKILL.md";

/// AI-22: a request whose context would pass this share of the context window is compacted
/// first, in tenths.
const AUTO_COMPACT_TENTHS: u64 = 9;

/// The rough text-to-token ratio of estimates, the same as the panel's meter (AI-20).
const CHARS_PER_TOKEN: usize = 4;

/// The most characters a conversation's title holds (AI-23): a rename longer is refused and a
/// fork's title is cut to it.
pub const MAX_TITLE_CHARS: usize = 200;

/// Characters of a search snippet (AI-24), and how many of them come before the match.
const SNIPPET_CHARS: usize = 120;
const SNIPPET_BEFORE: usize = 40;

/// Where a turn's events go: the `Channel` of `ai_send` / `ai_retry` in the app, a list in tests.
pub trait EventSink: Send + Sync + 'static {
    fn send(&self, event: AiTurnEvent);
}

impl EventSink for tauri::ipc::Channel<AiTurnEvent> {
    fn send(&self, event: AiTurnEvent) {
        // A reloaded WebView drops its channels; the turn goes on and its entries are stored.
        let _ = tauri::ipc::Channel::<AiTurnEvent>::send(self, event);
    }
}

/// What the runtime needs from the app.
pub trait AiEnv: Send + Sync + 'static {
    /// The vault changed: schedules a sync (`sync::local_change`). Never called with the vault
    /// guard held.
    fn changed(&self);
    /// What a terminal tab's session runs over, while it is open (AI-08): its SSH connection, or
    /// a local tab's shell (§7.5).
    fn session(&self, session_id: &str) -> Option<conn::Conn>;
    /// The identification string the server of a tab's open SSH session sent (§13.1).
    fn server_id(&self, session_id: &str) -> Option<String> {
        match self.session(session_id)? {
            conn::Conn::Ssh(s) if !s.is_closed() => s.server_id().map(str::to_owned),
            _ => None,
        }
    }
    /// The running app's version, which the built-in skill states (AI-34).
    fn app_version(&self) -> String;
}

/// A turn's task, spawned by the caller (`tauri::async_runtime::spawn` in the app).
pub type TurnTask = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// Running turns and tools. Cheap to clone; every clone is the same manager.
#[derive(Clone, Default)]
pub struct AiManager(Arc<Inner>);

#[derive(Default)]
struct Inner {
    /// The client for provider and search requests, built once
    /// (`hatoba_ai::provider::http_client`).
    http: OnceLock<reqwest::Client>,
    /// The MCP servers whose tools requests offer (§13.9).
    mcp: McpManager,
    /// Running turns by conversation id.
    turns: Mutex<HashMap<String, Arc<Turn>>>,
    /// Stops that wait for the commands of their turns to end (AI-18), by conversation id. They
    /// settle the turn once the commands have ended, unless another stop or the lock settles it
    /// first.
    stopping: Mutex<HashMap<String, Waiting>>,
    /// Running tools, compactions and provider requests, so that stop and lock reach them.
    ops: Mutex<HashMap<u64, Op>>,
    next_op: AtomicU64,
    /// What the approval card of an `edit_file` or `write_file` call read, by conversation and
    /// call id: the call writes only over that content (AI-39). File content, so kept only while
    /// the call waits: until it has a result, or its conversation stops, sends its next request,
    /// compacts or is deleted, or the vault locks.
    file_bases: Mutex<HashMap<(String, String), files::Base>>,
    /// The verdicts of the judge of Auto mode (AI-47) by conversation and call id, kept from
    /// `ai_judge` until the call's result is stored, which carries the verdict when Auto decided
    /// the call. Only while the call waits: dropped with its result, or when its conversation
    /// stops, sends its next request, compacts or is deleted, or the vault locks.
    pending_verdicts: Mutex<HashMap<(String, String), AutoVerdict>>,
    /// Room for the judge's requests (AI-47), shared by every conversation.
    judge_slots: JudgeSlots,
    /// What the diff of a revert showed the file holding, by conversation, response and call:
    /// Restore anyway writes only over that (AI-41).
    restore_bases: Mutex<HashMap<(String, String, String), files::Found>>,
    /// The conversations whose file change a revert is putting back (AI-41). No turn starts in
    /// them meanwhile, so the model never acts while the file changes under it, and the next
    /// message notes the revert.
    reverts: Mutex<HashSet<String>>,
    /// What the model has seen of each file, by conversation (AI-38…40).
    seen: files::Seen,
    /// The `edit_file` and `write_file` calls running now, by session: a lock that disconnects
    /// SSH sessions closes a session once they have ended.
    writes: files::Writes,
    /// What `web_search` returned when a provider of no known search turned its native search
    /// away (AI-14), by base URL and model: later calls get it without asking the provider again,
    /// until the app quits. The tool stays offered, so a conversation's tools never change.
    native_misses: Mutex<HashMap<(String, String), String>>,
    /// Cancels the ChatGPT sign-in that waits for the browser (AI-52); one at a time.
    chatgpt_sign_in: Mutex<Option<CancellationToken>>,
}

/// Most requests to the judge of Auto mode (AI-47) at once, across conversations, so calls that
/// several conversations make together do not send the judge's provider a burst it refuses.
const JUDGE_REQUESTS: usize = 4;

/// Room for [`JUDGE_REQUESTS`] requests to the judge at once.
struct JudgeSlots(tokio::sync::Semaphore);

impl Default for JudgeSlots {
    fn default() -> Self {
        Self(tokio::sync::Semaphore::new(JUDGE_REQUESTS))
    }
}

struct Op {
    /// `None` for work outside a conversation, which only the lock stops.
    conversation_id: Option<String>,
    /// The tool call an `ai_tool_run` runs, which nothing else runs or answers meanwhile.
    tool_call_id: Option<String>,
    kind: OpKind,
    cancel: CancellationToken,
    /// Cancelled when the op ends.
    ended: CancellationToken,
    /// What the tool of an `ai_tool_run` returned, from when it ended until its result is
    /// stored, while the result waits for the earlier calls' (§13.1). A stop stores it rather
    /// than a cancelled result, and does not count the call as a running command.
    ran: Option<Ran>,
}

/// The result of a tool that has ended, waiting to be stored.
struct Ran {
    status: ToolStatus,
    content: String,
    /// The file the model sees in the result (AI-38…40), with the response of the call.
    shown: Option<(String, files::Shown)>,
}

/// What an op does, where something else depends on it.
#[derive(Clone, PartialEq, Eq)]
enum OpKind {
    /// A tool, the file read of an approval card, or a provider request outside a conversation.
    Other,
    /// A `run_command` call, whose command a stop waits for (AI-18).
    Command,
    /// A compaction of the op's conversation (AI-21, AI-22), which refuses a side question
    /// meanwhile (AI-42).
    Compact,
    /// A side question (AI-42) about `conversation_id`, by the panel's number for it. Its op
    /// belongs to no conversation, so a stop of the turn leaves it alone, and the conversation
    /// does not count as busy while it runs.
    Side {
        conversation_id: String,
        question_id: u32,
    },
}

impl Op {
    /// The number of the side question (AI-42) about the conversation that the op is.
    fn side_of(&self, conversation_id: &str) -> Option<u32> {
        match &self.kind {
            OpKind::Side {
                conversation_id: of,
                question_id,
            } if of == conversation_id => Some(*question_id),
            _ => None,
        }
    }
}

/// A running tool, compaction or provider request; unregisters itself when dropped.
pub struct OpGuard<'a> {
    inner: &'a Inner,
    id: u64,
    cancel: CancellationToken,
    ended: CancellationToken,
}

impl OpGuard<'_> {
    /// Cancelled by a stop of the op's conversation, or by the lock.
    pub fn token(&self) -> &CancellationToken {
        &self.cancel
    }
}

impl Drop for OpGuard<'_> {
    fn drop(&mut self) {
        guard(&self.inner.ops).remove(&self.id);
        self.ended.cancel();
    }
}

/// One running turn.
struct Turn {
    cancel: CancellationToken,
    sink: Arc<dyn EventSink>,
    context: AiTurnContext,
    /// The identification string of the tab's SSH server when the turn started, so every
    /// request of the turn has the same system prompt.
    server_id: Option<String>,
    /// Woken when a tool result of the conversation is stored.
    results: Notify,
    /// Wakes the results of the turn that wait for an earlier call's ([`AiManager::in_call_order`])
    /// when a result of the conversation is stored, and when the turn ends or stops.
    waiting: Notify,
    /// `turn_ended` was sent; nothing else is sent after it.
    ended: AtomicBool,
    /// What the response being streamed has sent so far, from its request until it is stored or
    /// fails. A stop takes it under the vault guard and stores what the user saw (§13.1).
    streamed: Mutex<Option<Streamed>>,
}

/// A stop that waits for the commands of its turn to end (AI-18).
struct Waiting {
    /// Tells the stop from a later one of the same conversation, which may start once another
    /// stop or a message settled this one.
    id: u64,
    halted: Halted,
    /// The `run_command` calls it waits for.
    waited: Vec<String>,
}

/// What the first part of a stop took ([`AiManager::halt`]), for the rest of it.
struct Halted {
    turn: Option<Arc<Turn>>,
    /// What the response being streamed had shown, with the turn's tab.
    streamed: Option<(Streamed, bool)>,
}

/// The part of a response that has streamed so far, kept for a stop.
struct Streamed {
    provider_id: String,
    model_id: String,
    text: String,
    /// Calls whose arguments are complete; a call still streaming has sent nothing yet.
    tool_calls: Vec<ToolCall>,
}

impl Streamed {
    /// The `interrupted` entry a stop stores (§13.1): the text and complete calls, without the
    /// reasoning (a signed thinking block cannot be replayed half done) or a `raw`, so later
    /// requests rebuild it from its fields. `None` when no text or call has arrived.
    fn into_entry(self, terminal: bool) -> Option<AssistantEntry> {
        if self.text.trim().is_empty() && self.tool_calls.is_empty() {
            return None;
        }
        Some(AssistantEntry {
            provider_id: self.provider_id,
            model_id: self.model_id,
            text: self.text,
            finish: if self.tool_calls.is_empty() {
                Finish::Stop
            } else {
                Finish::ToolCalls
            },
            tool_calls: self.tool_calls,
            terminal: Some(terminal),
            interrupted: true,
            ..AssistantEntry::default()
        })
    }
}

impl Turn {
    fn emit(&self, event: AiTurnEvent) {
        if !self.ended.load(Ordering::Acquire) {
            self.sink.send(event);
        }
    }

    /// Sends `turn_ended` once; later calls do nothing.
    fn end(&self, reason: AiTurnEndReason) {
        if !self.ended.swap(true, Ordering::AcqRel) {
            self.sink.send(AiTurnEvent::TurnEnded { reason });
        }
    }

    /// Ends a turn whose task has finished. A cancelled turn is ended by what cancelled it (a
    /// stop or a deletion), after the entries a stop stores, so its task never sends
    /// `turn_ended` before them.
    fn finish(&self, reason: AiTurnEndReason) {
        if !self.cancel.is_cancelled() {
            self.end(reason);
        }
    }
}

/// Why a turn stops before its next step.
enum Halt {
    /// `ai_stop`, a new message, the lock, or a locked vault.
    Stopped,
    /// An `error` event with this message, then `turn_ended { error }`.
    Failed(String),
    /// The stored conversation has nothing to send.
    Idle,
}

impl From<AppError> for Halt {
    fn from(e: AppError) -> Self {
        if e.code == ErrorCode::Locked {
            Self::Stopped
        } else {
            Self::Failed(e.detail)
        }
    }
}

/// What one request is made of, read from the vault.
struct Request {
    provider_id: String,
    provider: ProviderConfig,
    model: ModelSpec,
    /// The turn's thinking level (AI-05); `None` is Default.
    effort: Option<Effort>,
    system: String,
    tools: Vec<ToolDef>,
    entries: Vec<AiEntry>,
}

impl Request {
    fn chat(&self) -> ChatRequest<'_> {
        ChatRequest {
            provider_id: &self.provider_id,
            model: &self.model,
            system: &self.system,
            tools: &self.tools,
            entries: &self.entries,
            effort: self.effort,
        }
    }
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn lock(vault: &SharedVault) -> MutexGuard<'_, Vault> {
    guard(vault)
}

fn unlocked(vault: &SharedVault) -> AppResult<MutexGuard<'_, Vault>> {
    let v = lock(vault);
    if v.is_unlocked() {
        Ok(v)
    } else {
        Err(AppError::locked())
    }
}

impl AiManager {
    /// A manager whose requests offer the tools of `mcp`'s servers.
    pub fn with_mcp(mcp: McpManager) -> Self {
        Self(Arc::new(Inner {
            mcp,
            ..Inner::default()
        }))
    }

    /// A ChatGPT sign-in starts (AI-52): returns the token that cancels it, which `parent` (the
    /// sign-in's op, which the lock cancels) cancels too. A sign-in still waiting is cancelled.
    pub fn chatgpt_sign_in_started(&self, parent: &CancellationToken) -> CancellationToken {
        let cancel = parent.child_token();
        if let Some(previous) = guard(&self.0.chatgpt_sign_in).replace(cancel.clone()) {
            previous.cancel();
        }
        cancel
    }

    /// Cancels the ChatGPT sign-in that waits for the browser, if any.
    pub fn chatgpt_sign_in_cancel(&self) {
        if let Some(cancel) = guard(&self.0.chatgpt_sign_in).take() {
            cancel.cancel();
        }
    }

    /// The shared client for provider and search requests.
    pub fn http(&self) -> reqwest::Client {
        self.0
            .http
            .get_or_init(hatoba_ai::provider::http_client)
            .clone()
    }

    /// A turn of the conversation is running, or a stop of one waits for its commands to end.
    pub fn is_running(&self, conversation_id: &str) -> bool {
        guard(&self.0.turns).contains_key(conversation_id)
            || guard(&self.0.stopping).contains_key(conversation_id)
    }

    fn running(&self, conversation_id: &str) -> Option<Arc<Turn>> {
        guard(&self.0.turns).get(conversation_id).cloned()
    }

    /// Registers a turn. The caller stopped the previous one under the same vault guard.
    fn register(
        &self,
        conversation_id: &str,
        context: AiTurnContext,
        server_id: Option<String>,
        sink: Arc<dyn EventSink>,
    ) -> Arc<Turn> {
        let turn = Arc::new(Turn {
            cancel: CancellationToken::new(),
            sink,
            context,
            server_id,
            results: Notify::new(),
            waiting: Notify::new(),
            ended: AtomicBool::new(false),
            streamed: Mutex::new(None),
        });
        guard(&self.0.turns).insert(conversation_id.to_owned(), Arc::clone(&turn));
        turn
    }

    /// Drops a finished turn, unless a newer turn of the conversation replaced it.
    fn forget(&self, conversation_id: &str, turn: &Arc<Turn>) {
        let mut turns = guard(&self.0.turns);
        if turns
            .get(conversation_id)
            .is_some_and(|t| Arc::ptr_eq(t, turn))
        {
            turns.remove(conversation_id);
            drop(turns);
            // A result waiting for an earlier call's waits no more once the turn is over.
            turn.waiting.notify_waiters();
        }
    }

    /// Cancels the conversation's turn and its running tools, and returns the turn.
    fn cancel(&self, conversation_id: &str) -> Option<Arc<Turn>> {
        let turn = guard(&self.0.turns).remove(conversation_id);
        if let Some(turn) = &turn {
            turn.cancel.cancel();
            turn.waiting.notify_waiters();
        }
        for op in guard(&self.0.ops).values() {
            if op.conversation_id.as_deref() == Some(conversation_id) {
                op.cancel.cancel();
            }
        }
        turn
    }

    /// Registers a tool of the conversation. Called under the vault guard that checked the call,
    /// so a stop either came first (and the check failed) or cancels it.
    fn start_op(&self, conversation_id: &str) -> OpGuard<'_> {
        let cancel = self.turn_token(conversation_id);
        self.register_op(Some(conversation_id), None, OpKind::Other, cancel)
    }

    /// Registers a compaction of the conversation (AI-21, AI-22), like [`Self::start_op`]. The
    /// conversation counts as compacting until the guard drops ([`Self::is_compacting`]).
    fn start_compaction(&self, conversation_id: &str) -> OpGuard<'_> {
        let cancel = self.turn_token(conversation_id);
        self.register_op(Some(conversation_id), None, OpKind::Compact, cancel)
    }

    /// A compaction of the conversation runs, by hand or automatically.
    fn is_compacting(&self, conversation_id: &str) -> bool {
        guard(&self.0.ops).values().any(|op| {
            op.kind == OpKind::Compact && op.conversation_id.as_deref() == Some(conversation_id)
        })
    }

    /// Registers the run of a tool call, like [`Self::start_op`]. Refused while the call runs
    /// already, so that a call with side effects (`run_command`, an MCP tool) runs once: called
    /// under the vault guard that checked the call, as is [`Self::call_running`]. `kind` is
    /// [`OpKind::Command`] for a `run_command` call.
    fn start_call(
        &self,
        conversation_id: &str,
        tool_call_id: &str,
        kind: OpKind,
    ) -> AppResult<OpGuard<'_>> {
        if self.call_running(conversation_id, tool_call_id) {
            return Err(AppError::invalid(
                "tool_call_id",
                "the tool call is already running",
            ));
        }
        let cancel = self.turn_token(conversation_id);
        Ok(self.register_op(Some(conversation_id), Some(tool_call_id), kind, cancel))
    }

    /// An `ai_tool_run` of the call has not finished.
    fn call_running(&self, conversation_id: &str, tool_call_id: &str) -> bool {
        guard(&self.0.ops).values().any(|op| {
            op.conversation_id.as_deref() == Some(conversation_id)
                && op.tool_call_id.as_deref() == Some(tool_call_id)
        })
    }

    /// A token that a stop of the conversation's running turn cancels.
    /// While a stop waits for the conversation's commands, the token is cancelled already, so
    /// nothing new starts in the stopped turn.
    fn turn_token(&self, conversation_id: &str) -> CancellationToken {
        if guard(&self.0.stopping).contains_key(conversation_id) {
            let stopped = CancellationToken::new();
            stopped.cancel();
            return stopped;
        }
        self.running(conversation_id)
            .map_or_else(CancellationToken::new, |turn| turn.cancel.child_token())
    }

    /// A provider or search request outside a conversation (a model list, a connection test):
    /// the lock cancels its token (§13.1). Call it under the vault guard that read the provider,
    /// so that a lock either came first or cancels the request.
    pub fn background_op(&self) -> OpGuard<'_> {
        self.register_op(None, None, OpKind::Other, CancellationToken::new())
    }

    /// Registers the side question `question_id` of the conversation (AI-42) and stops the one
    /// on its way before it, at once, so that two side questions never both go on. A stop of the
    /// turn leaves it alone and the lock reaches it; [`Self::side_stop`] stops it.
    fn start_side(&self, conversation_id: &str, question_id: u32) -> OpGuard<'_> {
        let mut ops = guard(&self.0.ops);
        for op in ops.values() {
            if op.side_of(conversation_id).is_some() {
                op.cancel.cancel();
            }
        }
        let kind = OpKind::Side {
            conversation_id: conversation_id.to_owned(),
            question_id,
        };
        self.insert_op(&mut ops, None, None, kind, CancellationToken::new())
    }

    /// `ai_side_stop` (AI-42): stops the conversation's side question `question_id`, if it is on
    /// its way. Its turn goes on, and so does a newer side question that replaced it.
    pub fn side_stop(&self, conversation_id: &str, question_id: u32) {
        for op in guard(&self.0.ops).values() {
            if op.side_of(conversation_id) == Some(question_id) {
                op.cancel.cancel();
            }
        }
    }

    /// Stops every side question of the conversation (AI-42).
    fn stop_sides(&self, conversation_id: &str) {
        for op in guard(&self.0.ops).values() {
            if op.side_of(conversation_id).is_some() {
                op.cancel.cancel();
            }
        }
    }

    fn register_op(
        &self,
        conversation_id: Option<&str>,
        tool_call_id: Option<&str>,
        kind: OpKind,
        cancel: CancellationToken,
    ) -> OpGuard<'_> {
        let mut ops = guard(&self.0.ops);
        self.insert_op(&mut ops, conversation_id, tool_call_id, kind, cancel)
    }

    fn insert_op(
        &self,
        ops: &mut HashMap<u64, Op>,
        conversation_id: Option<&str>,
        tool_call_id: Option<&str>,
        kind: OpKind,
        cancel: CancellationToken,
    ) -> OpGuard<'_> {
        let id = self.0.next_op.fetch_add(1, Ordering::Relaxed);
        let ended = CancellationToken::new();
        ops.insert(
            id,
            Op {
                conversation_id: conversation_id.map(str::to_owned),
                tool_call_id: tool_call_id.map(str::to_owned),
                kind,
                cancel: cancel.clone(),
                ended: ended.clone(),
                ran: None,
            },
        );
        OpGuard {
            inner: &self.0,
            id,
            cancel,
            ended,
        }
    }

    /// Stops a conversation with the vault guard held (§13.1): cancels its turn and running
    /// tools, stores what a response being streamed has shown as an `interrupted` entry, stores a
    /// result for every call without one (its tool's, when the tool has ended and the result
    /// waited for an earlier call's; a cancelled one otherwise), and sends those entries and
    /// `turn_ended { stopped }` on the turn's channel. Returns the entries it stored.
    /// A stop that waits for commands ([`Self::stop_commands`]) is settled here at once: its
    /// commands get results that say they may still be running.
    fn stop_locked(&self, v: &mut Vault, conversation_id: &str) -> Vec<AiEntryView> {
        let mut stored = Vec::new();
        let waiting = guard(&self.0.stopping).remove(conversation_id);
        if let Some(Waiting { halted, waited, .. }) = waiting {
            stored = self.settle(v, conversation_id, halted, &waited);
        }
        let halted = self.halt(conversation_id);
        stored.extend(self.settle(v, conversation_id, halted, &[]));
        stored
    }

    /// The first part of a stop, with the vault guard held: cancels the conversation's turn and
    /// running tools, and takes what the response being streamed has shown.
    fn halt(&self, conversation_id: &str) -> Halted {
        // Taken before the cancel wakes the turn's task, which drops what streamed when its
        // request ends.
        let streamed = self
            .running(conversation_id)
            .and_then(|turn| guard(&turn.streamed).take().map(|s| (s, turn.context.tab)));
        let turn = self.cancel(conversation_id);
        self.forget_file_bases(conversation_id);
        // The calls get cancelled results, which keep no verdict (AI-47).
        self.forget_verdicts(conversation_id);
        Halted { turn, streamed }
    }

    /// The rest of a stop, with the vault guard held: stores the `interrupted` entry and the
    /// results of the calls without one, and sends them and `turn_ended { stopped }` on the
    /// turn's channel.
    /// `waited`: the `run_command` calls whose commands the stop waited for, which stored their
    /// own results meanwhile, sent too ([`Self::stop_commands`]). Returns the entries it sent.
    fn settle(
        &self,
        v: &mut Vault,
        conversation_id: &str,
        Halted { turn, streamed }: Halted,
        waited: &[String],
    ) -> Vec<AiEntryView> {
        let mut stored = Vec::new();
        if v.is_unlocked() && find_conversation(v, conversation_id).is_ok() {
            if !waited.is_empty()
                && let Ok(entries) = Entries::load(v, conversation_id)
            {
                stored.extend(entries.newest_results(waited));
            }
            if let Some(response) =
                streamed.and_then(|(streamed, terminal)| streamed.into_entry(terminal))
            {
                let entry = AiEntry::assistant(now_ms(), response);
                match append(v, conversation_id, &entry) {
                    Ok(entry_id) => stored.push(entry_view(&entry_id, &entry)),
                    Err(e) => tracing::warn!(
                        conversation_id,
                        code = ?e.code,
                        "could not store the stopped response"
                    ),
                }
            }
            let cancelled = Entries::load(v, conversation_id).and_then(|entries| {
                let mut known: HashMap<String, (ToolStatus, String)> = self
                    .running_writes(conversation_id, &entries)
                    .into_iter()
                    .chain(self.running_commands(conversation_id))
                    .map(|(id, content)| (id, (ToolStatus::Cancelled, content.to_owned())))
                    .collect();
                // A call whose tool has ended keeps its result.
                let mut shown = Vec::new();
                for (id, ran) in self.take_ran_of(conversation_id) {
                    shown.extend(ran.shown);
                    known.insert(id, (ran.status, ran.content));
                }
                let stored = cancel_open_calls(v, conversation_id, &entries, known)?;
                for (response, file) in shown {
                    self.0.seen.insert(conversation_id, &response, file);
                }
                Ok(stored)
            });
            match cancelled {
                Ok(views) => stored.extend(views),
                Err(e) => tracing::warn!(
                    conversation_id,
                    code = ?e.code,
                    "could not store cancelled tool results"
                ),
            }
        }
        if let Some(turn) = turn {
            for entry in &stored {
                turn.emit(AiTurnEvent::Entry {
                    entry: entry.clone(),
                });
            }
            turn.end(AiTurnEndReason::Stopped);
            tracing::info!(conversation_id, "AI turn stopped");
        }
        stored
    }

    /// The `edit_file` and `write_file` calls of the conversation's newest response that run now,
    /// with the content of their cancelled results: a stop does not cut off a write that has
    /// begun (§13.4, "File tools").
    fn running_writes(
        &self,
        conversation_id: &str,
        entries: &Entries,
    ) -> Vec<(String, &'static str)> {
        let newest = entries.list.iter().rev().find_map(|e| match &e.body {
            EntryBody::Assistant(a) => Some(a),
            _ => None,
        });
        newest.map_or_else(Vec::new, |a| {
            a.tool_calls
                .iter()
                .filter(|c| matches!(c.name.as_str(), tools::EDIT_FILE | tools::WRITE_FILE))
                .filter(|c| self.call_running(conversation_id, &c.id))
                .map(|c| (c.id.clone(), files::WRITE_STOPPED))
                .collect()
        })
    }

    /// The conversation's `run_command` calls whose commands run now, with the content of their
    /// cancelled results: a stop that does not wait for a command cannot tell whether it ended
    /// (AI-18). A command that has ended and whose result waits for an earlier call's runs no
    /// more.
    fn running_commands(&self, conversation_id: &str) -> Vec<(String, &'static str)> {
        guard(&self.0.ops)
            .values()
            .filter(|op| {
                op.kind == OpKind::Command
                    && op.ran.is_none()
                    && op.conversation_id.as_deref() == Some(conversation_id)
            })
            .filter_map(|op| op.tool_call_id.clone())
            .map(|id| (id, tools::COMMAND_STOP_UNCONFIRMED))
            .collect()
    }

    /// Keeps what the tool of the op's call returned, while the call's result waits for the
    /// earlier calls' ([`Op::ran`]).
    fn ran(&self, op: &OpGuard<'_>, ran: Ran) {
        if let Some(op) = guard(&self.0.ops).get_mut(&op.id) {
            op.ran = Some(ran);
        }
    }

    /// Takes what the tool of the op's call returned, unless a stop took it to store it.
    fn take_ran(&self, op: &OpGuard<'_>) -> Option<Ran> {
        guard(&self.0.ops)
            .get_mut(&op.id)
            .and_then(|op| op.ran.take())
    }

    /// Takes the results of the conversation's calls whose tools have ended, by call, for a stop
    /// to store.
    fn take_ran_of(&self, conversation_id: &str) -> Vec<(String, Ran)> {
        guard(&self.0.ops)
            .values_mut()
            .filter(|op| op.conversation_id.as_deref() == Some(conversation_id))
            .filter_map(|op| Some((op.tool_call_id.clone()?, op.ran.take()?)))
            .collect()
    }

    /// AI-18: stops the conversation when its turn runs commands (`run_command`), after waiting
    /// for them: the stop ends them, and waits up to [`COMMAND_STOP_WAIT`] for their calls to
    /// store results that say whether they ended. Meanwhile the conversation counts as running,
    /// and another stop, a message or the lock settles the stop at once ([`Self::stop_locked`]).
    /// Without a command, does nothing and returns `None`. Returns the entries the stop stored.
    async fn stop_commands(
        &self,
        vault: &SharedVault,
        conversation_id: &str,
    ) -> Option<Vec<AiEntryView>> {
        let id = self.0.next_op.fetch_add(1, Ordering::Relaxed);
        let ends = {
            let _v = lock(vault);
            self.running(conversation_id)?;
            let (waited, ends): (Vec<String>, Vec<CancellationToken>) = guard(&self.0.ops)
                .values()
                .filter(|op| {
                    op.kind == OpKind::Command
                        && op.conversation_id.as_deref() == Some(conversation_id)
                })
                .filter_map(|op| op.tool_call_id.clone().map(|id| (id, op.ended.clone())))
                .unzip();
            if waited.is_empty() {
                return None;
            }
            let halted = self.halt(conversation_id);
            let waiting = Waiting { id, halted, waited };
            guard(&self.0.stopping).insert(conversation_id.to_owned(), waiting);
            ends
        };
        let ended = async {
            for ended in &ends {
                ended.cancelled().await;
            }
        };
        if tokio::time::timeout(COMMAND_STOP_WAIT, ended)
            .await
            .is_err()
        {
            tracing::warn!(conversation_id, "a stopped command did not report in time");
        }
        let mut v = lock(vault);
        let waiting = {
            let mut stopping = guard(&self.0.stopping);
            match stopping.get(conversation_id) {
                Some(waiting) if waiting.id == id => stopping.remove(conversation_id),
                // Settled meanwhile by another stop, a message or the lock, and perhaps followed
                // by a newer stop, which is left to its own wait.
                _ => None,
            }
        };
        let Some(Waiting { halted, waited, .. }) = waiting else {
            return Some(Vec::new());
        };
        Some(self.settle(&mut v, conversation_id, halted, &waited))
    }

    /// `ai_stop`: stops the conversation's turn and tools, if any, and cancels its open calls.
    /// A command that runs is ended first, so its result says whether it did (AI-18).
    pub async fn stop(&self, vault: &SharedVault, env: &dyn AiEnv, conversation_id: &str) {
        let stored = match self.stop_commands(vault, conversation_id).await {
            Some(stored) => stored,
            None => self.stop_locked(&mut lock(vault), conversation_id),
        };
        if !stored.is_empty() {
            env.changed();
        }
    }

    /// The vault is about to lock (SEC-02): stops every turn and tool, and every provider request
    /// outside a turn, side questions (AI-42) included, so the assistant never acts behind the
    /// lock screen (§13.1). Called with the vault guard, before `Vault::lock`.
    pub fn stop_all(&self, v: &mut Vault) {
        let mut running: Vec<String> = guard(&self.0.turns).keys().cloned().collect();
        running.extend(guard(&self.0.stopping).keys().cloned());
        running.sort_unstable();
        running.dedup();
        for conversation_id in &running {
            self.stop_locked(v, conversation_id);
        }
        for op in guard(&self.0.ops).values() {
            op.cancel.cancel();
        }
        // File content stays in memory only while the vault is unlocked, and so do verdicts.
        guard(&self.0.file_bases).clear();
        guard(&self.0.restore_bases).clear();
        guard(&self.0.pending_verdicts).clear();
    }

    // ---- conversations ----

    /// A conversation with its entries (raw provider messages left out).
    pub fn conversation_detail(
        &self,
        vault: &SharedVault,
        conversation_id: &str,
    ) -> AppResult<AiConversationDetail> {
        let v = unlocked(vault)?;
        let conversation = find_conversation(&v, conversation_id)?;
        let entries = Entries::load(&v, conversation_id)?;
        Ok(AiConversationDetail {
            conversation: conversation_view(&v, conversation_id, &conversation),
            entries: entries.views(),
            running: self.is_running(conversation_id),
            // A conversation stays readable when its snapshots cannot be read; the edit's question
            // reads them again through `ai_file_changes`, which reports the error.
            file_changes: file_changes(&v, conversation_id).unwrap_or_else(|e| {
                tracing::warn!(conversation_id, code = ?e.code, "could not read the file snapshots");
                Vec::new()
            }),
        })
    }

    /// The changes of the conversation's `edit_file` and `write_file` calls that this device
    /// keeps (AI-41), for the panel to offer Revert after a call ran.
    pub fn file_changes(
        &self,
        vault: &SharedVault,
        conversation_id: &str,
    ) -> AppResult<Vec<AiFileChange>> {
        let v = unlocked(vault)?;
        find_conversation(&v, conversation_id)?;
        file_changes(&v, conversation_id)
    }

    /// What the call of `key` changed, from the snapshot this device keeps (AI-41), as the panel
    /// diffs it. `None` once the change was reverted or dropped.
    pub fn file_change_diff(
        &self,
        vault: &SharedVault,
        key: SnapshotKey<'_>,
    ) -> AppResult<Option<AiFilePreview>> {
        let v = unlocked(vault)?;
        find_conversation(&v, key.conversation_id)?;
        let Some(stored) = v.ai_file_snapshot(&key)? else {
            return Err(AppError::not_found("file change"));
        };
        let Some(content) = stored.content else {
            return Ok(None);
        };
        let unreadable = |_| AppError::internal("unreadable file snapshot");
        let change = files::Change {
            meta: serde_json::from_str(&stored.meta).map_err(unreadable)?,
            content: serde_json::from_str(&content).map_err(unreadable)?,
        };
        Ok(Some(files::change_preview(&change)))
    }

    /// Deletes a conversation and its entries, stopping its turn first.
    pub fn delete_conversation(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        conversation_id: &str,
    ) -> AppResult<()> {
        {
            let mut v = unlocked(vault)?;
            find_conversation(&v, conversation_id)?;
            if let Some(turn) = self.cancel(conversation_id) {
                turn.end(AiTurnEndReason::Stopped);
            }
            let waiting = guard(&self.0.stopping).remove(conversation_id);
            if let Some(turn) = waiting.and_then(|waiting| waiting.halted.turn) {
                turn.end(AiTurnEndReason::Stopped);
            }
            self.stop_sides(conversation_id);
            v.ai_delete_conversation(conversation_id)?;
            self.0.seen.forget(conversation_id);
            self.forget_file_bases(conversation_id);
            self.forget_verdicts(conversation_id);
        }
        tracing::info!(conversation_id, "AI conversation deleted");
        env.changed();
        Ok(())
    }

    /// `ai_conversation_fork` (AI-43): copies the conversation up to the entry `entry_id`, or all
    /// of it without one, into a new conversation titled `title` ([`fork_copies`]), and returns
    /// the new conversation. The original is not touched. Refused while a turn of the
    /// conversation runs, waiting for approval included, or while it is compacted.
    ///
    /// Decrypts every entry of the conversation and encrypts every copy, so run it off the async
    /// runtime. The vault guard is held to read the entries and to write the fork, and released
    /// while the entries are decoded and the copies chosen.
    pub fn fork_conversation(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        conversation_id: &str,
        entry_id: Option<&str>,
        title: &str,
    ) -> AppResult<AiConversationView> {
        let title: String = title.trim().chars().take(MAX_TITLE_CHARS).collect();
        if title.is_empty() {
            return Err(AppError::invalid("title", "the title is empty"));
        }
        let (original, stored, hosts) = {
            let v = unlocked(vault)?;
            let original = find_conversation(&v, conversation_id)?;
            if self.busy(conversation_id) {
                return Err(AppError::invalid(
                    "conversation_id",
                    "The conversation can be forked once its turn or compaction has ended.",
                ));
            }
            let hosts: Vec<(String, String)> = v
                .hosts()
                .into_iter()
                .map(|(id, h)| (id, h.name.clone()))
                .collect();
            (original, v.ai_entries(conversation_id)?, hosts)
        };
        // The stored JSON is copied as it is, raw messages included; the decoded entries decide
        // what is copied. Unreadable entries are left out, as from every request.
        let mut ids = Vec::new();
        let mut jsons = Vec::new();
        let mut entries = Vec::new();
        for hatoba_core::StoredEntry { entry_id, json } in stored {
            match serde_json::from_str::<AiEntry>(&json) {
                Ok(entry) => {
                    ids.push(entry_id);
                    jsons.push(json);
                    entries.push(entry);
                }
                Err(_) => tracing::warn!(
                    conversation_id,
                    entry_id = %entry_id,
                    "unreadable conversation entry not forked"
                ),
            }
        }
        let at = match entry_id {
            Some(entry_id) => Some(
                ids.iter()
                    .position(|id| id == entry_id)
                    .ok_or_else(|| AppError::not_found("message"))?,
            ),
            None => entries.len().checked_sub(1),
        };
        let Some(at) = at else {
            return Err(AppError::invalid(
                "conversation_id",
                "the conversation has no messages",
            ));
        };
        let copies = fork_copies(&entries, at, entry_id.is_none()).ok_or_else(|| {
            AppError::invalid(
                "entry_id",
                "A conversation can be forked from one of your messages or from a finished reply.",
            )
        })?;
        // Indices up to `at` are copied in place, so they are also their copies' indices.
        let context_start = original.context_start.as_deref().and_then(|start| {
            let from = ids.partition_point(|id| id.as_str() < start);
            if from <= at {
                Some(from)
            } else {
                entries[..=at]
                    .iter()
                    .rposition(|e| matches!(e.body, EntryBody::Summary { .. }))
            }
        });
        let (place, origin) = fork_place(&entries[at + 1..], &original, &hosts);
        let fork = AiConversation {
            title,
            host_id: place.host_id,
            quick_target: place.quick_target,
            local: place.local,
            origin,
            pinned: false,
            context_start: None,
            effort: original.effort,
            created_at: now_ms(),
            updated_at: 0,
        };
        let copied: Vec<&str> = copies.iter().map(|&i| jsons[i].as_str()).collect();
        let view = {
            let mut v = unlocked(vault)?;
            let (id, _) = v.ai_create_conversation(fork, &copied, context_start)?;
            tracing::info!(
                conversation_id,
                fork = %id,
                entries = copied.len(),
                "AI conversation forked"
            );
            conversation_view(&v, &id, &find_conversation(&v, &id)?)
        };
        env.changed();
        Ok(view)
    }

    // ---- turns ----

    /// `ai_send` (§13.1 step 1): stores the user's message, creating the conversation when it has
    /// no id yet, and returns the turn's task. A running turn of the conversation stops first.
    ///
    /// AI-22: when the context and the new message together would pass 90% of the model's
    /// context window, the context is compacted before the message is stored, so the summary
    /// (where `context_start` moves) comes before the message and the request carries both.
    pub async fn send(
        &self,
        vault: &SharedVault,
        env: Arc<dyn AiEnv>,
        input: AiSendInput,
        sink: Arc<dyn EventSink>,
    ) -> AppResult<(AiSendStarted, TurnTask)> {
        let AiSendInput {
            conversation_id,
            text,
            context,
        } = input;
        self.start_turn(vault, env, conversation_id, None, text, context, sink)
            .await
    }

    /// `ai_edit_resend` (AI-26): stops a running turn, deletes the user's message `entry_id`
    /// and every later entry (entries are written once, so the edited text becomes a new
    /// entry), moves a `context_start` that pointed at a deleted entry to the newest remaining
    /// summary (or clears it), then goes on like [`Self::send`] with `text`.
    #[expect(
        clippy::too_many_arguments,
        reason = "the command's own arguments, plus the vault, the app and the channel"
    )]
    pub async fn edit_resend(
        &self,
        vault: &SharedVault,
        env: Arc<dyn AiEnv>,
        conversation_id: String,
        entry_id: &str,
        text: String,
        context: AiTurnContext,
        sink: Arc<dyn EventSink>,
    ) -> AppResult<(AiSendStarted, TurnTask)> {
        self.start_turn(
            vault,
            env,
            Some(conversation_id),
            Some(entry_id),
            text,
            context,
            sink,
        )
        .await
    }

    /// Registers the turn, starts the MCP servers its requests use (AI-45), deletes the entries an
    /// edit replaces (AI-26), compacts first when the new message needs it (AI-22), stores the
    /// user's message with the thinking level (AI-05), the move its note tells of (AI-09) and the
    /// list of deferred MCP tools (AI-45), and returns the turn's task. A new conversation is
    /// created with its message.
    #[expect(
        clippy::too_many_arguments,
        reason = "the two commands that start a turn pass their own pieces"
    )]
    async fn start_turn(
        &self,
        vault: &SharedVault,
        env: Arc<dyn AiEnv>,
        conversation_id: Option<String>,
        replace: Option<&str>,
        text: String,
        context: AiTurnContext,
        sink: Arc<dyn EventSink>,
    ) -> AppResult<(AiSendStarted, TurnTask)> {
        if text.trim().is_empty() {
            return Err(AppError::invalid("text", "The message is empty."));
        }
        let server_id = turn_server_id(env.as_ref(), &context);
        let mut stored = false;
        let registered = self.open_turn(
            vault,
            conversation_id,
            replace,
            &text,
            (context, server_id),
            sink,
            &mut stored,
        );
        if stored {
            env.changed();
        }
        let (id, turn, notes, moved_to, came_from, created, edit) = registered?;
        // AI-45: the panel learns a new conversation's id before the servers start, so a stop
        // meanwhile names its turn, as for a conversation it knows.
        if created.is_some() {
            turn.emit(AiTurnEvent::Opened {
                conversation_id: id.clone(),
            });
        }
        let stopped = || {
            AppError::new(
                ErrorCode::Cancelled,
                "the message was stopped before it was sent",
            )
        };

        // AI-45: the servers the message's requests use start, and list their tools again after
        // `tools/list_changed`, before the message is stored, which decides whether it defers
        // them and lists them. A server that fails offers no tools, as for any request.
        let offer = self.0.mcp.offer(vault, &turn.context, &turn.cancel).await;

        // AI-26: an edit deletes its message and the entries after it now, so a stop while the
        // servers started leaves them. AI-22: whether the context needs a compaction before the
        // message, with the notes it would start with. A new conversation has nothing to compact.
        let mut deleted = false;
        let compact = match created {
            Some(_) => Ok((false, false)),
            None => (|| {
                let mut v = unlocked(vault)?;
                if turn.cancel.is_cancelled() {
                    return Err(stopped());
                }
                if let Some((entry_id, summary_before)) = &edit {
                    delete_edited(&mut v, &id, entry_id, summary_before.as_deref())?;
                    // The deleted entries held what the model read of files (AI-38…40).
                    self.0.seen.forget(&id);
                    deleted = true;
                }
                let conversation = find_conversation(&v, &id)?;
                let entries = Entries::load(&v, &id)?;
                let from = entries.context_from(conversation.context_start.as_deref());
                let listed = mcp_note(&v, &turn.context, &entries.list, from, &offer);
                // AI-45: the message that defers the MCP tools takes their definitions out of its
                // requests, which the last response's usage counts, and out of its Compact request.
                let defers = !listed.is_empty() && !mcp_deferred(&entries.list);
                let saved = if defers {
                    deferred_saving(&v, &turn.context, &self.0.mcp.latest(&id))
                } else {
                    0
                };
                let compact = compaction_before(
                    &v,
                    &id,
                    &turn.context,
                    &format!("{notes}{listed}{text}"),
                    saved,
                )?;
                Ok((compact, defers))
            })(),
        };
        if deleted {
            env.changed();
        }
        let (compact, defers) = match compact {
            Ok(decided) => decided,
            Err(e) => {
                turn.end(AiTurnEndReason::Stopped);
                self.forget(&id, &turn);
                return Err(e);
            }
        };

        // AI-22: the summary is stored before the message, so the message stays in the context.
        // A failed compaction is logged and the message goes anyway: the request may still fit,
        // and if it does not, the provider's error says so. A stop meanwhile is checked below.
        if compact
            && let Err(CompactFailure::Failed { .. }) = self
                .compact_before(vault, env.as_ref(), &id, &turn, defers)
                .await
        {
            tracing::warn!(conversation_id = %id, "AI message sent without compaction");
        }

        let started = (|| {
            let mut v = unlocked(vault)?;
            // Stopped, or replaced by a newer message, while the servers started or the context
            // was compacted: the message is not stored, and a new conversation is not created.
            // Checked under the guard a stop takes, like every write.
            if turn.cancel.is_cancelled() {
                return Err(stopped());
            }
            if let Some(conversation) = created {
                v.put(Some(&id), Item::AiConversation(conversation))?;
            }
            // AI-45: the list of deferred MCP tools, when the model has not seen it, read after a
            // compaction, whose summary leaves it out.
            let listed = {
                let conversation = find_conversation(&v, &id)?;
                let entries = Entries::load(&v, &id)?;
                let from = entries.context_from(conversation.context_start.as_deref());
                let listed = mcp_note(&v, &turn.context, &entries.list, from, &offer);
                let deferred = mcp_deferred(&entries.list);
                if !listed.is_empty() && !deferred {
                    tracing::info!(
                        conversation_id = %id,
                        tools = offer.listed().len(),
                        "AI conversation's MCP tools deferred"
                    );
                }
                // AI-45: once deferred, names resolve through the tools the model was told of,
                // which this message lists (or listed before, unchanged), until a search finds
                // others; the turn's requests change them no more.
                if deferred || !listed.is_empty() {
                    self.0.mcp.record_offer(&id, offer.clone());
                }
                listed
            };
            // AI-05, AI-09, AI-45: Hatoba's notes come before what the panel sent (spec §13.3).
            let text = format!("{notes}{listed}{text}");
            let entry = match came_from {
                Some(came_from) => AiEntry::user_from(now_ms(), text, came_from),
                None => AiEntry::user(now_ms(), text),
            };
            let entry_id = append(&mut v, &id, &entry)?;
            let mut conversation = find_conversation(&v, &id)?;
            let before = conversation.clone();
            // AI-05: the conversation keeps the level of its last message.
            conversation.effort = turn.context.effort.map(core_effort);
            if let Some(place) = moved_to {
                conversation.host_id = place.host_id;
                conversation.quick_target = place.quick_target;
                conversation.local = place.local;
                // AI-43: the move's note named the server a fork's copies came from.
                conversation.origin = None;
            }
            if conversation != before {
                v.put(Some(&id), Item::AiConversation(conversation.clone()))?;
            }
            Ok(AiSendStarted {
                conversation: conversation_view(&v, &id, &conversation),
                user_entry: entry_view(&entry_id, &entry),
            })
        })();
        let started = match started {
            Ok(started) => started,
            Err(e) => {
                // Stopped, locked or deleted meanwhile: the turn cannot go on.
                turn.end(AiTurnEndReason::Stopped);
                self.forget(&id, &turn);
                return Err(e);
            }
        };
        env.changed();
        if replace.is_some() {
            tracing::info!(conversation_id = %id, "AI message edited; turn started");
        } else {
            tracing::info!(conversation_id = %id, "AI turn started");
        }
        Ok((started, self.task(vault.clone(), env, id, turn, false)))
    }

    /// The first step of [`Self::start_turn`], under the vault guard: the stop of the
    /// conversation's running turn, an edit's deletion, the move to the tab's host or
    /// quick-connect target (AI-09), the notes Hatoba puts before the message (AI-05, AI-09), and
    /// the registered turn. Returns the conversation's id, the turn, the notes, the move, which is
    /// stored with the message, like the thinking level: a stop while the MCP servers start or the
    /// context is compacted stores no message, where the entries before the message came from,
    /// which the message records (AI-43), and a new conversation, which is created with its
    /// message under the id registered here, so such a stop leaves no empty conversation.
    #[expect(
        clippy::too_many_arguments,
        reason = "the pieces of start_turn, which owns them"
    )]
    fn open_turn(
        &self,
        vault: &SharedVault,
        conversation_id: Option<String>,
        replace: Option<&str>,
        text: &str,
        (context, server_id): (AiTurnContext, Option<String>),
        sink: Arc<dyn EventSink>,
        stored: &mut bool,
    ) -> AppResult<Opened> {
        let mut v = unlocked(vault)?;
        let mut cancelled = Vec::new();
        let (id, notes, moved_to, recorded, created, edit) = match conversation_id {
            Some(id) => {
                let conversation = find_conversation(&v, &id)?;
                self.not_reverting(&id)?;
                // Checked before the stop, so a refused edit leaves the turn running.
                let edit = match replace {
                    Some(entry_id) => Some(edited_entry(&v, &id, entry_id)?),
                    None => None,
                };
                cancelled = self.stop_locked(&mut v, &id);
                *stored |= !cancelled.is_empty();
                // The entries before the new message, and the host the context came from before
                // the entries an edit deletes, as their earliest host_change note names it.
                let Entries {
                    ids,
                    list: mut earlier,
                } = Entries::load(&v, &id)?;
                let mut came_from = None;
                // AI-43: where the entries before the message came from, which it records: where
                // the conversation is, or for an edit, what the edited message recorded.
                let mut recorded = Some(entry_place(&conversation));
                if let Some((entry_id, _)) = &edit {
                    recorded = None;
                    if let Some(at) = ids.iter().position(|e| e == entry_id) {
                        if let EntryBody::User {
                            came_from: edited, ..
                        } = &earlier[at].body
                        {
                            recorded.clone_from(edited);
                        }
                        came_from = earlier[at..].iter().find_map(|e| match &e.body {
                            EntryBody::User { text, .. } => note_attr(text, HOST_CHANGE, "from"),
                            _ => None,
                        });
                        earlier.truncate(at);
                    }
                    // What the stop stored came after the edited message, which goes with it
                    // once the MCP servers run ([`delete_edited`]).
                    cancelled.clear();
                }
                let noted = came_from.is_some();
                // AI-09: the next message moves the conversation to the tab's host, or to its
                // quick-connect target (HOST-12), which is no saved host, so the conversation
                // records the target instead. On the home tab it stays where it was.
                let target = tab_target(&context);
                let local = tab_local(&context);
                let moves = moves_to(
                    &conversation,
                    context.host_id.as_deref(),
                    target.as_ref(),
                    local.as_deref(),
                );
                let mut moved_to = None;
                if moves {
                    if came_from.is_none() {
                        came_from = moved_from(
                            &v,
                            &conversation,
                            context.host_id.as_deref(),
                            target.as_ref(),
                        );
                    }
                    moved_to = Some(Place {
                        host_id: context.host_id.clone(),
                        quick_target: target,
                        local,
                    });
                }
                let (host_id, quick_target, local) = match &moved_to {
                    Some(place) => (&place.host_id, &place.quick_target, &place.local),
                    None => (
                        &conversation.host_id,
                        &conversation.quick_target,
                        &conversation.local,
                    ),
                };
                // An edit keeps the move the deleted messages noted, unless the note names the
                // server the message goes from under its other name: a target and the saved host
                // with its address, port, and user.
                let here = host_id
                    .as_deref()
                    .and_then(|id| host_target(&v, id))
                    .or_else(|| quick_target.clone());
                if noted
                    && let (Some(from), Some(here)) = (&came_from, &here)
                    && names_server(&v, from, here)
                {
                    came_from = None;
                }
                let host_now = host_id
                    .as_deref()
                    .and_then(|id| host_name(&v, id))
                    .or_else(|| quick_target.as_ref().map(target_name))
                    .or_else(|| local.as_deref().map(local_name));
                let reverts = pending_reverts(&v, &id, &ids[..earlier.len()], &earlier);
                let notes = notes_before(
                    &v,
                    &earlier,
                    came_from.as_deref(),
                    host_now.as_deref(),
                    &context,
                    &reverts,
                );
                (id, notes, moved_to, recorded, None, edit)
            }
            None => {
                let created = AiConversation {
                    title: title_of(text),
                    host_id: context.host_id.clone(),
                    quick_target: tab_target(&context),
                    local: tab_local(&context),
                    origin: None,
                    pinned: false,
                    context_start: None,
                    effort: context.effort.map(core_effort),
                    created_at: now_ms(),
                    updated_at: 0,
                };
                let id = hatoba_core::new_id();
                let host_now = context
                    .host_id
                    .as_deref()
                    .and_then(|id| host_name(&v, id))
                    .or_else(|| created.quick_target.as_ref().map(target_name))
                    .or_else(|| created.local.as_deref().map(local_name));
                let notes = notes_before(&v, &[], None, host_now.as_deref(), &context, &[]);
                (
                    id,
                    notes,
                    None,
                    Some(EntryPlace::default()),
                    Some(created),
                    None,
                )
            }
        };
        let turn = self.register(&id, context, server_id, sink);
        // The panel stopped listening to the previous turn's channel, so the entries the stop
        // stored reach it on this one.
        for entry in cancelled {
            turn.emit(AiTurnEvent::Entry { entry });
        }
        Ok((id, turn, notes, moved_to, recorded, created, edit))
    }

    /// `ai_retry`: sends the next request from the stored conversation on a new channel. A
    /// running turn of the conversation stops first. A conversation that ends with the model's
    /// answer has nothing to retry: the request would end with an assistant message, which
    /// newer models reject as a prefill.
    ///
    /// AI-22: the turn first compacts a context that would pass 90% of the model's context
    /// window, as [`Self::send`] does before a new message (retrying with a model whose window
    /// is smaller, say), unless the context ends with a summary, so it never compacts twice in a
    /// row. The messages being retried are stored already and a summary can only follow them, so
    /// they are part of what it summarizes.
    pub fn retry(
        &self,
        vault: &SharedVault,
        env: Arc<dyn AiEnv>,
        conversation_id: String,
        context: AiTurnContext,
        sink: Arc<dyn EventSink>,
    ) -> AppResult<TurnTask> {
        let server_id = turn_server_id(env.as_ref(), &context);
        let mut stored = false;
        let registered = (|| {
            let mut v = unlocked(vault)?;
            let mut conversation = find_conversation(&v, &conversation_id)?;
            self.not_reverting(&conversation_id)?;
            let cancelled = self.stop_locked(&mut v, &conversation_id);
            stored = !cancelled.is_empty();
            let entries = Entries::load(&v, &conversation_id)?;
            let from = entries.context_from(conversation.context_start.as_deref());
            if nothing_to_send(&entries.list[from..]) {
                return Err(AppError::invalid(
                    "conversation_id",
                    "nothing to retry: the conversation ends with the model's answer",
                ));
            }
            // AI-05: a retry at another level is the one the conversation used last, as for a
            // new message.
            let effort = context.effort.map(core_effort);
            if conversation.effort != effort {
                conversation.effort = effort;
                v.put(
                    Some(&conversation_id),
                    Item::AiConversation(conversation.clone()),
                )?;
                stored = true;
            }
            let compact = compaction_before(&v, &conversation_id, &context, "", 0)?;
            let turn = self.register(&conversation_id, context, server_id, sink);
            for entry in cancelled {
                turn.emit(AiTurnEvent::Entry { entry });
            }
            Ok((turn, compact))
        })();
        if stored {
            env.changed();
        }
        let (turn, compact) = registered?;
        tracing::info!(conversation_id = %conversation_id, "AI turn retried");
        Ok(self.task(vault.clone(), env, conversation_id, turn, compact))
    }

    /// The turn's task: a compaction first when `compact` says the context needs one (AI-22),
    /// then the turn.
    fn task(
        &self,
        vault: SharedVault,
        env: Arc<dyn AiEnv>,
        conversation_id: String,
        turn: Arc<Turn>,
        compact: bool,
    ) -> TurnTask {
        let manager = self.clone();
        Box::pin(async move {
            let compacted = if compact {
                manager
                    .compact_before(&vault, env.as_ref(), &conversation_id, &turn, false)
                    .await
            } else {
                Ok(())
            };
            let reason = match compacted {
                Err(CompactFailure::Stopped) => AiTurnEndReason::Stopped,
                // As for a new message: the request may still fit, and if it does not, the
                // provider's error says so.
                Ok(()) | Err(CompactFailure::Failed { .. }) => {
                    if compacted.is_err() {
                        tracing::warn!(
                            conversation_id = %conversation_id,
                            "AI turn retried without compaction"
                        );
                    }
                    manager
                        .turn_loop(&vault, env.as_ref(), &conversation_id, &turn)
                        .await
                }
            };
            turn.finish(reason);
            manager.forget(&conversation_id, &turn);
            tracing::info!(conversation_id = %conversation_id, ?reason, "AI turn ended");
        })
    }

    /// The turn (§13.1 steps 2 and 3): request, stream, store; with tool calls, wait for their
    /// results and go again.
    async fn turn_loop(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        conversation_id: &str,
        turn: &Turn,
    ) -> AiTurnEndReason {
        loop {
            // AI-30: start the servers whose tools this request offers.
            let offer = self.0.mcp.offer(vault, &turn.context, &turn.cancel).await;
            let request = match prepare(vault, env, conversation_id, turn, &offer) {
                Ok(request) => request,
                Err(halt) => return halted(turn, conversation_id, halt),
            };
            // Every call so far has a result now, so no approval card waits on a read or a verdict.
            self.forget_file_bases(conversation_id);
            self.forget_verdicts(conversation_id);
            if compaction_due(&request) {
                let compact = {
                    let v = lock(vault);
                    if turn.cancel.is_cancelled() || !v.is_unlocked() {
                        return AiTurnEndReason::Stopped;
                    }
                    // The request the turn was about to send, asking where the task stands.
                    SideRequest::compact(&v, &turn.context, request, COMPACT_MID_TURN_INSTRUCTION)
                };
                match self
                    .auto_compact(vault, env, conversation_id, turn, compact)
                    .await
                {
                    // The next request starts at the summary.
                    Ok(()) => continue,
                    Err(CompactFailure::Stopped) => return AiTurnEndReason::Stopped,
                    Err(CompactFailure::Failed { status, message }) => {
                        turn.emit(AiTurnEvent::Error { status, message });
                        return AiTurnEndReason::Error;
                    }
                }
            }
            // Calls of this response map back through the names this request offered, or, once
            // the conversation's MCP tools are deferred (AI-45), through the names the model was
            // told of: by the newest message's list or the latest search, which record their own,
            // so a name the model read keeps meaning that tool when servers change mid-turn. A
            // retry after a restart or the lock, which forget them, records this request's, so
            // names never resolve through every server running on the device.
            if !offers_deferred(&request.tools) || !self.0.mcp.has_offer(conversation_id) {
                self.0.mcp.record_offer(conversation_id, offer);
            }
            turn.emit(AiTurnEvent::RequestStarted);
            let http = self.http();
            *guard(&turn.streamed) = Some(Streamed {
                provider_id: request.provider_id.clone(),
                model_id: request.model.id.clone(),
                text: String::new(),
                tool_calls: Vec::new(),
            });
            let result = {
                let mut forward = |event: StreamEvent| {
                    if let Some(streamed) = guard(&turn.streamed).as_mut() {
                        match &event {
                            StreamEvent::Text(delta) => streamed.text.push_str(delta),
                            StreamEvent::ToolCall(call) => streamed.tool_calls.push(call.clone()),
                            _ => {}
                        }
                    }
                    turn.emit(stream_event(event));
                };
                stream_chat(
                    &http,
                    &request.provider,
                    &request.chat(),
                    &turn.cancel,
                    &mut forward,
                )
                .await
            };
            // A stop took what had streamed before it cancelled the request; a failed request
            // stores nothing. A complete response stays until it is stored, so a stop before that
            // keeps it.
            if result.is_err() {
                guard(&turn.streamed).take();
            }
            let response = match result {
                Ok(response) => response,
                Err(_) if turn.cancel.is_cancelled() => return AiTurnEndReason::Stopped,
                Err(AiError::Cancelled) => return AiTurnEndReason::Stopped,
                Err(e) => {
                    tracing::warn!(
                        conversation_id,
                        kind = error_kind(&e),
                        status = ?e.status(),
                        "AI request failed"
                    );
                    turn.emit(AiTurnEvent::Error {
                        status: e.status(),
                        message: error_text(&e),
                    });
                    return AiTurnEndReason::Error;
                }
            };
            let finish = response.finish;
            // The frontend handles calls only after `done { tool_calls }`; calls of a response that
            // was cut off or declined get cancelled results before the next request.
            let wants_results = finish == Finish::ToolCalls && !response.tool_calls.is_empty();
            // Neither text nor calls, such as a reply of reasoning only: no answer (§13.1).
            let empty = response.text.trim().is_empty() && response.tool_calls.is_empty();
            let entry_id = match store_response(vault, env, conversation_id, turn, response) {
                Ok(entry_id) => entry_id,
                Err(halt) => return halted(turn, conversation_id, halt),
            };
            if !wants_results {
                return match finish {
                    Finish::Length => AiTurnEndReason::Length,
                    Finish::Refused => AiTurnEndReason::Refused,
                    Finish::Stop | Finish::ToolCalls if empty => AiTurnEndReason::Empty,
                    Finish::Stop | Finish::ToolCalls => AiTurnEndReason::Completed,
                };
            }
            if let Err(halt) = wait_for_results(vault, conversation_id, turn, &entry_id).await {
                return halted(turn, conversation_id, halt);
            }
        }
    }

    // ---- tool results ----

    /// `ai_tool_result`: stores a result the frontend produced (`read_terminal`, `send_input`, a
    /// rejection). The call must belong to the conversation's newest response, have no result,
    /// and not be running in Rust. With `judged`, the result keeps the judge's verdict (AI-47).
    /// It is stored once every earlier call of the response has a result
    /// ([`Self::in_call_order`]).
    pub async fn tool_result(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        conversation_id: &str,
        tool_call_id: &str,
        input: &AiToolResultInput,
    ) -> AppResult<AiEntryView> {
        let status = core_status(input.status);
        let content = match status {
            // The user's reason; the adapter tells the model the call was rejected.
            ToolStatus::Rejected => input.content.trim().to_owned(),
            _ => result_content(
                status,
                tools::truncate_result(&input.content),
                input.edited_arguments.as_deref(),
            ),
        };
        let cancel = self.turn_token(conversation_id);
        let view = self
            .in_call_order(
                vault,
                conversation_id,
                tool_call_id,
                &cancel,
                |entries| {
                    open_call(entries, tool_call_id)?;
                    if self.call_running(conversation_id, tool_call_id) {
                        return Err(AppError::invalid(
                            "tool_call_id",
                            "the tool call is already running",
                        ));
                    }
                    Ok(None)
                },
                |v| {
                    self.store_result(
                        v,
                        conversation_id,
                        tool_call_id,
                        status,
                        content,
                        input.judged,
                    )
                },
            )
            .await?;
        env.changed();
        Ok(view)
    }

    /// `ai_tool_run`: runs a tool that runs in Rust and stores its result. Every failure of the
    /// tool itself is an `error` result; only a precondition (locked, unknown conversation or
    /// call, a call that already has a result or is running) rejects. With `judged`, the result
    /// keeps the judge's verdict (AI-47).
    #[expect(
        clippy::too_many_arguments,
        reason = "the arguments of `ai_tool_run`, with the shared vault and the app"
    )]
    pub async fn tool_run(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        conversation_id: &str,
        tool_call_id: &str,
        session_id: Option<&str>,
        edited_arguments: Option<&str>,
        judged: bool,
    ) -> AppResult<AiToolRun> {
        let (job, op, place, _writing, keeps) = {
            let v = unlocked(vault)?;
            let conversation = find_conversation(&v, conversation_id)?;
            let entries = Entries::load(&v, conversation_id)?;
            let call = open_call(&entries, tool_call_id)?;
            let arguments = edited_arguments.unwrap_or(&call.arguments);
            let mut job = if call.name.starts_with(MCP_PREFIX) {
                Job::mcp(
                    &v,
                    self.0.mcp.offered(conversation_id, &call.name),
                    &call.name,
                    arguments,
                )
            } else if call.name == tools::CALL_MCP_TOOL {
                match proxy_call(&v, &self.0.mcp, conversation_id, arguments) {
                    Ok((tool, arguments)) => Job::Mcp(Box::new(tool), arguments),
                    Err(refusal) => Job::error(refusal),
                }
            } else {
                let (at, _) = open_call_at(&entries, tool_call_id)?;
                let caller = match &entries.list[at].body {
                    EntryBody::Assistant(response) => {
                        (response.provider_id.as_str(), response.model_id.as_str())
                    }
                    _ => ("", ""),
                };
                Job::new(&v, &call.name, arguments, &env.app_version(), caller)
            };
            let kind = if matches!(job, Job::RunCommand(_)) {
                OpKind::Command
            } else {
                OpKind::Other
            };
            let op = self.start_call(conversation_id, tool_call_id, kind)?;
            let mut place = files::Place::default();
            // AI-41: snapshots are kept by call id, so a call whose id another call of its
            // response shares (stored by an older build) keeps none: neither block could tell
            // whose change it holds.
            let keeps = !shares_id(&entries, tool_call_id);
            // Counted under the guard a lock takes first, so the lock sees every write.
            let writing = session_id
                .filter(|_| matches!(job, Job::File(FileJob::Edit(_) | FileJob::Write(_), _)))
                .map(|id| self.0.writes.begin(id));
            if let Job::File(_, base) = &mut job {
                *base = guard(&self.0.file_bases)
                    .remove(&(conversation_id.to_owned(), tool_call_id.to_owned()));
                place = file_place(&conversation, &entries);
            }
            (job, op, place, writing, keeps)
        };
        let shown = Mutex::default();
        let keep = KeepSnapshot::new(
            vault,
            SnapshotKey {
                conversation_id,
                response_id: &place.response,
                tool_call_id,
            },
        );
        let seen = files::SeenBy {
            seen: &self.0.seen,
            conversation_id,
            place: &place,
            shown: &shown,
            keep: keeps.then_some(&keep as &dyn files::Keep),
        };
        let (status, content) = match job {
            Job::SearchTools(args) => {
                self.search_tools(vault, conversation_id, &args, &op.cancel)
                    .await
            }
            Job::NativeSearch(native, query) => {
                let (provider, model) = *native;
                self.native_search(&provider, &model, &query, &op.cancel)
                    .await
            }
            job => {
                job.run(
                    &self.http(),
                    &self.0.mcp,
                    vault,
                    env,
                    session_id,
                    seen,
                    &op.cancel,
                )
                .await
            }
        };
        let ran = Ran {
            status,
            content: result_content(status, content, edited_arguments),
            shown: guard(&shown)
                .take()
                .map(|file| (place.response.clone(), file)),
        };
        self.ran(&op, ran);
        // The tool's channel is closed: while its result waits for the earlier calls', the panel
        // may start another call that opens one (§13.1).
        if let Some(turn) = self.running(conversation_id) {
            turn.emit(AiTurnEvent::ToolRan {
                tool_call_id: tool_call_id.to_owned(),
            });
        }
        let stopped = || AppError::new(ErrorCode::Cancelled, "the tool call was stopped");
        let view = self
            .in_call_order(
                vault,
                conversation_id,
                tool_call_id,
                &op.cancel,
                |entries| {
                    let Err(e) = open_call(entries, tool_call_id) else {
                        return Ok(None);
                    };
                    // A stop answered the call: with the tool's result when it took it, else
                    // with a cancelled one while the tool ran.
                    if self.take_ran(&op).is_none()
                        && let Some(view) = entries.newest_results(&[tool_call_id.to_owned()]).pop()
                    {
                        return Ok(Some(view));
                    }
                    Err(if op.cancel.is_cancelled() {
                        stopped()
                    } else {
                        e
                    })
                },
                |v| {
                    let Ran {
                        status,
                        content,
                        shown,
                    } = self.take_ran(&op).ok_or_else(stopped)?;
                    let view = self.store_result(
                        v,
                        conversation_id,
                        tool_call_id,
                        status,
                        content,
                        judged,
                    )?;
                    // The model sees the file now that the result is stored (AI-38…40), under
                    // the guard that a stop or the lock would have taken first.
                    if let Some((response, file)) = shown {
                        self.0.seen.insert(conversation_id, &response, file);
                    }
                    Ok(view)
                },
            )
            .await;
        drop(op);
        let view = view?;
        env.changed();
        Ok(AiToolRun {
            entry: view,
            dropped_file_changes: keep.dropped(),
        })
    }

    /// Stores the result of the open call `tool_call_id` with `store`, in the order of the calls
    /// (§13.1): while the conversation's turn runs and is not being stopped (`cancel`), only once
    /// every earlier call of its response has a result, whichever finishes first. Before that it
    /// waits, and is woken when a result of the turn is stored or the turn ends. `check` runs
    /// first each time, under the same vault guard, with the entries from the response on: an
    /// error or a value ends the wait without storing.
    async fn in_call_order<T>(
        &self,
        vault: &SharedVault,
        conversation_id: &str,
        tool_call_id: &str,
        cancel: &CancellationToken,
        mut check: impl FnMut(&Entries) -> AppResult<Option<T>>,
        store: impl FnOnce(&mut Vault) -> AppResult<T>,
    ) -> AppResult<T> {
        // Once read, the response the call belongs to: a wait reads it and what follows it.
        let mut response = None;
        loop {
            let turn;
            let mut waiting;
            {
                let mut v = unlocked(vault)?;
                find_conversation(&v, conversation_id)?;
                let entries = Entries::load_open(&v, conversation_id, response.as_deref())?;
                if let Some(done) = check(&entries)? {
                    return Ok(done);
                }
                // Results are stored under the guard, so none is stored before the wait is
                // registered; the turn, which ends without it, is checked again after.
                turn = match self.running(conversation_id) {
                    Some(turn) if !cancel.is_cancelled() => turn,
                    _ => return store(&mut v),
                };
                waiting = Box::pin(turn.waiting.notified());
                waiting.as_mut().enable();
                if self.running(conversation_id).is_none()
                    || !earlier_call_open(&entries, tool_call_id)?
                {
                    return store(&mut v);
                }
                response = entries.newest_response();
            }
            tokio::select! {
                () = cancel.cancelled() => {}
                () = waiting => {}
            }
        }
    }

    /// `web_search` with the search of the model's own provider (AI-14). A provider of no known
    /// search ([`NativeSearch::Other`]) that turns it away gets the same result for later calls
    /// with its base URL and model, without a request, until the app quits.
    async fn native_search(
        &self,
        provider: &ProviderConfig,
        model: &ModelSpec,
        query: &str,
        cancel: &CancellationToken,
    ) -> (ToolStatus, String) {
        let tried = NativeSearch::of(provider) == Some(NativeSearch::Other);
        let key = (provider.base_url.trim().to_owned(), model.id.clone());
        if tried && let Some(miss) = guard(&self.0.native_misses).get(&key) {
            return (ToolStatus::Error, miss.clone());
        }
        match web::native_search(&self.http(), provider, model, query, cancel).await {
            Ok(found) => (
                ToolStatus::Ok,
                tools::truncate_result(&web::format_native_results(&found)),
            ),
            Err(AiError::Cancelled) => (ToolStatus::Cancelled, String::new()),
            Err(e) if tried && web::turned_away(&e) => {
                let miss = format!(
                    "The search failed: {e}. The model's provider does not seem to offer web \
                     search; a search provider chosen in Settings → AI works with any model."
                );
                guard(&self.0.native_misses).insert(key, miss.clone());
                (ToolStatus::Error, miss)
            }
            Err(e) => (ToolStatus::Error, format!("The search failed: {e}")),
        }
    }

    /// `search_tools` (AI-45): searches the MCP tools the running turn's requests use, refreshed
    /// as for a request first, so a server that sent `tools/list_changed` during the turn is
    /// relisted before the search, and keeps them as the conversation's latest tools, so a call
    /// of a tool the search found resolves to it. Without a running turn, which a call of the
    /// turn's newest response always has, the conversation's latest tools.
    async fn search_tools(
        &self,
        vault: &SharedVault,
        conversation_id: &str,
        args: &deferred::SearchArgs,
        cancel: &CancellationToken,
    ) -> (ToolStatus, String) {
        let offer = match self.running(conversation_id) {
            Some(turn) => {
                let offer = self.0.mcp.offer(vault, &turn.context, cancel).await;
                if cancel.is_cancelled() {
                    return (ToolStatus::Cancelled, String::new());
                }
                self.0.mcp.record_offer(conversation_id, offer.clone());
                Arc::new(offer)
            }
            None => self.0.mcp.latest(conversation_id),
        };
        let listed = offer.listed();
        let matches = deferred::search(&listed, &args.query, args.max_results);
        tracing::info!(
            conversation_id,
            tools = listed.len(),
            matches = matches.len(),
            "MCP tools searched"
        );
        (
            ToolStatus::Ok,
            tools::truncate_result(&deferred::search_result(&listed, &matches, &args.query)),
        )
    }

    /// `ai_file_preview` (AI-39, AI-40): the change an open `edit_file` or `write_file` call
    /// would make, for its approval card, with `edited_arguments` when the user edits the call on
    /// the card. The file is read once per call; what was read is kept for the run, which writes
    /// only over that content. The read is an op of the conversation, so a stop or the lock ends
    /// it.
    pub async fn file_preview(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        conversation_id: &str,
        tool_call_id: &str,
        session_id: Option<&str>,
        edited_arguments: Option<&str>,
    ) -> AppResult<AiFilePreview> {
        let (call, op, place) = {
            let v = unlocked(vault)?;
            let conversation = find_conversation(&v, conversation_id)?;
            let entries = Entries::load(&v, conversation_id)?;
            let call = open_call(&entries, tool_call_id)?;
            let place = file_place(&conversation, &entries);
            (call, self.start_op(conversation_id), place)
        };
        let (job, path) =
            match files::preview_job(&call.name, edited_arguments.unwrap_or(&call.arguments)) {
                Ok(job) => job,
                Err(preview) => return Ok(preview),
            };
        let key = (conversation_id.to_owned(), tool_call_id.to_owned());
        let cached = guard(&self.0.file_bases)
            .get(&key)
            .filter(|b| b.path == path)
            .cloned();
        let base = match cached {
            Some(base) => base,
            None => {
                let read = match files::read_base(live_session(env, session_id), &path, op.token())
                    .await
                {
                    Ok(base) => base,
                    Err(files::Stop::Failed(message) | files::Stop::Unfinished(message)) => {
                        return Ok(files::failed_preview(&path, message));
                    }
                    Err(files::Stop::Cancelled) => {
                        return Err(AppError::new(
                            ErrorCode::Cancelled,
                            "the preview was stopped",
                        ));
                    }
                };
                let v = unlocked(vault)?;
                let waiting = open_call(&Entries::load(&v, conversation_id)?, tool_call_id).is_ok()
                    && !self.call_running(conversation_id, tool_call_id);
                let mut bases = guard(&self.0.file_bases);
                match bases.get(&key) {
                    // Previews can overlap (Edit while the first read runs): the read kept first
                    // stays, so every diff the card shows is against the content the run checks.
                    Some(kept) if kept.path == path => kept.clone(),
                    // Kept only while the call waits: not after its result, a stop or the lock.
                    _ if waiting => {
                        bases.insert(key, read.clone());
                        read
                    }
                    _ => read,
                }
            }
        };
        drop(op);
        let seen = files::SeenBy {
            seen: &self.0.seen,
            conversation_id,
            place: &place,
            // A preview shows the model nothing, and writes nothing.
            shown: &Mutex::default(),
            keep: None,
        };
        Ok(files::preview_of(&job, &base, seen))
    }

    // ---- the judge of Auto mode (AI-47) ----

    /// `ai_judge` (AI-47): asks the judge of Auto mode about the open call `tool_call_id`, one
    /// that manual approval would ask about, and returns its verdict: run the call without the
    /// approval card, or ask. The judge reads what the user said and what the calls do
    /// ([`judge_state`]); `screen_tail` is the screen's last lines, which a `send_input` call
    /// shows it. Without a judge in `Settings.ai`, or with one whose provider or model is gone,
    /// the verdict asks as `unavailable` and nothing is sent; so does one cleared, replaced or
    /// gone while it was asked, whose verdict no longer counts. The request is an op of the
    /// conversation, so a stop or the lock ends it, and the verdict asks as `cancelled`. At most
    /// [`JUDGE_REQUESTS`] requests run at once across conversations; a call waits for room, and a
    /// stop or the lock ends the wait the same way.
    ///
    /// The verdict is kept, in place of an earlier one, until the call's result is stored, which
    /// carries it when Auto decided the call ([`Self::store_result`]). A call that got its result
    /// while the judge was asked keeps none.
    pub async fn judge(
        &self,
        vault: &SharedVault,
        conversation_id: &str,
        tool_call_id: &str,
        screen_tail: Option<&str>,
    ) -> AppResult<AiAutoVerdict> {
        let (chosen, asked) = {
            let v = unlocked(vault)?;
            let conversation = find_conversation(&v, conversation_id)?;
            let entries = Entries::load(&v, conversation_id)?;
            let (at, index) = open_call_at(&entries, tool_call_id)?;
            let chosen = v.settings().ai.judge_model;
            let asked = chosen
                .as_ref()
                .and_then(|m| resolve_model(&v, &m.provider_id, &m.model_id).ok())
                .map(|(provider, model)| {
                    let state = judge_state(
                        &v,
                        &self.0.mcp,
                        conversation_id,
                        &conversation,
                        &entries.list,
                        (at, index),
                        screen_tail,
                    );
                    (provider, model, state, self.start_op(conversation_id))
                });
            // No judge names no provider or model.
            (chosen.unwrap_or_default(), asked)
        };
        let (verdict, op) = match asked {
            None => (Verdict::failed(JudgeFailure::Unavailable), None),
            Some((provider, model, state, op)) => {
                let room = tokio::select! {
                    biased;
                    () = op.token().cancelled() => None,
                    room = self.0.judge_slots.0.acquire() => room.ok(),
                };
                let verdict = match room {
                    Some(_room) => {
                        judge::judge(&self.http(), &provider, &model, state, op.token()).await
                    }
                    // Stopped while it waited; the slots are never closed.
                    None => Verdict::failed(JudgeFailure::Cancelled),
                };
                (verdict, Some(op))
            }
        };
        let (verdict, auto) = {
            let v = unlocked(vault)?;
            find_conversation(&v, conversation_id)?;
            // The judge the user chose now, which may have changed while it was asked.
            let current =
                v.settings().ai.judge_model.filter(|m| {
                    *m == chosen && resolve_model(&v, &m.provider_id, &m.model_id).is_ok()
                });
            let verdict = if current.is_some() {
                verdict
            } else {
                Verdict::failed(JudgeFailure::Unavailable)
            };
            let auto = verdict.to_auto(&chosen.provider_id, &chosen.model_id);
            // Kept only while the call waits: not after its result, a stop or the lock.
            if open_call_at(&Entries::load(&v, conversation_id)?, tool_call_id).is_ok() {
                guard(&self.0.pending_verdicts).insert(
                    (conversation_id.to_owned(), tool_call_id.to_owned()),
                    auto.clone(),
                );
            }
            (verdict, auto)
        };
        drop(op);
        tracing::info!(
            conversation_id,
            decision = ?verdict.decision,
            failure = verdict.failure.map(JudgeFailure::as_str),
            "AI call judged"
        );
        Ok(auto_view(&auto))
    }

    /// `ai_judge_test` (AI-47): **Test** beside the judge in Settings → AI. Judges the sample
    /// calls of [`judge::samples`] with `judge`, the model the pane shows before it is saved, or
    /// with the saved judge when that is `None`. The model must be listed by its provider, of any
    /// protocol. The requests are a background op, so the lock ends them and the outcomes ask as
    /// `cancelled`. Only how many came out as expected and the kinds of failure are logged.
    pub async fn judge_test(
        &self,
        vault: &SharedVault,
        judge: Option<AiModelRef>,
    ) -> AppResult<Vec<AiJudgeSampleOutcome>> {
        let (chosen, provider, model, op) = {
            let v = unlocked(vault)?;
            let chosen = judge
                .map(|m| CoreModelRef {
                    provider_id: m.provider_id,
                    model_id: m.model_id,
                })
                .or_else(|| v.settings().ai.judge_model)
                .ok_or_else(|| AppError::invalid("judge_model", "no judge model is chosen"))?;
            let (provider, model) = resolve_model(&v, &chosen.provider_id, &chosen.model_id)
                .map_err(|e| AppError::invalid("judge_model", e))?;
            (chosen, provider, model, self.background_op())
        };
        let outcomes = judge::test_judge(&self.http(), &provider, &model, op.token()).await;
        drop(op);
        let as_expected = outcomes
            .iter()
            .filter(|o| o.verdict.failure.is_none() && o.verdict.decision == o.expected)
            .count();
        let failures: BTreeSet<&str> = outcomes
            .iter()
            .filter_map(|o| o.verdict.failure.map(JudgeFailure::as_str))
            .collect();
        tracing::info!(
            samples = outcomes.len(),
            as_expected,
            ?failures,
            "AI judge tested"
        );
        Ok(outcomes
            .into_iter()
            .map(|o| AiJudgeSampleOutcome {
                name: o.name,
                expected: decision_view(o.expected),
                verdict: auto_view(&o.verdict.to_auto(&chosen.provider_id, &chosen.model_id)),
            })
            .collect())
    }

    /// Returns once no `edit_file` or `write_file` call runs on the SSH session: a write that has
    /// begun runs to its end, within 30 seconds, so a lock closes the session after it (§13.4,
    /// "File tools").
    pub async fn file_writes_ended(&self, session_id: &str) {
        self.0.writes.ended(session_id).await;
    }

    /// `ai_file_revert` (AI-41): reverts the change the call of `key` made, on the conversation's
    /// tab `session_id`, while the conversation is idle. With `restore` (Restore anyway), it
    /// writes over what the diff of the last revert showed. The read is an op of the
    /// conversation, which the lock ends; a write that has begun runs to its end, and a lock that
    /// disconnects SSH sessions waits for it. A revert ends the model's record of the file, and
    /// the next message notes it.
    pub async fn file_revert(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        key: SnapshotKey<'_>,
        session_id: Option<&str>,
        restore: bool,
    ) -> AppResult<AiFileRevert> {
        let conversation_id = key.conversation_id;
        let base_key = (
            conversation_id.to_owned(),
            key.response_id.to_owned(),
            key.tool_call_id.to_owned(),
        );
        let (change, later_writes, op, _running) = {
            let v = unlocked(vault)?;
            find_conversation(&v, conversation_id)?;
            if self.busy(conversation_id) {
                return Err(AppError::invalid(
                    "conversation_id",
                    "The conversation is busy; revert the change once it is idle.",
                ));
            }
            let Some(stored) = v.ai_file_snapshot(&key)? else {
                return Err(AppError::not_found("file change"));
            };
            let content = match (stored.state, stored.content) {
                (SnapshotState::Reverted, _) => {
                    return Err(AppError::invalid(
                        "tool_call_id",
                        "the change was reverted already",
                    ));
                }
                (state, Some(content)) if state.revertible() => content,
                _ => {
                    return Err(AppError::invalid(
                        "tool_call_id",
                        "the change can no longer be reverted: the snapshot limit dropped it",
                    ));
                }
            };
            let unreadable = |_| AppError::internal("unreadable file snapshot");
            let change = files::Change {
                meta: serde_json::from_str(&stored.meta).map_err(unreadable)?,
                content: serde_json::from_str(&content).map_err(unreadable)?,
            };
            let later = later_writes(&v, &key, &change.meta);
            // Counted under the guard a lock takes first, so the lock sees the write.
            let writing = session_id.map(|id| self.0.writes.begin(id));
            // Under the guard a turn takes to start too: none starts until the revert ends.
            let reverting = Reverting::new(&self.0, conversation_id);
            (
                change,
                later,
                self.start_op(conversation_id),
                (writing, reverting),
            )
        };
        let shown = if restore {
            guard(&self.0.restore_bases).get(&base_key).copied()
        } else {
            None
        };
        let Some(files::Revert { mut outcome, found }) =
            files::revert(live_session(env, session_id), &change, shown, op.token()).await
        else {
            return Err(AppError::new(
                ErrorCode::Cancelled,
                "the revert was stopped",
            ));
        };
        match (&mut outcome, found) {
            (AiFileRevert::Reverted { wrote }, _) => {
                guard(&self.0.restore_bases).remove(&base_key);
                // The file changed on the host either way, so the model reads it again.
                self.0
                    .seen
                    .forget_file(conversation_id, &change.meta.server, &change.meta.path);
                // Success is reported only once recorded, so the next message notes it. Reverting
                // again finds the file as it held before the call, and records it then.
                if let Err(e) = unlocked(vault).and_then(|mut v| Ok(v.ai_mark_file_reverted(&key)?))
                {
                    tracing::warn!(
                        conversation_id,
                        code = ?e.code,
                        "could not record a reverted file change"
                    );
                    return Err(e);
                }
                tracing::info!(conversation_id, wrote = *wrote, "AI file change reverted");
            }
            (AiFileRevert::Changed { later, .. }, Some(found)) => {
                *later = later_writes;
                guard(&self.0.restore_bases).insert(base_key, found);
            }
            _ => {}
        }
        Ok(outcome)
    }

    /// Refuses to start a turn while a revert of the conversation runs (AI-41).
    fn not_reverting(&self, conversation_id: &str) -> AppResult<()> {
        if guard(&self.0.reverts).contains(conversation_id) {
            return Err(AppError::invalid(
                "conversation_id",
                "A file change of this conversation is being reverted; send the message once \
                 the revert has ended.",
            ));
        }
        Ok(())
    }

    /// A turn, a tool, a preview, a compaction or a revert of the conversation runs.
    fn busy(&self, conversation_id: &str) -> bool {
        self.is_running(conversation_id)
            || guard(&self.0.ops)
                .values()
                .any(|op| op.conversation_id.as_deref() == Some(conversation_id))
    }

    /// Drops what the approval cards of the conversation read (AI-39): none of its calls waits for
    /// one any more.
    fn forget_file_bases(&self, conversation_id: &str) {
        guard(&self.0.file_bases).retain(|(id, _), _| id != conversation_id);
    }

    /// Drops the judge's verdicts on the conversation's calls (AI-47): none of them waits any more.
    fn forget_verdicts(&self, conversation_id: &str) {
        guard(&self.0.pending_verdicts).retain(|(id, _), _| id != conversation_id);
    }

    /// Stores a tool result, sends it on the running turn's channel and wakes the turn. The
    /// judge's verdict on the call (AI-47) goes with the result when Auto decided the call
    /// (`judged`), and is dropped otherwise.
    fn store_result(
        &self,
        v: &mut Vault,
        conversation_id: &str,
        tool_call_id: &str,
        status: ToolStatus,
        content: String,
        judged: bool,
    ) -> AppResult<AiEntryView> {
        let key = (conversation_id.to_owned(), tool_call_id.to_owned());
        let mut entry = AiEntry::tool(now_ms(), tool_call_id, status, content);
        if judged && let Some(verdict) = guard(&self.0.pending_verdicts).get(&key).cloned() {
            entry = entry.with_auto(verdict);
        }
        let entry_id = append(v, conversation_id, &entry)?;
        guard(&self.0.pending_verdicts).remove(&key);
        guard(&self.0.file_bases).remove(&key);
        let view = entry_view(&entry_id, &entry);
        if let Some(turn) = self.running(conversation_id) {
            turn.emit(AiTurnEvent::Entry {
                entry: view.clone(),
            });
            turn.results.notify_one();
            turn.waiting.notify_waiters();
        }
        tracing::info!(conversation_id, ?status, "AI tool result stored");
        Ok(view)
    }

    // ---- compaction ----

    /// `ai_compact` (AI-21): asks the model for a summary of the context with the request a turn
    /// would send next ([`compact_request`]), stores it as a `summary` entry and moves
    /// `context_start` to it. Refused while a turn runs. The conversation counts as compacting
    /// until the summary is stored, which refuses a side question (AI-42). `instructions`: the
    /// user's own for the summary, which `/compact <instructions>` gives (AI-50).
    pub async fn compact(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        conversation_id: &str,
        context: &AiTurnContext,
        instructions: Option<&str>,
    ) -> AppResult<AiEntryView> {
        let mut stored = false;
        let prepared = self.prepare_compact(vault, conversation_id, context, &mut stored);
        if stored {
            env.changed();
        }
        let op = prepared?;
        // AI-30: the MCP tools the next request offers, so the Compact request offers them too.
        let offer = self.0.mcp.offer(vault, context, &op.cancel).await;
        let server_id = turn_server_id(env, context);
        let compact = {
            let v = unlocked(vault)?;
            if op.cancel.is_cancelled() {
                return Err(AppError::new(
                    ErrorCode::Cancelled,
                    "compaction was stopped",
                ));
            }
            compact_request(
                &v,
                conversation_id,
                context,
                server_id.as_deref(),
                &offer,
                &compact_instruction(instructions),
                false,
            )?
        };
        let summary = match self.summarize(conversation_id, compact, &op.cancel).await {
            Ok(summary) => summary,
            Err(e) => {
                tracing::warn!(
                    conversation_id,
                    kind = error_kind(&e),
                    status = ?e.status(),
                    "AI compaction failed"
                );
                return Err(e.into());
            }
        };
        let Some(summary) = summary else {
            return Err(AppError::new(
                ErrorCode::Ai,
                "the model returned no summary",
            ));
        };
        let view = {
            let mut v = unlocked(vault)?;
            if op.cancel.is_cancelled() {
                return Err(AppError::new(
                    ErrorCode::Cancelled,
                    "compaction was stopped",
                ));
            }
            let view = store_summary(&mut v, conversation_id, &summary)?;
            // The summary no longer holds the files the model read (AI-38…40).
            self.0.seen.forget(conversation_id);
            view
        };
        drop(op);
        tracing::info!(conversation_id, "AI conversation compacted");
        env.changed();
        Ok(view)
    }

    /// The checks of `ai_compact` under the vault guard, before any server starts: no turn runs,
    /// the context has something to summarize, and the model exists. Gives the calls without a
    /// result a cancelled one, and registers the compaction so that stop and lock reach it.
    fn prepare_compact(
        &self,
        vault: &SharedVault,
        conversation_id: &str,
        context: &AiTurnContext,
        stored: &mut bool,
    ) -> AppResult<OpGuard<'_>> {
        let mut v = unlocked(vault)?;
        find_conversation(&v, conversation_id)?;
        if self.is_running(conversation_id) {
            return Err(AppError::invalid("conversation_id", "a turn is running"));
        }
        // AI-41: the summary would be built before the revert is recorded.
        self.not_reverting(conversation_id)?;
        let entries = Entries::load(&v, conversation_id)?;
        if !cancel_open_calls(&mut v, conversation_id, &entries, HashMap::new())?.is_empty() {
            *stored = true;
        }
        self.forget_file_bases(conversation_id);
        self.forget_verdicts(conversation_id);
        if !compactable(&context_entries(&v, conversation_id)?) {
            return Err(AppError::invalid("conversation_id", "nothing to compact"));
        }
        resolve_model(&v, &context.provider_id, &context.model_id)
            .map_err(|message| AppError::invalid("model_id", message))?;
        Ok(self.start_compaction(conversation_id))
    }

    /// Asks the model for the summary a Compact request wants (AI-21). An answer that calls tools
    /// or has no text is dropped (none of its calls runs or is stored), and the request goes once
    /// more without tools, with the system prompt that says so. `None` when that answer is no
    /// summary either: an endpoint can still return calls without tools, and their text is then
    /// only what comes before them.
    async fn summarize(
        &self,
        conversation_id: &str,
        compact: SideRequest,
        cancel: &CancellationToken,
    ) -> Result<Option<String>, AiError> {
        let SideRequest {
            mut request,
            plain_system,
        } = compact;
        let http = self.http();
        let response = complete(&http, &request.provider, &request.chat(), cancel).await?;
        log_side_request_usage(conversation_id, COMPACT, false, response.usage);
        if let Some(summary) = summary_of(&response) {
            return Ok(Some(summary));
        }
        tracing::info!(
            conversation_id,
            tool_calls = response.tool_calls.len(),
            "AI compaction answered without a summary; asking again without tools"
        );
        request.tools = Vec::new();
        request.system = plain_system;
        let response = complete(&http, &request.provider, &request.chat(), cancel).await?;
        log_side_request_usage(conversation_id, COMPACT, true, response.usage);
        if !response.tool_calls.is_empty() {
            tracing::info!(
                conversation_id,
                tool_calls = response.tool_calls.len(),
                "AI compaction answered with tool calls again"
            );
        }
        Ok(summary_of(&response))
    }

    /// AI-22, before a new message or a retry: builds the Compact request from the request the
    /// turn would send next, with the MCP tools it would offer (starting their servers, as the
    /// turn would), then compacts like [`Self::auto_compact`]. `defers`: the new message defers
    /// the conversation's MCP tools (AI-45), so its requests, and this one, offer `search_tools`
    /// and `call_mcp_tool`. The conversation counts as compacting from the start, which refuses a
    /// side question (AI-42).
    async fn compact_before(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        conversation_id: &str,
        turn: &Turn,
        defers: bool,
    ) -> Result<(), CompactFailure> {
        let _compaction = self.start_compaction(conversation_id);
        let offer = self.0.mcp.offer(vault, &turn.context, &turn.cancel).await;
        let compact = {
            let v = lock(vault);
            if turn.cancel.is_cancelled() || !v.is_unlocked() {
                return Err(CompactFailure::Stopped);
            }
            compact_request(
                &v,
                conversation_id,
                &turn.context,
                turn.server_id.as_deref(),
                &offer,
                COMPACT_INSTRUCTION,
                defers,
            )
            .map_err(|e| CompactFailure::Failed {
                status: None,
                message: e.detail,
            })?
        };
        self.auto_compact(vault, env, conversation_id, turn, compact)
            .await
    }

    /// AI-22: runs a Compact request inside a turn, exactly like `ai_compact`, stores the
    /// summary (moving `context_start` to it) and sends its entry on the turn's channel. The
    /// conversation counts as compacting until the summary is stored (AI-42).
    async fn auto_compact(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        conversation_id: &str,
        turn: &Turn,
        compact: SideRequest,
    ) -> Result<(), CompactFailure> {
        let _compaction = self.start_compaction(conversation_id);
        tracing::info!(conversation_id, "AI context nearly full; compacting");
        let summary = match self.summarize(conversation_id, compact, &turn.cancel).await {
            Ok(summary) => summary,
            Err(_) if turn.cancel.is_cancelled() => return Err(CompactFailure::Stopped),
            Err(AiError::Cancelled) => return Err(CompactFailure::Stopped),
            Err(e) => {
                tracing::warn!(
                    conversation_id,
                    kind = error_kind(&e),
                    status = ?e.status(),
                    "AI automatic compaction failed"
                );
                return Err(CompactFailure::Failed {
                    status: e.status(),
                    message: error_text(&e),
                });
            }
        };
        let Some(summary) = summary else {
            tracing::warn!(conversation_id, "AI automatic compaction got no summary");
            return Err(CompactFailure::Failed {
                status: None,
                message: "The context is nearly full, and the model returned no summary \
                          when asked to compact it."
                    .into(),
            });
        };
        {
            let mut v = lock(vault);
            if turn.cancel.is_cancelled() || !v.is_unlocked() {
                return Err(CompactFailure::Stopped);
            }
            match store_summary(&mut v, conversation_id, &summary) {
                Ok(view) => {
                    self.0.seen.forget(conversation_id);
                    turn.emit(AiTurnEvent::Entry { entry: view });
                }
                Err(e) if e.code == ErrorCode::Locked => return Err(CompactFailure::Stopped),
                Err(e) => {
                    return Err(CompactFailure::Failed {
                        status: None,
                        message: e.detail,
                    });
                }
            }
        }
        tracing::info!(conversation_id, "AI conversation compacted automatically");
        env.changed();
        Ok(())
    }

    // ---- side questions ----

    /// `ai_side_question` (AI-42): asks the conversation's model `input.text` with the request
    /// the conversation would send next ([`side_request`]), streams the answer to `on_event`, and
    /// returns it. Stores nothing and leaves a running turn as it is: the turn's stop does not
    /// reach it, while [`Self::side_stop`], a newer side question of the conversation (even one
    /// that is refused), deleting the conversation and the lock do. Refused while the
    /// conversation is compacting.
    pub async fn side_question(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        input: AiSideQuestionInput,
        on_event: &(dyn Fn(AiSideEvent) + Send + Sync),
    ) -> AppResult<AiSideAnswer> {
        let AiSideQuestionInput {
            conversation_id,
            question_id,
            text,
            context: next_context,
            approval_call_id,
        } = input;
        let stopped = || AppError::new(ErrorCode::Cancelled, "the side question was stopped");
        let next_server_id = turn_server_id(env, &next_context);
        // The running turn's next request, whose prefix the provider has cached, or the next
        // message's when no turn runs, read under one vault guard: a turn starts and gives way to
        // a new message under it, so its context goes with the entries read here.
        let (side, request) = {
            let v = unlocked(vault)?;
            let side = self.start_side(&conversation_id, question_id);
            find_conversation(&v, &conversation_id)?;
            if self.is_compacting(&conversation_id) {
                return Err(AppError::invalid(
                    "conversation_id",
                    "the conversation is being compacted",
                ));
            }
            let turn = self.running(&conversation_id);
            let (context, server_id, timing) = match &turn {
                Some(turn) => (
                    &turn.context,
                    turn.server_id.as_deref(),
                    SideTiming::DuringTurn {
                        approval: approval_call_id.as_deref(),
                    },
                ),
                None => (&next_context, next_server_id.as_deref(), SideTiming::Idle),
            };
            // AI-30: the MCP tools that request offers, of the servers that run now; none starts
            // for a side question.
            let offer = self.0.mcp.running_offer(&v, context);
            let request = side_request(
                &v,
                &conversation_id,
                context,
                server_id,
                &offer,
                timing,
                &text,
            )?;
            (side, request)
        };
        let cancel = side.token().clone();
        let response = match self
            .side_answer(&conversation_id, request, &cancel, on_event)
            .await
        {
            Ok(response) => response,
            Err(_) if cancel.is_cancelled() => return Err(stopped()),
            Err(e) => {
                tracing::warn!(
                    conversation_id = %conversation_id,
                    kind = error_kind(&e),
                    status = ?e.status(),
                    "AI side question failed"
                );
                return Err(e.into());
            }
        };
        if cancel.is_cancelled() {
            return Err(stopped());
        }
        drop(side);
        // An endpoint can still return calls without tools; none of them runs.
        if !response.tool_calls.is_empty() {
            tracing::info!(
                conversation_id = %conversation_id,
                tool_calls = response.tool_calls.len(),
                "AI side question answered with tool calls again"
            );
            return Err(AppError::new(
                ErrorCode::Ai,
                "the model answered with tool calls instead of text",
            ));
        }
        tracing::info!(conversation_id = %conversation_id, "AI side question answered");
        Ok(AiSideAnswer {
            text: response.text,
            finish: finish_view(response.finish),
        })
    }

    /// Streams the answer to a side question (AI-42). An answer that calls tools is dropped (none
    /// of its calls runs), `restarted` tells the panel to discard what streamed, and the question
    /// goes once more without tools, with the system prompt that says so. Returns the last answer.
    async fn side_answer(
        &self,
        conversation_id: &str,
        request: SideRequest,
        cancel: &CancellationToken,
        on_event: &(dyn Fn(AiSideEvent) + Send + Sync),
    ) -> Result<AssistantEntry, AiError> {
        let SideRequest {
            mut request,
            plain_system,
        } = request;
        let http = self.http();
        // Nothing reaches the panel once the side question is stopped.
        let send = |event: AiSideEvent| {
            if !cancel.is_cancelled() {
                on_event(event);
            }
        };
        let mut forward = |event: StreamEvent| match event {
            StreamEvent::Text(delta) => send(AiSideEvent::Text { delta }),
            StreamEvent::Reasoning(delta) => send(AiSideEvent::Reasoning { delta }),
            _ => {}
        };
        let response = stream_chat(
            &http,
            &request.provider,
            &request.chat(),
            cancel,
            &mut forward,
        )
        .await?;
        log_side_request_usage(conversation_id, SIDE_QUESTION, false, response.usage);
        if response.tool_calls.is_empty() {
            return Ok(response);
        }
        tracing::info!(
            conversation_id,
            tool_calls = response.tool_calls.len(),
            "AI side question answered with tool calls; asking again without tools"
        );
        send(AiSideEvent::Restarted);
        request.tools = Vec::new();
        request.system = plain_system;
        let response = stream_chat(
            &http,
            &request.provider,
            &request.chat(),
            cancel,
            &mut forward,
        )
        .await?;
        log_side_request_usage(conversation_id, SIDE_QUESTION, true, response.usage);
        Ok(response)
    }
}

/// Why an automatic compaction did not happen.
enum CompactFailure {
    /// The turn was stopped, or the vault locked.
    Stopped,
    /// The request failed or the summary could not be stored.
    Failed {
        status: Option<u16>,
        message: String,
    },
}

/// A request beside the conversation's turns, whose answer is no reply of the conversation: a
/// Compact request (AI-21) or a side question's (AI-42). It is the conversation's next request, as
/// [`next_request`] builds it for a turn, with a last user message after it. It offers the same
/// tools with the same system prompt and replays the same entries, so the provider reads its
/// prefix from the cache the conversation's requests wrote. A Compact request asks for less
/// output ([`compact_output_tokens`]), which is not part of that prefix; a side question asks for
/// as much as the next request. `plain_system` is the system prompt of the fallback without tools
/// ([`AiManager::summarize`], [`AiManager::side_answer`]).
struct SideRequest {
    request: Request,
    plain_system: String,
}

impl SideRequest {
    /// `next`, the request a turn with `context` sends next, with `message` appended to it.
    /// `plain` says what the fallback without tools asks for.
    fn new(
        v: &Vault,
        context: &AiTurnContext,
        mut next: Request,
        message: impl Into<String>,
        plain: PromptTools,
    ) -> Self {
        next.entries.push(AiEntry::user(now_ms(), message));
        let (plain_system, _) = prompt(v, context, Some(plain), None);
        Self {
            request: next,
            plain_system,
        }
    }

    /// A Compact request (AI-21, AI-22): `next` with `instruction` appended, asking for less
    /// output, so that the summary fits beside a nearly full context.
    fn compact(v: &Vault, context: &AiTurnContext, mut next: Request, instruction: &str) -> Self {
        next.model.max_output_tokens = Some(compact_output_tokens(&next.model));
        Self::new(v, context, next, instruction, PromptTools::None)
    }
}

/// The instruction of a Compact request (AI-21), with the user's own instructions for the summary
/// when `/compact <instructions>` gave some (AI-50).
fn compact_instruction(instructions: Option<&str>) -> String {
    match instructions.map(str::trim).filter(|s| !s.is_empty()) {
        None => COMPACT_INSTRUCTION.to_owned(),
        Some(extra) => format!(
            "{COMPACT_INSTRUCTION}\n\nThe user gave these instructions for the summary, which \
             come first:\n<compact_instructions>\n{}\n</compact_instructions>",
            escape_closing_tag(extra, "compact_instructions")
        ),
    }
}

/// The summary in an answer to a Compact request: its text, when it has some and calls no tools.
fn summary_of(response: &AssistantEntry) -> Option<String> {
    let text = response.text.trim();
    (response.tool_calls.is_empty() && !text.is_empty()).then(|| text.to_owned())
}

/// The Compact request (AI-21) for the conversation's context, built from the next request of a
/// turn with `context` and `server_id` whose MCP tools are `offer`, with less output. `defers`:
/// the message it comes before defers the MCP tools (AI-45), which its stored entries do not say
/// yet.
fn compact_request(
    v: &Vault,
    conversation_id: &str,
    context: &AiTurnContext,
    server_id: Option<&str>,
    offer: &Offer,
    instruction: &str,
    defers: bool,
) -> AppResult<SideRequest> {
    let conversation = find_conversation(v, conversation_id)?;
    let mut entries = Entries::load(v, conversation_id)?;
    let from = entries.context_from(conversation.context_start.as_deref());
    let deferred = defers || mcp_deferred(&entries.list);
    let list = entries.list.split_off(from);
    if !compactable(&list) {
        return Err(AppError::invalid("conversation_id", "nothing to compact"));
    }
    let next = next_request(v, context, server_id, offer, list, deferred)
        .map_err(|message| AppError::invalid("model_id", message))?;
    Ok(SideRequest::compact(v, context, next, instruction))
}

/// When a side question is asked (AI-42), which decides what its request adds for this request
/// only.
#[derive(Clone, Copy)]
enum SideTiming<'a> {
    /// No turn runs: the request is the next message's, so the question starts with the notes
    /// that message would start with (spec §13.3), and calls without a result go as cancelled.
    Idle,
    /// A turn runs, whose stored messages carry their notes ([`side_results`]).
    DuringTurn { approval: Option<&'a str> },
}

/// A side question's request (AI-42), built from the next request of a turn with `context` and
/// `server_id` whose MCP tools are `offer`, like a Compact request. Its last user message is laid
/// out like a message's: what `timing` adds (the notes), the attachment blocks the input area put
/// before the question, then the instruction and the typed question. Nothing of it is stored.
fn side_request(
    v: &Vault,
    conversation_id: &str,
    context: &AiTurnContext,
    server_id: Option<&str>,
    offer: &Offer,
    timing: SideTiming<'_>,
    question: &str,
) -> AppResult<SideRequest> {
    let (attachments, typed) = side_parts(question);
    let typed = typed.trim();
    if typed.is_empty() {
        return Err(AppError::invalid("text", "The question is empty."));
    }
    let conversation = find_conversation(v, conversation_id)?;
    let mut entries = Entries::load(v, conversation_id)?;
    let from = entries.context_from(conversation.context_start.as_deref());
    let notes = match timing {
        SideTiming::Idle => {
            next_message_notes(v, conversation_id, &conversation, &entries, context)
        }
        SideTiming::DuringTurn { .. } => String::new(),
    };
    let deferred = mcp_deferred(&entries.list);
    let mut list = entries.list.split_off(from);
    if let SideTiming::DuringTurn { approval } = timing {
        side_results(&mut list, approval);
    }
    let next = next_request(v, context, server_id, offer, list, deferred)
        .map_err(|message| AppError::invalid("model_id", message))?;
    let attachments = attachments.trim_end_matches('\n');
    let attachments = if attachments.is_empty() {
        String::new()
    } else {
        format!("{attachments}\n\n")
    };
    let message = format!(
        "{notes}{attachments}{SIDE_QUESTION_INSTRUCTION}\n\n<side_question>\n{}\n</side_question>",
        escape_closing_tag(typed, "side_question")
    );
    Ok(SideRequest::new(
        v,
        context,
        next,
        message,
        PromptTools::SideQuestion,
    ))
}

/// A side question's text (AI-42) split into the attachment blocks the input area put before it
/// (spec §13.3) and the typed question. Text that starts with a note is all typed, since only
/// Hatoba writes notes.
fn side_parts(text: &str) -> (&str, &str) {
    let (blocks, typed) = leading_blocks(text);
    if blocks.iter().any(|b| b.kind.note) {
        return ("", text);
    }
    (&text[..text.len() - typed.len()], typed)
}

/// The results a side question's request (AI-42) gives the calls of the running turn's `context`
/// that have none, for that request only. Only the calls of the turn's newest response, which the
/// context ends with but for results, get a result that says the call waits for the user's
/// approval (the call `approval`, whose card waits) or has not finished. Any other call without a
/// result (one left by a crash, or merged from another device) goes as cancelled, as in the
/// turn's own requests.
fn side_results(context: &mut Vec<AiEntry>, approval: Option<&str>) {
    let newest = context
        .iter()
        .rposition(|e| !matches!(e.body, EntryBody::Tool { .. }))
        .filter(|&i| matches!(context[i].body, EntryBody::Assistant(_)));
    let open_now = newest.map_or(0, |i| fix_up_missing_results(&context[i..]).len());
    let missing = fix_up_missing_results(context);
    let older = missing.len().saturating_sub(open_now);
    let now = now_ms();
    // In conversation order, so each result answers the call it is meant for (`pair_results`).
    for (n, call_id) in missing.into_iter().enumerate() {
        let entry = if n < older {
            AiEntry::cancelled_result(now, call_id)
        } else if approval == Some(call_id.as_str()) {
            AiEntry::tool(now, call_id, ToolStatus::Ok, SIDE_AWAITING_APPROVAL)
        } else {
            AiEntry::tool(now, call_id, ToolStatus::Ok, SIDE_PENDING)
        };
        context.push(entry);
    }
}

/// The conversation's context: its entries from `context_start` on (AI-21).
fn context_entries(v: &Vault, conversation_id: &str) -> AppResult<Vec<AiEntry>> {
    let conversation = find_conversation(v, conversation_id)?;
    let mut entries = Entries::load(v, conversation_id)?;
    let from = entries.context_from(conversation.context_start.as_deref());
    Ok(entries.list.split_off(from))
}

/// Whether a context has something to summarize: a message or a response.
fn compactable(context: &[AiEntry]) -> bool {
    context
        .iter()
        .any(|e| matches!(e.body, EntryBody::User { .. } | EntryBody::Assistant(_)))
}

/// The output tokens a Compact request asks for: at most 16,000, the model's output limit, and 9%
/// of its context window, so that a context compacted at 90% of the window (AI-22) leaves room
/// for the summary. Anthropic refuses a request whose input and `max_tokens` pass the window, and
/// a normal request asks for the model's whole output limit.
fn compact_output_tokens(model: &ModelSpec) -> u64 {
    let mut tokens = COMPACT_MAX_TOKENS;
    if let Some(output) = model.max_output_tokens.filter(|n| *n > 0) {
        tokens = tokens.min(output);
    }
    if let Some(window) = model.context_window.filter(|w| *w > 0) {
        tokens = tokens.min(window.saturating_mul(COMPACT_OUTPUT_PERCENT) / 100);
    }
    tokens.max(1)
}

/// What [`log_side_request_usage`] logs a [`SideRequest`] as: Compact (AI-21) or a side question
/// (AI-42).
const COMPACT: &str = "compact";
const SIDE_QUESTION: &str = "side_question";

/// Logs the token counts of a response to a [`SideRequest`], never its text (SEC-04), so the share
/// the prompt cache served can be measured.
fn log_side_request_usage(
    conversation_id: &str,
    request: &str,
    fallback: bool,
    usage: Option<Usage>,
) {
    let usage = usage.unwrap_or_default();
    tracing::info!(
        conversation_id,
        request,
        fallback,
        input_tokens = usage.input_tokens,
        cache_read_tokens = usage.cache_read_tokens,
        cache_write_tokens = usage.cache_write_tokens,
        output_tokens = usage.output_tokens,
        estimated = usage.estimated,
        "AI side request response"
    );
}

/// AI-22, before a new message is stored: whether the context plus the message would pass 90% of
/// the model's known context window. Never when the context is empty or ends with a summary (it
/// was just compacted, and no entry has followed it since), or the model cannot be resolved (the
/// turn then says why).
fn compaction_before(
    v: &Vault,
    conversation_id: &str,
    context: &AiTurnContext,
    text: &str,
    saved: u64,
) -> AppResult<bool> {
    let Ok((_, model)) = resolve_model(v, &context.provider_id, &context.model_id) else {
        return Ok(false);
    };
    let Some(window) = model.context_window.filter(|w| *w > 0) else {
        return Ok(false);
    };
    let list = context_entries(v, conversation_id)?;
    if !compactable(&list)
        || matches!(
            list.last().map(|e| &e.body),
            Some(EntryBody::Summary { .. })
        )
    {
        return Ok(false);
    }
    let mut tokens = context_tokens(&list).saturating_add(estimate_tokens(text));
    // `saved`: tool definitions the last response's usage counts and the message's requests leave
    // out (AI-45). Without a usage, the estimate counted no tool definitions.
    if list
        .iter()
        .any(|e| matches!(&e.body, EntryBody::Assistant(a) if a.usage.is_some()))
    {
        tokens = tokens.saturating_sub(saved);
    }
    Ok(over_threshold(tokens, window))
}

/// AI-45: the tokens of tool definitions a message that defers the MCP tools takes out of its
/// requests, against the last request, which offered the tools of `last` directly.
fn deferred_saving(v: &Vault, context: &AiTurnContext, last: &Offer) -> u64 {
    let Ok((provider, _)) = resolve_model(v, &context.provider_id, &context.model_id) else {
        return 0;
    };
    let size = |defs: &[ToolDef]| estimate_tokens(&serde_json::to_string(defs).unwrap_or_default());
    size(&last.tool_defs(provider.protocol)).saturating_sub(size(&tools::deferred_mcp_tools()))
}

/// Whether a request's tools are those of a conversation whose MCP tools are deferred (AI-45).
fn offers_deferred(tools: &[ToolDef]) -> bool {
    tools.iter().any(|t| t.name == tools::CALL_MCP_TOOL)
}

fn over_threshold(tokens: u64, window: u64) -> bool {
    tokens.saturating_mul(10) > window.saturating_mul(AUTO_COMPACT_TENTHS)
}

/// Stores a summary entry and moves `context_start` to it (AI-21). The model sees nothing before
/// a summary, so the summary starts with the `permission_mode` note (AI-16) of the mode the model
/// was told of, which the request puts before the summary's block: the mode stays that one, also
/// for the rest of a turn compacted between tool calls (AI-22). A conversation whose messages never
/// named one gets no note, and its next message names the mode. The `memory` note (AI-53) follows
/// it, with the memories as they are now.
fn store_summary(v: &mut Vault, conversation_id: &str, summary: &str) -> AppResult<AiEntryView> {
    let mut conversation = find_conversation(v, conversation_id)?;
    let noted = noted_permission(&Entries::load(v, conversation_id)?.list);
    let text = format!(
        "{}{}{summary}",
        noted.map(tools::permission_mode_block).unwrap_or_default(),
        memory::note(v)
    );
    let entry = AiEntry::summary(now_ms(), text);
    let entry_id = append(v, conversation_id, &entry)?;
    conversation.context_start = Some(entry_id.clone());
    v.put(Some(conversation_id), Item::AiConversation(conversation))?;
    Ok(entry_view(&entry_id, &entry))
}

/// AI-22, inside a turn: whether the request's context would pass 90% of the model's known
/// context window. Only between tool calls, when the context ends with a tool result: a new
/// message is checked before it is stored ([`compaction_before`]), because a summary stored after
/// it would leave it outside the context. After a compaction the context ends with the summary,
/// so it is never compacted twice in a row.
fn compaction_due(request: &Request) -> bool {
    let Some(window) = request.model.context_window.filter(|w| *w > 0) else {
        return false;
    };
    matches!(
        request.entries.last().map(|e| &e.body),
        Some(EntryBody::Tool { .. })
    ) && over_threshold(context_tokens(&request.entries), window)
}

/// The tokens a context uses, the way the panel's meter counts them (AI-20): the last response's
/// input and output tokens, plus about 4 characters a token for every entry stored after it.
fn context_tokens(entries: &[AiEntry]) -> u64 {
    let last_usage = entries
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, e)| match &e.body {
            EntryBody::Assistant(a) => a
                .usage
                .map(|u| (i + 1, u.input_tokens.saturating_add(u.output_tokens))),
            _ => None,
        });
    let (from, base) = last_usage.unwrap_or((0, 0));
    entries[from..]
        .iter()
        .map(|e| estimate_tokens(&entry_text(e)))
        .fold(base, u64::saturating_add)
}

/// The text of an entry the estimate counts (the meter's `entryText`).
fn entry_text(entry: &AiEntry) -> std::borrow::Cow<'_, str> {
    match &entry.body {
        EntryBody::User { text, .. } | EntryBody::Summary { text } => text.as_str().into(),
        EntryBody::Tool { content, .. } => content.as_str().into(),
        EntryBody::Assistant(a) => {
            let mut text = a.text.clone();
            for call in &a.tool_calls {
                text.push_str(&call.name);
                text.push_str(&call.arguments);
            }
            text.into()
        }
    }
}

/// Tokens from text length, rounded up, counting UTF-16 units like the WebView's `length`.
fn estimate_tokens(text: &str) -> u64 {
    text.encode_utf16().count().div_ceil(CHARS_PER_TOKEN) as u64
}

// ---- turn steps ----

/// Reads what the next request needs, after storing a cancelled result for every call without
/// one (§13.1).
fn prepare(
    vault: &SharedVault,
    env: &dyn AiEnv,
    conversation_id: &str,
    turn: &Turn,
    offer: &Offer,
) -> Result<Request, Halt> {
    let mut stored = false;
    let result = prepare_locked(&mut lock(vault), conversation_id, turn, offer, &mut stored);
    if stored {
        env.changed();
    }
    result
}

fn prepare_locked(
    v: &mut Vault,
    conversation_id: &str,
    turn: &Turn,
    offer: &Offer,
    stored: &mut bool,
) -> Result<Request, Halt> {
    if turn.cancel.is_cancelled() || !v.is_unlocked() {
        return Err(Halt::Stopped);
    }
    let conversation = find_conversation(v, conversation_id)
        .map_err(|_| Halt::Failed("The conversation no longer exists.".into()))?;
    let mut entries = Entries::load(v, conversation_id)?;
    let cancelled = cancel_open_calls(v, conversation_id, &entries, HashMap::new())?;
    if !cancelled.is_empty() {
        *stored = true;
        for entry in cancelled {
            turn.emit(AiTurnEvent::Entry { entry });
        }
        entries = Entries::load(v, conversation_id)?;
    }
    let from = entries.context_from(conversation.context_start.as_deref());
    let deferred = mcp_deferred(&entries.list);
    let context = entries.list.split_off(from);
    if nothing_to_send(&context) {
        return Err(Halt::Idle);
    }
    next_request(
        v,
        &turn.context,
        turn.server_id.as_deref(),
        offer,
        context,
        deferred,
    )
    .map_err(Halt::Failed)
}

/// The conversation's next request (§13.1) for its context `entries`: the system prompt that
/// offers tools, the built-in tools, then the MCP tools of `offer`, or `search_tools` and
/// `call_mcp_tool` when the conversation's MCP tools are `deferred` (AI-45). Every request of a
/// turn, every Compact request (AI-21) and every side question's (AI-42) is built here, so the last
/// two have their turn's prefix. The error is a sentence for the panel.
fn next_request(
    v: &Vault,
    context: &AiTurnContext,
    server_id: Option<&str>,
    offer: &Offer,
    entries: Vec<AiEntry>,
    deferred: bool,
) -> Result<Request, String> {
    let (provider, model) = resolve_model(v, &context.provider_id, &context.model_id)?;
    let (system, mut tools) = prompt(v, context, None, server_id);
    // AI-30: after the built-in tools, those of the MCP servers, with or without a tab (AI-09),
    // or, once the conversation's MCP tools are deferred, the two tools that stand for them
    // whatever servers are on (AI-45).
    if deferred {
        tools.extend(tools::deferred_mcp_tools());
    } else {
        tools.extend(offer.tool_defs(provider.protocol));
    }
    Ok(Request {
        provider_id: context.provider_id.clone(),
        provider,
        model,
        effort: context.effort.map(request_effort),
        system,
        tools,
        entries,
    })
}

/// Stores a response and sends its `entry` and `done` events (under the vault guard, so they
/// keep the store's order against a concurrent stop). Returns its entry id.
///
/// Calls that share their id with another call of the response cannot be told apart by their
/// results (a result answers the first open call with its id), so none of them runs: each gets
/// an error result here, sent before `done` so the frontend never handles them.
fn store_response(
    vault: &SharedVault,
    env: &dyn AiEnv,
    conversation_id: &str,
    turn: &Turn,
    response: AssistantEntry,
) -> Result<String, Halt> {
    // AI-09: the next message is noted when it has the other set of tools.
    let response = AssistantEntry {
        terminal: Some(turn.context.tab),
        ..response
    };
    let finish = response.finish;
    let shared = shared_call_ids(&response.tool_calls);
    let entry_id = {
        let mut v = lock(vault);
        if turn.cancel.is_cancelled() || !v.is_unlocked() {
            return Err(Halt::Stopped);
        }
        // Stored whole now, so a stop no longer stores the part that streamed.
        guard(&turn.streamed).take();
        let entry = AiEntry::assistant(now_ms(), response);
        let entry_id = append(&mut v, conversation_id, &entry)?;
        turn.emit(AiTurnEvent::Entry {
            entry: entry_view(&entry_id, &entry),
        });
        for (id, count) in shared {
            let result = AiEntry::tool(
                now_ms(),
                id.as_str(),
                ToolStatus::Error,
                format!(
                    "The call did not run: {count} tool calls of this response have the id \
                     \"{id}\", and each call needs an id of its own."
                ),
            );
            let result_id = append(&mut v, conversation_id, &result)?;
            turn.emit(AiTurnEvent::Entry {
                entry: entry_view(&result_id, &result),
            });
        }
        turn.emit(AiTurnEvent::Done {
            finish: finish_view(finish),
        });
        entry_id
    };
    env.changed();
    Ok(entry_id)
}

/// Waits until every call of the response `entry_id` has a stored result.
async fn wait_for_results(
    vault: &SharedVault,
    conversation_id: &str,
    turn: &Turn,
    entry_id: &str,
) -> Result<(), Halt> {
    loop {
        let notified = turn.results.notified();
        tokio::pin!(notified);
        // Registered before the check, so a result stored in between still wakes the turn.
        notified.as_mut().enable();
        {
            let v = lock(vault);
            if turn.cancel.is_cancelled() || !v.is_unlocked() {
                return Err(Halt::Stopped);
            }
            // The response and the entries after it; the history before it is not read.
            let entries = Entries::load_from(&v, conversation_id, entry_id)?;
            if entries.ids.first().is_none_or(|id| id != entry_id) {
                return Err(Halt::Failed(
                    "The model's response is no longer stored.".into(),
                ));
            }
            if fix_up_missing_results(&entries.list).is_empty() {
                return Ok(());
            }
        }
        tokio::select! {
            () = turn.cancel.cancelled() => return Err(Halt::Stopped),
            () = notified.as_mut() => {}
        }
    }
}

fn halted(turn: &Turn, conversation_id: &str, halt: Halt) -> AiTurnEndReason {
    match halt {
        Halt::Stopped => AiTurnEndReason::Stopped,
        Halt::Idle => AiTurnEndReason::Completed,
        Halt::Failed(message) => {
            tracing::warn!(conversation_id, "AI turn could not continue");
            turn.emit(AiTurnEvent::Error {
                status: None,
                message,
            });
            AiTurnEndReason::Error
        }
    }
}

/// Whether a request built from these entries would carry nothing, or end with an assistant
/// message (newer models reject that as a prefill). The adapter skips blank assistant entries
/// and places every result right after its call, so those are looked past.
fn nothing_to_send(entries: &[AiEntry]) -> bool {
    for entry in entries.iter().rev() {
        match &entry.body {
            EntryBody::Tool { .. } => {}
            EntryBody::Assistant(a) if a.text.trim().is_empty() && a.tool_calls.is_empty() => {}
            EntryBody::Assistant(a) => return a.tool_calls.is_empty(),
            EntryBody::User { .. } | EntryBody::Summary { .. } => return false,
        }
    }
    true
}

/// For every call of a response whose id another call of it has too, in order: the id and how
/// many calls have it.
fn shared_call_ids(calls: &[ToolCall]) -> Vec<(String, usize)> {
    let count = |id: &str| calls.iter().filter(|c| c.id == id).count();
    calls
        .iter()
        .map(|c| (c.id.clone(), count(&c.id)))
        .filter(|(_, n)| *n > 1)
        .collect()
}

/// The call `tool_call_id` of the conversation's newest response, when it has no result yet.
/// Results answer the calls with their id in order (`fix_up_missing_results`), so of calls that
/// share an id (stored before such calls got their results in [`store_response`]), the open one
/// is the first that no result answers.
/// Where an open call is, for the file tools' record of what the model has seen (§13.4): the
/// response that made it, which is the newest, and the entries in the conversation's context.
fn file_place(conversation: &AiConversation, entries: &Entries) -> files::Place {
    let newest = entries
        .list
        .iter()
        .rposition(|e| matches!(e.body, EntryBody::Assistant(_)));
    let from = entries.context_from(conversation.context_start.as_deref());
    files::Place {
        response: newest.map(|i| entries.ids[i].clone()).unwrap_or_default(),
        context: entries.ids[from..].to_vec(),
    }
}

/// Whether a call before the open call `tool_call_id` in the conversation's newest response has
/// no result yet.
fn earlier_call_open(entries: &Entries, tool_call_id: &str) -> AppResult<bool> {
    let (from, index) = open_call_at(entries, tool_call_id)?;
    let answered = pair_results(&entries.list[from..]);
    Ok((0..index).any(|c| !answered.contains_key(&(0, c))))
}

fn open_call(entries: &Entries, tool_call_id: &str) -> AppResult<AiToolCall> {
    let (at, index) = open_call_at(entries, tool_call_id)?;
    let call = &calls_of(&entries.list[at])[index];
    Ok(AiToolCall {
        id: call.id.clone(),
        name: call.name.clone(),
        arguments: call.arguments.clone(),
    })
}

/// Where the open call `tool_call_id` is: the index of the conversation's newest response in
/// `entries`, and the call's index in it. The call must belong to that response and have no
/// result; of calls that share its id (stored by an older build), the first without one.
fn open_call_at(entries: &Entries, tool_call_id: &str) -> AppResult<(usize, usize)> {
    let newest = entries
        .list
        .iter()
        .rposition(|e| matches!(e.body, EntryBody::Assistant(_)));
    let calls: Vec<usize> = newest.map_or_else(Vec::new, |i| {
        calls_of(&entries.list[i])
            .iter()
            .enumerate()
            .filter(|(_, c)| c.id == tool_call_id)
            .map(|(index, _)| index)
            .collect()
    });
    let (Some(from), false) = (newest, calls.is_empty()) else {
        return Err(AppError::not_found("tool call"));
    };
    let open = fix_up_missing_results(&entries.list[from..])
        .iter()
        .filter(|id| *id == tool_call_id)
        .count();
    let Some(&index) = calls.len().checked_sub(open).and_then(|i| calls.get(i)) else {
        return Err(AppError::invalid(
            "tool_call_id",
            "the tool call already has a result",
        ));
    };
    Ok((from, index))
}

/// The tool calls of an entry: an assistant entry's, none for any other.
fn calls_of(entry: &AiEntry) -> &[ToolCall] {
    match &entry.body {
        EntryBody::Assistant(a) => &a.tool_calls,
        _ => &[],
    }
}

// ---- what the judge of Auto mode reads (AI-47) ----

/// Most lines of the screen that a `send_input` call shows the judge (AI-47). The panel sends
/// the screen's last lines (`SCREEN_TAIL_LINES` in `features/ai/terminalText.ts`); this only
/// bounds what it sends, so a larger screen from the WebView never reaches the judge.
const MAX_SCREEN_TAIL_LINES: usize = 10;
/// Most characters of them.
const MAX_SCREEN_TAIL_CHARS: usize = 2_000;

/// AI-47: what the judge reads about the call `index` of the response `at` in `entries`, every
/// entry of the conversation, as spec §13.5 lists it, never what the model wrote or a tool
/// returned:
///
/// - `host`: the conversation's host, as the system prompt names it (§13.1): its display name,
///   or its quick-connect target (HOST-12).
/// - `user_messages`: every message the user sent, those before a compaction included, as
///   [`typed_text`] gives it; messages left empty are left out. Summaries are the model's.
/// - `user_rules`: the custom instructions (AI-36) and the host's AI notes (AI-37), each cut to
///   its limit as the system prompt cuts it, under a line that says which they are.
/// - `earlier_calls`: the calls before it, oldest first, those of its own response included, as
///   [`shown_call`] gives them. A call the user rejected adds the reason they typed
///   (`rejection`): the user's own words, which no user message carries.
/// - `call`: the call itself. An MCP tool, called directly or through `call_mcp_tool` (AI-45),
///   adds its server's name and the tool's description and annotations; `send_input` adds
///   `screen_tail` ([`screen_tail`]).
fn judge_state(
    v: &Vault,
    mcp: &McpManager,
    conversation_id: &str,
    conversation: &AiConversation,
    entries: &[AiEntry],
    (at, index): (usize, usize),
    screen: Option<&str>,
) -> JudgeState {
    let host = conversation
        .host_id
        .as_deref()
        .and_then(|id| host_name(v, id))
        .or_else(|| conversation.quick_target.as_ref().map(target_name))
        .or_else(|| conversation.local.as_deref().map(local_name));
    let user_messages = entries[..at]
        .iter()
        .filter_map(|e| match &e.body {
            EntryBody::User { text, .. } => Some(typed_text(text)),
            _ => None,
        })
        .filter(|text| !text.is_empty())
        .collect();
    let results = pair_results(entries);
    let rejection = |response: usize, call: usize| match results
        .get(&(response, call))
        .map(|&result| &entries[result].body)
    {
        Some(EntryBody::Tool {
            status: ToolStatus::Rejected,
            content,
            ..
        }) => Some(content.clone()),
        _ => None,
    };
    let calls = calls_of(&entries[at]);
    let earlier_calls = (0..at)
        .flat_map(|response| (0..calls_of(&entries[response]).len()).map(move |c| (response, c)))
        .chain((0..index).map(|c| (at, c)))
        .map(|(response, c)| {
            let call = &calls_of(&entries[response])[c];
            JudgeCall {
                rejection: rejection(response, c),
                ..shown_call(&call.name, &call.arguments)
            }
        })
        .collect();
    let call = &calls[index];
    let mut judged = shown_call(&call.name, &call.arguments);
    let offered = if call.name.starts_with(MCP_PREFIX) {
        mcp.offered(conversation_id, &call.name)
    } else if call.name == tools::CALL_MCP_TOOL {
        proxy_call(v, mcp, conversation_id, &call.arguments)
            .ok()
            .map(|(tool, arguments)| {
                judged = JudgeCall::new(tool.name.clone(), Value::Object(arguments));
                tool
            })
    } else {
        None
    };
    if let Some(tool) = offered {
        judged.server = Some(tool.server_name);
        judged.description = Some(tool.tool.description).filter(|d| !d.trim().is_empty());
        judged.annotations = annotations_of(&tool.tool.annotations);
    }
    if call.name == tools::SEND_INPUT {
        judged.screen_tail = screen.and_then(screen_tail);
    }
    JudgeState {
        host,
        user_rules: judge_rules(v, conversation),
        user_messages,
        earlier_calls,
        call: judged,
    }
}

/// AI-47: a user message as the judge reads it: the text the user typed, without the notes
/// Hatoba adds (spec §13.3), and each attachment as a placeholder that names its kind and size,
/// such as `[file app.log, 40 lines]`, on a line of its own before the text. Empty when the
/// message has neither, as one of notes alone.
fn typed_text(text: &str) -> String {
    let (blocks, typed) = leading_blocks(text);
    let placeholders: Vec<String> = blocks
        .iter()
        .filter(|b| !b.kind.note)
        .map(|b| (b.kind.placeholder)(&b.attrs))
        .collect();
    let typed = typed.trim();
    match (placeholders.is_empty(), typed.is_empty()) {
        (true, _) => typed.to_owned(),
        (false, true) => placeholders.join("\n"),
        (false, false) => format!("{}\n\n{typed}", placeholders.join("\n")),
    }
}

/// AI-47: a call as the judge reads it, its arguments parsed (or kept as text when they are not
/// JSON). A `call_mcp_tool` call (AI-45) is the MCP tool it names with that tool's arguments, as
/// the panel shows it; one that names no MCP tool is shown as it is.
fn shown_call(name: &str, arguments: &str) -> JudgeCall {
    if name == tools::CALL_MCP_TOOL
        && let Ok(proxy) = deferred::parse_call(arguments)
        && proxy.tool.starts_with(MCP_PREFIX)
    {
        return JudgeCall::new(proxy.tool, Value::Object(proxy.arguments));
    }
    JudgeCall::parse(name, arguments)
}

/// An MCP tool's annotations as its server named them (AI-30), for the judge; `None` without
/// any.
fn annotations_of(a: &hatoba_ai::mcp::ToolAnnotations) -> Option<Value> {
    let mut hints = Map::new();
    if let Some(title) = &a.title {
        hints.insert("title".into(), Value::from(title.as_str()));
    }
    for (name, hint) in [
        ("readOnlyHint", a.read_only_hint),
        ("destructiveHint", a.destructive_hint),
        ("idempotentHint", a.idempotent_hint),
        ("openWorldHint", a.open_world_hint),
    ] {
        if let Some(hint) = hint {
            hints.insert(name.into(), Value::Bool(hint));
        }
    }
    (!hints.is_empty()).then_some(Value::Object(hints))
}

/// AI-47: the screen's last lines as a `send_input` call shows them to the judge: control
/// characters removed, the blank lines at the end left out, then at most the last
/// [`MAX_SCREEN_TAIL_LINES`] lines and [`MAX_SCREEN_TAIL_CHARS`] characters. `None` when nothing
/// is left.
fn screen_tail(screen: &str) -> Option<String> {
    let cleaned: String = screen
        .chars()
        .filter(|c| *c == '\n' || !c.is_control())
        .collect();
    let lines: Vec<&str> = cleaned.lines().map(str::trim_end).collect();
    let end = lines.iter().rposition(|l| !l.is_empty())? + 1;
    let tail = lines[end.saturating_sub(MAX_SCREEN_TAIL_LINES)..end].join("\n");
    let surplus = tail.chars().count().saturating_sub(MAX_SCREEN_TAIL_CHARS);
    Some(tail.chars().skip(surplus).collect())
}

/// AI-47: the custom instructions (AI-36) and the AI notes of the conversation's host (AI-37) as
/// the judge reads them, each cut to its limit as the system prompt cuts it and under a line that
/// says which they are. Empty when both are.
fn judge_rules(v: &Vault, conversation: &AiConversation) -> String {
    let settings = v.settings();
    let instructions = bytes_prefix(
        &settings.ai.custom_instructions,
        MAX_CUSTOM_INSTRUCTIONS_BYTES,
    );
    let host = conversation
        .host_id
        .as_deref()
        .and_then(|id| v.get(id))
        .and_then(Item::as_host);
    let notes = host.map_or("", |h| bytes_prefix(&h.ai_notes, MAX_HOST_AI_NOTES_BYTES));
    [
        ("Custom instructions:", instructions),
        ("Notes for this host:", notes),
    ]
    .into_iter()
    .filter(|(_, text)| !text.trim().is_empty())
    .map(|(label, text)| format!("{label}\n{}", text.trim()))
    .collect::<Vec<_>>()
    .join("\n\n")
}

/// AI-47: a stored verdict of the judge for the panel, its answers in the order the judge asks
/// its questions ([`question_order`]), then any others by id.
fn auto_view(verdict: &AutoVerdict) -> AiAutoVerdict {
    let order = question_order();
    let mut answers: Vec<(&String, &f64)> = verdict.answers.iter().collect();
    // Stable, so answers to questions this version does not ask stay in id order.
    answers.sort_by_key(|(id, _)| order.iter().position(|q| q == id).unwrap_or(order.len()));
    AiAutoVerdict {
        provider_id: verdict.provider_id.clone(),
        model_id: verdict.model_id.clone(),
        decision: decision_view(verdict.decision),
        answers: answers
            .into_iter()
            .map(|(question, probability)| AiJudgeAnswer {
                question: question.clone(),
                probability: *probability,
            })
            .collect(),
        concerns: verdict.concerns.clone(),
        failure: verdict.failure.clone(),
    }
}

/// A decision of the judge, for the panel (AI-47).
fn decision_view(decision: AutoDecision) -> AiJudgeDecision {
    match decision {
        AutoDecision::Run => AiJudgeDecision::Run,
        AutoDecision::Ask => AiJudgeDecision::Ask,
    }
}

/// The ids of the judge's questions in the order of its table (spec §13.5). Every tool's questions
/// ([`judge::question_ids`]) keep this order, so the answers of a stored verdict, which does not
/// name its call's tool, show in the order they were asked.
fn question_order() -> Vec<&'static str> {
    let mut order: Vec<&'static str> = Vec::new();
    for tool in [tools::RUN_COMMAND, tools::EDIT_FILE, tools::FETCH_URL] {
        for id in judge::question_ids(tool) {
            if !order.contains(&id) {
                order.push(id);
            }
        }
    }
    order
}

/// AI-43: the entries a fork copies, by index, in order: every entry up to `at`, `at` included,
/// and each tool result stored after it that answers a call of a copied assistant entry
/// ([`pair_results`]), so no copied call loses its result; `whole` copies every entry. `None`
/// when `at` is no fork point: a user entry, or an assistant entry whose every call has a result.
fn fork_copies(entries: &[AiEntry], at: usize, whole: bool) -> Option<Vec<usize>> {
    if whole {
        return Some((0..entries.len()).collect());
    }
    let pairs = pair_results(entries);
    match &entries.get(at)?.body {
        EntryBody::User { .. } => {}
        EntryBody::Assistant(a)
            if (0..a.tool_calls.len()).all(|c| pairs.contains_key(&(at, c))) => {}
        _ => return None,
    }
    let mut answers: Vec<usize> = pairs
        .iter()
        .filter(|&(&(call, _), &result)| call <= at && result > at)
        .map(|(_, &result)| result)
        .collect();
    answers.sort_unstable();
    Some((0..=at).chain(answers).collect())
}

/// AI-26: the user's message `entry_id` that an edit replaces, and the newest summary before it
/// (where a `context_start` that pointed at a deleted entry moves).
fn edited_entry(
    v: &Vault,
    conversation_id: &str,
    entry_id: &str,
) -> AppResult<(String, Option<String>)> {
    let entries = Entries::load(v, conversation_id)?;
    let Some(at) = entries.ids.iter().position(|id| id == entry_id) else {
        return Err(AppError::not_found("message"));
    };
    if !matches!(entries.list[at].body, EntryBody::User { .. }) {
        return Err(AppError::invalid(
            "entry_id",
            "Only your own messages can be edited.",
        ));
    }
    let summary = entries.ids[..at]
        .iter()
        .zip(&entries.list[..at])
        .rev()
        .find(|(_, e)| matches!(e.body, EntryBody::Summary { .. }))
        .map(|(id, _)| id.clone());
    Ok((entry_id.to_owned(), summary))
}

/// A result's stored text: with the edit note in front when the user changed the call (AI-17).
fn result_content(status: ToolStatus, content: String, edited: Option<&str>) -> String {
    match (status, edited.map(str::trim).filter(|a| !a.is_empty())) {
        (ToolStatus::Ok | ToolStatus::Error, Some(arguments)) => {
            format!("{EDIT_NOTE} {arguments}\n\n{content}")
        }
        _ => content,
    }
}

/// The system prompt and built-in tools of a request (§13.1): the terminal tools only with a
/// connected tab (AI-09), `web_search` when it can run (AI-14), `fetch_url` always, and
/// `read_skill` when an enabled skill exists, the built-in one included (AI-34). `fallback` is
/// `None` but for the request without tools that a Compact request (AI-21) or a side question
/// (AI-42) falls back to, whose `PromptTools` says what it asks for. `server_id` is the tab's SSH
/// server's identification string.
/// The prompt names the model and its provider, and carries the user's custom instructions
/// (AI-36) and the host's AI notes (AI-37), each cut to its limit.
fn prompt(
    v: &Vault,
    context: &AiTurnContext,
    fallback: Option<PromptTools>,
    server_id: Option<&str>,
) -> (String, Vec<ToolDef>) {
    let host = context
        .host_id
        .as_deref()
        .and_then(|id| v.get(id))
        .and_then(Item::as_host);
    let target = context.target.as_ref().map(quick_label);
    // §7.5: a local tab's computer, named like a host, and its shell.
    let local = local_tab(context);
    let local_named = local.map(|l| local_name(&l.device));
    // A WSL distribution's user is its own, which Windows does not know.
    let local_user = local
        .filter(|l| l.kind != Some(LocalShellKind::Wsl))
        .map(|_| crate::platform::system::default_user());
    let os = crate::platform::os_label();
    let local_prompt = local.and_then(|l| {
        Some(LocalPrompt {
            device: &l.device,
            os: &os,
            shell: l.shell.as_deref()?,
            place: tool_place(context),
            wsl: l.kind == Some(LocalShellKind::Wsl),
        })
    });
    let provider = v.get(&context.provider_id).and_then(Item::as_ai_provider);
    let model_name = provider
        .and_then(|p| p.models.iter().find(|m| m.id == context.model_id))
        .map_or("", |m| m.name.as_str());
    let settings = v.settings();
    let mut skills: Vec<(String, String)> = enabled_skills(v)
        .into_iter()
        .map(|(_, s)| (s.name, s.description))
        .collect();
    if settings.ai.builtin_skill_enabled {
        skills.push((BUILTIN_NAME.to_owned(), builtin_description()));
    }
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let offered = match (fallback, context.tab) {
        (Some(plain), _) => plain,
        (None, false) => PromptTools::NoTerminal,
        (None, true) => PromptTools::Terminal,
    };
    let system = tools::system_prompt(&PromptContext {
        model: PromptModel {
            id: &context.model_id,
            name: model_name,
            provider: provider.map_or("", |p| p.name.as_str()),
        },
        host_name: host
            .map(|h| h.name.as_str())
            .or(target.as_deref())
            .or(local_named.as_deref()),
        host_user: host
            .map(|h| h.username.as_str())
            .or(context.target.as_ref().map(|t| t.username.as_str()))
            .or(local_user.as_deref()),
        server_id,
        local: local_prompt,
        date: &date,
        instructions: bytes_prefix(
            &settings.ai.custom_instructions,
            MAX_CUSTOM_INSTRUCTIONS_BYTES,
        ),
        host_notes: host.map_or("", |h| bytes_prefix(&h.ai_notes, MAX_HOST_AI_NOTES_BYTES)),
        skills: &skills,
        memory: settings.ai.memory,
        tools: offered,
    });
    let defs = if fallback.is_none() {
        builtin_tools(v, context)
    } else {
        Vec::new()
    };
    (system, defs)
}

/// The built-in tools of a request (§13.4): the terminal tools only with a connected tab (AI-09),
/// `web_search` when a search provider is chosen or the model's provider has a search of its own
/// (AI-14), `fetch_url` always, `read_skill` when an enabled skill exists, the built-in one
/// included (AI-34), and `memory` while memory is on (AI-53).
fn builtin_tools(v: &Vault, context: &AiTurnContext) -> Vec<ToolDef> {
    tools::builtin_tools(&ToolSet {
        terminal: context.tab,
        web_search: web_search_for(v, &context.provider_id, &context.model_id).is_some(),
        read_skill: !enabled_skills(v).is_empty() || v.settings().ai.builtin_skill_enabled,
        memory: memory::on(v),
        place: tool_place(context),
    })
}

/// A local tab's computer and shell (§7.5), while the context names neither a host nor a target.
fn local_tab(context: &AiTurnContext) -> Option<&AiLocalTab> {
    context
        .local
        .as_ref()
        .filter(|_| context.host_id.is_none() && context.target.is_none())
}

/// Where the terminal tools act, which their descriptions say: a host, or this computer with
/// `sh` (macOS, Linux, WSL) or Windows itself (§7.5).
fn tool_place(context: &AiTurnContext) -> ToolPlace {
    match local_tab(context).and_then(|l| l.kind) {
        None => ToolPlace::Ssh,
        Some(LocalShellKind::Unix | LocalShellKind::Wsl) => ToolPlace::Local,
        Some(LocalShellKind::Powershell | LocalShellKind::Cmd | LocalShellKind::GitBash) => {
            ToolPlace::LocalWindows
        }
    }
}

/// At most the first `max` bytes of `s`, cut at a character. Saving refuses longer text; text
/// synced from elsewhere (an older build counted characters) is cut.
fn bytes_prefix(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// §13.1: the identification string of the tab's SSH server, read once when a turn starts, so
/// every request of the turn has the same system prompt. Only with a connected tab.
fn turn_server_id(env: &dyn AiEnv, context: &AiTurnContext) -> Option<String> {
    if !context.tab {
        return None;
    }
    context
        .session_id
        .as_deref()
        .and_then(|id| env.server_id(id))
}

/// The provider and model a request uses (AI-05). The error is a sentence for the panel.
fn resolve_model(
    v: &Vault,
    provider_id: &str,
    model_id: &str,
) -> Result<(ProviderConfig, ModelSpec), String> {
    let provider = v
        .get(provider_id)
        .and_then(Item::as_ai_provider)
        .ok_or_else(|| "The model's provider no longer exists. Choose another model.".to_owned())?;
    let model = provider
        .models
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| {
            format!("The model \"{model_id}\" is not in its provider's model list. Choose another model.")
        })?;
    Ok((
        provider_config(provider_id, provider),
        model_spec(provider, model),
    ))
}

/// Saved model `model` of `provider` as a request takes it: what the row leaves unknown comes
/// from the table of well-known models for the provider's protocol (AI-03), so a model saved
/// before the table knew it, or typed in, gets the same as one fetched now.
pub fn model_spec(provider: &AiProvider, model: &CoreModel) -> ModelSpec {
    let mut spec = ModelSpec {
        id: model.id.clone(),
        context_window: model.context_window,
        max_output_tokens: model.max_output_tokens,
        efforts: model
            .efforts
            .as_ref()
            .map(|levels| levels.iter().copied().map(model_effort).collect()),
        adaptive_thinking: model.adaptive_thinking,
    };
    spec.fill_known(request_protocol(provider));
    spec
}

/// The protocol of saved `provider`'s requests: a ChatGPT provider (AI-52) speaks Responses
/// whatever its stored protocol says.
fn request_protocol(provider: &AiProvider) -> Protocol {
    if provider.chatgpt.is_some() {
        Protocol::Responses
    } else {
        protocol(provider.protocol)
    }
}

/// AI-03: when the form saves `provider` with another protocol than `before` had, the levels and
/// adaptive thinking a row holds as the table of well-known models gave them for the old protocol
/// are dropped, so the table fills what the new one takes. The form shows a row's unknown fields
/// as the table has them and saves what it shows, so without this a gateway's Claude model moved
/// to Chat Completions would keep sending Anthropic's levels there.
pub fn drop_levels_of_old_protocol(before: &AiProvider, provider: &mut AiProvider) {
    let old = request_protocol(before);
    if old == request_protocol(provider) {
        return;
    }
    for model in &mut provider.models {
        let Some(known) = known_model(&model.id, old) else {
            continue;
        };
        let levels = model
            .efforts
            .as_ref()
            .map(|levels| levels.iter().copied().map(model_effort).collect::<Vec<_>>());
        if known.efforts.is_some() && levels.as_deref() == known.efforts {
            model.efforts = None;
        }
        if known.adaptive_thinking.is_some() && model.adaptive_thinking == known.adaptive_thinking {
            model.adaptive_thinking = None;
        }
    }
}

/// What a request needs to reach saved provider `id`. A ChatGPT provider (AI-52) is a Responses
/// provider at OpenAI's API, whose requests carry an access token of this device's session.
pub fn provider_config(id: &str, p: &AiProvider) -> ProviderConfig {
    if p.chatgpt.is_some() {
        return ProviderConfig {
            protocol: Protocol::Responses,
            base_url: chatgpt::BASE_URL.to_owned(),
            api_key: Zeroizing::default(),
            auth_header: AuthHeader::Authorization,
            chatgpt: Some(hatoba_ai::chatgpt::ChatgptAuth {
                sessions: chatgpt::sessions().clone(),
                id: id.to_owned(),
            }),
        };
    }
    ProviderConfig {
        protocol: protocol(p.protocol),
        base_url: p.base_url.clone(),
        api_key: p.api_key.clone(),
        auth_header: auth_header(p.auth_header),
        chatgpt: None,
    }
}

pub fn protocol(p: CoreProtocol) -> Protocol {
    match p {
        CoreProtocol::ChatCompletions => Protocol::ChatCompletions,
        CoreProtocol::Anthropic => Protocol::Anthropic,
        CoreProtocol::Responses => Protocol::Responses,
        CoreProtocol::SystemOne => Protocol::SystemOne,
    }
}

pub fn auth_header(h: AiAuthHeader) -> AuthHeader {
    match h {
        AiAuthHeader::XApiKey => AuthHeader::XApiKey,
        AiAuthHeader::Authorization => AuthHeader::Authorization,
    }
}

/// A thinking level from the panel, as stored (AI-05).
pub fn core_effort(e: AiEffort) -> CoreEffort {
    match e {
        AiEffort::Low => CoreEffort::Low,
        AiEffort::Medium => CoreEffort::Medium,
        AiEffort::High => CoreEffort::High,
        AiEffort::Xhigh => CoreEffort::Xhigh,
        AiEffort::Max => CoreEffort::Max,
    }
}

/// A stored thinking level, for the panel.
pub fn effort_view(e: CoreEffort) -> AiEffort {
    match e {
        CoreEffort::Low => AiEffort::Low,
        CoreEffort::Medium => AiEffort::Medium,
        CoreEffort::High => AiEffort::High,
        CoreEffort::Xhigh => AiEffort::Xhigh,
        CoreEffort::Max => AiEffort::Max,
    }
}

/// A thinking level from the panel, as a request takes it.
fn request_effort(e: AiEffort) -> Effort {
    model_effort(core_effort(e))
}

/// A level a stored model offers, as a request takes it.
fn model_effort(e: CoreEffort) -> Effort {
    match e {
        CoreEffort::Low => Effort::Low,
        CoreEffort::Medium => Effort::Medium,
        CoreEffort::High => Effort::High,
        CoreEffort::Xhigh => Effort::Xhigh,
        CoreEffort::Max => Effort::Max,
    }
}

/// A level from a provider's model list (AI-03), for the panel.
pub fn listed_effort(e: Effort) -> AiEffort {
    match e {
        Effort::Low => AiEffort::Low,
        Effort::Medium => AiEffort::Medium,
        Effort::High => AiEffort::High,
        Effort::Xhigh => AiEffort::Xhigh,
        Effort::Max => AiEffort::Max,
    }
}

/// What runs `web_search` (AI-14).
enum WebSearch {
    /// The search provider `Settings.ai` chose.
    Service(SearchConfig),
    /// The search of the model's own provider.
    Native(ProviderConfig, ModelSpec),
}

/// What runs `web_search` for model `model_id` of `provider_id` (AI-14): the search provider
/// `Settings.ai` chose, when it exists, or else, unless native search is off, the model's provider
/// with its own search. Every provider that chats has one; one Hatoba does not know is tried as
/// OpenAI is ([`NativeSearch::Other`]), so the tools a conversation is offered do not depend on
/// how a search went.
fn web_search_for(v: &Vault, provider_id: &str, model_id: &str) -> Option<WebSearch> {
    let settings = v.settings().ai;
    let chosen = settings
        .search_provider_id
        .as_deref()
        .and_then(|id| v.get(id))
        .and_then(Item::as_search_provider);
    if let Some(chosen) = chosen {
        return Some(WebSearch::Service(search_config(chosen)));
    }
    if !settings.native_search {
        return None;
    }
    let (provider, model) = resolve_model(v, provider_id, model_id).ok()?;
    NativeSearch::of(&provider)?;
    Some(WebSearch::Native(provider, model))
}

/// Why a `web_search` call cannot run (AI-14).
fn no_web_search(v: &Vault) -> &'static str {
    if v.settings().ai.native_search {
        "No search provider is chosen in Settings → AI, and the model that made the call is no \
         longer in its provider's model list, so web_search cannot run."
    } else {
        "No search provider is chosen in Settings → AI, so web_search cannot run."
    }
}

pub fn search_config(p: &SearchProvider) -> SearchConfig {
    SearchConfig {
        kind: search_kind(p.kind),
        base_url: p.base_url.clone(),
        api_key: p.api_key.clone(),
    }
}

pub fn search_kind(kind: CoreSearchKind) -> WebSearchKind {
    match kind {
        CoreSearchKind::Brave => WebSearchKind::Brave,
        CoreSearchKind::Tavily => WebSearchKind::Tavily,
        CoreSearchKind::Searxng => WebSearchKind::Searxng,
    }
}

// ---- tools that run in Rust ----

/// A tool call that runs in Rust, with what it needs read from the vault beforehand.
enum Job {
    /// The result is known without running anything: a vault read, or a call that cannot run.
    Ready(ToolStatus, String),
    RunCommand(RunCommandArgs),
    /// `read_file`, `edit_file` or `write_file`, with what the approval card read for it.
    File(FileJob, Option<files::Base>),
    /// `grep` or `glob`.
    Search(SearchJob),
    /// `web_search` through the search provider `Settings.ai` chose.
    WebSearch(SearchConfig, String),
    /// `web_search` with the search of the model's own provider (AI-14), which
    /// [`AiManager::native_search`] runs, since it keeps the providers that turned it away.
    NativeSearch(Box<(ProviderConfig, ModelSpec)>, String),
    /// `fetch_url`: the URL, the offset, and whether the whole page is asked for.
    FetchUrl(String, u64, bool),
    /// An MCP tool the request offered, with its arguments (AI-30), called directly or through
    /// `call_mcp_tool` (AI-45).
    Mcp(Box<OfferedTool>, Map<String, Value>),
    /// `search_tools` (AI-45), which [`AiManager::search_tools`] runs, since it reads the
    /// conversation's MCP tools.
    SearchTools(deferred::SearchArgs),
    /// `memory` (AI-53), which writes to the vault.
    Memory(MemoryArgs),
}

impl Job {
    fn error(message: impl Into<String>) -> Self {
        Self::Ready(ToolStatus::Error, message.into())
    }

    /// A call of an MCP tool: `tool` is what the offer that made the call names `name`. A server
    /// whose item is gone (deleted on another device, before a request stopped it here) runs
    /// nothing, even while its connection is still up.
    fn mcp(v: &Vault, tool: Option<OfferedTool>, name: &str, arguments: &str) -> Self {
        let Some(tool) = tool else {
            return Self::error(format!(
                "There is no MCP tool named \"{name}\": no MCP server offers it now."
            ));
        };
        if !crate::mcp::server_exists(v, &tool.server_id) {
            return Self::error(crate::mcp::server_gone(&tool.server_name));
        }
        match parse::<Map<String, Value>>(arguments) {
            Ok(arguments) => Self::Mcp(Box::new(tool), arguments),
            Err(message) => Self::error(message),
        }
    }

    /// `version`: the app's, for the built-in skill (AI-34). `caller`: the provider and model of
    /// the response that made the call, whose own search `web_search` may use (AI-14).
    fn new(v: &Vault, name: &str, arguments: &str, version: &str, caller: (&str, &str)) -> Self {
        if let Some(parsed) = FileJob::parse(name, arguments) {
            return match parsed {
                Ok(job) => Self::File(job, None),
                Err(message) => Self::error(message),
            };
        }
        if let Some(parsed) = SearchJob::parse(name, arguments) {
            return match parsed {
                Ok(job) => Self::Search(job),
                Err(message) => Self::error(message),
            };
        }
        match name {
            tools::RUN_COMMAND => match parse::<RunCommandArgs>(arguments) {
                Ok(args) if args.command.trim().is_empty() => Self::error("The command is empty."),
                Ok(args) => Self::RunCommand(args),
                Err(message) => Self::error(message),
            },
            tools::WEB_SEARCH => match web_search_for(v, caller.0, caller.1) {
                None => Self::error(no_web_search(v)),
                Some(search) => match (parse::<WebSearchArgs>(arguments), search) {
                    (Ok(args), WebSearch::Service(config)) => Self::WebSearch(config, args.query),
                    (Ok(args), WebSearch::Native(provider, model)) => {
                        Self::NativeSearch(Box::new((provider, model)), args.query)
                    }
                    (Err(message), _) => Self::error(message),
                },
            },
            tools::FETCH_URL => match parse::<FetchUrlArgs>(arguments) {
                Ok(args) => Self::FetchUrl(
                    args.url,
                    args.offset.unwrap_or(0),
                    args.full_page.unwrap_or(false),
                ),
                Err(message) => Self::error(message),
            },
            tools::READ_SKILL => match parse::<ReadSkillArgs>(arguments) {
                Ok(args) => read_skill(v, &args, version),
                Err(message) => Self::error(message),
            },
            tools::SEARCH_TOOLS => match deferred::parse_search(arguments) {
                Ok(args) => Self::SearchTools(args),
                Err(message) => Self::error(message),
            },
            tools::MEMORY => match parse::<MemoryArgs>(arguments) {
                Ok(args) => Self::Memory(args),
                Err(message) => Self::error(message),
            },
            tools::READ_TERMINAL | tools::SEND_INPUT => Self::error(format!(
                "{name} runs in the terminal tab and cannot run here."
            )),
            _ => Self::error(format!("There is no tool named \"{name}\".")),
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "what the tools that run in Rust need, each from its own place"
    )]
    async fn run(
        self,
        http: &reqwest::Client,
        mcp: &McpManager,
        vault: &SharedVault,
        env: &dyn AiEnv,
        session_id: Option<&str>,
        seen: files::SeenBy<'_>,
        cancel: &CancellationToken,
    ) -> (ToolStatus, String) {
        match self {
            Self::Ready(status, content) => (status, content),
            Self::Mcp(tool, arguments) => mcp.call(vault, &tool, arguments, cancel).await,
            // `tool_run` runs them: with the conversation's MCP tools, and with what the manager
            // keeps of native searches.
            Self::SearchTools(_) => (
                ToolStatus::Error,
                "search_tools cannot run here.".to_owned(),
            ),
            Self::NativeSearch(..) => (ToolStatus::Error, "web_search cannot run here.".to_owned()),
            Self::RunCommand(args) => run_command(env, session_id, &args, cancel).await,
            Self::Memory(args) => {
                let mut v = lock(vault);
                if v.is_unlocked() {
                    memory::run(&mut v, &args)
                } else {
                    (ToolStatus::Error, "The vault is locked.".to_owned())
                }
            }
            Self::File(job, base) => {
                files::run(live_session(env, session_id), job, base, seen, cancel).await
            }
            Self::Search(job) => search::run(live_session(env, session_id), job, cancel).await,
            Self::WebSearch(config, query) => {
                match web::web_search(http, &config, &query, cancel).await {
                    Ok(results) => (
                        ToolStatus::Ok,
                        tools::truncate_result(&web::format_search_results(&results)),
                    ),
                    Err(AiError::Cancelled) => (ToolStatus::Cancelled, String::new()),
                    Err(e) => (ToolStatus::Error, format!("The search failed: {e}")),
                }
            }
            // Pages through long content instead of being shortened (§13.4).
            Self::FetchUrl(url, offset, full_page) => {
                match web::fetch_url(&url, offset, full_page, cancel).await {
                    Ok(page) => (ToolStatus::Ok, web::format_fetch_result(&page)),
                    Err(AiError::Cancelled) => (ToolStatus::Cancelled, String::new()),
                    Err(e) => (
                        ToolStatus::Error,
                        format!("The page could not be fetched: {e}"),
                    ),
                }
            }
        }
    }
}

/// AI-45: the MCP tool a `call_mcp_tool` call with `arguments` calls in the conversation, with the
/// tool's arguments, resolved through the offer recorded last (the newest message's or the latest
/// search's, AI-30). The error is the call's result when it cannot run, so it neither asks nor
/// runs: arguments that cannot be read, a name the conversation has no tool of, a server that was
/// deleted, or arguments that do not match the tool's input schema. The approval card
/// (`mcp_proxy_info`) and the run (`ai_tool_run`) both resolve here.
pub fn proxy_call(
    v: &Vault,
    mcp: &McpManager,
    conversation_id: &str,
    arguments: &str,
) -> Result<(OfferedTool, Map<String, Value>), String> {
    let call = deferred::parse_call(arguments)?;
    let Some(tool) = mcp.offered(conversation_id, &call.tool) else {
        return Err(format!(
            "The call did not run: this conversation has no MCP tool named \"{}\" now. Call {} to \
             find the tools you can call.",
            call.tool,
            tools::SEARCH_TOOLS
        ));
    };
    if !crate::mcp::server_exists(v, &tool.server_id) {
        return Err(crate::mcp::server_gone(&tool.server_name));
    }
    let problems = deferred::check_arguments(&tool.tool.input_schema, &call.arguments);
    if !problems.is_empty() {
        return Err(tools::truncate_result(&deferred::mismatch_result(
            &tool.name,
            &problems,
            &tool.tool.input_schema,
        )));
    }
    Ok((tool, call.arguments))
}

fn parse<T: DeserializeOwned>(arguments: &str) -> Result<T, String> {
    let arguments = if arguments.trim().is_empty() {
        "{}"
    } else {
        arguments
    };
    serde_json::from_str(arguments).map_err(|e| format!("The arguments are not valid: {e}."))
}

/// What the tab's tools run over while it is open (AI-08).
fn live_session(env: &dyn AiEnv, session_id: Option<&str>) -> Option<conn::Conn> {
    session_id
        .and_then(|id| env.session(id))
        .filter(|s| !s.is_closed())
}

/// The server a revert (AI-41) on the tab `session_id` runs on, as file changes name it; `None`
/// while the tab is not connected.
pub fn session_server(env: &dyn AiEnv, session_id: Option<&str>) -> Option<String> {
    live_session(env, session_id).map(|s| s.server())
}

/// AI-12: `command` on a new exec channel of the tab's connection, or apart from a local tab's
/// shell (§7.5), in the call's `workdir` and with its timeout. A stop or the timeout ends the
/// command and the processes it started, and the result says whether they ended (AI-18).
async fn run_command(
    env: &dyn AiEnv,
    session_id: Option<&str>,
    args: &RunCommandArgs,
    cancel: &CancellationToken,
) -> (ToolStatus, String) {
    let Some(session) = live_session(env, session_id) else {
        return (ToolStatus::Error, DISCONNECTED.to_owned());
    };
    let limit = args.timeout();
    let stop = async {
        tokio::select! {
            () = cancel.cancelled() => {}
            () = tokio::time::sleep(limit) => {}
        }
    };
    let workdir = args.workdir();
    let ran = match &session {
        conn::Conn::Ssh(ssh) => remote_command::run(ssh, &args.command, workdir, stop)
            .await
            .map_err(|e| e.message),
        conn::Conn::Local(local) => local.tools.run_command(&args.command, workdir, stop).await,
    };
    match ran {
        Ok(run) => {
            let stdout = String::from_utf8_lossy(&run.output.stdout);
            let stderr = String::from_utf8_lossy(&run.output.stderr);
            let Some(stopped) = run.stopped else {
                return (
                    ToolStatus::Ok,
                    tools::truncate_result(&tools::format_exec_result(
                        run.output.exit_status,
                        &stdout,
                        &stderr,
                    )),
                );
            };
            let state = match stopped {
                remote_command::Stopped::NotStarted => StoppedCommand::NotStarted,
                remote_command::Stopped::Ended => StoppedCommand::Ended,
                remote_command::Stopped::MayRun => StoppedCommand::MayRun,
            };
            let (status, stop) = if cancel.is_cancelled() {
                (ToolStatus::Cancelled, CommandStop::User)
            } else {
                (ToolStatus::Error, CommandStop::Timeout(limit.as_secs()))
            };
            let content = tools::stopped_command_result(stop, state, &stdout, &stderr);
            (status, tools::truncate_result(&content))
        }
        Err(_) if session.is_closed() => (ToolStatus::Error, DISCONNECTED.to_owned()),
        Err(message) => (
            ToolStatus::Error,
            format!("The command could not run: {message}"),
        ),
    }
}

/// The enabled skills the user saved. One named like the built-in skill (from a build before
/// the name was reserved) is left out: the name means the built-in skill (AI-34).
fn enabled_skills(v: &Vault) -> Vec<(String, hatoba_core::model::Skill)> {
    v.skills()
        .into_iter()
        .filter(|(_, s)| s.enabled && s.name != BUILTIN_NAME)
        .collect()
}

/// AI-28: the `SKILL.md` body of an enabled skill, or the file at `path` inside it. `hatoba` is
/// the built-in skill while it is switched on, with `version` filled in (AI-34).
fn read_skill(v: &Vault, args: &ReadSkillArgs, version: &str) -> Job {
    let name = args.name.trim();
    let Some(files) = enabled_skill_files(v, name, version) else {
        let mut names: Vec<String> = skill_commands(v).into_iter().map(|c| c.name).collect();
        names.sort_unstable();
        let list = if names.is_empty() {
            "none".to_owned()
        } else {
            names.join(", ")
        };
        return Job::error(format!(
            "There is no enabled skill named \"{name}\". Enabled skills: {list}."
        ));
    };
    let path = args
        .path
        .as_deref()
        .map(skill_path)
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| SKILL_MD.to_owned());
    match files.iter().find(|(p, _)| *p == path) {
        Some((_, content)) => Job::Ready(ToolStatus::Ok, tools::truncate_result(content)),
        None => {
            let mut paths: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
            paths.sort_unstable();
            Job::error(format!(
                "The skill \"{name}\" has no file \"{path}\". Its files: {}.",
                if paths.is_empty() {
                    "none".to_owned()
                } else {
                    paths.join(", ")
                }
            ))
        }
    }
}

/// `(path, content)` of every file of the enabled skill `name`, `SKILL.md` included: the built-in
/// one while it is switched on, with `version` filled in (AI-34). `None` when no enabled skill
/// has that name.
fn enabled_skill_files(v: &Vault, name: &str, version: &str) -> Option<Vec<(String, String)>> {
    if name == BUILTIN_NAME {
        if !v.settings().ai.builtin_skill_enabled {
            return None;
        }
        let skill = builtin_skill(version);
        return Some(
            std::iter::once((SKILL_MD.to_owned(), skill.body))
                .chain(skill.files.into_iter().map(|f| (f.path, f.content)))
                .collect(),
        );
    }
    let enabled = enabled_skills(v);
    let (skill_id, _) = enabled.iter().find(|(_, s)| s.name == name)?;
    Some(
        v.skill_files(skill_id)
            .into_iter()
            .map(|(_, f)| (f.path, f.content))
            .collect(),
    )
}

/// AI-50: the skills the panel's `/` menu offers as commands: every enabled skill, and the
/// built-in one while it is switched on (AI-34), sorted by name, as the system prompt lists them
/// (AI-28).
pub fn skill_commands(v: &Vault) -> Vec<AiSkillCommand> {
    let mut commands: Vec<AiSkillCommand> = enabled_skills(v)
        .into_iter()
        .map(|(_, s)| AiSkillCommand {
            name: s.name,
            description: s.description,
        })
        .collect();
    if v.settings().ai.builtin_skill_enabled {
        commands.push(AiSkillCommand {
            name: BUILTIN_NAME.to_owned(),
            description: builtin_description(),
        });
    }
    commands.sort_by(|a, b| a.name.cmp(&b.name));
    commands
}

/// AI-50: what a `/<skill>` command attaches in its `skill` block: the skill's `SKILL.md` as
/// `read_skill` reads it, but whole, where `read_skill` cuts a result past 30,000 characters.
pub fn skill_text(v: &Vault, name: &str, version: &str) -> AppResult<String> {
    enabled_skill_files(v, name.trim(), version)
        .and_then(|files| files.into_iter().find(|(p, _)| p == SKILL_MD))
        .map(|(_, content)| content)
        .ok_or_else(|| AppError::not_found("skill"))
}

/// A path inside a skill as the model may write it: `./references\x.md` is `references/x.md`.
fn skill_path(path: &str) -> String {
    let path = path.trim().replace('\\', "/");
    let mut rest = path.as_str();
    while let Some(stripped) = rest.strip_prefix("./").or_else(|| rest.strip_prefix('/')) {
        rest = stripped;
    }
    rest.to_owned()
}

// ---- stored entries ----

/// A conversation's stored entries, decoded, oldest first. Unreadable entries are left out.
struct Entries {
    ids: Vec<String>,
    list: Vec<AiEntry>,
}

impl Entries {
    fn load(v: &Vault, conversation_id: &str) -> AppResult<Self> {
        Ok(Self::decode(
            conversation_id,
            v.ai_entries(conversation_id)?,
        ))
    }

    /// The entries from `entry_id` on, that entry included if it is stored. Earlier entries are
    /// not read from the store.
    fn load_from(v: &Vault, conversation_id: &str, entry_id: &str) -> AppResult<Self> {
        Ok(Self::decode(
            conversation_id,
            v.ai_entries_from(conversation_id, entry_id)?,
        ))
    }

    /// The entries from `response` on, or all of them before it is known: the checks of an open
    /// call read only the conversation's newest response and what follows it.
    fn load_open(v: &Vault, conversation_id: &str, response: Option<&str>) -> AppResult<Self> {
        match response {
            Some(id) => Self::load_from(v, conversation_id, id),
            None => Self::load(v, conversation_id),
        }
    }

    /// The id of the newest response.
    fn newest_response(&self) -> Option<String> {
        let at = self
            .list
            .iter()
            .rposition(|e| matches!(e.body, EntryBody::Assistant(_)))?;
        Some(self.ids[at].clone())
    }

    fn decode(conversation_id: &str, stored: Vec<hatoba_core::StoredEntry>) -> Self {
        let mut entries = Self {
            ids: Vec::with_capacity(stored.len()),
            list: Vec::with_capacity(stored.len()),
        };
        for hatoba_core::StoredEntry { entry_id, json } in stored {
            match serde_json::from_str::<AiEntry>(&json) {
                Ok(entry) => {
                    entries.ids.push(entry_id);
                    entries.list.push(entry);
                }
                Err(_) => tracing::warn!(
                    conversation_id,
                    entry_id = %entry_id,
                    "unreadable conversation entry skipped"
                ),
            }
        }
        entries
    }

    /// Index of the first entry of the context (AI-21): at or after `context_start`, which
    /// entry ids order.
    fn context_from(&self, context_start: Option<&str>) -> usize {
        context_start.map_or(0, |start| {
            self.ids.partition_point(|id| id.as_str() < start)
        })
    }

    fn views(&self) -> Vec<AiEntryView> {
        self.ids
            .iter()
            .zip(&self.list)
            .map(|(id, entry)| entry_view(id, entry))
            .collect()
    }

    /// The results stored after the newest response for its calls `call_ids`, oldest first.
    fn newest_results(&self, call_ids: &[String]) -> Vec<AiEntryView> {
        let after = self
            .list
            .iter()
            .rposition(|e| matches!(e.body, EntryBody::Assistant(_)))
            .map_or(0, |i| i + 1);
        self.ids[after..]
            .iter()
            .zip(&self.list[after..])
            .filter(|(_, e)| {
                matches!(&e.body, EntryBody::Tool { tool_call_id, .. } if call_ids.contains(tool_call_id))
            })
            .map(|(id, entry)| entry_view(id, entry))
            .collect()
    }
}

/// Stores a result for every stored call that has none (§13.1), a cancelled one unless `known`
/// has it, and returns them.
/// `known`: the results of calls that ran when the turn stopped: what a tool that had ended
/// returned, or a cancelled result that says what the call may have done (a write that may have
/// changed the file, a command that may still run).
fn cancel_open_calls(
    v: &mut Vault,
    conversation_id: &str,
    entries: &Entries,
    mut known: HashMap<String, (ToolStatus, String)>,
) -> AppResult<Vec<AiEntryView>> {
    let now = now_ms();
    fix_up_missing_results(&entries.list)
        .into_iter()
        .map(|call_id| {
            let entry = match known.remove(&call_id) {
                Some((status, content)) => AiEntry::tool(now, call_id, status, content),
                None => AiEntry::cancelled_result(now, call_id),
            };
            let entry_id = append(v, conversation_id, &entry)?;
            Ok(entry_view(&entry_id, &entry))
        })
        .collect()
}

fn append(v: &mut Vault, conversation_id: &str, entry: &AiEntry) -> AppResult<String> {
    let json = Zeroizing::new(
        serde_json::to_string(entry)
            .map_err(|_| AppError::internal("could not encode a conversation entry"))?,
    );
    Ok(v.ai_append_entry(conversation_id, &json)?)
}

pub fn find_conversation(v: &Vault, id: &str) -> AppResult<AiConversation> {
    v.get(id)
        .and_then(Item::as_ai_conversation)
        .cloned()
        .ok_or_else(|| AppError::not_found("conversation"))
}

/// AI-23: the title starts as the first line of the first message, cut to 60 characters. The
/// blocks the message starts with (spec §13.3) are skipped, so the title is the first non-empty
/// line the user typed, or the first attachment's name when nothing was typed (the panel sends
/// nothing without typed text, so that is only a fallback). Hatoba's notes never title it.
fn title_of(text: &str) -> String {
    let (blocks, typed) = leading_blocks(text);
    let line = typed
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            blocks
                .iter()
                .find(|b| !b.kind.note)
                .map(|b| (b.kind.title)(&b.attrs))
        })
        .unwrap_or_default();
    line.chars().take(60).collect()
}

/// The tag of the note Hatoba puts before the first message after a reply the user stopped
/// (§13.1).
const INTERRUPTED: &str = "interrupted";
/// The tag of the note Hatoba puts before a message that moves the conversation to another
/// host (AI-09).
const HOST_CHANGE: &str = "host_change";
/// The tag of the note Hatoba puts before a message whose requests offer the terminal tools when
/// the newest reply's did not, or the other way round (AI-09).
const TERMINAL_CHANGE: &str = "terminal_change";
/// The tag of the note Hatoba puts before the first message to another model (AI-05).
const MODEL_CHANGE: &str = "model_change";
/// The tag of the note Hatoba puts before a message sent in another permission mode than the
/// model was last told of (AI-16), and before a summary of a conversation in bypass or Auto
/// (AI-47).
const PERMISSION_MODE: &str = "permission_mode";
/// The tag of the note Hatoba puts before the first message after the user reverted a change of
/// a file tool call (AI-41), one for each call.
const FILE_REVERT: &str = "file_revert";
/// The tag of the note Hatoba puts before a message that comes with another list of deferred MCP
/// tools than the model last saw (AI-45). A conversation with one is deferred.
const MCP_TOOLS: &str = deferred::LIST_TAG;
/// The tag of the note Hatoba puts before a message sent with a connected terminal attached,
/// which shows its screen (AI-46).
const TERMINAL_STATE: &str = "terminal_state";
/// The tag of the note Hatoba puts before a conversation's first message and in a summary, which
/// shows the memories (AI-53).
const MEMORY: &str = "memory";

/// A kind of block a user entry may start with (spec §13.3, "Attachment blocks").
struct AttachmentKind {
    tag: &'static str,
    /// A note Hatoba writes, rather than something the user attached.
    note: bool,
    /// The title of a conversation whose first message has only this block, from its attributes.
    title: fn(&[(&str, &str)]) -> String,
    /// What the judge of Auto mode reads in place of the block (AI-47), from its attributes: its
    /// kind and size. Empty for a note, which the judge does not read.
    placeholder: fn(&[(&str, &str)]) -> String,
    /// Whether the opening tag's attributes, in order, are the ones this kind writes.
    attributes: fn(&[(&str, &str)]) -> bool,
}

/// A `lines` attribute: a decimal count.
fn is_count(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

/// The size an attachment's `lines` attribute gives, such as `40 lines`, for a placeholder.
fn lines_of(attrs: &[(&str, &str)]) -> String {
    match attrs.iter().find(|(name, _)| *name == "lines") {
        Some((_, "1")) => "1 line".to_owned(),
        Some((_, count)) => format!("{count} lines"),
        None => "no lines".to_owned(),
    }
}

/// An attribute value as written, with `&amp;`, `&quot;`, `&lt;` and `&gt;` read back.
fn attribute_text(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The blocks a user entry may start with. Hatoba writes the notes here, before the blocks the
/// panel writes (`features/ai/attachments.ts`), whose parser follows the same rules.
const ATTACHMENTS: &[AttachmentKind] = &[
    AttachmentKind {
        tag: INTERRUPTED,
        note: true,
        title: |_| String::new(),
        placeholder: |_| String::new(),
        attributes: |attrs| attrs.is_empty(),
    },
    AttachmentKind {
        tag: HOST_CHANGE,
        note: true,
        title: |_| String::new(),
        placeholder: |_| String::new(),
        attributes: |attrs| matches!(attrs, [("from", _), ("to", _)]),
    },
    AttachmentKind {
        tag: TERMINAL_CHANGE,
        note: true,
        title: |_| String::new(),
        placeholder: |_| String::new(),
        attributes: |attrs| matches!(attrs, [("to", "attached" | "detached")]),
    },
    AttachmentKind {
        tag: MODEL_CHANGE,
        note: true,
        title: |_| String::new(),
        placeholder: |_| String::new(),
        attributes: |attrs| matches!(attrs, [("from", _), ("to", _)]),
    },
    AttachmentKind {
        tag: PERMISSION_MODE,
        note: true,
        title: |_| String::new(),
        placeholder: |_| String::new(),
        attributes: |attrs| matches!(attrs, [("mode", "manual" | "auto" | "bypass")]),
    },
    AttachmentKind {
        tag: MEMORY,
        note: true,
        title: |_| String::new(),
        placeholder: |_| String::new(),
        attributes: |attrs| matches!(attrs, [("entries", entries)] if is_count(entries)),
    },
    AttachmentKind {
        tag: FILE_REVERT,
        note: true,
        title: |_| String::new(),
        placeholder: |_| String::new(),
        attributes: |attrs| matches!(attrs, [("path", _), ("tool_call_id", _)]),
    },
    AttachmentKind {
        tag: TERMINAL_STATE,
        note: true,
        title: |_| String::new(),
        placeholder: |_| String::new(),
        attributes: |attrs| {
            matches!(attrs, [("host", _), ("alternate_screen", "true" | "false"), ("lines", lines)]
                if is_count(lines))
        },
    },
    AttachmentKind {
        tag: MCP_TOOLS,
        note: true,
        title: |_| String::new(),
        placeholder: |_| String::new(),
        attributes: |attrs| {
            matches!(attrs, [("servers", servers), ("tools", tools)]
                if is_count(servers) && is_count(tools))
        },
    },
    AttachmentKind {
        tag: "terminal_selection",
        note: false,
        title: |_| "Terminal selection".to_owned(),
        placeholder: |attrs| format!("[terminal selection, {}]", lines_of(attrs)),
        attributes: |attrs| match attrs {
            [("host", _), ("lines", lines)]
            | [("host", _), ("lines", lines), ("truncated", "true")] => is_count(lines),
            _ => false,
        },
    },
    AttachmentKind {
        tag: "connection_diagnostics",
        note: false,
        title: |_| "Connection diagnostics".to_owned(),
        placeholder: |_| "[connection diagnostics]".to_owned(),
        attributes: |attrs| matches!(attrs, [("host", _)]),
    },
    AttachmentKind {
        tag: "pasted_text",
        note: false,
        title: |_| "Pasted text".to_owned(),
        placeholder: |attrs| format!("[pasted text, {}]", lines_of(attrs)),
        attributes: |attrs| matches!(attrs, [("lines", lines)] if is_count(lines)),
    },
    AttachmentKind {
        tag: "file",
        note: false,
        title: |attrs| {
            attrs
                .first()
                .map(|(_, name)| attribute_text(name))
                .unwrap_or_default()
        },
        placeholder: |attrs| {
            let name = attrs.first().map(|(_, name)| attribute_text(name));
            format!("[file {}, {}]", name.unwrap_or_default(), lines_of(attrs))
        },
        attributes: |attrs| matches!(attrs, [("name", _), ("lines", lines)] if is_count(lines)),
    },
    AttachmentKind {
        tag: "host_file",
        note: false,
        title: |attrs| {
            let path = attrs.get(1).map(|(_, path)| attribute_text(path));
            path.as_deref()
                .map(|p| p.rsplit('/').find(|part| !part.is_empty()).unwrap_or(p))
                .unwrap_or_default()
                .to_owned()
        },
        placeholder: |attrs| {
            let path = attrs.get(1).map(|(_, path)| attribute_text(path));
            format!(
                "[file {} on the host, {}]",
                path.unwrap_or_default(),
                lines_of(attrs)
            )
        },
        attributes: |attrs| matches!(attrs, [("host", _), ("path", _), ("lines", lines)] if is_count(lines)),
    },
    AttachmentKind {
        tag: "host_directory",
        note: false,
        title: |attrs| {
            attrs
                .get(1)
                .map(|(_, path)| attribute_text(path))
                .unwrap_or_default()
        },
        placeholder: |attrs| {
            let path = attrs.get(1).map(|(_, path)| attribute_text(path));
            let entries = attrs.get(2).map_or("0", |(_, n)| *n);
            format!(
                "[listing of the directory {} on the host, {entries} entries]",
                path.unwrap_or_default()
            )
        },
        attributes: |attrs| match attrs {
            [("host", _), ("path", _), ("entries", entries)]
            | [
                ("host", _),
                ("path", _),
                ("entries", entries),
                ("truncated", "true"),
            ] => is_count(entries),
            _ => false,
        },
    },
    AttachmentKind {
        tag: "mcp_resource",
        note: false,
        title: |attrs| {
            attrs
                .get(1)
                .map(|(_, uri)| attribute_text(uri))
                .unwrap_or_default()
        },
        placeholder: |attrs| {
            let server = attrs.first().map(|(_, server)| attribute_text(server));
            let uri = attrs.get(1).map(|(_, uri)| attribute_text(uri));
            format!(
                "[MCP resource {} of {}, {}]",
                uri.unwrap_or_default(),
                server.unwrap_or_default(),
                lines_of(attrs)
            )
        },
        attributes: |attrs| matches!(attrs, [("server", _), ("uri", _), ("lines", lines)] if is_count(lines)),
    },
    AttachmentKind {
        tag: "skill",
        note: false,
        title: |attrs| {
            attrs
                .first()
                .map(|(_, name)| format!("/{}", attribute_text(name)))
                .unwrap_or_default()
        },
        placeholder: |attrs| {
            let name = attrs.first().map(|(_, name)| attribute_text(name));
            format!("[skill /{} run]", name.unwrap_or_default())
        },
        attributes: |attrs| matches!(attrs, [("name", _)]),
    },
    AttachmentKind {
        tag: "mcp_prompt",
        note: false,
        title: |attrs| {
            attrs
                .get(1)
                .map(|(_, name)| attribute_text(name))
                .unwrap_or_default()
        },
        placeholder: |attrs| {
            let server = attrs.first().map(|(_, server)| attribute_text(server));
            let name = attrs.get(1).map(|(_, name)| attribute_text(name));
            format!(
                "[MCP prompt {} of {} run]",
                name.unwrap_or_default(),
                server.unwrap_or_default()
            )
        },
        attributes: |attrs| matches!(attrs, [("server", _), ("name", _)]),
    },
];

/// One block a user entry starts with: its kind and its attributes as written.
struct Block<'a> {
    kind: &'static AttachmentKind,
    attrs: Vec<(&'a str, &'a str)>,
}

/// The attachments a message has at most one of, as the panel attaches them (AI-10).
const SINGLE_ATTACHMENTS: [&str; 2] = ["terminal_selection", "connection_diagnostics"];

/// The blocks `text` starts with, in order, and the text after them. Notes come first, at most
/// one of each kind but `file_revert`, which comes once for each reverted call: a note after an
/// attachment, or a second one of another kind, is typed text, and so is a second selection or
/// diagnostics block.
fn leading_blocks(text: &str) -> (Vec<Block<'_>>, &str) {
    let mut blocks: Vec<Block<'_>> = Vec::new();
    let mut rest = text;
    while let Some((block, after)) = leading_block(rest) {
        let repeated = blocks.iter().any(|b| b.kind.tag == block.kind.tag);
        let typed = if block.kind.note {
            blocks.iter().any(|b| !b.kind.note) || (repeated && block.kind.tag != FILE_REVERT)
        } else {
            repeated && SINGLE_ATTACHMENTS.contains(&block.kind.tag)
        };
        if typed {
            break;
        }
        blocks.push(block);
        rest = after;
    }
    (blocks, rest)
}

/// An attribute of the note `tag` among the notes a user entry starts with, read back.
fn note_attr(text: &str, tag: &str, name: &str) -> Option<String> {
    leading_blocks(text)
        .0
        .iter()
        .take_while(|b| b.kind.note)
        .find(|b| b.kind.tag == tag)
        .and_then(|b| b.attrs.iter().find(|(n, _)| *n == name))
        .map(|(_, value)| attribute_text(value))
}

/// Whether a user entry starts with the note `tag` among its notes.
fn has_note(text: &str, tag: &str) -> bool {
    leading_blocks(text)
        .0
        .iter()
        .take_while(|b| b.kind.note)
        .any(|b| b.kind.tag == tag)
}

/// The whole text of the note `tag` among the notes a user entry starts with, its blank line
/// included.
fn note_block<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let (blocks, _) = leading_blocks(text);
    let mut rest = text;
    for block in blocks.iter().take_while(|b| b.kind.note) {
        let (_, after) = leading_block(rest)?;
        if block.kind.tag == tag {
            return Some(&rest[..rest.len() - after.len()]);
        }
        rest = after;
    }
    None
}

/// AI-45: whether a conversation's MCP tools are deferred, which they are from the first message
/// that listed them in an `mcp_tools` note. `entries` is every entry of the conversation, so a
/// compaction leaves the choice as it was.
fn mcp_deferred(entries: &[AiEntry]) -> bool {
    entries.iter().any(|e| match &e.body {
        EntryBody::User { text, .. } => has_note(text, MCP_TOOLS),
        _ => false,
    })
}

/// AI-45: the `mcp_tools` note the conversation's next message starts with, when its requests
/// offer the MCP tools of `offer`: none while its MCP tools are offered directly, which they stay
/// until their definitions come to a tenth of the model's context window or the requests would
/// offer more than 128 tools, built-in ones included; then the list of those
/// tools, unless the newest note in the context (`entries` from `from` on) lists them already. No
/// note in the context counts as an empty list, so an empty list after a compaction is not noted.
fn mcp_note(
    v: &Vault,
    context: &AiTurnContext,
    entries: &[AiEntry],
    from: usize,
    offer: &Offer,
) -> String {
    let Ok((provider, model)) = resolve_model(v, &context.provider_id, &context.model_id) else {
        return String::new();
    };
    let is_deferred = mcp_deferred(entries) || {
        let defs = offer.tool_defs(provider.protocol);
        !defs.is_empty()
            && deferred::defer_due(
                estimate_tokens(&serde_json::to_string(&defs).unwrap_or_default()),
                model.context_window,
                builtin_tools(v, context).len() + defs.len(),
            )
    };
    if !is_deferred {
        return String::new();
    }
    let note = deferred::list_block(&offer.listed(), deferred::list_budget(model.context_window));
    let seen = entries[from.min(entries.len())..]
        .iter()
        .rev()
        .find_map(|e| match &e.body {
            EntryBody::User { text, .. } => note_block(text, MCP_TOOLS),
            _ => None,
        })
        .map_or_else(|| deferred::list_block(&[], u64::MAX), str::to_owned);
    if note.trim_end() == seen.trim_end() {
        String::new()
    } else {
        note
    }
}

/// The block `text` starts with (spec §13.3), and the text after it: the opening tag with its
/// attributes (`name="value"`, values without `"`), a line break, the body, a line break and the
/// closing tag, then one blank line, one line break or the end. The body ends at the first
/// closing tag that such a break or the end follows; its writer escapes closing tags inside it.
fn leading_block(text: &str) -> Option<(Block<'_>, &str)> {
    let rest = text.strip_prefix('<')?;
    let kind = ATTACHMENTS.iter().find(|k| {
        rest.strip_prefix(k.tag)
            .is_some_and(|after| after.starts_with([' ', '>']))
    })?;
    let mut rest = &rest[kind.tag.len()..];
    let mut attrs = Vec::new();
    while let Some(after) = rest.strip_prefix(' ') {
        let (name, after) = after.split_once("=\"")?;
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
            return None;
        }
        let (value, after) = after.split_once('"')?;
        attrs.push((name, value));
        rest = after;
    }
    if !(kind.attributes)(&attrs) {
        return None;
    }
    let body = rest.strip_prefix(">\n")?;
    let close = format!("\n</{}>", kind.tag);
    let mut from = 0;
    while let Some(at) = body[from..].find(&close) {
        let after = &body[from + at + close.len()..];
        if let Some(typed) = after
            .strip_prefix("\n\n")
            .or_else(|| after.strip_prefix('\n'))
        {
            return Some((Block { kind, attrs }, typed));
        }
        if after.is_empty() {
            return Some((Block { kind, attrs }, after));
        }
        from += at + 1;
    }
    None
}

/// How a quick-connect target (HOST-12) is named where a host's display name would be.
fn quick_label(target: &QuickTarget) -> String {
    crate::ssh::target_label(&target.username, &target.address, target.port)
}

/// The quick-connect target of a tab without a saved host, as the conversation item records it
/// (AI-09).
fn tab_target(context: &AiTurnContext) -> Option<AiQuickTarget> {
    context
        .target
        .as_ref()
        .filter(|_| context.host_id.is_none())
        .map(|t| AiQuickTarget::new(&t.username, &t.address, t.port))
}

/// The computer of a local tab (§7.5), or the one a home tab conversation recorded, while the
/// context names neither a host nor a target.
fn tab_local(context: &AiTurnContext) -> Option<String> {
    context
        .local
        .as_ref()
        .filter(|_| context.host_id.is_none() && context.target.is_none())
        .map(|l| l.device.clone())
}

/// What a local tab's computer (§7.5) is called where a host's display name would be: in notes,
/// the judge's state and the system prompt of a conversation without a terminal.
fn local_name(device: &str) -> String {
    format!("{device} {LOCAL_SUFFIX}")
}

/// What follows a computer's name in [`local_name`].
const LOCAL_SUFFIX: &str = "(local terminal)";

/// A saved host's address, port, and user in the form a conversation records a target, which is
/// equal to a target's when HOST-12 would pick the host for it.
fn host_target(v: &Vault, host_id: &str) -> Option<AiQuickTarget> {
    v.get(host_id)
        .and_then(Item::as_host)
        .map(|h| AiQuickTarget::new(&h.username, &h.address, h.port))
}

/// How a recorded quick-connect target is named in a `host_change` note: `user@host:port`.
fn target_name(target: &AiQuickTarget) -> String {
    crate::ssh::target_label(&target.username, &target.address, target.port)
}

/// Whether a `host_change` note's `from` names the server at `here` (AI-09): the target's name,
/// or the display name of a saved host with its address, port, and user.
fn names_server(v: &Vault, from: &str, here: &AiQuickTarget) -> bool {
    target_name(here) == from
        || v.hosts().iter().any(|(_, h)| {
            h.name == from && AiQuickTarget::new(&h.username, &h.address, h.port) == *here
        })
}

/// AI-43: where the copies of a fork came from (AI-09), from `later`, the original's entries after
/// the fork point, and the fork's `origin` (the field's doc). The first later user message that
/// records where the entries before it came from tells it. In a conversation from before messages
/// recorded it, the first later `host_change` note does instead, by name ([`place_named`]). With
/// neither, the original has stayed where the copies came from.
fn fork_place(
    later: &[AiEntry],
    original: &AiConversation,
    hosts: &[(String, String)],
) -> (Place, Option<String>) {
    for entry in later {
        let EntryBody::User { text, came_from } = &entry.body else {
            continue;
        };
        if let Some(came_from) = came_from {
            let place = Place {
                host_id: came_from.host_id.clone(),
                quick_target: came_from
                    .quick_target
                    .as_ref()
                    .map(|t| AiQuickTarget::new(&t.username, &t.address, t.port)),
                local: came_from.local.clone(),
            };
            return (place, came_from.origin.clone());
        }
        if let Some(from) = note_attr(text, HOST_CHANGE, "from") {
            return place_named(hosts, from);
        }
    }
    let place = Place {
        host_id: original.host_id.clone(),
        quick_target: original.quick_target.clone(),
        local: original.local.clone(),
    };
    (place, original.origin.clone())
}

/// AI-43: the place a `host_change` note's `from` names, among `hosts` (ids and display names):
/// the only saved host with that display name, or else the quick-connect target it writes as
/// `user@host:port`. A display name may look like a target, so the saved host comes first. When it
/// names neither, as for a host renamed or deleted since (whose name the note leaves empty) or a
/// name two hosts share, no place, and `from` as the fork's `origin`.
fn place_named(hosts: &[(String, String)], from: String) -> (Place, Option<String>) {
    let mut named = hosts
        .iter()
        .filter(|(_, name)| !from.is_empty() && *name == from);
    if let (Some((host_id, _)), None) = (named.next(), named.next()) {
        let place = Place {
            host_id: Some(host_id.clone()),
            quick_target: None,
            local: None,
        };
        return (place, None);
    }
    if let Some(device) = from
        .strip_suffix(LOCAL_SUFFIX)
        .and_then(|d| d.strip_suffix(' '))
        .filter(|d| !d.is_empty())
    {
        let place = Place {
            host_id: None,
            quick_target: None,
            local: Some(device.to_owned()),
        };
        return (place, None);
    }
    let place = Place {
        host_id: None,
        quick_target: parse_target(&from),
        local: None,
    };
    let origin = place.quick_target.is_none().then_some(from);
    (place, origin)
}

/// Where a conversation is (AI-09, AI-43), as a user message records where the entries before it
/// came from.
fn entry_place(conversation: &AiConversation) -> EntryPlace {
    EntryPlace {
        host_id: conversation.host_id.clone(),
        quick_target: conversation.quick_target.as_ref().map(|t| EntryTarget {
            username: t.username.clone(),
            address: t.address.clone(),
            port: t.port,
        }),
        origin: conversation.origin.clone(),
        local: conversation.local.clone(),
    }
}

/// A quick-connect target as [`target_name`] writes it (`user@host:port`, an IPv6 address in
/// brackets), in the form a conversation records it. `None` for anything else, such as a display
/// name.
fn parse_target(label: &str) -> Option<AiQuickTarget> {
    if label.chars().any(char::is_whitespace) {
        return None;
    }
    let (username, rest) = label.rsplit_once('@')?;
    let (address, port) = rest.rsplit_once(':')?;
    if !port.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let port = port.parse().ok()?;
    let address = match address.strip_prefix('[') {
        Some(inner) => inner.strip_suffix(']')?,
        None if address.contains(':') => return None,
        None => address,
    };
    if username.is_empty() || address.is_empty() {
        return None;
    }
    Some(AiQuickTarget::new(username, address, port))
}

/// Where a message moves its conversation (AI-09): to a saved host, to the quick-connect target
/// it records in place of one (HOST-12), or to a computer's local terminal (§7.5).
struct Place {
    host_id: Option<String>,
    quick_target: Option<AiQuickTarget>,
    local: Option<String>,
}

/// What [`AiManager::open_turn`] hands on: the conversation's id, the turn, the notes, the move,
/// where the entries before the message came from (AI-43), a new conversation to create with the
/// message, and an edit's message with the summary before it (AI-26), which [`delete_edited`]
/// deletes with the entries after it.
type Opened = (
    String,
    Arc<Turn>,
    String,
    Option<Place>,
    Option<EntryPlace>,
    Option<AiConversation>,
    Option<(String, Option<String>)>,
);

/// AI-26: deletes an edit's message `entry_id` and the entries after it, and moves the context's
/// start back to `summary_before` when it was at one of them. Done once the MCP servers run, just
/// before the context is checked for a compaction (AI-45), so a stop while they start leaves the
/// conversation as it was.
fn delete_edited(
    v: &mut Vault,
    conversation_id: &str,
    entry_id: &str,
    summary_before: Option<&str>,
) -> AppResult<()> {
    v.ai_delete_entries_from(conversation_id, entry_id)?;
    let mut conversation = find_conversation(v, conversation_id)?;
    if conversation
        .context_start
        .as_deref()
        .is_some_and(|start| start >= entry_id)
    {
        conversation.context_start = summary_before.map(str::to_owned);
        v.put(Some(conversation_id), Item::AiConversation(conversation))?;
    }
    Ok(())
}

/// A host's display name, when the host exists.
fn host_name(v: &Vault, host_id: &str) -> Option<String> {
    v.get(host_id)
        .and_then(Item::as_host)
        .map(|h| h.name.clone())
}

/// Where a conversation that moves to the host `host_id` or the quick-connect `target` (AI-09)
/// was, as a `host_change` note names it: its host's display name, empty when the host no longer
/// exists, or its target (HOST-12), or with neither, a fork's `origin` (AI-43). `None` when it had
/// none of them, as a home tab chat, or when the move is between a target and the saved host with
/// its address, port, and user (as **Save as Host…** makes), which is the same server.
fn moved_from(
    v: &Vault,
    conversation: &AiConversation,
    host_id: Option<&str>,
    target: Option<&AiQuickTarget>,
) -> Option<String> {
    match (&conversation.host_id, &conversation.quick_target) {
        (Some(old), _) if target.is_some() && host_target(v, old).as_ref() == target => None,
        (Some(old), _) => Some(host_name(v, old).unwrap_or_default()),
        (None, Some(old)) if host_id.and_then(|id| host_target(v, id)).as_ref() == Some(old) => {
            None
        }
        (None, Some(old)) => Some(target_name(old)),
        (None, None) => conversation
            .local
            .as_deref()
            .map(local_name)
            .or_else(|| conversation.origin.clone()),
    }
}

/// AI-09: whether the next message moves the conversation to the tab's host `host_id`, to the
/// tab's quick-connect `target` (HOST-12) when it has no saved host, or to the computer of a local
/// tab (§7.5). On the home tab, with none of them, it stays where it was.
fn moves_to(
    conversation: &AiConversation,
    host_id: Option<&str>,
    target: Option<&AiQuickTarget>,
    local: Option<&str>,
) -> bool {
    match (host_id, target, local) {
        (Some(host_id), _, _) => conversation.host_id.as_deref() != Some(host_id),
        (None, Some(target), _) => {
            conversation.host_id.is_some() || conversation.quick_target.as_ref() != Some(target)
        }
        (None, None, Some(device)) => {
            conversation.host_id.is_some()
                || conversation.quick_target.is_some()
                || conversation.local.as_deref() != Some(device)
        }
        (None, None, None) => false,
    }
}

/// The notes the conversation's next message would start with if it went with `context` (spec
/// §13.3), as [`AiManager::open_turn`] works them out for a message that edits nothing. A side
/// question of an idle conversation (AI-42) starts with them, unstored, so that its request reads
/// the earlier entries the way the next message's will. `earlier` is every entry of the
/// conversation.
fn next_message_notes(
    v: &Vault,
    conversation_id: &str,
    conversation: &AiConversation,
    earlier: &Entries,
    context: &AiTurnContext,
) -> String {
    let target = tab_target(context);
    let local = tab_local(context);
    let host_id = context.host_id.as_deref();
    let (came_from, host_now) =
        if moves_to(conversation, host_id, target.as_ref(), local.as_deref()) {
            let came_from = moved_from(v, conversation, host_id, target.as_ref());
            let host_now = host_id
                .and_then(|id| host_name(v, id))
                .or_else(|| target.as_ref().map(target_name))
                .or_else(|| local.as_deref().map(local_name));
            (came_from, host_now)
        } else {
            let host_now = conversation
                .host_id
                .as_deref()
                .and_then(|id| host_name(v, id))
                .or_else(|| conversation.quick_target.as_ref().map(target_name))
                .or_else(|| conversation.local.as_deref().map(local_name));
            (None, host_now)
        };
    let reverts = pending_reverts(v, conversation_id, &earlier.ids, &earlier.list);
    notes_before(
        v,
        &earlier.list,
        came_from.as_deref(),
        host_now.as_deref(),
        context,
        &reverts,
    )
}

/// How a stored model is named in a `model_change` note: its display name and ID, or the ID
/// alone when its provider or the model is gone.
fn stored_model_label(v: &Vault, provider_id: &str, model_id: &str) -> String {
    let name = v
        .get(provider_id)
        .and_then(Item::as_ai_provider)
        .and_then(|p| p.models.iter().find(|m| m.id == model_id))
        .map_or("", |m| m.name.as_str());
    tools::model_label(model_id, name)
}

/// The notes Hatoba puts before a new message (spec §13.3), in this order:
///
/// - `interrupted` (§13.1) when the newest reply in `earlier` is one the user stopped while it
///   streamed. A stop that an earlier message noted already, which got no reply, is not noted
///   again, and neither is one before a summary, which replaced it.
/// - `host_change` (AI-09) when the entries before the message came from another host than the
///   conversation's host now: `came_from` names it (empty when it no longer exists) or the
///   quick-connect target (HOST-12) they came from, and is `None` when the conversation did not
///   move. `host_now` is the display name of the host (or the target) the conversation is on now.
/// - `terminal_change` (AI-09) when the message's requests offer the terminal tools and, as far as
///   `earlier` tells the model, there was none before, or the other way round
///   ([`terminal_before`]).
/// - `model_change` (AI-05) when the message goes to another model ID than the newest reply in
///   `earlier` (the entries before the message) came from. A switch an earlier message noted
///   already, which got no reply (a failed request, say), is not noted again.
/// - `permission_mode` (AI-16) before a conversation's first message, and when the conversation's
///   mode on this device is not the one the model was last told of ([`noted_permission`]).
/// - `memory` (AI-53) before a conversation's first message, while memory is on and there are
///   memories ([`memory::note`]); a summary starts with one too.
/// - `file_revert` (AI-41) for each of `reverts`, from [`pending_reverts`].
/// - `terminal_state` (AI-46), the screen of the attached terminal ([`screen_note`]), also before
///   a conversation's first message.
fn notes_before(
    v: &Vault,
    earlier: &[AiEntry],
    came_from: Option<&str>,
    host_now: Option<&str>,
    context: &AiTurnContext,
    reverts: &[(String, files::ChangeMeta)],
) -> String {
    let mut notes = String::new();
    let mode = permission_mode(context.permission_mode);
    let permission = if noted_permission(earlier) == Some(mode) {
        String::new()
    } else {
        tools::permission_mode_block(mode)
    };
    // A message with nothing before it has no earlier screens or replies to tell apart, but it can
    // name its mode, show the memories and show the screen now.
    if earlier.is_empty() {
        return permission + &memory::note(v) + &screen_note(context, host_now);
    }
    for entry in earlier.iter().rev() {
        match &entry.body {
            EntryBody::Assistant(a) => {
                if a.interrupted {
                    notes.push_str(&tools::interrupted_block());
                }
                break;
            }
            EntryBody::User { text, .. } if has_note(text, INTERRUPTED) => break,
            EntryBody::Summary { .. } => break,
            EntryBody::User { .. } | EntryBody::Tool { .. } => {}
        }
    }
    if let (Some(from), Some(to)) = (came_from, host_now)
        && from != to
    {
        notes.push_str(&tools::host_change_block(from, to));
    }
    if terminal_before(earlier) == Some(!context.tab) {
        notes.push_str(&tools::terminal_change_block(context.tab));
    }
    let to = stored_model_label(v, &context.provider_id, &context.model_id);
    for entry in earlier.iter().rev() {
        match &entry.body {
            EntryBody::Assistant(a) => {
                if a.model_id != context.model_id {
                    let from = stored_model_label(v, &a.provider_id, &a.model_id);
                    notes.push_str(&tools::model_change_block(&from, &to));
                }
                break;
            }
            EntryBody::User { text, .. } => {
                if note_attr(text, MODEL_CHANGE, "to").is_some_and(|noted| noted == to) {
                    break;
                }
            }
            EntryBody::Tool { .. } | EntryBody::Summary { .. } => {}
        }
    }
    notes.push_str(&permission);
    for (tool_call_id, meta) in reverts {
        notes.push_str(&tools::file_revert_block(
            &meta.path,
            tool_call_id,
            &meta.tool,
            meta.created,
        ));
    }
    notes.push_str(&screen_note(context, host_now));
    notes
}

/// A conversation's permission mode on this device (AI-16) as a `permission_mode` note names it.
/// The system prompt names no mode of its own, since the default is the user's setting, so the
/// conversation's first message names it, whichever it is.
fn permission_mode(mode: AiPermissionMode) -> PermissionMode {
    match mode {
        AiPermissionMode::Manual => PermissionMode::Manual,
        AiPermissionMode::Auto => PermissionMode::Auto,
        AiPermissionMode::Bypass => PermissionMode::Bypass,
    }
}

/// AI-16: the permission mode the model was last told of in `entries`, a conversation's entries
/// oldest first: the newest `permission_mode` note, or the one the newest summary starts with.
/// None when the model was told of none: before the first message, or in a conversation from
/// before every first message named its mode. The model sees nothing before a summary, so one
/// without a note tells of none either.
fn noted_permission(entries: &[AiEntry]) -> Option<PermissionMode> {
    for entry in entries.iter().rev() {
        match &entry.body {
            EntryBody::User { text, .. } => {
                match note_attr(text, PERMISSION_MODE, "mode").as_deref() {
                    Some("bypass") => return Some(PermissionMode::Bypass),
                    Some("auto") => return Some(PermissionMode::Auto),
                    Some(_) => return Some(PermissionMode::Manual),
                    None => {}
                }
            }
            EntryBody::Summary { text } => return tools::split_permission_note(text).0,
            EntryBody::Assistant(_) | EntryBody::Tool { .. } => {}
        }
    }
    None
}

/// AI-46: the `terminal_state` note of a message whose `context` has the screen of a connected
/// tab, on the host `host` (its display name or quick-connect target). The panel sends the screen
/// while **Send Terminal Screen** is on for the conversation. Empty otherwise, and for a blank
/// screen.
fn screen_note(context: &AiTurnContext, host: Option<&str>) -> String {
    match &context.screen {
        Some(screen) if context.tab => {
            tools::terminal_state_block(host.unwrap_or_default(), screen.alternate, &screen.text)
        }
        _ => String::new(),
    }
}

/// The calls whose `file_revert` notes (AI-41) a user entry starts with.
fn noted_reverts(text: &str) -> Vec<String> {
    leading_blocks(text)
        .0
        .iter()
        .take_while(|b| b.kind.note)
        .filter(|b| b.kind.tag == FILE_REVERT)
        .filter_map(|b| b.attrs.iter().find(|(n, _)| *n == "tool_call_id"))
        .map(|(_, value)| attribute_text(value))
        .collect()
}

/// The reverts (AI-41) a new message notes: of the calls in `earlier` (the entries before it, with
/// their `ids`) whose change this device reverted, those that no user entry after the call notes
/// yet, oldest call first, each with its call id and what its snapshot says.
fn pending_reverts(
    v: &Vault,
    conversation_id: &str,
    ids: &[String],
    earlier: &[AiEntry],
) -> Vec<(String, files::ChangeMeta)> {
    let snapshots = match v.ai_file_snapshots(conversation_id) {
        Ok(snapshots) => snapshots,
        Err(e) => {
            tracing::warn!(conversation_id, "could not read the file snapshots: {e}");
            return Vec::new();
        }
    };
    let mut reverts = Vec::new();
    for snapshot in snapshots
        .iter()
        .filter(|s| s.state == SnapshotState::Reverted)
    {
        let Some(at) = ids.iter().position(|id| *id == snapshot.response_id) else {
            continue;
        };
        let Some(EntryBody::Assistant(response)) = earlier.get(at).map(|e| &e.body) else {
            continue;
        };
        let Some(order) = response
            .tool_calls
            .iter()
            .position(|c| c.id == snapshot.tool_call_id)
        else {
            continue;
        };
        // Responses may reuse a call id: a note is about the call of the newest response before it.
        let noted = earlier
            .iter()
            .enumerate()
            .skip(at + 1)
            .any(|(j, e)| match &e.body {
                EntryBody::User { text, .. } => {
                    noted_reverts(text).contains(&snapshot.tool_call_id)
                        && response_with_call(&earlier[..j], &snapshot.tool_call_id) == Some(at)
                }
                _ => false,
            });
        let meta = serde_json::from_str::<files::ChangeMeta>(&snapshot.meta);
        if let (false, Ok(meta)) = (noted, meta) {
            reverts.push(((at, order), snapshot.tool_call_id.clone(), meta));
        }
    }
    reverts.sort_by_key(|(place, ..)| *place);
    reverts
        .into_iter()
        .map(|(_, id, meta)| (id, meta))
        .collect()
}

/// The index of the newest response in `entries` that has a call with the id `tool_call_id`.
fn response_with_call(entries: &[AiEntry], tool_call_id: &str) -> Option<usize> {
    entries.iter().rposition(|e| match &e.body {
        EntryBody::Assistant(a) => a.tool_calls.iter().any(|c| c.id == tool_call_id),
        _ => false,
    })
}

/// The changes of a conversation's file tool calls this device keeps (AI-41).
fn file_changes(v: &Vault, conversation_id: &str) -> AppResult<Vec<AiFileChange>> {
    Ok(v.ai_file_snapshots(conversation_id)?
        .into_iter()
        .filter_map(|s| {
            let meta: files::ChangeMeta = serde_json::from_str(&s.meta).ok()?;
            Some(AiFileChange {
                response_id: s.response_id,
                tool_call_id: s.tool_call_id,
                server: meta.server,
                path: meta.path,
                created: meta.created,
                state: match s.state {
                    SnapshotState::Unfinished => AiFileChangeState::Unfinished,
                    SnapshotState::Written => AiFileChangeState::Written,
                    SnapshotState::Reverted => AiFileChangeState::Reverted,
                    SnapshotState::Dropped => AiFileChangeState::Dropped,
                },
            })
        })
        .collect())
}

/// A revert of a conversation's file change runs (AI-41) until this drops.
struct Reverting<'a>(&'a Inner, String);

impl<'a> Reverting<'a> {
    fn new(inner: &'a Inner, conversation_id: &str) -> Self {
        guard(&inner.reverts).insert(conversation_id.to_owned());
        Self(inner, conversation_id.to_owned())
    }
}

impl Drop for Reverting<'_> {
    fn drop(&mut self) {
        guard(&self.0.reverts).remove(&self.1);
    }
}

/// Another call of the newest response has the id of the call `tool_call_id` too.
fn shares_id(entries: &Entries, tool_call_id: &str) -> bool {
    entries
        .list
        .iter()
        .rev()
        .find_map(|e| match &e.body {
            EntryBody::Assistant(a) => Some(a),
            _ => None,
        })
        .is_some_and(|a| a.tool_calls.iter().filter(|c| c.id == tool_call_id).count() > 1)
}

/// Whether a call of the conversation after the one of `key` wrote the same file, and its change
/// can still be reverted (AI-41): reverting it first, newest first, may bring the file back to
/// what this call wrote.
fn later_writes(v: &Vault, key: &SnapshotKey<'_>, meta: &files::ChangeMeta) -> bool {
    let Ok(snapshots) = v.ai_file_snapshots(key.conversation_id) else {
        return false;
    };
    snapshots
        .iter()
        .skip_while(|s| s.response_id != key.response_id || s.tool_call_id != key.tool_call_id)
        .skip(1)
        .filter(|s| s.state.revertible())
        .filter_map(|s| serde_json::from_str::<files::ChangeMeta>(&s.meta).ok())
        .any(|later| later.server == meta.server && later.path == meta.path)
}

/// Keeps the snapshot of the change an `edit_file` or `write_file` call makes, in the vault,
/// before its write begins (AI-41).
struct KeepSnapshot<'a> {
    vault: &'a SharedVault,
    key: SnapshotKey<'a>,
    /// What the kept snapshot says, until the write resolved the path.
    meta: Mutex<Option<files::ChangeMeta>>,
    /// Keeping it dropped older snapshots over the size limit, of any conversation.
    dropped: AtomicBool,
}

impl<'a> KeepSnapshot<'a> {
    fn new(vault: &'a SharedVault, key: SnapshotKey<'a>) -> Self {
        Self {
            vault,
            key,
            meta: Mutex::default(),
            dropped: AtomicBool::new(false),
        }
    }

    fn dropped(&self) -> bool {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl files::Keep for KeepSnapshot<'_> {
    fn keep(&self, change: &files::Change) -> Result<(), files::Stop> {
        let mut v = unlocked(self.vault).map_err(|_| files::Stop::Cancelled)?;
        let Some((meta, content)) = change.encode() else {
            return Ok(());
        };
        // Any other failure is logged, and the call writes without a snapshot.
        match v.ai_put_file_snapshot(&self.key, &meta, &content) {
            Ok(dropped) => {
                *guard(&self.meta) = Some(change.meta.clone());
                self.dropped.store(dropped > 0, Ordering::Relaxed);
            }
            Err(hatoba_core::Error::Locked) => return Err(files::Stop::Cancelled),
            Err(e) => tracing::warn!(
                conversation_id = self.key.conversation_id,
                "could not store a file snapshot: {e}"
            ),
        }
        Ok(())
    }

    fn ended(&self, resolved: Option<&str>) {
        let Some(mut meta) = guard(&self.meta).take() else {
            return;
        };
        // Locked meanwhile: the snapshot stays unfinished, and can be reverted as such.
        let Ok(mut v) = unlocked(self.vault) else {
            return;
        };
        let result = match resolved {
            Some(path) if path != meta.path => {
                meta.path = path.to_owned();
                let meta = serde_json::to_string(&meta).ok();
                v.ai_mark_file_written(&self.key, meta.as_deref()).map(drop)
            }
            Some(_) => v.ai_mark_file_written(&self.key, None).map(drop),
            None => v.ai_delete_file_snapshot(&self.key),
        };
        if let Err(e) = result {
            tracing::warn!(
                conversation_id = self.key.conversation_id,
                "could not record the end of a file write: {e}"
            );
        }
    }
}

/// Whether a terminal was attached as far as `earlier` (the entries before a new message) tells
/// the model (AI-09): what the newest reply's request offered, or what a `terminal_change` note
/// says on a newer message that got no reply. `None` when the newest reply is from before replies
/// recorded it, or when there is no reply.
fn terminal_before(earlier: &[AiEntry]) -> Option<bool> {
    for entry in earlier.iter().rev() {
        match &entry.body {
            EntryBody::Assistant(a) => return a.terminal,
            EntryBody::User { text, .. } => {
                if let Some(to) = note_attr(text, TERMINAL_CHANGE, "to") {
                    return Some(to == "attached");
                }
            }
            EntryBody::Tool { .. } | EntryBody::Summary { .. } => {}
        }
    }
    None
}

// ---- history search (AI-24) ----

/// `ai_search`: conversations whose title or message text contains `query`, ignoring case, newest
/// activity first. One hit per conversation: its first matching user, assistant or summary entry
/// (tool results are not searched), or its title. Decrypts every conversation, one at a time
/// under the vault guard, so run it off the async runtime.
pub fn search(vault: &SharedVault, query: &str) -> AppResult<Vec<AiSearchHit>> {
    let needle = fold(query.trim());
    if needle.is_empty() {
        return Ok(Vec::new());
    }
    let mut conversations: Vec<(String, String, i64)> = {
        let v = unlocked(vault)?;
        let last = v.ai_last_activities();
        v.ai_conversations()
            .into_iter()
            .map(|(id, c)| {
                let at = last.get(&id).copied().unwrap_or(0);
                (id, c.title, at)
            })
            .collect()
    };
    conversations.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| b.0.cmp(&a.0)));
    let mut hits = Vec::new();
    for (conversation_id, title, _) in conversations {
        let entries = {
            let v = unlocked(vault)?;
            Entries::load(&v, &conversation_id)?
        };
        let in_entries = entries.ids.iter().zip(&entries.list).find_map(|(id, e)| {
            let text = match &e.body {
                // The screen a message showed is the host's text, not the conversation's (AI-46),
                // and the permission mode and memory notes are Hatoba's (§13.5, AI-53).
                EntryBody::User { text, .. } => {
                    let mut text = Cow::Borrowed(text.as_str());
                    for tag in [PERMISSION_MODE, MEMORY, TERMINAL_STATE] {
                        if let Some(block) = note_block(&text, tag) {
                            text = Cow::Owned(text.replacen(block, "", 1));
                        }
                    }
                    text
                }
                EntryBody::Summary { text } => {
                    let summary = tools::split_permission_note(text).1;
                    match note_block(summary, MEMORY) {
                        Some(block) => Cow::Borrowed(&summary[block.len()..]),
                        None => Cow::Borrowed(summary),
                    }
                }
                EntryBody::Assistant(a) => Cow::Borrowed(a.text.as_str()),
                EntryBody::Tool { .. } => return None,
            };
            snippet(&text, &needle).map(|s| (Some(id.clone()), s))
        });
        if let Some((entry_id, snippet)) =
            in_entries.or_else(|| snippet(&title, &needle).map(|s| (None, s)))
        {
            hits.push(AiSearchHit {
                conversation_id,
                entry_id,
                snippet,
            });
        }
    }
    Ok(hits)
}

/// Text in lowercase, character by character, for matching that ignores case.
fn fold(text: &str) -> Vec<char> {
    text.chars().flat_map(char::to_lowercase).collect()
}

/// About [`SNIPPET_CHARS`] characters of `text` around the first match of `needle` (folded),
/// whitespace collapsed, with `…` where it was cut. `None` without a match.
fn snippet(text: &str, needle: &[char]) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    // The folded text, and for each of its characters the original one it came from.
    let mut folded = Vec::with_capacity(chars.len());
    let mut origin = Vec::with_capacity(chars.len());
    for (i, c) in chars.iter().enumerate() {
        for lower in c.to_lowercase() {
            folded.push(lower);
            origin.push(i);
        }
    }
    let at = folded.windows(needle.len()).position(|w| w == needle)?;
    let (start, end) = (origin[at], origin[at + needle.len() - 1] + 1);
    let mut from = start.saturating_sub(SNIPPET_BEFORE);
    let to = chars.len().min(from + SNIPPET_CHARS.max(end - from));
    // Near the end of the text, the window takes more of what comes before.
    if to - from < SNIPPET_CHARS {
        from = to.saturating_sub(SNIPPET_CHARS);
    }
    let mut out = String::new();
    if from > 0 {
        out.push('…');
    }
    let mut space = false;
    for c in &chars[from..to] {
        if c.is_whitespace() {
            space = true;
        } else {
            if space && !out.is_empty() && !out.ends_with('…') {
                out.push(' ');
            }
            space = false;
            out.push(*c);
        }
    }
    if to < chars.len() {
        out.push('…');
    }
    Some(out)
}

// ---- views ----

pub fn conversation_view(v: &Vault, id: &str, c: &AiConversation) -> AiConversationView {
    conversation_view_at(id, c, v.ai_last_activity(id))
}

/// [`conversation_view`] with the conversation's last activity read already, for lists
/// (`Vault::ai_last_activities` reads every conversation's in one pass).
pub fn conversation_view_at(
    id: &str,
    c: &AiConversation,
    last_activity: i64,
) -> AiConversationView {
    AiConversationView {
        id: id.to_owned(),
        title: c.title.clone(),
        host_id: c.host_id.clone(),
        quick_target: c.quick_target.as_ref().map(|t| QuickTarget {
            address: t.address.clone(),
            port: t.port,
            username: t.username.clone(),
        }),
        origin: c.origin.clone(),
        local: c.local.clone(),
        pinned: c.pinned,
        context_start: c.context_start.clone(),
        effort: c.effort.map(effort_view),
        created_at: c.created_at,
        updated_at: c.updated_at,
        last_activity,
    }
}

/// An entry for the WebView: everything but the provider's raw message.
pub fn entry_view(entry_id: &str, entry: &AiEntry) -> AiEntryView {
    let entry_id = entry_id.to_owned();
    let created_at = entry.created_at;
    match &entry.body {
        EntryBody::User { text, .. } => AiEntryView::User {
            entry_id,
            created_at,
            text: text.clone(),
        },
        EntryBody::Assistant(a) => AiEntryView::Assistant {
            entry_id,
            created_at,
            provider_id: a.provider_id.clone(),
            model_id: a.model_id.clone(),
            text: a.text.clone(),
            reasoning: a.reasoning.clone(),
            tool_calls: a
                .tool_calls
                .iter()
                .map(|c| AiToolCall {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    arguments: c.arguments.clone(),
                })
                .collect(),
            finish: finish_view(a.finish),
            usage: a.usage.map(|u| AiUsage {
                input_tokens: u.input_tokens,
                output_tokens: u.output_tokens,
                estimated: u.estimated,
            }),
        },
        EntryBody::Tool {
            tool_call_id,
            status,
            content,
            auto,
        } => AiEntryView::Tool {
            entry_id,
            created_at,
            tool_call_id: tool_call_id.clone(),
            status: status_view(*status),
            content: content.clone(),
            auto: auto.as_ref().map(auto_view),
        },
        EntryBody::Summary { text } => AiEntryView::Summary {
            entry_id,
            created_at,
            text: text.clone(),
        },
    }
}

fn finish_view(finish: Finish) -> AiFinish {
    match finish {
        Finish::Stop => AiFinish::Stop,
        Finish::ToolCalls => AiFinish::ToolCalls,
        Finish::Length => AiFinish::Length,
        Finish::Refused => AiFinish::Refused,
    }
}

fn status_view(status: ToolStatus) -> AiToolStatus {
    match status {
        ToolStatus::Ok => AiToolStatus::Ok,
        ToolStatus::Error => AiToolStatus::Error,
        ToolStatus::Rejected => AiToolStatus::Rejected,
        ToolStatus::Cancelled => AiToolStatus::Cancelled,
    }
}

fn core_status(status: AiToolStatus) -> ToolStatus {
    match status {
        AiToolStatus::Ok => ToolStatus::Ok,
        AiToolStatus::Error => ToolStatus::Error,
        AiToolStatus::Rejected => ToolStatus::Rejected,
        AiToolStatus::Cancelled => ToolStatus::Cancelled,
    }
}

fn stream_event(event: StreamEvent) -> AiTurnEvent {
    match event {
        StreamEvent::Text(delta) => AiTurnEvent::Text { delta },
        StreamEvent::Reasoning(delta) => AiTurnEvent::Reasoning { delta },
        StreamEvent::ToolCall(call) => AiTurnEvent::ToolCall {
            id: call.id,
            name: call.name,
            arguments: call.arguments,
        },
        StreamEvent::Usage(usage) => AiTurnEvent::Usage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            estimated: usage.estimated,
        },
        StreamEvent::EffortIgnored => AiTurnEvent::EffortIgnored,
    }
}

/// The `error` event's message: the provider's own message for an HTTP error (its status goes
/// in `status`), otherwise the failure.
fn error_text(e: &AiError) -> String {
    match e {
        AiError::Http { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

/// The kind of a failure, for logs (never its message, which may quote the provider).
pub fn error_kind(e: &AiError) -> &'static str {
    match e {
        AiError::InvalidUrl(_) => "invalid_url",
        AiError::Config(_) => "config",
        AiError::Http { .. } => "http",
        AiError::Network(_) => "network",
        AiError::Protocol(_) => "protocol",
        AiError::Blocked(_) => "blocked",
        AiError::UnsupportedContentType(_) => "content_type",
        AiError::SignIn(_) => "sign_in",
        AiError::Cancelled => "cancelled",
    }
}

pub mod chatgpt;
pub mod conn;
mod files;
pub mod memory;
pub mod mentions;
mod search;
#[cfg(test)]
mod tests;
