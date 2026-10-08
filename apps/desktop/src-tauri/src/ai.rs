//! The AI assistant's runtime (spec §13.1): running turns, the tools that run in Rust, and the
//! conversation operations behind the `ai_*` commands.
//!
//! A turn is a task that builds a request from the stored conversation, streams the response to
//! the turn's channel, stores the assistant entry and, when the response has tool calls, waits
//! until every call has a stored result before it sends the next request. The frontend runs
//! `read_terminal` and `send_input` and reports with `ai_tool_result`; `ai_tool_run` runs the
//! other tools here. What is stored is exactly what the model receives (§13.7).
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
    AiEntry, AssistantEntry, EntryBody, Finish, ToolStatus, fix_up_missing_results,
};
use hatoba_ai::provider::{AuthHeader, ModelSpec, Protocol, ProviderConfig};
use hatoba_ai::tools::{
    self, FetchUrlArgs, PromptContext, ReadSkillArgs, RunCommandArgs, ToolSet, WebSearchArgs,
};
use hatoba_ai::web::{self, SearchConfig, SearchKind as WebSearchKind};
use hatoba_core::Vault;
use hatoba_core::model::{
    AiAuthHeader, AiConversation, AiProtocol as CoreProtocol, AiProvider, Item,
    SearchKind as CoreSearchKind, SearchProvider,
};
use hatoba_core::sync::SharedVault;
use serde::de::DeserializeOwned;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::dto::{
    AiConversationDetail, AiConversationView, AiEntryView, AiFinish, AiSendInput, AiSendStarted,
    AiToolCall, AiToolResultInput, AiToolStatus, AiTurnContext, AiTurnEndReason, AiTurnEvent,
    AiUsage,
};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::state::now_ms;

/// The line a result starts with when the user edited the call before it ran (AI-17).
const EDIT_NOTE: &str = "The user edited the arguments before the call ran; it ran with";

/// The `run_command` result while the tab has no live session (AI-08).
const DISCONNECTED: &str = "The terminal tab is disconnected, so the command did not run. Ask \
     the user to reconnect the tab.";

/// The final instruction of a Compact request (AI-21).
const COMPACT_INSTRUCTION: &str = "Summarize the conversation so far so that the summary can \
     replace it as the context of the rest of this conversation. Keep everything later work \
     needs: the user's goals and requests, the hosts and systems involved, the commands that \
     were run and their outcomes, files and settings that were changed, decisions made, and open \
     problems and next steps. Leave out what no longer matters. Write the summary in the user's \
     language and reply with the summary only.";

/// A skill's main file (§13.8).
const SKILL_MD: &str = "SKILL.md";

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
    /// Running turns by conversation id.
    turns: Mutex<HashMap<String, Arc<Turn>>>,
    /// Running tools and compactions, so that stop and lock reach them.
    ops: Mutex<HashMap<u64, Op>>,
    next_op: AtomicU64,
}

struct Op {
    conversation_id: String,
    cancel: CancellationToken,
}

/// A running tool or compaction; unregisters itself when dropped.
struct OpGuard<'a> {
    inner: &'a Inner,
    id: u64,
    cancel: CancellationToken,
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
        sink: Arc<dyn EventSink>,
    ) -> Arc<Turn> {
        let turn = Arc::new(Turn {
            cancel: CancellationToken::new(),
            sink,
            context,
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
            if op.conversation_id == conversation_id {
                op.cancel.cancel();
            }
        }
        turn
    }

    /// Registers a tool or compaction of the conversation. Called under the vault guard that
    /// checked the call, so a stop either came first (and the check failed) or cancels it.
    fn start_op(&self, conversation_id: &str) -> OpGuard<'_> {
        let cancel = self
            .running(conversation_id)
            .map_or_else(CancellationToken::new, |turn| turn.cancel.child_token());
        let id = self.0.next_op.fetch_add(1, Ordering::Relaxed);
        guard(&self.0.ops).insert(
            id,
            Op {
                conversation_id: conversation_id.to_owned(),
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

    /// The vault is about to lock (SEC-02): stops every turn and tool, so the assistant never
    /// acts behind the lock screen (§13.1). Called with the vault guard, before `Vault::lock`.
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
    pub fn send(
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
        if text.trim().is_empty() {
            return Err(AppError::invalid("text", "the message is empty"));
        }
        let (started, conversation_id, turn) = {
            let mut v = unlocked(vault)?;
            let now = now_ms();
            let mut cancelled = Vec::new();
            let id = match conversation_id {
                Some(id) => {
                    let mut conversation = find_conversation(&v, &id)?;
                    cancelled = self.stop_locked(&mut v, &id);
                    // AI-09: the next message moves the conversation to the tab's host.
                    if let Some(host_id) = &context.host_id
                        && conversation.host_id.as_ref() != Some(host_id)
                    {
                        conversation.host_id = Some(host_id.clone());
                        v.put(Some(&id), Item::AiConversation(conversation))?;
                    }
                    id
                }
                None => v.put(
                    None,
                    Item::AiConversation(AiConversation {
                        title: title_of(&text),
                        host_id: context.host_id.clone(),
                        pinned: false,
                        context_start: None,
                        created_at: now,
                        updated_at: 0,
                    }),
                )?,
            };
            let entry = AiEntry::user(now, text);
            let entry_id = append(&mut v, &id, &entry)?;
            let conversation = find_conversation(&v, &id)?;
            let started = AiSendStarted {
                conversation: conversation_view(&v, &id, &conversation),
                user_entry: entry_view(&entry_id, &entry),
            };
            let turn = self.register(&id, context, sink);
            // The panel stopped listening to the previous turn's channel, so the results the
            // stop stored reach it on this one.
            for entry in cancelled {
                turn.emit(AiTurnEvent::Entry { entry });
            }
            (started, id, turn)
        };
        env.changed();
        tracing::info!(conversation_id = %conversation_id, "AI turn started");
        Ok((
            started,
            self.task(vault.clone(), env, conversation_id, turn),
        ))
    }

    /// `ai_retry`: sends the next request from the stored conversation on a new channel. A
    /// running turn of the conversation stops first. A conversation that ends with the model's
    /// answer has nothing to retry: the request would end with an assistant message, which
    /// newer models reject as a prefill.
    pub fn retry(
        &self,
        vault: &SharedVault,
        env: Arc<dyn AiEnv>,
        conversation_id: String,
        context: AiTurnContext,
        sink: Arc<dyn EventSink>,
    ) -> AppResult<TurnTask> {
        let mut stored = false;
        let registered = (|| {
            let mut v = unlocked(vault)?;
            let conversation = find_conversation(&v, &conversation_id)?;
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
            let turn = self.register(&conversation_id, context, sink);
            for entry in cancelled {
                turn.emit(AiTurnEvent::Entry { entry });
            }
            Ok(turn)
        })();
        if stored {
            env.changed();
        }
        let turn = registered?;
        tracing::info!(conversation_id = %conversation_id, "AI turn retried");
        Ok(self.task(vault.clone(), env, conversation_id, turn))
    }

    fn task(
        &self,
        vault: SharedVault,
        env: Arc<dyn AiEnv>,
        conversation_id: String,
        turn: Arc<Turn>,
    ) -> TurnTask {
        let manager = self.clone();
        Box::pin(async move {
            let reason = manager
                .turn_loop(&vault, env.as_ref(), &conversation_id, &turn)
                .await;
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
            let request = match prepare(vault, env, conversation_id, turn) {
                Ok(request) => request,
                Err(halt) => return halted(turn, conversation_id, halt),
            };
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
    /// rejection). The call must belong to the conversation's newest response and have no result.
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
            self.store_result(&mut v, conversation_id, tool_call_id, status, content)?
        };
        env.changed();
        Ok(view)
    }

    /// `ai_tool_run`: runs a tool that runs in Rust and stores its result. Every failure of the
    /// tool itself is an `error` result; only a precondition (locked, unknown conversation or
    /// call, a call that already has a result) rejects.
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
            let arguments = edited_arguments.unwrap_or(&call.arguments);
            (
                Job::new(&v, &call.name, arguments),
                self.start_op(conversation_id),
            )
        };
        let (status, content) = job.run(&self.http(), env, session_id, &op.cancel).await;
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

    /// `ai_compact` (AI-21): asks the model for a summary of the context, with no tools, stores
    /// it as a `summary` entry and moves `context_start` to it. Refused while a turn runs.
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
        let (request, op) = prepared?;
        let response =
            match complete(&self.http(), &request.provider, &request.chat(), &op.cancel).await {
                Ok(response) => response,
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
        let summary = response.text.trim();
        if summary.is_empty() {
            return Err(AppError::new(
                ErrorCode::Ai,
                "the model returned an empty summary",
            ));
        }
        let view = {
            let mut v = unlocked(vault)?;
            if op.cancel.is_cancelled() {
                return Err(AppError::new(
                    ErrorCode::Cancelled,
                    "compaction was stopped",
                ));
            }
            let mut conversation = find_conversation(&v, conversation_id)?;
            let entry = AiEntry::summary(now_ms(), summary);
            let entry_id = append(&mut v, conversation_id, &entry)?;
            conversation.context_start = Some(entry_id.clone());
            v.put(Some(conversation_id), Item::AiConversation(conversation))?;
            entry_view(&entry_id, &entry)
        };
        drop(op);
        tracing::info!(conversation_id, "AI conversation compacted");
        env.changed();
        Ok(view)
    }

    fn prepare_compact(
        &self,
        vault: &SharedVault,
        conversation_id: &str,
        context: &AiTurnContext,
        stored: &mut bool,
    ) -> AppResult<(Request, OpGuard<'_>)> {
        let mut v = unlocked(vault)?;
        let conversation = find_conversation(&v, conversation_id)?;
        if self.is_running(conversation_id) {
            return Err(AppError::invalid("conversation_id", "a turn is running"));
        }
        let mut entries = Entries::load(&v, conversation_id)?;
        if !cancel_open_calls(&mut v, conversation_id, &entries)?.is_empty() {
            *stored = true;
            entries = Entries::load(&v, conversation_id)?;
        }
        let from = entries.context_from(conversation.context_start.as_deref());
        let mut list = entries.list.split_off(from);
        if !list
            .iter()
            .any(|e| matches!(e.body, EntryBody::User { .. } | EntryBody::Assistant(_)))
        {
            return Err(AppError::invalid("conversation_id", "nothing to compact"));
        }
        list.push(AiEntry::user(now_ms(), COMPACT_INSTRUCTION));
        let (provider, model) = resolve_model(&v, &context.provider_id, &context.model_id)
            .map_err(|message| AppError::invalid("model_id", message))?;
        let (system, _) = prompt(&v, context, false);
        let request = Request {
            provider_id: context.provider_id.clone(),
            provider,
            model,
            system,
            tools: Vec::new(),
            entries: list,
        };
        Ok((request, self.start_op(conversation_id)))
    }
}

// ---- turn steps ----

/// Reads what the next request needs, after storing a cancelled result for every call without
/// one (§13.1).
fn prepare(
    vault: &SharedVault,
    env: &dyn AiEnv,
    conversation_id: &str,
    turn: &Turn,
) -> Result<Request, Halt> {
    let mut stored = false;
    let result = prepare_locked(&mut lock(vault), conversation_id, turn, &mut stored);
    if stored {
        env.changed();
    }
    result
}

fn prepare_locked(
    v: &mut Vault,
    conversation_id: &str,
    turn: &Turn,
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
    let (provider, model) = resolve_model(v, &turn.context.provider_id, &turn.context.model_id)
        .map_err(Halt::Failed)?;
    let (system, tools) = prompt(v, &turn.context, true);
    Ok(Request {
        provider_id: turn.context.provider_id.clone(),
        provider,
        model,
        system,
        tools,
        entries: context,
    })
}

/// Stores a response and sends its `entry` and `done` events (under the vault guard, so they
/// keep the store's order against a concurrent stop). Returns its entry id.
fn store_response(
    vault: &SharedVault,
    env: &dyn AiEnv,
    conversation_id: &str,
    turn: &Turn,
    response: AssistantEntry,
) -> Result<String, Halt> {
    let finish = response.finish;
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

/// The call `tool_call_id` of the conversation's newest response, when it has no result yet.
fn open_call(entries: &Entries, tool_call_id: &str) -> AppResult<AiToolCall> {
    let newest = entries
        .list
        .iter()
        .rposition(|e| matches!(e.body, EntryBody::Assistant(_)));
    let call = newest.and_then(|i| match &entries.list[i].body {
        EntryBody::Assistant(a) => a
            .tool_calls
            .iter()
            .find(|c| c.id == tool_call_id)
            .map(|c| (i, c)),
        _ => None,
    });
    let Some((from, call)) = call else {
        return Err(AppError::not_found("tool call"));
    };
    if !fix_up_missing_results(&entries.list[from..])
        .iter()
        .any(|id| id == tool_call_id)
    {
        return Err(AppError::invalid(
            "tool_call_id",
            "the tool call already has a result",
        ));
    }
    Ok(AiToolCall {
        id: call.id.clone(),
        name: call.name.clone(),
        arguments: call.arguments.clone(),
    })
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

/// The system prompt and tools of a request (§13.1). Tools are offered only with a connected tab
/// (AI-09): the built-in set, `web_search` when a search provider is chosen, `read_skill` when
/// an enabled skill exists.
fn prompt(v: &Vault, context: &AiTurnContext, offer_tools: bool) -> (String, Vec<ToolDef>) {
    let host = context
        .host_id
        .as_deref()
        .and_then(|id| v.get(id))
        .and_then(Item::as_host);
    let skills: Vec<(String, String)> = v
        .skills()
        .into_iter()
        .filter(|(_, s)| s.enabled)
        .map(|(_, s)| (s.name, s.description))
        .collect();
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let with_tools = offer_tools && context.tab;
    let system = tools::system_prompt(&PromptContext {
        host_name: host.map(|h| h.name.as_str()),
        host_user: host.map(|h| h.username.as_str()),
        date: &date,
        skills: &skills,
        tools: with_tools,
    });
    let defs = if with_tools {
        tools::builtin_tools(&ToolSet {
            terminal: true,
            web_search: chosen_search(v).is_some(),
            read_skill: !skills.is_empty(),
        })
    } else {
        Vec::new()
    };
    (system, defs)
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
}

impl Job {
    fn error(message: impl Into<String>) -> Self {
        Self::Ready(ToolStatus::Error, message.into())
    }

    fn new(v: &Vault, name: &str, arguments: &str) -> Self {
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
                Ok(args) => read_skill(v, &args),
                Err(message) => Self::error(message),
            },
            tools::READ_TERMINAL | tools::SEND_INPUT => Self::error(format!(
                "{name} runs in the terminal tab and cannot run here."
            )),
            _ if name.starts_with("mcp__") => {
                Self::error("MCP servers are not available yet, so this tool cannot run.")
            }
            _ => Self::error(format!("There is no tool named \"{name}\".")),
        }
    }

    async fn run(
        self,
        http: &reqwest::Client,
        env: &dyn AiEnv,
        session_id: Option<&str>,
        cancel: &CancellationToken,
    ) -> (ToolStatus, String) {
        match self {
            Self::Ready(status, content) => (status, content),
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

/// AI-28: the `SKILL.md` body of an enabled skill, or the file at `path` inside it.
fn read_skill(v: &Vault, args: &ReadSkillArgs) -> Job {
    let name = args.name.trim();
    let enabled: Vec<(String, hatoba_core::model::Skill)> =
        v.skills().into_iter().filter(|(_, s)| s.enabled).collect();
    let Some((skill_id, skill)) = enabled.iter().find(|(_, s)| s.name == name) else {
        let mut names: Vec<&str> = enabled.iter().map(|(_, s)| s.name.as_str()).collect();
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
    let files = v.skill_files(skill_id);
    match files.iter().find(|(_, f)| f.path == path) {
        Some((_, file)) => Job::Ready(ToolStatus::Ok, tools::truncate_result(&file.content)),
        None => {
            let mut paths: Vec<&str> = files.iter().map(|(_, f)| f.path.as_str()).collect();
            paths.sort_unstable();
            Job::error(format!(
                "The skill \"{}\" has no file \"{path}\". Its files: {}.",
                skill.name,
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

/// AI-23: the title starts as the first line of the first message, cut to 60 characters.
fn title_of(text: &str) -> String {
    text.trim()
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(60)
        .collect()
}

// ---- views ----

pub fn conversation_view(v: &Vault, id: &str, c: &AiConversation) -> AiConversationView {
    AiConversationView {
        id: id.to_owned(),
        title: c.title.clone(),
        host_id: c.host_id.clone(),
        pinned: c.pinned,
        context_start: c.context_start.clone(),
        created_at: c.created_at,
        updated_at: c.updated_at,
        last_activity: v.ai_last_activity(id),
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
