//! The AI assistant's runtime (spec §13.1): running turns, the tools that run in Rust, and the
//! conversation operations behind the `ai_*` commands.
//!
//! A turn is a task that builds a request from the stored conversation, streams the response to
//! the turn's channel, stores the assistant entry and, when the response has tool calls, waits
//! until every call has a stored result before it sends the next request. The frontend runs
//! `read_terminal` and `send_input` and reports with `ai_tool_result`; `ai_tool_run` runs the
//! other tools here. What is stored is exactly what the model receives (§13.7).
//!
//! Before each request the turn asks the [`McpManager`] for the MCP tools to offer (AI-30),
//! which starts the servers that need it, and compacts the context first when the request would
//! pass 90% of the model's context window (AI-22). A Compact request (AI-21) is the request a
//! turn would send next with the instruction after it, built by the same function, so it reuses
//! the prefix the provider cached. History search (AI-24) and edit and resend (AI-26) are here
//! too.
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

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use hatoba_ai::AiError;
use hatoba_ai::chat::{ChatRequest, StreamEvent, ToolDef, complete, stream_chat};
use hatoba_ai::entry::{
    AiEntry, AssistantEntry, EntryBody, Finish, ToolCall, ToolStatus, Usage, fix_up_missing_results,
};
use hatoba_ai::provider::{AuthHeader, Effort, ModelSpec, Protocol, ProviderConfig};
use hatoba_ai::skills::{BUILTIN_NAME, builtin_description, builtin_skill};
use hatoba_ai::tools::{
    self, FetchUrlArgs, PromptContext, PromptModel, PromptTools, ReadSkillArgs, RunCommandArgs,
    ToolSet, WebSearchArgs,
};
use hatoba_ai::web::{self, SearchConfig, SearchKind as WebSearchKind};
use hatoba_core::Vault;
use hatoba_core::model::{
    AiAuthHeader, AiConversation, AiEffort as CoreEffort, AiProtocol as CoreProtocol, AiProvider,
    Item, MAX_CUSTOM_INSTRUCTIONS_CHARS, MAX_HOST_AI_NOTES_CHARS, SearchKind as CoreSearchKind,
    SearchProvider,
};
use hatoba_core::sync::SharedVault;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::dto::{
    AiConversationDetail, AiConversationView, AiEffort, AiEntryView, AiFinish, AiSearchHit,
    AiSendInput, AiSendStarted, AiToolCall, AiToolResultInput, AiToolStatus, AiTurnContext,
    AiTurnEndReason, AiTurnEvent, AiUsage, QuickTarget,
};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::mcp::{McpManager, Offer, OfferedTool};
use crate::state::now_ms;

/// The line a result starts with when the user edited the call before it ran (AI-17).
const EDIT_NOTE: &str = "The user edited the arguments before the call ran; it ran with";

/// What every MCP tool name starts with (AI-30).
const MCP_PREFIX: &str = "mcp__";

/// The `run_command` result while the tab has no live session (AI-08).
const DISCONNECTED: &str = "The terminal tab is disconnected, so the command did not run. Ask \
     the user to reconnect the tab.";

/// The final instruction of a Compact request (AI-21). The request offers the conversation's
/// tools, so it first asks for text only. The summary leaves out what the assistant can use:
/// every request states that in its system prompt, and a summary written by the fallback without
/// tools would otherwise tell the requests after it that there are none.
const COMPACT_INSTRUCTION: &str = "Respond with text only. Do not call any tools.\n\n\
     Summarize the conversation so far so that the summary can replace it as the context of the \
     rest of this conversation. Keep everything later work needs: the user's goals and requests, \
     the hosts and systems involved, the commands that were run and their outcomes, files and \
     settings that were changed, decisions made, and open problems and next steps. Leave out what \
     no longer matters, and leave out what the assistant can use, such as its tools, skills and \
     terminal connection: every request states that in its system prompt, and it can change \
     between requests. Write the summary in the user's language and reply with the summary only.";

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
     requests. Write the summary in the user's language and reply with the summary only.";

/// The most output tokens a Compact request asks for (Anthropic's `max_tokens`, AI-21).
const COMPACT_MAX_TOKENS: u64 = 16_000;

/// The most output tokens a Compact request asks for, in hundredths of the context window: what
/// a context just under the AI-22 threshold leaves free, less a hundredth for the instruction and
/// the error of the estimate, so the request stays inside the window.
const COMPACT_OUTPUT_PERCENT: u64 = 9;

/// A skill's main file (§13.8).
const SKILL_MD: &str = "SKILL.md";

/// AI-22: a request whose context would pass this share of the context window is compacted
/// first, in tenths.
const AUTO_COMPACT_TENTHS: u64 = 9;

/// The rough text-to-token ratio of estimates, the same as the panel's meter (AI-20).
const CHARS_PER_TOKEN: usize = 4;

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
    /// The SSH connection of a terminal tab's session, while it is open (AI-08).
    fn session(&self, session_id: &str) -> Option<hatoba_ssh::SshSession>;
    /// The identification string the server of a tab's open session sent (§13.1).
    fn server_id(&self, session_id: &str) -> Option<String> {
        self.session(session_id)
            .filter(|s| !s.is_closed())
            .and_then(|s| s.server_id().map(str::to_owned))
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
    /// The client for provider and search requests, built once (`hatoba_ai::provider::http_client`).
    http: OnceLock<reqwest::Client>,
    /// The MCP servers whose tools requests offer (§13.9).
    mcp: McpManager,
    /// Running turns by conversation id.
    turns: Mutex<HashMap<String, Arc<Turn>>>,
    /// Running tools, compactions and provider requests, so that stop and lock reach them.
    ops: Mutex<HashMap<u64, Op>>,
    next_op: AtomicU64,
}

struct Op {
    /// `None` for work outside a conversation, which only the lock stops.
    conversation_id: Option<String>,
    /// The tool call an `ai_tool_run` runs, which nothing else runs or answers meanwhile.
    tool_call_id: Option<String>,
    cancel: CancellationToken,
}

/// A running tool, compaction or provider request; unregisters itself when dropped.
pub struct OpGuard<'a> {
    inner: &'a Inner,
    id: u64,
    cancel: CancellationToken,
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
    /// `turn_ended` was sent; nothing else is sent after it.
    ended: AtomicBool,
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

    /// The shared client for provider and search requests.
    pub fn http(&self) -> reqwest::Client {
        self.0
            .http
            .get_or_init(hatoba_ai::provider::http_client)
            .clone()
    }

    /// A turn of the conversation is running.
    pub fn is_running(&self, conversation_id: &str) -> bool {
        guard(&self.0.turns).contains_key(conversation_id)
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
            ended: AtomicBool::new(false),
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
        }
    }

    /// Cancels the conversation's turn and its running tools, and returns the turn.
    fn cancel(&self, conversation_id: &str) -> Option<Arc<Turn>> {
        let turn = guard(&self.0.turns).remove(conversation_id);
        if let Some(turn) = &turn {
            turn.cancel.cancel();
        }
        for op in guard(&self.0.ops).values() {
            if op.conversation_id.as_deref() == Some(conversation_id) {
                op.cancel.cancel();
            }
        }
        turn
    }

    /// Registers a tool or compaction of the conversation. Called under the vault guard that
    /// checked the call, so a stop either came first (and the check failed) or cancels it.
    fn start_op(&self, conversation_id: &str) -> OpGuard<'_> {
        let cancel = self.turn_token(conversation_id);
        self.register_op(Some(conversation_id), None, cancel)
    }

    /// Registers the run of a tool call, like [`Self::start_op`]. Refused while the call runs
    /// already, so that a call with side effects (`run_command`, an MCP tool) runs once: called
    /// under the vault guard that checked the call, as is [`Self::call_running`].
    fn start_call(&self, conversation_id: &str, tool_call_id: &str) -> AppResult<OpGuard<'_>> {
        if self.call_running(conversation_id, tool_call_id) {
            return Err(AppError::invalid(
                "tool_call_id",
                "the tool call is already running",
            ));
        }
        let cancel = self.turn_token(conversation_id);
        Ok(self.register_op(Some(conversation_id), Some(tool_call_id), cancel))
    }

    /// An `ai_tool_run` of the call has not finished.
    fn call_running(&self, conversation_id: &str, tool_call_id: &str) -> bool {
        guard(&self.0.ops).values().any(|op| {
            op.conversation_id.as_deref() == Some(conversation_id)
                && op.tool_call_id.as_deref() == Some(tool_call_id)
        })
    }

    /// A token that a stop of the conversation's running turn cancels.
    fn turn_token(&self, conversation_id: &str) -> CancellationToken {
        self.running(conversation_id)
            .map_or_else(CancellationToken::new, |turn| turn.cancel.child_token())
    }

    /// A provider or search request outside a conversation (a model list, a connection test):
    /// the lock cancels its token (§13.1). Call it under the vault guard that read the provider,
    /// so that a lock either came first or cancels the request.
    pub fn background_op(&self) -> OpGuard<'_> {
        self.register_op(None, None, CancellationToken::new())
    }

    fn register_op(
        &self,
        conversation_id: Option<&str>,
        tool_call_id: Option<&str>,
        cancel: CancellationToken,
    ) -> OpGuard<'_> {
        let id = self.0.next_op.fetch_add(1, Ordering::Relaxed);
        guard(&self.0.ops).insert(
            id,
            Op {
                conversation_id: conversation_id.map(str::to_owned),
                tool_call_id: tool_call_id.map(str::to_owned),
                cancel: cancel.clone(),
            },
        );
        OpGuard {
            inner: &self.0,
            id,
            cancel,
        }
    }

    /// Stops a conversation with the vault guard held (§13.1): cancels its turn and running
    /// tools, stores a cancelled result for every call without one, and sends those entries and
    /// `turn_ended { stopped }` on the turn's channel. Returns the cancelled results it stored.
    fn stop_locked(&self, v: &mut Vault, conversation_id: &str) -> Vec<AiEntryView> {
        let turn = self.cancel(conversation_id);
        let mut stored = Vec::new();
        if v.is_unlocked() && find_conversation(v, conversation_id).is_ok() {
            let cancelled = Entries::load(v, conversation_id)
                .and_then(|entries| cancel_open_calls(v, conversation_id, &entries));
            match cancelled {
                Ok(views) => stored = views,
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

    /// `ai_stop`: stops the conversation's turn and tools, if any, and cancels its open calls.
    pub fn stop(&self, vault: &SharedVault, env: &dyn AiEnv, conversation_id: &str) {
        let stored = self.stop_locked(&mut lock(vault), conversation_id);
        if !stored.is_empty() {
            env.changed();
        }
    }

    /// The vault is about to lock (SEC-02): stops every turn and tool, and every provider request
    /// outside a conversation, so the assistant never acts behind the lock screen (§13.1). Called
    /// with the vault guard, before `Vault::lock`.
    pub fn stop_all(&self, v: &mut Vault) {
        let running: Vec<String> = guard(&self.0.turns).keys().cloned().collect();
        for conversation_id in &running {
            self.stop_locked(v, conversation_id);
        }
        for op in guard(&self.0.ops).values() {
            op.cancel.cancel();
        }
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
        })
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
            v.ai_delete_conversation(conversation_id)?;
        }
        tracing::info!(conversation_id, "AI conversation deleted");
        env.changed();
        Ok(())
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

    /// Registers the turn, compacts first when the new message needs it (AI-22), stores the
    /// user's message (in place of `replace` and the entries after it, for an edit) and returns
    /// the turn's task.
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
        let (id, turn, compact, notes) = registered?;

        // AI-22: the summary is stored before the message, so the message stays in the context.
        // A failed compaction is logged and the message goes anyway: the request may still fit,
        // and if it does not, the provider's error says so. A stop meanwhile is checked below.
        if compact
            && let Err(CompactFailure::Failed { .. }) =
                self.compact_before(vault, env.as_ref(), &id, &turn).await
        {
            tracing::warn!(conversation_id = %id, "AI message sent without compaction");
        }

        let started = (|| {
            let mut v = unlocked(vault)?;
            // Stopped, or replaced by a newer message, while the context was compacted: the
            // message is not stored. Checked under the guard a stop takes, like every write.
            if turn.cancel.is_cancelled() {
                return Err(AppError::new(
                    ErrorCode::Cancelled,
                    "the message was stopped before it was sent",
                ));
            }
            // AI-05, AI-09: Hatoba's notes come before what the panel sent (spec §13.3).
            let entry = AiEntry::user(now_ms(), format!("{notes}{text}"));
            let entry_id = append(&mut v, &id, &entry)?;
            let conversation = find_conversation(&v, &id)?;
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

    /// The first step of [`Self::start_turn`], under the vault guard: the conversation (created
    /// when new), the stop of its running turn, an edit's deletion, the move to the tab's host
    /// (AI-09), the notes Hatoba puts before the message (AI-05, AI-09), the registered turn, and
    /// whether the new message needs a compaction first (AI-22). Returns the conversation's id,
    /// the turn, that answer, and the notes.
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
    ) -> AppResult<(String, Arc<Turn>, bool, String)> {
        let mut v = unlocked(vault)?;
        let mut cancelled = Vec::new();
        let (id, compact, notes) = match conversation_id {
            Some(id) => {
                let mut conversation = find_conversation(&v, &id)?;
                let before = conversation.clone();
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
                if let Some((entry_id, summary_before)) = edit {
                    if let Some(at) = ids.iter().position(|e| *e == entry_id) {
                        came_from = earlier[at..].iter().find_map(|e| match &e.body {
                            EntryBody::User { text } => note_attr(text, HOST_CHANGE, "from"),
                            _ => None,
                        });
                        earlier.truncate(at);
                    }
                    v.ai_delete_entries_from(&id, &entry_id)?;
                    *stored = true;
                    // The cancelled results came after the edited message: deleted too.
                    cancelled.clear();
                    if conversation
                        .context_start
                        .as_deref()
                        .is_some_and(|start| start >= entry_id.as_str())
                    {
                        conversation.context_start = summary_before;
                    }
                }
                // AI-09: the next message moves the conversation to the tab's host.
                if let Some(host_id) = &context.host_id
                    && conversation.host_id.as_ref() != Some(host_id)
                {
                    // A conversation that had no host (a home tab chat) did not come from one.
                    if came_from.is_none()
                        && let Some(old) = conversation.host_id.as_deref()
                    {
                        came_from = Some(host_name(&v, old).unwrap_or_default());
                    }
                    conversation.host_id = Some(host_id.clone());
                } else if context.host_id.is_none()
                    && context.target.is_some()
                    && let Some(old) = conversation.host_id.take()
                {
                    // A quick connection (HOST-12) is no saved host: the conversation leaves its own.
                    if came_from.is_none() {
                        came_from = Some(host_name(&v, &old).unwrap_or_default());
                    }
                }
                // AI-05: the conversation keeps the level of its last message.
                conversation.effort = context.effort.map(core_effort);
                if conversation != before {
                    v.put(Some(&id), Item::AiConversation(conversation.clone()))?;
                    *stored = true;
                }
                let host_now = conversation
                    .host_id
                    .as_deref()
                    .and_then(|id| host_name(&v, id))
                    .or_else(|| context.target.as_ref().map(quick_label));
                let notes = notes_before(
                    &v,
                    &earlier,
                    came_from.as_deref(),
                    host_now.as_deref(),
                    &context,
                );
                let full = format!("{notes}{text}");
                let compact = compaction_before(&v, &id, &context, &full)?;
                (id, compact, notes)
            }
            None => {
                let id = v.put(
                    None,
                    Item::AiConversation(AiConversation {
                        title: title_of(text),
                        host_id: context.host_id.clone(),
                        pinned: false,
                        context_start: None,
                        effort: context.effort.map(core_effort),
                        created_at: now_ms(),
                        updated_at: 0,
                    }),
                )?;
                *stored = true;
                (id, false, String::new())
            }
        };
        let turn = self.register(&id, context, server_id, sink);
        // The panel stopped listening to the previous turn's channel, so the results the stop
        // stored reach it on this one.
        for entry in cancelled {
            turn.emit(AiTurnEvent::Entry { entry });
        }
        Ok((id, turn, compact, notes))
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
            let compact = compaction_before(&v, &conversation_id, &context, "")?;
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
                    .compact_before(&vault, env.as_ref(), &conversation_id, &turn)
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
            turn.end(reason);
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
            if compaction_due(&request) {
                let compact = {
                    let v = lock(vault);
                    if turn.cancel.is_cancelled() || !v.is_unlocked() {
                        return AiTurnEndReason::Stopped;
                    }
                    // The request the turn was about to send, asking where the task stands.
                    Compact::fork(&v, &turn.context, request, COMPACT_MID_TURN_INSTRUCTION)
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
            // Calls of this response map back through the names this request offered.
            self.0.mcp.record_offer(conversation_id, offer);
            turn.emit(AiTurnEvent::RequestStarted);
            let http = self.http();
            let result = {
                let mut forward = |event: StreamEvent| turn.emit(stream_event(event));
                stream_chat(
                    &http,
                    &request.provider,
                    &request.chat(),
                    &turn.cancel,
                    &mut forward,
                )
                .await
            };
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
            let entry_id = match store_response(vault, env, conversation_id, turn, response) {
                Ok(entry_id) => entry_id,
                Err(halt) => return halted(turn, conversation_id, halt),
            };
            if !wants_results {
                return match finish {
                    Finish::Length => AiTurnEndReason::Length,
                    Finish::Refused => AiTurnEndReason::Refused,
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
    /// and not be running in Rust.
    pub fn tool_result(
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
        let view = {
            let mut v = unlocked(vault)?;
            find_conversation(&v, conversation_id)?;
            open_call(&Entries::load(&v, conversation_id)?, tool_call_id)?;
            if self.call_running(conversation_id, tool_call_id) {
                return Err(AppError::invalid(
                    "tool_call_id",
                    "the tool call is already running",
                ));
            }
            self.store_result(&mut v, conversation_id, tool_call_id, status, content)?
        };
        env.changed();
        Ok(view)
    }

    /// `ai_tool_run`: runs a tool that runs in Rust and stores its result. Every failure of the
    /// tool itself is an `error` result; only a precondition (locked, unknown conversation or
    /// call, a call that already has a result or is running) rejects.
    pub async fn tool_run(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        conversation_id: &str,
        tool_call_id: &str,
        session_id: Option<&str>,
        edited_arguments: Option<&str>,
    ) -> AppResult<AiEntryView> {
        let (job, op) = {
            let v = unlocked(vault)?;
            find_conversation(&v, conversation_id)?;
            let call = open_call(&Entries::load(&v, conversation_id)?, tool_call_id)?;
            let op = self.start_call(conversation_id, tool_call_id)?;
            let arguments = edited_arguments.unwrap_or(&call.arguments);
            let job = if call.name.starts_with(MCP_PREFIX) {
                Job::mcp(
                    &v,
                    self.0.mcp.offered(conversation_id, &call.name),
                    &call.name,
                    arguments,
                )
            } else {
                Job::new(&v, &call.name, arguments, &env.app_version())
            };
            (job, op)
        };
        let (status, content) = job
            .run(
                &self.http(),
                &self.0.mcp,
                vault,
                env,
                session_id,
                &op.cancel,
            )
            .await;
        let content = result_content(status, content, edited_arguments);
        let view = {
            let mut v = unlocked(vault)?;
            find_conversation(&v, conversation_id)?;
            if let Err(e) = open_call(&Entries::load(&v, conversation_id)?, tool_call_id) {
                // A stop stored a cancelled result while the tool ran.
                return Err(if op.cancel.is_cancelled() {
                    AppError::new(ErrorCode::Cancelled, "the tool call was stopped")
                } else {
                    e
                });
            }
            self.store_result(&mut v, conversation_id, tool_call_id, status, content)?
        };
        drop(op);
        env.changed();
        Ok(view)
    }

    /// Stores a tool result, sends it on the running turn's channel and wakes the turn.
    fn store_result(
        &self,
        v: &mut Vault,
        conversation_id: &str,
        tool_call_id: &str,
        status: ToolStatus,
        content: String,
    ) -> AppResult<AiEntryView> {
        let entry = AiEntry::tool(now_ms(), tool_call_id, status, content);
        let entry_id = append(v, conversation_id, &entry)?;
        let view = entry_view(&entry_id, &entry);
        if let Some(turn) = self.running(conversation_id) {
            turn.emit(AiTurnEvent::Entry {
                entry: view.clone(),
            });
            turn.results.notify_one();
        }
        tracing::info!(conversation_id, ?status, "AI tool result stored");
        Ok(view)
    }

    // ---- compaction ----

    /// `ai_compact` (AI-21): asks the model for a summary of the context with the request a turn
    /// would send next ([`compact_request`]), stores it as a `summary` entry and moves
    /// `context_start` to it. Refused while a turn runs.
    pub async fn compact(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        conversation_id: &str,
        context: &AiTurnContext,
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
                COMPACT_INSTRUCTION,
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
                "the model returned an empty summary",
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
            store_summary(&mut v, conversation_id, &summary)?
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
        let entries = Entries::load(&v, conversation_id)?;
        if !cancel_open_calls(&mut v, conversation_id, &entries)?.is_empty() {
            *stored = true;
        }
        if !compactable(&context_entries(&v, conversation_id)?) {
            return Err(AppError::invalid("conversation_id", "nothing to compact"));
        }
        resolve_model(&v, &context.provider_id, &context.model_id)
            .map_err(|message| AppError::invalid("model_id", message))?;
        Ok(self.start_op(conversation_id))
    }

    /// Asks the model for the summary a Compact request wants (AI-21). An answer that calls tools
    /// or has no text is dropped (none of its calls runs or is stored), and the request goes once
    /// more without tools, with the system prompt that says so. `None` when that answer has no
    /// text either.
    async fn summarize(
        &self,
        conversation_id: &str,
        compact: Compact,
        cancel: &CancellationToken,
    ) -> Result<Option<String>, AiError> {
        let Compact {
            mut request,
            plain_system,
        } = compact;
        let http = self.http();
        let response = complete(&http, &request.provider, &request.chat(), cancel).await?;
        log_compact_usage(conversation_id, false, response.usage);
        let summary = response.text.trim();
        if response.tool_calls.is_empty() && !summary.is_empty() {
            return Ok(Some(summary.to_owned()));
        }
        tracing::info!(
            conversation_id,
            tool_calls = response.tool_calls.len(),
            "AI compaction answered without a summary; asking again without tools"
        );
        request.tools = Vec::new();
        request.system = plain_system;
        let response = complete(&http, &request.provider, &request.chat(), cancel).await?;
        log_compact_usage(conversation_id, true, response.usage);
        let summary = response.text.trim();
        Ok((!summary.is_empty()).then(|| summary.to_owned()))
    }

    /// AI-22, before a new message or a retry: builds the Compact request from the request the
    /// turn would send next, with the MCP tools it would offer (starting their servers, as the
    /// turn would), then compacts like [`Self::auto_compact`].
    async fn compact_before(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        conversation_id: &str,
        turn: &Turn,
    ) -> Result<(), CompactFailure> {
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
    /// summary (moving `context_start` to it) and sends its entry on the turn's channel.
    async fn auto_compact(
        &self,
        vault: &SharedVault,
        env: &dyn AiEnv,
        conversation_id: &str,
        turn: &Turn,
        compact: Compact,
    ) -> Result<(), CompactFailure> {
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
            tracing::warn!(
                conversation_id,
                "AI automatic compaction got an empty summary"
            );
            return Err(CompactFailure::Failed {
                status: None,
                message: "The context is nearly full, and the model returned an empty summary \
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
                Ok(view) => turn.emit(AiTurnEvent::Entry { entry: view }),
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

/// A Compact request (AI-21): the conversation's next request, as [`next_request`] builds it for
/// a turn, with the instruction after it. It offers the same tools with the same system prompt
/// and replays the same entries, so the provider reads its prefix from the cache the
/// conversation's requests wrote. Only its output limit differs ([`compact_output_tokens`]),
/// which is not part of that prefix. `plain_system` is the system prompt of the fallback without
/// tools ([`AiManager::summarize`]).
struct Compact {
    request: Request,
    plain_system: String,
}

impl Compact {
    /// Forks `next`, the request a turn with `context` sends next, by appending `instruction`.
    fn fork(v: &Vault, context: &AiTurnContext, mut next: Request, instruction: &str) -> Self {
        next.entries.push(AiEntry::user(now_ms(), instruction));
        next.model.max_output_tokens = Some(compact_output_tokens(&next.model));
        let (plain_system, _) = prompt(v, context, false, None);
        Self {
            request: next,
            plain_system,
        }
    }
}

/// The Compact request (AI-21) for the conversation's context, forked from the next request of a
/// turn with `context` and `server_id` whose MCP tools are `offer`.
fn compact_request(
    v: &Vault,
    conversation_id: &str,
    context: &AiTurnContext,
    server_id: Option<&str>,
    offer: &Offer,
    instruction: &str,
) -> AppResult<Compact> {
    let list = context_entries(v, conversation_id)?;
    if !compactable(&list) {
        return Err(AppError::invalid("conversation_id", "nothing to compact"));
    }
    let next = next_request(v, context, server_id, offer, list)
        .map_err(|message| AppError::invalid("model_id", message))?;
    Ok(Compact::fork(v, context, next, instruction))
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

/// Logs the token counts of a Compact response, never its text (SEC-04), so the share the prompt
/// cache served can be measured.
fn log_compact_usage(conversation_id: &str, fallback: bool, usage: Option<Usage>) {
    let usage = usage.unwrap_or_default();
    tracing::info!(
        conversation_id,
        fallback,
        input_tokens = usage.input_tokens,
        cache_read_tokens = usage.cache_read_tokens,
        cache_write_tokens = usage.cache_write_tokens,
        output_tokens = usage.output_tokens,
        estimated = usage.estimated,
        "AI compaction response"
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
    let tokens = context_tokens(&list).saturating_add(estimate_tokens(text));
    Ok(over_threshold(tokens, window))
}

fn over_threshold(tokens: u64, window: u64) -> bool {
    tokens.saturating_mul(10) > window.saturating_mul(AUTO_COMPACT_TENTHS)
}

/// Stores a summary entry and moves `context_start` to it (AI-21).
fn store_summary(v: &mut Vault, conversation_id: &str, summary: &str) -> AppResult<AiEntryView> {
    let mut conversation = find_conversation(v, conversation_id)?;
    let entry = AiEntry::summary(now_ms(), summary);
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
        EntryBody::User { text } | EntryBody::Summary { text } => text.as_str().into(),
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
    let cancelled = cancel_open_calls(v, conversation_id, &entries)?;
    if !cancelled.is_empty() {
        *stored = true;
        for entry in cancelled {
            turn.emit(AiTurnEvent::Entry { entry });
        }
        entries = Entries::load(v, conversation_id)?;
    }
    let from = entries.context_from(conversation.context_start.as_deref());
    let context = entries.list.split_off(from);
    if nothing_to_send(&context) {
        return Err(Halt::Idle);
    }
    next_request(v, &turn.context, turn.server_id.as_deref(), offer, context).map_err(Halt::Failed)
}

/// The conversation's next request (§13.1) for its context `entries`: the system prompt that
/// offers tools, the built-in tools, then the MCP tools of `offer`. Every request of a turn and
/// every Compact request (AI-21) is built here, so a Compact request has its turn's prefix. The
/// error is a sentence for the panel.
fn next_request(
    v: &Vault,
    context: &AiTurnContext,
    server_id: Option<&str>,
    offer: &Offer,
    entries: Vec<AiEntry>,
) -> Result<Request, String> {
    let (provider, model) = resolve_model(v, &context.provider_id, &context.model_id)?;
    let (system, mut tools) = prompt(v, context, true, server_id);
    // AI-30: after the built-in tools, those of the MCP servers, with or without a tab (AI-09).
    tools.extend(offer.tool_defs(provider.protocol));
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
    let finish = response.finish;
    let shared = shared_call_ids(&response.tool_calls);
    let entry_id = {
        let mut v = lock(vault);
        if turn.cancel.is_cancelled() || !v.is_unlocked() {
            return Err(Halt::Stopped);
        }
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
            let entries = Entries::load(&v, conversation_id)?;
            let Some(from) = entries.ids.iter().position(|id| id == entry_id) else {
                return Err(Halt::Failed(
                    "The model's response is no longer stored.".into(),
                ));
            };
            if fix_up_missing_results(&entries.list[from..]).is_empty() {
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
fn open_call(entries: &Entries, tool_call_id: &str) -> AppResult<AiToolCall> {
    let newest = entries
        .list
        .iter()
        .rposition(|e| matches!(e.body, EntryBody::Assistant(_)));
    let calls: Vec<&ToolCall> = match newest.map(|i| &entries.list[i].body) {
        Some(EntryBody::Assistant(a)) => a
            .tool_calls
            .iter()
            .filter(|c| c.id == tool_call_id)
            .collect(),
        _ => Vec::new(),
    };
    let (Some(from), false) = (newest, calls.is_empty()) else {
        return Err(AppError::not_found("tool call"));
    };
    let open = fix_up_missing_results(&entries.list[from..])
        .iter()
        .filter(|id| *id == tool_call_id)
        .count();
    let Some(call) = calls.len().checked_sub(open).and_then(|i| calls.get(i)) else {
        return Err(AppError::invalid(
            "tool_call_id",
            "the tool call already has a result",
        ));
    };
    Ok(AiToolCall {
        id: call.id.clone(),
        name: call.name.clone(),
        arguments: call.arguments.clone(),
    })
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
/// connected tab (AI-09), `web_search` when a search provider is chosen, `fetch_url` always, and
/// `read_skill` when an enabled skill exists, the built-in one included (AI-34). `offer_tools`
/// is false only for the fallback of Compact (AI-21). `server_id` is the tab's SSH server's
/// identification string.
/// The prompt names the model and its provider, and carries the user's custom instructions
/// (AI-36) and the host's AI notes (AI-37), each cut to its limit.
fn prompt(
    v: &Vault,
    context: &AiTurnContext,
    offer_tools: bool,
    server_id: Option<&str>,
) -> (String, Vec<ToolDef>) {
    let host = context
        .host_id
        .as_deref()
        .and_then(|id| v.get(id))
        .and_then(Item::as_host);
    let target = context.target.as_ref().map(quick_label);
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
    let offered = match (offer_tools, context.tab) {
        (false, _) => PromptTools::None,
        (true, false) => PromptTools::NoTerminal,
        (true, true) => PromptTools::Terminal,
    };
    let system = tools::system_prompt(&PromptContext {
        model: PromptModel {
            id: &context.model_id,
            name: model_name,
            provider: provider.map_or("", |p| p.name.as_str()),
        },
        host_name: host.map(|h| h.name.as_str()).or(target.as_deref()),
        host_user: host
            .map(|h| h.username.as_str())
            .or(context.target.as_ref().map(|t| t.username.as_str())),
        server_id,
        date: &date,
        instructions: chars_prefix(
            &settings.ai.custom_instructions,
            MAX_CUSTOM_INSTRUCTIONS_CHARS,
        ),
        host_notes: host.map_or("", |h| chars_prefix(&h.ai_notes, MAX_HOST_AI_NOTES_CHARS)),
        skills: &skills,
        tools: offered,
    });
    let defs = if offer_tools {
        tools::builtin_tools(&ToolSet {
            terminal: context.tab,
            web_search: chosen_search(v).is_some(),
            read_skill: !skills.is_empty(),
        })
    } else {
        Vec::new()
    };
    (system, defs)
}

/// The first `max` characters of `s`. Saving refuses longer text; one synced from elsewhere is cut.
fn chars_prefix(s: &str, max: usize) -> &str {
    s.char_indices().nth(max).map_or(s, |(i, _)| &s[..i])
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
        provider_config(provider),
        ModelSpec {
            id: model.id.clone(),
            context_window: model.context_window,
            max_output_tokens: model.max_output_tokens,
            efforts: model
                .efforts
                .as_ref()
                .map(|levels| levels.iter().copied().map(model_effort).collect()),
            adaptive_thinking: model.adaptive_thinking,
        },
    ))
}

/// What a request needs to reach a saved provider.
pub fn provider_config(p: &AiProvider) -> ProviderConfig {
    ProviderConfig {
        protocol: protocol(p.protocol),
        base_url: p.base_url.clone(),
        api_key: p.api_key.clone(),
        auth_header: auth_header(p.auth_header),
    }
}

pub fn protocol(p: CoreProtocol) -> Protocol {
    match p {
        CoreProtocol::ChatCompletions => Protocol::ChatCompletions,
        CoreProtocol::Anthropic => Protocol::Anthropic,
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

/// The search provider `Settings.ai` chose, when it exists.
fn chosen_search(v: &Vault) -> Option<SearchConfig> {
    let id = v.settings().ai.search_provider_id?;
    v.get(&id)
        .and_then(Item::as_search_provider)
        .map(search_config)
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
    WebSearch(SearchConfig, String),
    FetchUrl(String, u64),
    /// An MCP tool the request offered, with its arguments (AI-30).
    Mcp(Box<OfferedTool>, Map<String, Value>),
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

    /// `version`: the app's, for the built-in skill (AI-34).
    fn new(v: &Vault, name: &str, arguments: &str, version: &str) -> Self {
        match name {
            tools::RUN_COMMAND => match parse::<RunCommandArgs>(arguments) {
                Ok(args) if args.command.trim().is_empty() => Self::error("The command is empty."),
                Ok(args) => Self::RunCommand(args),
                Err(message) => Self::error(message),
            },
            tools::WEB_SEARCH => match chosen_search(v) {
                None => Self::error(
                    "No search provider is chosen in Settings → AI, so web_search cannot run.",
                ),
                Some(config) => match parse::<WebSearchArgs>(arguments) {
                    Ok(args) => Self::WebSearch(config, args.query),
                    Err(message) => Self::error(message),
                },
            },
            tools::FETCH_URL => match parse::<FetchUrlArgs>(arguments) {
                Ok(args) => Self::FetchUrl(args.url, args.offset.unwrap_or(0)),
                Err(message) => Self::error(message),
            },
            tools::READ_SKILL => match parse::<ReadSkillArgs>(arguments) {
                Ok(args) => read_skill(v, &args, version),
                Err(message) => Self::error(message),
            },
            tools::READ_TERMINAL | tools::SEND_INPUT => Self::error(format!(
                "{name} runs in the terminal tab and cannot run here."
            )),
            _ => Self::error(format!("There is no tool named \"{name}\".")),
        }
    }

    async fn run(
        self,
        http: &reqwest::Client,
        mcp: &McpManager,
        vault: &SharedVault,
        env: &dyn AiEnv,
        session_id: Option<&str>,
        cancel: &CancellationToken,
    ) -> (ToolStatus, String) {
        match self {
            Self::Ready(status, content) => (status, content),
            Self::Mcp(tool, arguments) => mcp.call(vault, &tool, arguments, cancel).await,
            Self::RunCommand(args) => run_command(env, session_id, &args, cancel).await,
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
            Self::FetchUrl(url, offset) => match web::fetch_url(&url, offset, cancel).await {
                Ok(page) => (ToolStatus::Ok, web::format_fetch_result(&page)),
                Err(AiError::Cancelled) => (ToolStatus::Cancelled, String::new()),
                Err(e) => (
                    ToolStatus::Error,
                    format!("The page could not be fetched: {e}"),
                ),
            },
        }
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

/// AI-12: `command` on a new exec channel of the tab's connection, with the call's timeout.
async fn run_command(
    env: &dyn AiEnv,
    session_id: Option<&str>,
    args: &RunCommandArgs,
    cancel: &CancellationToken,
) -> (ToolStatus, String) {
    let Some(session) = session_id
        .and_then(|id| env.session(id))
        .filter(|s| !s.is_closed())
    else {
        return (ToolStatus::Error, DISCONNECTED.to_owned());
    };
    let limit = args.timeout();
    tokio::select! {
        biased;
        () = cancel.cancelled() => (ToolStatus::Cancelled, String::new()),
        result = tokio::time::timeout(limit, session.exec_output(&args.command, None)) => match result {
            Ok(Ok(out)) => (
                ToolStatus::Ok,
                tools::truncate_result(&tools::format_exec_result(
                    out.exit_status,
                    &String::from_utf8_lossy(&out.stdout),
                    &String::from_utf8_lossy(&out.stderr),
                )),
            ),
            Ok(Err(_)) if session.is_closed() => (ToolStatus::Error, DISCONNECTED.to_owned()),
            Ok(Err(e)) => (ToolStatus::Error, format!("The command could not run: {}", e.message)),
            Err(_) => (
                ToolStatus::Error,
                format!(
                    "The command did not finish within {} seconds. Hatoba closed its channel; \
                     the command may still be running on the host.",
                    limit.as_secs()
                ),
            ),
        }
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
    let builtin = v.settings().ai.builtin_skill_enabled;
    let enabled = enabled_skills(v);
    // `(path, content)` of every file of the skill, SKILL.md included.
    let files: Vec<(String, String)> = if builtin && name == BUILTIN_NAME {
        let skill = builtin_skill(version);
        std::iter::once((SKILL_MD.to_owned(), skill.body))
            .chain(skill.files.into_iter().map(|f| (f.path, f.content)))
            .collect()
    } else if let Some((skill_id, _)) = enabled.iter().find(|(_, s)| s.name == name) {
        v.skill_files(skill_id)
            .into_iter()
            .map(|(_, f)| (f.path, f.content))
            .collect()
    } else {
        let mut names: Vec<&str> = enabled.iter().map(|(_, s)| s.name.as_str()).collect();
        if builtin {
            names.push(BUILTIN_NAME);
        }
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
        let stored = v.ai_entries(conversation_id)?;
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
        Ok(entries)
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
}

/// Stores a cancelled result for every stored call that has none (§13.1) and returns them.
fn cancel_open_calls(
    v: &mut Vault,
    conversation_id: &str,
    entries: &Entries,
) -> AppResult<Vec<AiEntryView>> {
    let now = now_ms();
    fix_up_missing_results(&entries.list)
        .into_iter()
        .map(|call_id| {
            let entry = AiEntry::cancelled_result(now, call_id);
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

/// The tag of the note Hatoba puts before a message that moves the conversation to another
/// host (AI-09).
const HOST_CHANGE: &str = "host_change";
/// The tag of the note Hatoba puts before the first message to another model (AI-05).
const MODEL_CHANGE: &str = "model_change";

/// A kind of block a user entry may start with (spec §13.3, "Attachment blocks").
struct AttachmentKind {
    tag: &'static str,
    /// A note Hatoba writes, rather than something the user attached.
    note: bool,
    /// The title of a conversation whose first message has only this block, from its attributes.
    title: fn(&[(&str, &str)]) -> String,
    /// Whether the opening tag's attributes, in order, are the ones this kind writes.
    attributes: fn(&[(&str, &str)]) -> bool,
}

/// A `lines` attribute: a decimal count.
fn is_count(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
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
        tag: HOST_CHANGE,
        note: true,
        title: |_| String::new(),
        attributes: |attrs| matches!(attrs, [("from", _), ("to", _)]),
    },
    AttachmentKind {
        tag: MODEL_CHANGE,
        note: true,
        title: |_| String::new(),
        attributes: |attrs| matches!(attrs, [("from", _), ("to", _)]),
    },
    AttachmentKind {
        tag: "terminal_selection",
        note: false,
        title: |_| "Terminal selection".to_owned(),
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
        attributes: |attrs| matches!(attrs, [("host", _)]),
    },
    AttachmentKind {
        tag: "pasted_text",
        note: false,
        title: |_| "Pasted text".to_owned(),
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
        attributes: |attrs| matches!(attrs, [("name", _), ("lines", lines)] if is_count(lines)),
    },
];

/// One block a user entry starts with: its kind and its attributes as written.
struct Block<'a> {
    kind: &'static AttachmentKind,
    attrs: Vec<(&'a str, &'a str)>,
}

/// The blocks `text` starts with, in order, and the text after them. Notes come first, at most
/// one of each kind: a note after an attachment, or a second one of a kind, is typed text.
fn leading_blocks(text: &str) -> (Vec<Block<'_>>, &str) {
    let mut blocks: Vec<Block<'_>> = Vec::new();
    let mut rest = text;
    while let Some((block, after)) = leading_block(rest) {
        if block.kind.note
            && blocks
                .iter()
                .any(|b| !b.kind.note || b.kind.tag == block.kind.tag)
        {
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

/// A host's display name, when the host exists.
fn host_name(v: &Vault, host_id: &str) -> Option<String> {
    v.get(host_id)
        .and_then(Item::as_host)
        .map(|h| h.name.clone())
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

/// The notes Hatoba puts before a new message (spec §13.3), the host's first:
///
/// - `host_change` (AI-09) when the entries before the message came from another host than the
///   conversation's host now: `came_from` names it (empty when it no longer exists), and is
///   `None` when the conversation did not move. `host_now` is the display name of the host (or
///   quick-connect target) the conversation is on now.
/// - `model_change` (AI-05) when the message goes to another model ID than the newest reply in
///   `earlier` (the entries before the message) came from. A switch an earlier message noted
///   already, which got no reply (a failed request, say), is not noted again.
fn notes_before(
    v: &Vault,
    earlier: &[AiEntry],
    came_from: Option<&str>,
    host_now: Option<&str>,
    context: &AiTurnContext,
) -> String {
    let mut notes = String::new();
    // A message with nothing before it has no earlier screens to tell apart.
    if let (Some(from), Some(to), false) = (came_from, host_now, earlier.is_empty())
        && from != to
    {
        notes.push_str(&tools::host_change_block(from, to));
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
            EntryBody::User { text } => {
                if note_attr(text, MODEL_CHANGE, "to").is_some_and(|noted| noted == to) {
                    break;
                }
            }
            EntryBody::Tool { .. } | EntryBody::Summary { .. } => {}
        }
    }
    notes
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
                EntryBody::User { text } | EntryBody::Summary { text } => text.as_str(),
                EntryBody::Assistant(a) => a.text.as_str(),
                EntryBody::Tool { .. } => return None,
            };
            snippet(text, &needle).map(|s| (Some(id.clone()), s))
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
        EntryBody::User { text } => AiEntryView::User {
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
        } => AiEntryView::Tool {
            entry_id,
            created_at,
            tool_call_id: tool_call_id.clone(),
            status: status_view(*status),
            content: content.clone(),
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
        AiError::Cancelled => "cancelled",
    }
}

#[cfg(test)]
mod tests;
