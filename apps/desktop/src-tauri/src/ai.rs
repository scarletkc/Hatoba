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

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use hatoba_ai::chat::{ChatRequest, ToolDef};
use hatoba_ai::entry::{AiEntry, AssistantEntry, AutoVerdict, Finish, ToolCall, ToolStatus};
use hatoba_ai::provider::{Effort, ModelSpec, ProviderConfig};
use hatoba_core::Vault;
use hatoba_core::sync::SharedVault;
use serde::de::DeserializeOwned;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::dto::{AiTurnContext, AiTurnEndReason, AiTurnEvent};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::mcp::McpManager;

pub use entries::find_conversation;
pub use history::search;
pub use job::{proxy_call, search_kind, session_server};
pub use request::{
    auth_header, core_effort, drop_levels_of_old_protocol, effort_view, listed_effort, model_spec,
    protocol, provider_config,
};
pub use skills::{skill_commands, skill_text};
pub use view::{conversation_view, conversation_view_at, error_kind};

/// What every MCP tool name starts with (AI-30).
const MCP_PREFIX: &str = "mcp__";

/// The most characters a conversation's title holds (AI-23): a rename longer is refused and a
/// fork's title is cut to it.
pub const MAX_TITLE_CHARS: usize = 200;

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
    /// The servers whose MCP tools the request offered directly (AI-30), as a stored response
    /// records them.
    mcp_servers: Option<Vec<String>>,
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
            mcp_servers: self.mcp_servers,
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
}

fn parse<T: DeserializeOwned>(arguments: &str) -> Result<T, String> {
    let arguments = if arguments.trim().is_empty() {
        "{}"
    } else {
        arguments
    };
    serde_json::from_str(arguments).map_err(|e| format!("The arguments are not valid: {e}."))
}

mod blocks;
mod calls;
mod changes;
pub mod chatgpt;
mod compact;
pub mod conn;
mod conversations;
mod entries;
mod files;
mod history;
mod job;
mod judge;
pub mod memory;
pub mod mentions;
pub mod models_dev;
mod notes;
mod ops;
mod places;
mod request;
mod search;
mod side;
mod skills;
#[cfg(test)]
mod tests;
mod turn;
mod view;
