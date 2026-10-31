/**
 * Conversations in slots: each tab's own, and those left in its background (AI-07); opened, moved between slots,
 * detached with their tab, and read again from Rust.
 */

import { useTabs, type TabSource } from "@/app/tabs";
import {
  connectHost,
  connectTarget,
  connectTelnetHost,
  connectTelnetTarget,
  openLocalTerminal,
} from "@/session/connect";
import { api, toAppError } from "@/ipc/api";
import type { AiEffort, AiModelRef, AiPermissionMode, NotificationTarget } from "@/ipc/types";
import { resetAllowances } from "../rules/allowances";
import {
  blankSlot,
  defaultMode,
  getSlot,
  HOME_SLOT,
  inBackground,
  offersHomeConversation,
  patchSlot,
  setSlot,
  slotOf,
  tabOf,
  useAi,
  type Slot,
} from "../store";
import { pendingAttachments } from "./attach";
import { focusInput, toggleAiPanel } from "./panel";
import { giveBackQueued } from "./queued";
import { lastChange, noteChange, rooms, runners } from "./runner";
import { stopSideQuestion } from "./sideQuestions";

let backgroundSeq = 0;

/**
 * The slot's conversation leaves it, for another one (AI-07): its side question stops (AI-42). A conversation
 * with work under way, a turn or a compaction, goes on in the background of the slot's tab, with its queued
 * message (§13.1), which is sent when the turn ends; an idle one's queued message goes back into the input area.
 */
function release(slotId: string) {
  noteChange(slotId);
  stopSideQuestion(slotId);
  if (slotWorking(slotId)) toBackground(slotId);
  else giveBackQueued(slotId);
}

/**
 * AI-07: moves the slot's conversation, with its runner, to a background slot of the slot's tab, where its turn
 * goes on: its tools still act on that tab (AI-08), and its approval card waits there until the conversation is
 * shown again. The input area stays with the tab, and the side question, stopped, is discarded (AI-42).
 */
function toBackground(slotId: string) {
  const id = `bg-${++backgroundSeq}`;
  const source = getSlot(slotId);
  useAi.setState((st) => ({
    slots: { ...st.slots, [id]: { ...source, draft: "", extras: [], side: null, reveal: null } },
    background: { ...st.background, [id]: tabOf(slotId) },
  }));
  const runner = runners.get(slotId);
  if (runner) {
    runners.delete(slotId);
    runner.slotId = id;
    runners.set(id, runner);
  }
}

/** Two input areas' unsent text and attachments as one, `first`'s before `then`'s. */
function joinInput(
  first: Pick<Slot, "draft" | "extras">,
  then: Pick<Slot, "draft" | "extras">,
): Pick<Slot, "draft" | "extras"> {
  const draft = !first.draft.trim() ? then.draft : then.draft.trim() ? `${first.draft}\n\n${then.draft}` : first.draft;
  return { draft, extras: [...first.extras, ...then.extras] };
}

/**
 * AI-07: a background slot whose conversation is gone (deleted), or was never stored, goes; what its input area
 * holds, a message that went back into it (§13.1), goes back into its tab's, before what that holds.
 */
export function clearBackground(id: string) {
  const left = getSlot(id);
  const tab = tabOf(id);
  dropBackground(id);
  patchSlot(tab, (s) => joinInput(left, s));
}

/** A background slot goes: its conversation is shown again, deleted, or its tab closed (AI-07). */
function dropBackground(id: string) {
  lastChange.delete(id);
  useAi.setState((st) => {
    const slots = { ...st.slots };
    const background = { ...st.background };
    delete slots[id];
    delete background[id];
    return { slots, background };
  });
}

/**
 * The conversation stops being held where it is (its tab closes, or it is deleted): its turn, also one an earlier
 * page started, and its side question (AI-42) stop, and its queued message goes back into the input area (§13.1).
 */
export function stopSlot(slotId: string) {
  const runner = runners.get(slotId);
  const { remoteRunning, conversationId } = getSlot(slotId);
  if (runner) runner.abandon();
  else if (remoteRunning && conversationId) void api.ai_stop(conversationId).catch(() => {});
  stopSideQuestion(slotId);
  giveBackQueued(slotId);
}

/** The slot's conversation is gone (deleted): a tab's slot gets a new conversation, keeping its input area, and a background slot goes (`clearBackground`). */
export function clearSlot(slotId: string) {
  if (inBackground(slotId)) clearBackground(slotId);
  else setSlot(slotId, blankSlot(defaultMode(), getSlot(slotId).draft, getSlot(slotId).extras));
}

/** New conversation (AI-07): stored when its first message is sent. A busy one it replaces goes on in the background (`release`). */
export function newConversation(slotId: string) {
  release(slotId);
  setSlot(slotId, blankSlot(defaultMode(), getSlot(slotId).draft, getSlot(slotId).extras));
  focusInput();
}

/** The conversations whose turn an earlier page started the user is stopping: its end sends the queued message (§13.1). */
export const remoteStops = new Set<string>();

/** Reads the slot's conversation again from Rust (after unlock, a sync, or a stopped turn). */
export async function reloadSlot(slotId: string) {
  const id = getSlot(slotId).conversationId;
  if (!id) return;
  try {
    const detail = await api.ai_conversation_get(id);
    if (getSlot(slotId).conversationId !== id) return;
    // §13.1: a turn an earlier page started ended on its own, and nothing tells how, so its queued message goes
    // back. A poll that sees the end of one the user is stopping leaves it to the stop, which sends it.
    const remoteEnded = getSlot(slotId).remoteRunning && !detail.running && !remoteStops.has(id);
    patchSlot(slotId, (s) => ({
      conversation: detail.conversation,
      entries: s.turn ? s.entries : detail.entries,
      fileChanges: detail.file_changes,
      loading: false,
      loadFailed: false,
      remoteRunning: detail.running && !runners.has(slotId),
    }));
    if (remoteEnded) giveBackQueued(slotId);
  } catch (e) {
    if (getSlot(slotId).conversationId !== id) return;
    if (toAppError(e).code === "not_found") {
      giveBackQueued(slotId);
      clearSlot(slotId); // deleted on another device
      return;
    }
    patchSlot(slotId, { loading: false, loadFailed: true });
  }
}

/** Whether a slot's conversation has work under way: a turn running (here or in Rust) or waiting for approval, or a compaction. */
export function slotWorking(slotId: string): boolean {
  const s = getSlot(slotId);
  return !!s.turn || s.remoteRunning || s.compacting || runners.has(slotId);
}

/** Whether a slot's conversation is busy: working (`slotWorking`), or reverting a file change (AI-41). */
export function slotBusy(slotId: string): boolean {
  return slotWorking(slotId) || getSlot(slotId).reverting;
}

/** Whether a conversation is busy in the slot that holds it (`slotBusy`), in a tab or in the background; one no slot holds is not. */
export function conversationBusy(id: string): boolean {
  const slotId = slotOf(id);
  return !!slotId && slotBusy(slotId);
}

/**
 * Moves a slot's conversation to another slot; a tab's slot it leaves gets a new conversation, and a background
 * slot goes. A running turn moves with it (Connect to host moves the home tab's), but it never uses the terminal
 * of a tab it moved to: its runner stays pinned to the tab it started on (AI-08).
 */
function moveSlot(from: string, to: string) {
  noteChange(from);
  noteChange(to);
  const source = getSlot(from);
  const runner = runners.get(from);
  const target = getSlot(to);
  if (inBackground(from)) {
    // Back from the background (AI-07): the tab's input area keeps what it holds, after what went back into the
    // conversation's own (a queued message that was not sent, §13.1).
    setSlot(to, { ...source, ...joinInput(source, target) });
    dropBackground(from);
  } else {
    // The input area of the slot the conversation moves to wins when it holds something, a tab's selection or
    // diagnostics chip included (AI-10); the source's then stays.
    const keep = !!target.draft || pendingAttachments(to).length > 0;
    setSlot(to, { ...source, draft: keep ? target.draft : source.draft, extras: keep ? target.extras : source.extras });
    setSlot(from, keep ? blankSlot(defaultMode(), source.draft, source.extras) : blankSlot());
  }
  if (runner) {
    runners.delete(from);
    runner.slotId = to;
    runners.set(to, runner);
  }
}

/**
 * Opens a conversation from history in a slot (AI-09); a conversation shown elsewhere, or left in the
 * background (AI-07), moves here. One that is busy in another tab (a turn running or waiting for approval, or
 * a compaction) stays there, since its turn acts on that tab's terminal (AI-08): its tab comes to the front
 * instead, and shows it when it went on in that tab's background. The slot's own conversation leaves it
 * (`release`), so a busy one goes on in the background. `reveal`: an entry to scroll to and highlight, from a
 * history search hit (AI-24).
 */
export async function openConversation(slotId: string, id: string, reveal: string | null = null) {
  const current = getSlot(slotId);
  if (current.conversationId === id) {
    if (reveal) patchSlot(slotId, { reveal });
    return;
  }
  const from = slotOf(id);
  const owner = from ? tabOf(from) : null;
  if (from && owner && owner !== slotId && slotBusy(from)) {
    useTabs.getState().activate(owner);
    if (from !== owner) return openConversation(owner, id, reveal);
    if (reveal) patchSlot(from, { reveal });
    return;
  }
  release(slotId);
  if (from && from !== slotId) {
    moveSlot(from, slotId);
    if (reveal) patchSlot(slotId, { reveal });
    return;
  }
  const st = useAi.getState();
  const known = st.history?.find((c) => c.id === id) ?? null;
  // Read after `release`: a side question it stopped gave back what it took from the input area.
  const { draft, extras } = getSlot(slotId);
  setSlot(slotId, {
    ...blankSlot(st.modes[id] ?? defaultMode(), draft, extras),
    conversationId: id,
    conversation: known,
    loading: true,
    mcpOff: st.mcpOff[id] ?? [],
    screen: st.screens[id] ?? null,
    reveal,
  });
  await reloadSlot(slotId);
}

/**
 * AI-07: a click on a notification brought the window back. Its conversation shows in the tab that holds it, with
 * the panel open, also one that went on in the tab's background; with the conversation no longer held there (its
 * tab closed), its tab comes to the front if it is still open.
 */
export function openNotified({ tab, conversation_id }: NotificationTarget) {
  const tabs = useTabs.getState();
  const from = conversation_id ? slotOf(conversation_id) : undefined;
  if (from && conversation_id) {
    const owner = tabOf(from);
    tabs.activate(owner);
    if (from !== owner) void openConversation(owner, conversation_id);
    toggleAiPanel(true);
  } else if (tab && (tab === HOME_SLOT || tabs.tabs.some((x) => x.id === tab))) {
    tabs.activate(tab);
  }
}

/**
 * AI-09 "Connect to host": opens a tab on the host or quick-connect target (HOST-12), or a local terminal
 * (§7.5), and attaches the home tab's conversation to it.
 */
export async function connectConversation(source: TabSource | "local") {
  const before = new Set(useTabs.getState().tabs.map((x) => x.id));
  if (source !== "local" && source.telnet) {
    if (source.telnet.hostId !== null) connectTelnetHost(source.telnet.hostId);
    else connectTelnetTarget(source.telnet.address, source.telnet.port);
  } else
    await (source === "local" || source.local
      ? openLocalTerminal()
      : source.hostId !== null
        ? connectHost(source.hostId)
        : connectTarget(source.target));
  const tab = useTabs.getState().tabs.find((x) => !before.has(x.id));
  if (tab) moveSlot(HOME_SLOT, tab.id);
}

/**
 * AI-09 "Continue Here": attaches the home tab's conversation to a terminal tab that shows a new one with
 * no messages, never on its own. Not while the home conversation is busy, since its turn finishes there.
 */
export function continueHomeConversation(tabId: string): boolean {
  if (tabId === HOME_SLOT || slotBusy(HOME_SLOT) || slotBusy(tabId)) return false;
  if (!offersHomeConversation(getSlot(tabId), useAi.getState().slots[HOME_SLOT])) return false;
  moveSlot(HOME_SLOT, tabId);
  focusInput();
  return true;
}

/**
 * Closing a tab detaches its conversation, which stays in history (AI-08). Its turn stops, and so do those of the
 * conversations it left in the background (AI-07), whose queued messages go with the tab's input area (§13.1).
 */
export function detachSlot(slotId: string) {
  stopSlot(slotId);
  setSlot(slotId, null);
  lastChange.delete(slotId);
  for (const [id, tab] of Object.entries(useAi.getState().background)) {
    if (tab !== slotId) continue;
    stopSlot(id);
    dropBackground(id);
  }
  // Its stopped turns keep their own room until their calls settle.
  rooms.delete(slotId);
  useAi.setState((st) => {
    if (!st.selections[slotId] && !st.diagnostics[slotId]) return {};
    const selections = { ...st.selections };
    const diagnostics = { ...st.diagnostics };
    delete selections[slotId];
    delete diagnostics[slotId];
    return { selections, diagnostics };
  });
}

/** Locking stops every turn in Rust (§13.1); drop in-flight state and conversation text until unlock. */
export function resetForLock() {
  for (const runner of [...runners.values()]) runner.dispose();
  runners.clear();
  resetAllowances();
  // §13.1: queued messages go back into the input areas, unsent.
  for (const id of Object.keys(useAi.getState().slots)) giveBackQueued(id);
  useAi.setState((st) => ({
    history: null,
    slots: Object.fromEntries(
      Object.entries(st.slots).map(([id, s]) => [
        id,
        // File changes name servers and paths: conversation plaintext too.
        {
          ...s,
          conversation: null,
          entries: [],
          fileChanges: [],
          reverting: false,
          turn: null,
          outcome: null,
          effortIgnored: false,
          remoteRunning: false,
          compacting: false,
          side: null,
          loading: !!s.conversationId,
        },
      ]),
    ),
  }));
}

/** After unlock: read every open conversation again. */
export function reloadAll() {
  for (const id of Object.keys(useAi.getState().slots)) void reloadSlot(id);
}

export function setSlotMode(slotId: string, mode: AiPermissionMode) {
  const id = getSlot(slotId).conversationId;
  patchSlot(slotId, { mode });
  if (id) useAi.setState((st) => ({ modes: { ...st.modes, [id]: mode } }));
}

export function setSlotModel(slotId: string, model: AiModelRef) {
  patchSlot(slotId, { model, effortIgnored: false });
}

/** AI-05: the thinking level of the slot's next messages; the conversation keeps it once one is sent. */
export function setSlotEffort(slotId: string, effort: AiEffort | "default") {
  patchSlot(slotId, { effort, effortIgnored: false });
}

/** AI-30: switches an MCP server off (or back on) for the slot's conversation until the app quits. */
export function setMcpServerOff(slotId: string, serverId: string, off: boolean) {
  const slot = getSlot(slotId);
  const mcpOff = off
    ? [...slot.mcpOff.filter((x) => x !== serverId), serverId]
    : slot.mcpOff.filter((x) => x !== serverId);
  patchSlot(slotId, { mcpOff });
  const id = slot.conversationId;
  if (id) useAi.setState((st) => ({ mcpOff: { ...st.mcpOff, [id]: mcpOff } }));
}

/** AI-46: whether the slot's conversation shows the terminal's screen with each message, until the app quits. */
export function setSlotScreen(slotId: string, on: boolean) {
  const id = getSlot(slotId).conversationId;
  patchSlot(slotId, { screen: on });
  if (id) useAi.setState((st) => ({ screens: { ...st.screens, [id]: on } }));
}
