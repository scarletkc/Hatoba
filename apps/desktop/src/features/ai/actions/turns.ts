/** Turns (§13.1): a message sent, retried, edited and sent again (AI-26) or stopped (AI-18); compaction (AI-21). */

import { toast } from "@hatoba/ui";
import { t } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { AiConversationView, AiEntryView, AiFileChange, AiFileRevert, AiTurnEndReason } from "@/ipc/types";
import { composeMessage, notedPermission, parseMessage } from "../attachments";
import { askEditedFiles } from "../calls/EditFiles";
import { menuSources, useMcp } from "../mcp";
import { notifyAi } from "../notify";
import { joinMessages, messageBack, messageText, sendsQueued } from "../queue";
import {
  defaultMode,
  getSlot,
  inBackground,
  patchSlot,
  slotOf,
  useAi,
  type PendingAttachment,
  type QueuedMessage,
  type Slot,
} from "../store";
import {
  editedFiles,
  entriesBefore,
  laterContextStart,
  laterMessages,
  LOCAL_ENTRY,
  newTurn,
  placeUserEntry,
  upsertEntry,
} from "../turn";
import { extraId } from "./attach";
import { aiErrorMessage, turnContext, unlocked } from "./context";
import { loadHistory } from "./conversations";
import { fileChangeDiff } from "./fileChanges";
import { giveBackQueued, joinFits, putBack, queueMessage, takeInput } from "./queued";
import { revertChange, reverting, revertSession } from "./reverts";
import { noteChange, runners, TurnRunner } from "./runner";
import { discardSideQuestion } from "./sideQuestions";
import { clearBackground, reloadSlot, remoteStops, slotBusy } from "./slots";

/** The conversation view a command returned, without moving `context_start` back past a summary the turn already stored. */
function withContext(s: Slot, view: AiConversationView): AiConversationView {
  return {
    ...view,
    context_start: laterContextStart(s.entries, s.conversation?.context_start ?? null, view.context_start),
  };
}

/**
 * A new conversation keeps what was chosen in its slot before it had an id: the mode (AI-16), the
 * tools menu (AI-30) and Send Terminal Screen (AI-46).
 */
function rememberChoices(slotId: string, convId: string) {
  const { mode, mcpOff, screen } = getSlot(slotId);
  useAi.setState((st) => ({
    modes: mode !== defaultMode() || st.modes[convId] ? { ...st.modes, [convId]: mode } : st.modes,
    mcpOff: mcpOff.length > 0 || st.mcpOff[convId] ? { ...st.mcpOff, [convId]: mcpOff } : st.mcpOff,
    screens: screen !== null ? { ...st.screens, [convId]: screen } : st.screens,
  }));
}

/**
 * After a turn: a stopped turn's cancelled results may only be in the store, and history order changed.
 * Then the queued message goes, or goes back into the input area (§13.1). `stoppedByUser`: Stop or Esc.
 */
async function afterTurn(slotId: string, reason: AiTurnEndReason, stoppedByUser: boolean) {
  if (useAi.getState().history) void loadHistory();
  const convId = getSlot(slotId).conversationId;
  if (reason === "stopped") await reloadSlot(slotId);
  // The conversation may have moved meanwhile (Connect to host); its queued message moved with it.
  const target = (convId && slotOf(convId)) || slotId;
  const continues = sendsQueued(reason, stoppedByUser) && !!getSlot(target).queued;
  // AI-07: the conversation waits for the user now, unless its queued message goes on. A stop was the user's own.
  if (reason !== "stopped" && !continues) void notifyAi(target, reason === "error" ? "failed" : "done");
  if (sendsQueued(reason, stoppedByUser)) void sendQueued(target);
  else giveBackQueued(target);
}

/** §13.1: the turn ended, so the slot's queued message goes; one that cannot goes back into the input area. */
async function sendQueued(slotId: string) {
  const slot = getSlot(slotId);
  if (!slot.queued || slot.turn || slot.remoteRunning || runners.has(slotId)) return;
  if (!(await sendMessage(slotId, ""))) giveBackQueued(slotId);
}

/**
 * AI-07: the first message of a new conversation left in the background was not stored (it failed, or was
 * stopped): nothing can open that conversation again, so the message, and one queued after it, go back into the
 * input area of its tab, and the background slot goes.
 */
function giveBackUnstored(slotId: string, message: QueuedMessage) {
  giveBackQueued(slotId);
  putBack(slotId, message);
  clearBackground(slotId);
}

/**
 * Sends a message (§13.1 step 1): the queued message, if any, then what the input area holds (`raw` and the
 * attachments), as one message. Sending while a turn runs stops that turn first (Ctrl+Enter, §13.1); a side
 * question does not (AI-42), but the message discards a side question's bubble. The tab's selection chip goes
 * with it (AI-10), and is then hidden until the selection changes.
 */
export async function sendMessage(slotId: string, raw: string): Promise<boolean> {
  const typed = raw.trim();
  const slot = getSlot(slotId);
  const queued = slot.queued;
  if (!typed && !queued) return false;
  // Not while a message is on its way (`ai_send`, `ai_edit_resend` or `ai_retry` has not returned): a new
  // conversation has no id yet, so a second message would start another one, and a message waiting for its
  // compaction (AI-22) would be stopped, and its pending entry dropped here. It waits in the queue instead.
  if (slot.turn?.phase === "starting") return queueMessage(slotId, raw);
  if (reverting(slotId)) return false;
  const context = turnContext(slotId);
  if (!context) {
    toast(t("ai.err.noModel"), "error");
    return false;
  }
  if (!joinFits(slotId, typed, queued)) return false;
  discardSideQuestion(slotId);
  runners.get(slotId)?.dispose(); // Rust stops that turn when the message arrives

  const taken = typed ? takeInput(slotId, typed, queued) : null;
  const message = queued && taken ? joinMessages(queued, taken) : (taken ?? queued!);
  const text = messageText(message);
  const runner = new TurnRunner(slotId, slot.conversationId, context.permission_mode, afterTurn);
  runners.set(slotId, runner);
  const local: AiEntryView = { role: "user", entry_id: LOCAL_ENTRY, created_at: Date.now(), text };
  patchSlot(slotId, (s) => ({
    entries: [...s.entries.filter((e) => e.entry_id !== LOCAL_ENTRY), local],
    turn: newTurn(),
    outcome: null,
    effortIgnored: false,
    remoteRunning: false,
    queued: null,
  }));
  try {
    const started = await api.ai_send({ conversation_id: slot.conversationId, text, context }, runner.onEvent);
    const conv = started.conversation;
    runner.started(conv.id);
    if (useAi.getState().history) void loadHistory();
    if (runner.gone) {
      // The vault locked, or the tab closed, meanwhile. A new conversation left in the background has no slot to open
      // again, so what went back into its input area goes to its tab (AI-07).
      const target = runner.slotId;
      if (target in useAi.getState().slots && inBackground(target) && !getSlot(target).conversationId)
        clearBackground(target);
      return true;
    }
    const target = runner.slotId;
    rememberChoices(target, conv.id);
    patchSlot(target, (s) => ({
      conversationId: conv.id,
      conversation: withContext(s, conv),
      entries: placeUserEntry(s.entries, started.user_entry),
    }));
    return true;
  } catch (e) {
    // The conversation may have moved meanwhile (Connect to host, or into the background, AI-07); its runner moved
    // with it. Closing its tab meanwhile discarded the input area, so nothing goes back.
    const target = runner.slotId;
    runner.dispose();
    const { slots } = useAi.getState();
    const held = target in slots;
    if (held && inBackground(target) && !getSlot(target).conversationId) giveBackUnstored(target, message);
    else if (held) {
      // A message queued meanwhile goes back too, after this one.
      giveBackQueued(target);
      patchSlot(target, (s) => ({ entries: s.entries.filter((x) => x.entry_id !== LOCAL_ENTRY), turn: null }));
      putBack(target, message);
    } else if (runner.tab in slots) {
      // Deleted in the background, whose slot went with its input area into the tab's (`clearBackground`): the
      // message goes there too, before the one queued after it.
      putBack(runner.tab, message);
    }
    // Stopped before it was stored, while the MCP servers started (AI-45) or the context was compacted
    // (AI-22): nothing was stored, a new conversation was not created, and the message is back in the input.
    if (toAppError(e).code !== "cancelled") toast(aiErrorMessage(e), "error");
    return false;
  }
}

/** Provider error or a reply without an answer → Retry (§9, §13.1): the next request from the stored conversation. */
export async function retryTurn(slotId: string) {
  const slot = getSlot(slotId);
  const id = slot.conversationId;
  if (!id || slot.turn || reverting(slotId)) return;
  const context = turnContext(slotId);
  if (!context) {
    toast(t("ai.err.noModel"), "error");
    return;
  }
  discardSideQuestion(slotId);
  // A retry stores no message, so no note tells the model of a switch since (§13.5): it works in the
  // mode its messages named, or in manual approval when they named none.
  const runner = new TurnRunner(slotId, id, notedPermission(slot.entries) ?? "manual", afterTurn);
  runners.set(slotId, runner);
  patchSlot(slotId, { turn: newTurn(), outcome: null, effortIgnored: false });
  try {
    await api.ai_retry(id, context, runner.onEvent);
  } catch (e) {
    runner.dispose();
    settleFailedStart(id);
    toast(aiErrorMessage(e), "error");
  }
}

/**
 * A command that started a turn failed: the slot that shows the conversation now (it may have moved
 * meanwhile) has no turn, unless another one started there, and a message queued meanwhile goes back into
 * its input area.
 */
function settleFailedStart(convId: string, patch: Partial<Slot> = {}): string | undefined {
  const target = slotOf(convId);
  if (target && !runners.has(target)) {
    patchSlot(target, { ...patch, turn: null });
    giveBackQueued(target);
  }
  return target;
}

/**
 * AI-26: replaces an earlier message of the user with `raw`, deletes every entry after it, and starts
 * a turn. First offers to revert the file changes of the calls it deletes that this device keeps
 * (AI-41). Not while a turn runs.
 */
export async function editAndResend(slotId: string, entryId: string, raw: string): Promise<boolean> {
  const text = raw.trim();
  const slot = getSlot(slotId);
  const id = slot.conversationId;
  if (!text || !id || slot.turn || slot.remoteRunning || slot.compacting || reverting(slotId)) return false;
  const later = laterMessages(slot.entries, entryId);
  // Read again: the refresh after the turn's last write may not have arrived yet. Without them the edit
  // cannot tell which changes it would leave on the host, so it is not sent.
  let fileChanges: AiFileChange[];
  try {
    fileChanges = await api.ai_file_changes(id);
  } catch (e) {
    toast(aiErrorMessage(e), "error");
    return false;
  }
  if (!unlocked() || getSlot(slotId).conversationId !== id) return false;
  patchSlot(slotId, { fileChanges });
  const { revert } = editedFiles(slot.entries, fileChanges, entryId);
  if (revert.length > 0) {
    // Only the changes on the server the tab is connected to can be reverted from it; the dialog says so of the
    // others, which Cancel keeps revertible.
    const server = await api.ai_session_server(revertSession(slotId)).catch(() => null);
    // The list is the confirmation, also for a file a call created: each revert asks nothing more.
    // Each revert waits for an idle conversation, as on the block: a turn may have started meanwhile.
    const send = await askEditedFiles({
      later,
      changes: revert,
      server,
      revert: (change) =>
        slotBusy(slotId)
          ? Promise.resolve<AiFileRevert>({ status: "failed", reason: "failed", detail: t("ai.revert.busy") })
          : revertChange(slotId, id, change),
      diff: (change) => fileChangeDiff(slotId, change).catch(() => null),
    });
    if (!send) return false;
  }
  const now = getSlot(slotId);
  if (now.conversationId !== id || now.turn || now.remoteRunning || now.compacting) return false;
  const context = turnContext(slotId);
  if (!context) {
    toast(t("ai.err.noModel"), "error");
    return false;
  }
  discardSideQuestion(slotId);

  const runner = new TurnRunner(slotId, id, context.permission_mode, afterTurn);
  runners.set(slotId, runner);
  // Rust keeps the notes of the message it replaces (AI-05, AI-09), so its dividers stay while it is sent;
  // the screen and the permission mode it works out again (AI-46, §13.5).
  const original = now.entries.find((e) => e.entry_id === entryId);
  const kept =
    original?.role === "user"
      ? parseMessage(original.text).notes.filter((n) => n.kind !== "terminal_state" && n.kind !== "permission_mode")
      : [];
  const local: AiEntryView = {
    role: "user",
    entry_id: LOCAL_ENTRY,
    created_at: Date.now(),
    text: composeMessage(text, [], kept),
  };
  patchSlot(slotId, (s) => ({
    entries: [...entriesBefore(s.entries, entryId), local],
    turn: newTurn(),
    outcome: null,
    effortIgnored: false,
  }));
  try {
    const started = await api.ai_edit_resend(id, entryId, text, context, runner.onEvent);
    if (useAi.getState().history) void loadHistory();
    if (runner.gone) return true;
    patchSlot(runner.slotId, (s) => ({
      conversation: withContext(s, started.conversation),
      entries: placeUserEntry(s.entries, started.user_entry),
    }));
    return true;
  } catch (e) {
    runner.dispose();
    const target = settleFailedStart(id, { entries: now.entries });
    if (target) {
      // Stopped before it was stored: while the MCP servers started (AI-45), which leaves the message
      // and what followed it, or during its compaction (AI-22), which has deleted them. Either way the
      // edited text goes back into the input instead of being lost.
      if (toAppError(e).code === "cancelled") restoreDraft(target, text);
      void reloadSlot(target);
    }
    if (toAppError(e).code !== "cancelled") toast(aiErrorMessage(e), "error");
    return false;
  }
}

/**
 * Puts a message that was not sent back into the slot's input, before what it holds: its typed text, and its attachments
 * but the tab's selection and diagnostics (pastes, files, and what the `/` and `@` menus attached).
 */
function restoreDraft(slotId: string, text: string) {
  const { attachments, typed } = parseMessage(text);
  // A skill or prompt keeps the command that attached it, so that running the command again replaces it (AI-50): a
  // prompt's as the running servers name it now, whatever else the text holds (a joined message may start with another).
  const { servers, status } = useMcp.getState();
  const prompts = menuSources(servers ?? [], status, getSlot(slotId).mcpOff).prompts;
  const extras: PendingAttachment[] = attachments.flatMap((a): PendingAttachment[] => {
    if (a.kind === "selection" || a.kind === "diagnostics") return [];
    const by =
      a.kind === "skill"
        ? a.name
        : a.kind === "mcp_prompt"
          ? prompts.find((p) => p.server === a.server && p.prompt.name === a.name)?.command
          : undefined;
    return [{ id: extraId(), attachment: a, ...(by ? { command: by } : {}) }];
  });
  patchSlot(slotId, (s) => messageBack(s, { typed, chips: [], extras, restore: () => [] }));
}

/** Stop, or Esc in the panel (AI-18); the queued message goes once the turn has stopped (§13.1). */
export async function stopTurn(slotId: string) {
  const runner = runners.get(slotId);
  if (runner) return runner.stop();
  const slot = getSlot(slotId);
  const id = slot.conversationId;
  if (slot.remoteRunning && id) {
    remoteStops.add(id);
    try {
      await api.ai_stop(id).catch(() => {});
      // The conversation may have gone into the background meanwhile (AI-07), with its queued message.
      const target = slotOf(id);
      if (target) {
        patchSlot(target, { remoteRunning: false });
        await reloadSlot(target);
      }
    } finally {
      remoteStops.delete(id);
    }
    const target = slotOf(id);
    if (target) await sendQueued(target);
  }
}

/**
 * AI-21 Compact: the summary becomes the start of the context. `instructions`: what `/compact <instructions>` asks
 * of the summary (AI-50). Resolves to whether a summary was stored.
 */
export async function compactConversation(slotId: string, instructions: string | null = null): Promise<boolean> {
  const slot = getSlot(slotId);
  const id = slot.conversationId;
  if (!id || slot.turn || slot.compacting || reverting(slotId)) return false;
  const context = turnContext(slotId);
  if (!context) {
    toast(t("ai.err.noModel"), "error");
    return false;
  }
  patchSlot(slotId, { compacting: true });
  noteChange(slotId);
  try {
    const entry = await api.ai_compact(id, context, instructions);
    // The slot that shows the conversation now: it may have moved meanwhile (Connect to host).
    const target = slotOf(id);
    if (!target) return true;
    patchSlot(target, (s) => ({
      compacting: false,
      entries: upsertEntry(s.entries, entry),
      conversation: s.conversation ? { ...s.conversation, context_start: entry.entry_id } : s.conversation,
    }));
    void reloadSlot(target);
    return true;
  } catch (e) {
    const target = slotOf(id);
    if (target) patchSlot(target, { compacting: false });
    toast(aiErrorMessage(e), "error");
    return false;
  }
}
