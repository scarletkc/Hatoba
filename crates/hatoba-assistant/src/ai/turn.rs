//! Turns (§13.1): sending a message, editing and sending one again (AI-26), retrying, and the
//! loop that sends a request, stores its response and waits for the results of its calls.

use std::sync::Arc;

use hatoba_ai::AiError;
use hatoba_ai::chat::{StreamEvent, stream_chat};
use hatoba_ai::entry::{
    AiEntry, AssistantEntry, EntryBody, EntryPlace, Finish, ToolCall, ToolStatus,
    fix_up_missing_results,
};
use hatoba_core::Vault;
use hatoba_core::model::{AiConversation, Item};
use hatoba_core::sync::SharedVault;

use super::blocks::{HOST_CHANGE, note_attr};
use super::compact::{
    COMPACT_MID_TURN_INSTRUCTION, CompactFailure, compaction_before, compaction_due,
    deferred_saving, offers_deferred,
};
use super::entries::{Entries, append, find_conversation, title_of};
use super::notes::{
    mcp_change_note, mcp_deferred, mcp_note, notes_before, pending_reverts, with_place_memories,
};
use super::places::{
    Place, conversation_memory_place, entry_place, host_name, host_target, local_name,
    memory_place, moved_from, moves_to, names_server, tab_local, tab_target, target_name,
};
use super::request::{core_effort, nothing_to_send, prepare, turn_server_id};
use super::side::SideRequest;
use super::view::{
    conversation_view, entry_view, error_kind, error_text, finish_view, stream_event,
};
use super::{AiEnv, AiManager, EventSink, Halt, Streamed, Turn, TurnTask, guard, lock, unlocked};
use crate::dto::{AiSendInput, AiSendStarted, AiTurnContext, AiTurnEndReason, AiTurnEvent};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::now_ms;

impl AiManager {
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
                let changed = mcp_change_note(&entries.list, &offer, &listed);
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
                    &format!("{notes}{listed}{changed}{text}"),
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
            // compaction, whose summary leaves it out, or AI-30: the servers whose tools came or
            // went. AI-53: the memories of where the message is, which a summary of where the
            // conversation was can leave out too.
            let (listed, changed, notes) = {
                let conversation = find_conversation(&v, &id)?;
                let entries = Entries::load(&v, &id)?;
                let place = match &moved_to {
                    Some(to) => memory_place(
                        &v,
                        to.host_id.as_deref(),
                        to.quick_target.as_ref(),
                        to.local.as_deref(),
                    ),
                    None => conversation_memory_place(&v, &conversation),
                };
                let notes = with_place_memories(&v, &entries.list, &notes, place.as_ref());
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
                let changed = mcp_change_note(&entries.list, &offer, &listed);
                (listed, changed, notes)
            };
            // AI-05, AI-09, AI-30, AI-45: Hatoba's notes come before what the panel sent (spec
            // §13.3).
            let text = format!("{notes}{listed}{changed}{text}");
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
                let place = memory_place(
                    &v,
                    host_id.as_deref(),
                    quick_target.as_ref(),
                    local.as_deref(),
                );
                let reverts = pending_reverts(&v, &id, &ids[..earlier.len()], &earlier);
                let notes = notes_before(
                    &v,
                    &earlier,
                    came_from.as_deref(),
                    (host_now.as_deref(), place.as_ref()),
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
                let place = conversation_memory_place(&v, &created);
                let notes = notes_before(
                    &v,
                    &[],
                    None,
                    (host_now.as_deref(), place.as_ref()),
                    &context,
                    &[],
                );
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
            let deferred = offers_deferred(&request.tools);
            // AI-30: the next message is noted when it has other servers' tools.
            let mcp_servers = (!deferred).then(|| offer.server_names());
            if !deferred || !self.0.mcp.has_offer(conversation_id) {
                self.0.mcp.record_offer(conversation_id, offer);
            }
            turn.emit(AiTurnEvent::RequestStarted);
            let http = self.http();
            *guard(&turn.streamed) = Some(Streamed {
                provider_id: request.provider_id.clone(),
                model_id: request.model.id.clone(),
                text: String::new(),
                tool_calls: Vec::new(),
                mcp_servers: mcp_servers.clone(),
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
            let response = AssistantEntry {
                mcp_servers,
                ..response
            };
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
pub(super) async fn wait_for_results(
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
