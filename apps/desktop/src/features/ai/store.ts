import { create } from "zustand";
import { useApp } from "@/app/store";
import type { AiConversationView, AiEntryView, AiModelRef, AiPermissionMode, AiProviderView, AiSettingsView } from "@/ipc/types";
import type { SelectionState } from "./selection";
import type { TurnOutcome, TurnState } from "./turn";

/** The slot of the home tab: a conversation with no terminal, which gets every tool but the terminal's (AI-09). */
export const HOME_SLOT = "home";

/**
 * The conversation a tab shows in the panel (AI-07). Slots are keyed by tab id ("home" for the home
 * tab) and a conversation lives in at most one slot.
 */
export interface Slot {
  /** null: a new conversation, stored when its first message is sent. */
  conversationId: string | null;
  conversation: AiConversationView | null;
  entries: AiEntryView[];
  loading: boolean;
  loadFailed: boolean;
  compacting: boolean;
  /** The model the user picked in the selector; null follows the conversation (AI-05). */
  model: AiModelRef | null;
  /** AI-16: this conversation's permission mode on this device. */
  mode: AiPermissionMode;
  turn: TurnState | null;
  outcome: TurnOutcome | null;
  /** Rust runs a turn of this conversation that an earlier page started, so its events do not reach us. */
  remoteRunning: boolean;
  /** Unsent text in the input box. */
  draft: string;
  /** AI-30: MCP servers switched off for this conversation in the tools menu. */
  mcpOff: string[];
  /** AI-24: an entry to scroll to and highlight once it is shown (a history search hit). */
  reveal: string | null;
}

/** A tab's selection; `hidden` is the `seq` whose chip was removed or sent (it shows again once the selection changes). */
export interface TabSelection extends SelectionState {
  hidden: number | null;
}

export const NO_SELECTION: TabSelection = { text: "", seq: 0, hidden: null };

export interface Catalog {
  providers: AiProviderView[];
  settings: AiSettingsView | null;
  loaded: boolean;
}

interface AiState {
  slots: Record<string, Slot>;
  catalog: Catalog;
  /** AI-16: the mode picked in the panel per conversation, kept until the app quits. */
  modes: Record<string, AiPermissionMode>;
  /** AI-30: per conversation, the MCP servers switched off in the tools menu, kept until the app quits. */
  mcpOff: Record<string, string[]>;
  /** AI-19: per conversation, the tools chosen with Allow for this conversation, kept until the app quits. */
  allowed: Record<string, string[]>;
  /** AI-10: per terminal tab, its selection as the panel last saw it and the chip it shows. */
  selections: Record<string, TabSelection>;
  /** The history list, once the panel has asked for it (AI-23). */
  history: AiConversationView[] | null;
  historyFailed: boolean;
  /** Bumped to move focus to the input box. */
  focusTick: number;
}

export const useAi = create<AiState>(() => ({
  slots: {},
  catalog: { providers: [], settings: null, loaded: false },
  modes: {},
  mcpOff: {},
  allowed: {},
  selections: {},
  history: null,
  historyFailed: false,
  focusTick: 0,
}));

/** New conversations start in the device's default mode (AI-16). */
export function defaultMode(): AiPermissionMode {
  return useApp.getState().prefs.ai_permission_mode;
}

export function blankSlot(mode: AiPermissionMode = defaultMode(), draft = ""): Slot {
  return {
    conversationId: null,
    conversation: null,
    entries: [],
    loading: false,
    loadFailed: false,
    compacting: false,
    model: null,
    mode,
    turn: null,
    outcome: null,
    remoteRunning: false,
    draft,
    mcpOff: [],
    reveal: null,
  };
}

export function getSlot(id: string): Slot {
  return useAi.getState().slots[id] ?? blankSlot();
}

export function patchSlot(id: string, patch: Partial<Slot> | ((slot: Slot) => Partial<Slot>)) {
  useAi.setState((st) => {
    const cur = st.slots[id] ?? blankSlot();
    const p = typeof patch === "function" ? patch(cur) : patch;
    return { slots: { ...st.slots, [id]: { ...cur, ...p } } };
  });
}

export function setSlot(id: string, slot: Slot | null) {
  useAi.setState((st) => {
    const slots = { ...st.slots };
    if (slot) slots[id] = slot;
    else delete slots[id];
    return { slots };
  });
}

/** The slot that shows a conversation. */
export function slotOf(conversationId: string): string | undefined {
  return Object.entries(useAi.getState().slots).find(([, s]) => s.conversationId === conversationId)?.[0];
}

export type Activity = "idle" | "running" | "waiting";

/** What the tab bar shows for a slot (AI-07): a turn running, or one waiting for the user. */
export function slotActivity(slot: Slot | undefined): Activity {
  if (!slot) return "idle";
  const call = slot.turn?.call;
  if (call && (call.state === "approval" || call.state === "limit")) return "waiting";
  return slot.turn || slot.remoteRunning ? "running" : "idle";
}

/** Keeps a conversation's history entry in step with what a command returned. */
export function updateConversation(view: AiConversationView) {
  useAi.setState((st) => ({
    history: st.history?.some((c) => c.id === view.id) ? st.history.map((c) => (c.id === view.id ? view : c)) : st.history ? [view, ...st.history] : null,
    slots: Object.fromEntries(Object.entries(st.slots).map(([k, s]) => [k, s.conversationId === view.id ? { ...s, conversation: view } : s])),
  }));
}
