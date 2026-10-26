//! What runs now and how it stops (§13.1): the running turns, and the ops (tools, compactions,
//! side questions and provider requests) that a stop or the lock reaches, and the stop that waits
//! for the commands of `run_command` calls (AI-18).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hatoba_ai::entry::{AiEntry, EntryBody, ToolStatus};
use hatoba_ai::tools;
use hatoba_core::Vault;
use hatoba_core::sync::SharedVault;
use hatoba_ssh::remote_command;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::entries::{Entries, append, cancel_open_calls, find_conversation};
use super::view::entry_view;
use super::{
    AiEnv, AiManager, EventSink, Halted, Op, OpGuard, OpKind, Ran, Turn, Waiting, files, guard,
    lock,
};
use crate::dto::{AiEntryView, AiTurnContext, AiTurnEndReason, AiTurnEvent};
use crate::error::{AppError, AppResult};
use crate::state::now_ms;

impl AiManager {
    /// A turn of the conversation is running, or a stop of one waits for its commands to end.
    pub fn is_running(&self, conversation_id: &str) -> bool {
        guard(&self.0.turns).contains_key(conversation_id)
            || guard(&self.0.stopping).contains_key(conversation_id)
    }

    pub(super) fn running(&self, conversation_id: &str) -> Option<Arc<Turn>> {
        guard(&self.0.turns).get(conversation_id).cloned()
    }

    /// Registers a turn. The caller stopped the previous one under the same vault guard.
    pub(super) fn register(
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
    pub(super) fn forget(&self, conversation_id: &str, turn: &Arc<Turn>) {
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
    pub(super) fn cancel(&self, conversation_id: &str) -> Option<Arc<Turn>> {
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
    pub(super) fn start_op(&self, conversation_id: &str) -> OpGuard<'_> {
        let cancel = self.turn_token(conversation_id);
        self.register_op(Some(conversation_id), None, OpKind::Other, cancel)
    }

    /// Registers a compaction of the conversation (AI-21, AI-22), like [`Self::start_op`]. The
    /// conversation counts as compacting until the guard drops ([`Self::is_compacting`]).
    pub(super) fn start_compaction(&self, conversation_id: &str) -> OpGuard<'_> {
        let cancel = self.turn_token(conversation_id);
        self.register_op(Some(conversation_id), None, OpKind::Compact, cancel)
    }

    /// A compaction of the conversation runs, by hand or automatically.
    pub(super) fn is_compacting(&self, conversation_id: &str) -> bool {
        guard(&self.0.ops).values().any(|op| {
            op.kind == OpKind::Compact && op.conversation_id.as_deref() == Some(conversation_id)
        })
    }

    /// Registers the run of a tool call, like [`Self::start_op`]. Refused while the call runs
    /// already, so that a call with side effects (`run_command`, an MCP tool) runs once: called
    /// under the vault guard that checked the call, as is [`Self::call_running`]. `kind` is
    /// [`OpKind::Command`] for a `run_command` call.
    pub(super) fn start_call(
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
    pub(super) fn call_running(&self, conversation_id: &str, tool_call_id: &str) -> bool {
        guard(&self.0.ops).values().any(|op| {
            op.conversation_id.as_deref() == Some(conversation_id)
                && op.tool_call_id.as_deref() == Some(tool_call_id)
        })
    }

    /// A token that a stop of the conversation's running turn cancels.
    /// While a stop waits for the conversation's commands, the token is cancelled already, so
    /// nothing new starts in the stopped turn.
    pub(super) fn turn_token(&self, conversation_id: &str) -> CancellationToken {
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
    pub(super) fn start_side(&self, conversation_id: &str, question_id: u32) -> OpGuard<'_> {
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
    pub(super) fn stop_sides(&self, conversation_id: &str) {
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
    pub(super) fn stop_locked(&self, v: &mut Vault, conversation_id: &str) -> Vec<AiEntryView> {
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
    pub(super) fn running_commands(&self, conversation_id: &str) -> Vec<(String, &'static str)> {
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
    pub(super) fn ran(&self, op: &OpGuard<'_>, ran: Ran) {
        if let Some(op) = guard(&self.0.ops).get_mut(&op.id) {
            op.ran = Some(ran);
        }
    }

    /// Takes what the tool of the op's call returned, unless a stop took it to store it.
    pub(super) fn take_ran(&self, op: &OpGuard<'_>) -> Option<Ran> {
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

    /// A turn, a tool, a preview, a compaction or a revert of the conversation runs.
    pub(super) fn busy(&self, conversation_id: &str) -> bool {
        self.is_running(conversation_id)
            || guard(&self.0.ops)
                .values()
                .any(|op| op.conversation_id.as_deref() == Some(conversation_id))
    }

    /// Drops what the approval cards of the conversation read (AI-39): none of its calls waits for
    /// one any more.
    pub(super) fn forget_file_bases(&self, conversation_id: &str) {
        guard(&self.0.file_bases).retain(|(id, _), _| id != conversation_id);
    }

    /// Drops the judge's verdicts on the conversation's calls (AI-47): none of them waits any more.
    pub(super) fn forget_verdicts(&self, conversation_id: &str) {
        guard(&self.0.pending_verdicts).retain(|(id, _), _| id != conversation_id);
    }
}

/// How long a stop waits for the commands of `run_command` calls to end and their calls to store
/// results that say so (AI-18): the time `remote_command::run` takes at most after a stop, and a
/// moment to store the result.
pub(super) const COMMAND_STOP_WAIT: Duration =
    remote_command::STOP_WAIT.saturating_add(Duration::from_secs(1));
