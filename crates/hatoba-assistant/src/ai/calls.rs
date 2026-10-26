//! The tool calls of a response (§13.1): the results the frontend reports and the tools that
//! run here, both stored in the order of the calls, and the preview an approval card shows for
//! `edit_file` and `write_file` (AI-39).

use std::sync::{Arc, Mutex};

use hatoba_ai::AiError;
use hatoba_ai::entry::{
    AiEntry, EntryBody, ToolCall, ToolStatus, fix_up_missing_results, pair_results,
};
use hatoba_ai::mcp::deferred;
use hatoba_ai::provider::{ModelSpec, ProviderConfig};
use hatoba_ai::tools;
use hatoba_ai::web::{self, NativeSearch};
use hatoba_core::model::AiConversation;
use hatoba_core::sync::SharedVault;
use hatoba_core::{SnapshotKey, Vault};
use tokio_util::sync::CancellationToken;

use super::changes::KeepSnapshot;
use super::entries::{Entries, append, find_conversation};
use super::files::FileJob;
use super::job::{Job, live_session, proxy_call};
use super::places::conversation_memory_place;
use super::view::{core_status, entry_view};
use super::{AiEnv, AiManager, MCP_PREFIX, OpKind, Ran, files, guard, unlocked};
use crate::dto::{
    AiEntryView, AiFilePreview, AiToolCall, AiToolResultInput, AiToolRun, AiTurnEvent,
};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::now_ms;

impl AiManager {
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
            if let Job::Memory(_, at) = &mut job {
                *at = conversation_memory_place(&v, &conversation);
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

    /// Stores a tool result, sends it on the running turn's channel and wakes the turn. The
    /// judge's verdict on the call (AI-47) goes with the result when Auto decided the call
    /// (`judged`), and is dropped otherwise.
    pub(super) fn store_result(
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
}

/// The line a result starts with when the user edited the call before it ran (AI-17).
pub(super) const EDIT_NOTE: &str = "The user edited the arguments before the call ran; it ran with";

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

/// The call `tool_call_id` of the conversation's newest response, when it has no result yet.
/// Results answer the calls with their id in order (`fix_up_missing_results`), so of calls that
/// share an id (stored before such calls got their results in [`store_response`]), the open one
/// is the first that no result answers.
pub(super) fn open_call(entries: &Entries, tool_call_id: &str) -> AppResult<AiToolCall> {
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
pub(super) fn open_call_at(entries: &Entries, tool_call_id: &str) -> AppResult<(usize, usize)> {
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
pub(super) fn calls_of(entry: &AiEntry) -> &[ToolCall] {
    match &entry.body {
        EntryBody::Assistant(a) => &a.tool_calls,
        _ => &[],
    }
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

/// Another call of the newest response has the id of the call `tool_call_id` too.
pub(super) fn shares_id(entries: &Entries, tool_call_id: &str) -> bool {
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
