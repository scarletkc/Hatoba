import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { useTabs, type SessionTab } from "@/app/tabs";
import { confirm, toast } from "@/components/overlay";
import { connectHost } from "@/features/terminal/connect";
import { getSession, type LiveSession } from "@/features/terminal/session";
import { getLocale, t, type MessageKey, type Params } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type {
  AiConversationView,
  AiEntryView,
  AiModelRef,
  AiPermissionMode,
  AiToolCall,
  AiToolResultInput,
  AiTurnContext,
  AiTurnEndReason,
  AiTurnEvent,
  McpToolInfo,
} from "@/ipc/types";
import { pickSavePath } from "@/lib/native";
import { conversationMarkdown, exportFileName } from "./exportMarkdown";
import { conversationModel } from "./models";
import { chipShown, composeMessage, makeAttachment, makeDiagnostics, nextSelection, type SelectionAttachment } from "./selection";
import { blankSlot, defaultMode, getSlot, HOME_SLOT, NO_SELECTION, patchSlot, setSlot, slotOf, updateConversation, useAi, type Slot, type TabSelection } from "./store";
import { DISCONNECTED, NO_TAB, readTerminal, sendInput } from "./terminalTools";
import { mustAsk, needsSession, toolKind, type ToolKind } from "./tools";
import {
  entriesBefore,
  laterContextStart,
  laterMessages,
  newTurn,
  reduceTurnEvent,
  unansweredCalls,
  upsertEntry,
  type CallInfo,
  type CallState,
} from "./turn";

/** The user's message, shown until `ai_send` returns the stored entry. */
const LOCAL_ENTRY = "~local";

/** User-facing text for a failed AI command (`ai` errors carry the provider's status and message). */
export function aiErrorMessage(e: unknown, tr: (key: MessageKey, params?: Params) => string = t): string {
  return errorMessage(tr, e);
}

/** The terminal tab a slot is attached to; null for the home tab or a tab that is gone. */
export function attachedTab(slotId: string): { tab: SessionTab; session: LiveSession | undefined } | null {
  if (slotId === HOME_SLOT) return null;
  const tab = useTabs.getState().tabs.find((x) => x.id === slotId);
  return tab ? { tab, session: getSession(tab.id) } : null;
}

/** The model the next request of a slot uses (AI-05). */
export function slotModel(slotId: string): AiModelRef | null {
  const slot = getSlot(slotId);
  const { providers, settings } = useAi.getState().catalog;
  return conversationModel(slot.entries, providers, settings, slot.model);
}

/** What a request may act on: the tab's host and, when connected, its terminal tools (AI-08, AI-09). */
export function turnContext(slotId: string): AiTurnContext | null {
  const model = slotModel(slotId);
  if (!model) return null;
  const target = attachedTab(slotId);
  return {
    provider_id: model.provider_id,
    model_id: model.model_id,
    host_id: target ? target.tab.hostId : (getSlot(slotId).conversation?.host_id ?? null),
    tab: !!target && target.tab.status === "connected",
    disabled_mcp_servers: getSlot(slotId).mcpOff,
  };
}

/** Applies a streamed event to a slot; a stored summary moves the conversation's `context_start` (AI-21, AI-22). */
function applyEvent(slotId: string, ev: AiTurnEvent) {
  patchSlot(slotId, (s) => {
    const before = s.conversation?.context_start ?? null;
    const next = reduceTurnEvent({ entries: s.entries, turn: s.turn, outcome: s.outcome, contextStart: before }, ev);
    return {
      entries: next.entries,
      turn: next.turn,
      outcome: next.outcome,
      conversation: s.conversation && next.contextStart !== before ? { ...s.conversation, context_start: next.contextStart } : s.conversation,
    };
  });
}

/** The conversation view a command returned, without moving `context_start` back past a summary the turn already stored. */
function withContext(s: Slot, view: AiConversationView): AiConversationView {
  return { ...view, context_start: laterContextStart(s.entries, s.conversation?.context_start ?? null, view.context_start) };
}

// ───────────────────────── the turn runner ─────────────────────────

/** `allow`: AI-19 Allow for this conversation, or AI-31 Always allow (MCP tools, on this device). */
type Decision =
  | { kind: "run"; edited: string | null; allow?: "conversation" | "always" }
  | { kind: "reject"; reason: string }
  | { kind: "continue" }
  | { kind: "stop" };

/** AI-19: later calls of the tool in this conversation run without asking until the app quits. */
function allowInConversation(convId: string, tool: string) {
  useAi.setState((st) => {
    const list = st.allowed[convId] ?? [];
    return list.includes(tool) ? {} : { allowed: { ...st.allowed, [convId]: [...list, tool] } };
  });
}

function allowedInConversation(convId: string, tool: string): boolean {
  return useAi.getState().allowed[convId]?.includes(tool) ?? false;
}

const runners = new Map<string, TurnRunner>();

/**
 * Drives one turn (§13.1) for the slot it belongs to: applies the streamed events, then handles the
 * tool calls of each response one at a time, in order, applying the permission mode (§13.5) and the
 * tool call limit (AI-18). `read_terminal` and `send_input` run here; other tools run in Rust.
 */
class TurnRunner {
  /** Moves with the conversation when it changes slots. */
  slotId: string;
  convId: string | null;
  private ended = false;
  private disposed = false;
  private stopping = false;
  private processing = false;
  private waiter: ((d: Decision) => void) | null = null;
  private readonly abort = new AbortController();
  private readonly ready: Promise<string | null>;
  private resolveReady!: (id: string | null) => void;

  constructor(slotId: string, convId: string | null) {
    this.slotId = slotId;
    this.convId = convId;
    this.ready = new Promise((resolve) => (this.resolveReady = resolve));
    if (convId) this.resolveReady(convId);
  }

  get active(): boolean {
    return !this.disposed && !this.ended;
  }

  get gone(): boolean {
    return this.disposed;
  }

  /** `ai_send` stored the conversation. A stop asked for before that is sent now. */
  started(convId: string) {
    this.convId = convId;
    this.resolveReady(convId);
    if (this.stopping) void this.requestStop(convId);
  }

  onEvent = (ev: AiTurnEvent) => {
    if (this.disposed) return;
    // After the end (e.g. the fallback below), only stored entries still matter.
    if (this.ended) {
      if (ev.kind === "entry") this.addEntry(ev.entry);
      return;
    }
    applyEvent(this.slotId, ev);
    if (ev.kind === "done" && ev.finish === "tool_calls") void this.handleTools();
    else if (ev.kind === "turn_ended") this.finish(ev.reason);
  };

  /** Stops the turn (AI-18): Rust aborts the request and running tools and cancels calls without a result. */
  async stop() {
    if (!this.active || this.stopping) return;
    this.stopping = true;
    this.release();
    if (this.convId) await this.requestStop(this.convId);
  }

  private async requestStop(convId: string) {
    await api.ai_stop(convId).catch(() => {});
    // Rust ends the turn on its channel; if that never arrives, end it here.
    window.setTimeout(() => {
      if (this.active) this.onEvent({ kind: "turn_ended", reason: "stopped" });
    }, 3000);
  }

  /** The slot no longer shows this conversation: stop the turn in Rust and forget it here. */
  abandon() {
    this.stopping = true;
    const id = this.convId;
    this.dispose();
    if (id) void api.ai_stop(id).catch(() => {});
  }

  /** Forget the turn without telling Rust (Rust already stopped it, e.g. for a new message or the lock). */
  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.release();
    this.resolveReady(null);
    if (runners.get(this.slotId) === this) runners.delete(this.slotId);
  }

  decide(d: Decision) {
    const w = this.waiter;
    this.waiter = null;
    w?.(d);
  }

  private release() {
    this.decide({ kind: "stop" });
    this.abort.abort();
  }

  private finish(reason: AiTurnEndReason) {
    this.ended = true;
    this.release();
    if (runners.get(this.slotId) === this) runners.delete(this.slotId);
    void afterTurn(this.slotId, reason);
  }

  private setCall(call: CallInfo | null) {
    if (this.disposed) return;
    patchSlot(this.slotId, (s) => (s.turn ? { turn: { ...s.turn, call } } : {}));
  }

  private wait(callId: string, state: CallState, mcp?: McpToolInfo | null): Promise<Decision> {
    if (!this.active) return Promise.resolve({ kind: "stop" });
    this.setCall({ id: callId, state, mcp });
    return new Promise((resolve) => (this.waiter = resolve));
  }

  private countCall() {
    patchSlot(this.slotId, (s) => (s.turn ? { turn: { ...s.turn, toolCount: s.turn.toolCount + 1 } } : {}));
  }

  private addEntry(entry: AiEntryView) {
    if (!this.disposed) applyEvent(this.slotId, { kind: "entry", entry });
  }

  private async handleTools() {
    if (this.processing) return;
    this.processing = true;
    try {
      const convId = await this.ready;
      if (!convId) return;
      for (const call of unansweredCalls(getSlot(this.slotId).entries)) {
        if (!this.active || this.stopping) return;
        await this.handleCall(convId, call);
      }
    } finally {
      this.processing = false;
      if (this.active) this.setCall(null);
    }
  }

  private async handleCall(convId: string, call: AiToolCall) {
    // AI-18: pause after the limit until the user chooses Continue.
    const limit = Math.max(1, useApp.getState().prefs.ai_tool_call_limit || 25);
    if ((getSlot(this.slotId).turn?.toolCount ?? 0) >= limit) {
      const d = await this.wait(call.id, "limit");
      if (d.kind !== "continue" || !this.active) return;
      patchSlot(this.slotId, (s) => (s.turn ? { turn: { ...s.turn, toolCount: 0 } } : {}));
    }

    const kind = toolKind(call.name);
    // AI-09: with no terminal tab, a terminal tool cannot run, so it gets its error without asking first.
    if (needsSession(kind) && !attachedTab(this.slotId)) {
      this.setCall({ id: call.id, state: "running" });
      await this.report(convId, call.id, { status: "error", content: NO_TAB, edited_arguments: null });
      this.countCall();
      return;
    }
    const allowedHere = allowedInConversation(convId, call.name);
    // AI-31: whether an MCP call asks depends on its tool's Always allow and its server's Always ask.
    let mcp: McpToolInfo | null = null;
    if (kind === "mcp" && !allowedHere) {
      mcp = await api.mcp_tool_info(call.name).catch(() => null);
      if (!this.active) return;
    }
    let edited: string | null = null;
    if (mustAsk({ kind, mode: getSlot(this.slotId).mode, allowedHere, mcp })) {
      const d = await this.wait(call.id, "approval", mcp);
      if (!this.active || d.kind === "stop" || d.kind === "continue") return;
      if (d.kind === "reject") {
        this.setCall({ id: call.id, state: "running" });
        await this.report(convId, call.id, { status: "rejected", content: d.reason, edited_arguments: null });
        this.countCall();
        return;
      }
      edited = d.edited;
      if (d.allow === "conversation") allowInConversation(convId, call.name);
      else if (d.allow === "always" && mcp) await this.alwaysAllow(mcp);
    }
    if (!this.active) return;
    this.setCall({ id: call.id, state: "running" });
    await this.execute(convId, call, kind, edited);
    this.countCall();
  }

  /** AI-31 Always allow on the card: stored on this device, then the call runs. */
  private async alwaysAllow(mcp: McpToolInfo) {
    try {
      await api.mcp_set_always_allow(mcp.server_id, mcp.tool.tool, true);
    } catch (e) {
      // The user approved this call, so it runs; only the setting failed.
      toast(aiErrorMessage(e), "error");
    }
  }

  private async execute(convId: string, call: AiToolCall, kind: ToolKind, edited: string | null) {
    const args = edited ?? call.arguments;
    const error = (content: string): AiToolResultInput => ({ status: "error", content, edited_arguments: edited });
    if (kind === "unknown") return this.report(convId, call.id, error(`There is no tool named "${call.name}".`));

    const target = attachedTab(this.slotId);
    if (needsSession(kind)) {
      if (!target) return this.report(convId, call.id, error(NO_TAB));
      if (!target.session || target.session.status !== "connected" || !target.tab.sessionId) return this.report(convId, call.id, error(DISCONNECTED));
    }
    try {
      if (kind === "read_terminal") return this.report(convId, call.id, { ...readTerminal(target!.session!, args), edited_arguments: edited });
      if (kind === "send_input") {
        const result = await sendInput(target!.session!, args, this.abort.signal);
        if (!result || !this.active) return;
        return this.report(convId, call.id, { ...result, edited_arguments: edited });
      }
      // AI-08: tools use the tab's current session, so they follow a reconnect.
      const sessionId = target?.tab.status === "connected" ? target.tab.sessionId : null;
      this.addEntry(await api.ai_tool_run(convId, call.id, sessionId, edited));
    } catch (e) {
      if (!this.active) return;
      await this.report(convId, call.id, error(`The tool failed: ${toAppError(e).detail}`));
    }
  }

  private async report(convId: string, callId: string, result: AiToolResultInput) {
    if (!this.active) return;
    try {
      this.addEntry(await api.ai_tool_result(convId, callId, result));
    } catch {
      // The turn ended meanwhile; Rust stores a cancelled result for the call.
    }
  }
}

/** The stored user entry in place of the one shown while the command was on its way. */
function placeUserEntry(entries: AiEntryView[], entry: AiEntryView): AiEntryView[] {
  if (!entries.some((e) => e.entry_id === LOCAL_ENTRY)) return upsertEntry(entries, entry);
  return entries.filter((e) => e.entry_id !== entry.entry_id).map((e) => (e.entry_id === LOCAL_ENTRY ? entry : e));
}

/** A new conversation keeps what was chosen in its slot before it had an id: the mode (AI-16) and the tools menu (AI-30). */
function rememberChoices(slotId: string, convId: string) {
  const { mode, mcpOff } = getSlot(slotId);
  useAi.setState((st) => ({
    modes: mode !== defaultMode() || st.modes[convId] ? { ...st.modes, [convId]: mode } : st.modes,
    mcpOff: mcpOff.length > 0 || st.mcpOff[convId] ? { ...st.mcpOff, [convId]: mcpOff } : st.mcpOff,
  }));
}

/** After a turn: a stopped turn's cancelled results may only be in the store, and history order changed. */
async function afterTurn(slotId: string, reason: AiTurnEndReason) {
  if (useAi.getState().history) void loadHistory();
  if (reason === "stopped") await reloadSlot(slotId);
}

// ───────────────────────── turns ─────────────────────────

/**
 * Sends a message (§13.1 step 1). Sending while a turn runs stops that turn first. The tab's
 * selection chip goes with it (AI-10), and is then hidden until the selection changes.
 */
export async function sendMessage(slotId: string, raw: string): Promise<boolean> {
  const typed = raw.trim();
  const slot = getSlot(slotId);
  // A new conversation has no id until `ai_send` returns, so a second message would start another one.
  if (!typed || (slot.turn?.phase === "starting" && !slot.conversationId)) return false;
  const context = turnContext(slotId);
  if (!context) {
    toast(t("ai.err.noModel"), "error");
    return false;
  }
  runners.get(slotId)?.dispose(); // Rust stops that turn when the message arrives

  const attachment = selectionAttachment(slotId);
  const hiddenBefore = tabSelection(slotId).hidden;
  if (attachment) hideSelection(slotId);
  const diagnostics = slotId === HOME_SLOT ? null : (useAi.getState().diagnostics[slotId] ?? null);
  if (diagnostics) removeDiagnostics(slotId);
  const text = composeMessage(typed, { diagnostics, selection: attachment });
  const runner = new TurnRunner(slotId, slot.conversationId);
  runners.set(slotId, runner);
  const local: AiEntryView = { role: "user", entry_id: LOCAL_ENTRY, created_at: Date.now(), text };
  patchSlot(slotId, (s) => ({
    entries: [...s.entries.filter((e) => e.entry_id !== LOCAL_ENTRY), local],
    turn: newTurn(),
    outcome: null,
    remoteRunning: false,
    draft: "",
  }));
  try {
    const started = await api.ai_send({ conversation_id: slot.conversationId, text, context }, runner.onEvent);
    const conv = started.conversation;
    runner.started(conv.id);
    if (useAi.getState().history) void loadHistory();
    if (runner.gone) return true; // the slot moved on (or the vault locked) meanwhile
    const target = runner.slotId;
    rememberChoices(target, conv.id);
    patchSlot(target, (s) => ({ conversationId: conv.id, conversation: withContext(s, conv), entries: placeUserEntry(s.entries, started.user_entry) }));
    return true;
  } catch (e) {
    runner.dispose();
    patchSlot(slotId, (s) => ({
      entries: s.entries.filter((x) => x.entry_id !== LOCAL_ENTRY),
      turn: null,
      draft: s.draft || raw,
    }));
    if (attachment) setTabSelection(slotId, (sel) => ({ ...sel, hidden: hiddenBefore }));
    if (diagnostics) useAi.setState((st) => ({ diagnostics: { ...st.diagnostics, [slotId]: diagnostics } }));
    toast(aiErrorMessage(e), "error");
    return false;
  }
}

/** Provider error → Retry (§9): the next request from the stored conversation. */
export async function retryTurn(slotId: string) {
  const slot = getSlot(slotId);
  const id = slot.conversationId;
  if (!id || slot.turn) return;
  const context = turnContext(slotId);
  if (!context) {
    toast(t("ai.err.noModel"), "error");
    return;
  }
  const runner = new TurnRunner(slotId, id);
  runners.set(slotId, runner);
  patchSlot(slotId, { turn: newTurn(), outcome: null });
  try {
    await api.ai_retry(id, context, runner.onEvent);
  } catch (e) {
    runner.dispose();
    patchSlot(slotId, { turn: null });
    toast(aiErrorMessage(e), "error");
  }
}

/**
 * AI-26: replaces an earlier message of the user with `raw`, deletes every entry after it, and starts
 * a turn. Asks first when entries follow it. Not while a turn runs.
 */
export async function editAndResend(slotId: string, entryId: string, raw: string): Promise<boolean> {
  const text = raw.trim();
  const slot = getSlot(slotId);
  const id = slot.conversationId;
  if (!text || !id || slot.turn || slot.remoteRunning || slot.compacting) return false;
  const later = laterMessages(slot.entries, entryId);
  if (later > 0) {
    const ok = await confirm({
      title: t("ai.edit.confirmTitle"),
      body: t("ai.edit.confirmBody", { n: later }),
      confirmLabel: t("ai.edit.confirm"),
      danger: true,
    });
    if (!ok) return false;
  }
  const now = getSlot(slotId);
  if (now.conversationId !== id || now.turn || now.remoteRunning || now.compacting) return false;
  const context = turnContext(slotId);
  if (!context) {
    toast(t("ai.err.noModel"), "error");
    return false;
  }

  const runner = new TurnRunner(slotId, id);
  runners.set(slotId, runner);
  const local: AiEntryView = { role: "user", entry_id: LOCAL_ENTRY, created_at: Date.now(), text };
  patchSlot(slotId, (s) => ({ entries: [...entriesBefore(s.entries, entryId), local], turn: newTurn(), outcome: null }));
  try {
    const started = await api.ai_edit_resend(id, entryId, text, context, runner.onEvent);
    if (useAi.getState().history) void loadHistory();
    if (runner.gone) return true;
    patchSlot(runner.slotId, (s) => ({ conversation: withContext(s, started.conversation), entries: placeUserEntry(s.entries, started.user_entry) }));
    return true;
  } catch (e) {
    runner.dispose();
    patchSlot(slotId, { entries: now.entries, turn: null });
    toast(aiErrorMessage(e), "error");
    void reloadSlot(slotId);
    return false;
  }
}

/** Stop, or Esc in the panel (AI-18). */
export async function stopTurn(slotId: string) {
  const runner = runners.get(slotId);
  if (runner) return runner.stop();
  const slot = getSlot(slotId);
  if (slot.remoteRunning && slot.conversationId) {
    await api.ai_stop(slot.conversationId).catch(() => {});
    patchSlot(slotId, { remoteRunning: false });
    await reloadSlot(slotId);
  }
}

/** The user's answer to an approval card (AI-17) or the tool call limit (AI-18). */
export function decideCall(slotId: string, decision: Decision) {
  runners.get(slotId)?.decide(decision);
}

export type { Decision };

// ───────────────────────── conversations in slots ─────────────────────────

/** The slot's conversation leaves it: its turn stops. */
function release(slotId: string) {
  runners.get(slotId)?.abandon();
}

/** New conversation (AI-07): stored when its first message is sent. */
export function newConversation(slotId: string) {
  release(slotId);
  setSlot(slotId, blankSlot(defaultMode(), getSlot(slotId).draft));
  focusInput();
}

/** Reads the slot's conversation again from Rust (after unlock, a sync, or a stopped turn). */
export async function reloadSlot(slotId: string) {
  const id = getSlot(slotId).conversationId;
  if (!id) return;
  try {
    const detail = await api.ai_conversation_get(id);
    if (getSlot(slotId).conversationId !== id) return;
    patchSlot(slotId, (s) => ({
      conversation: detail.conversation,
      entries: s.turn ? s.entries : detail.entries,
      loading: false,
      loadFailed: false,
      remoteRunning: detail.running && !runners.has(slotId),
    }));
  } catch (e) {
    if (getSlot(slotId).conversationId !== id) return;
    if (toAppError(e).code === "not_found") {
      setSlot(slotId, blankSlot(defaultMode(), getSlot(slotId).draft)); // deleted on another device
      return;
    }
    patchSlot(slotId, { loading: false, loadFailed: true });
  }
}

/** Moves a slot's conversation, with its running turn, to another slot; the source gets a new conversation. */
function moveSlot(from: string, to: string) {
  const source = getSlot(from);
  const runner = runners.get(from);
  setSlot(to, { ...source, draft: getSlot(to).draft || source.draft });
  setSlot(from, blankSlot());
  if (runner) {
    runners.delete(from);
    runner.slotId = to;
    runners.set(to, runner);
  }
}

/**
 * Opens a conversation from history in a slot (AI-09); a conversation shown elsewhere moves here.
 * `reveal`: an entry to scroll to and highlight, from a history search hit (AI-24).
 */
export async function openConversation(slotId: string, id: string, reveal: string | null = null) {
  const current = getSlot(slotId);
  if (current.conversationId === id) {
    if (reveal) patchSlot(slotId, { reveal });
    return;
  }
  release(slotId);
  const from = slotOf(id);
  if (from && from !== slotId) {
    moveSlot(from, slotId);
    if (reveal) patchSlot(slotId, { reveal });
    return;
  }
  const st = useAi.getState();
  const known = st.history?.find((c) => c.id === id) ?? null;
  setSlot(slotId, {
    ...blankSlot(st.modes[id] ?? defaultMode(), current.draft),
    conversationId: id,
    conversation: known,
    loading: true,
    mcpOff: st.mcpOff[id] ?? [],
    reveal,
  });
  await reloadSlot(slotId);
}

/** AI-09 "Connect to host": opens a tab for the host and attaches the home tab's conversation to it. */
export async function connectConversation(hostId: string) {
  const before = new Set(useTabs.getState().tabs.map((x) => x.id));
  await connectHost(hostId);
  const tab = useTabs.getState().tabs.find((x) => !before.has(x.id));
  if (tab) moveSlot(HOME_SLOT, tab.id);
}

/** Closing a tab detaches its conversation, which stays in history (AI-08). Its turn stops. */
export function detachSlot(slotId: string) {
  release(slotId);
  setSlot(slotId, null);
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
  useAi.setState((st) => ({
    history: null,
    slots: Object.fromEntries(
      Object.entries(st.slots).map(([id, s]) => [
        id,
        { ...s, conversation: null, entries: [], turn: null, outcome: null, remoteRunning: false, compacting: false, loading: !!s.conversationId },
      ]),
    ),
  }));
}

/** After unlock: read every open conversation again. */
export function reloadAll() {
  for (const id of Object.keys(useAi.getState().slots)) void reloadSlot(id);
}

/** A sync pull may have brought entries from another device (§13.7). Running turns keep their state. */
export function refreshAfterSync() {
  void loadCatalog();
  if (useAi.getState().history) void loadHistory();
  for (const [id, s] of Object.entries(useAi.getState().slots)) {
    if (s.conversationId && !s.turn && !s.loading && !s.compacting && !runners.has(id)) void reloadSlot(id);
  }
}

export function setSlotMode(slotId: string, mode: AiPermissionMode) {
  const id = getSlot(slotId).conversationId;
  patchSlot(slotId, { mode });
  if (id) useAi.setState((st) => ({ modes: { ...st.modes, [id]: mode } }));
}

export function setSlotModel(slotId: string, model: AiModelRef) {
  patchSlot(slotId, { model });
}

/** AI-30: switches an MCP server off (or back on) for the slot's conversation until the app quits. */
export function setMcpServerOff(slotId: string, serverId: string, off: boolean) {
  const slot = getSlot(slotId);
  const mcpOff = off ? [...slot.mcpOff.filter((x) => x !== serverId), serverId] : slot.mcpOff.filter((x) => x !== serverId);
  patchSlot(slotId, { mcpOff });
  const id = slot.conversationId;
  if (id) useAi.setState((st) => ({ mcpOff: { ...st.mcpOff, [id]: mcpOff } }));
}

// ───────────────────────── the terminal selection (AI-10) ─────────────────────────

export function tabSelection(tabId: string): TabSelection {
  return useAi.getState().selections[tabId] ?? NO_SELECTION;
}

function setTabSelection(tabId: string, update: (sel: TabSelection) => TabSelection) {
  useAi.setState((st) => {
    const cur = st.selections[tabId] ?? NO_SELECTION;
    const next = update(cur);
    return next === cur ? {} : { selections: { ...st.selections, [tabId]: next } };
  });
}

/** xterm's selection of a tab changed; `renewed`: it was cleared on the way, as a new drag does. */
export function noteSelection(tabId: string, text: string, renewed = false) {
  setTabSelection(tabId, (sel) => {
    const next = nextSelection(sel, text, renewed);
    return next === sel ? sel : { ...sel, ...next };
  });
}

/** × on the chip, or the chip was sent: hidden until the selection changes. */
export function hideSelection(tabId: string) {
  setTabSelection(tabId, (sel) => (sel.hidden === sel.seq ? sel : { ...sel, hidden: sel.seq }));
}

/** The host name the selection block carries: the tab host's display name. */
function selectionHost(tabId: string): string {
  const tab = useTabs.getState().tabs.find((x) => x.id === tabId);
  if (!tab) return "";
  return useVaultData.getState().hosts.find((h) => h.id === tab.hostId)?.name ?? tab.title;
}

/** What the chip attaches to the next message, or null when it does not show. */
export function selectionAttachment(slotId: string): SelectionAttachment | null {
  if (slotId === HOME_SLOT) return null;
  const sel = tabSelection(slotId);
  return chipShown(sel, sel.hidden) ? makeAttachment(selectionHost(slotId), sel.text) : null;
}

/** AI-10 Ask AI: opens the panel on the tab, where the selection shows as the chip, and focuses the input. */
export function askAi(slotId: string, selection: string) {
  noteSelection(slotId, selection);
  setTabSelection(slotId, (sel) => (sel.hidden === null ? sel : { ...sel, hidden: null }));
  toggleAiPanel(true);
}

/**
 * Ask AI on the terminal's connection error card: attaches the connection's diagnostics to the
 * tab's next message, suggests a question when the input is empty (selected, so typing replaces
 * it), and opens the panel on the tab. Nothing is sent until the user sends it.
 */
export function askAiAboutConnection(tabId: string, host: string, text: string) {
  const diagnostics = makeDiagnostics(host, text);
  if (!diagnostics) return;
  useAi.setState((st) => ({ diagnostics: { ...st.diagnostics, [tabId]: diagnostics } }));
  const suggest = !getSlot(tabId).draft.trim();
  if (suggest) patchSlot(tabId, { draft: t("ai.connection.question") });
  toggleAiPanel(true);
  focusInput(suggest);
}

/** × on the diagnostics chip, or the diagnostics were sent. */
export function removeDiagnostics(tabId: string) {
  useAi.setState((st) => {
    if (!st.diagnostics[tabId]) return {};
    const diagnostics = { ...st.diagnostics };
    delete diagnostics[tabId];
    return { diagnostics };
  });
}

/** AI-21 Compact: the summary becomes the start of the context. */
export async function compactConversation(slotId: string) {
  const slot = getSlot(slotId);
  const id = slot.conversationId;
  if (!id || slot.turn || slot.compacting) return;
  const context = turnContext(slotId);
  if (!context) {
    toast(t("ai.err.noModel"), "error");
    return;
  }
  patchSlot(slotId, { compacting: true });
  try {
    const entry = await api.ai_compact(id, context);
    patchSlot(slotId, (s) =>
      s.conversationId === id
        ? {
            compacting: false,
            entries: upsertEntry(s.entries, entry),
            conversation: s.conversation ? { ...s.conversation, context_start: entry.entry_id } : s.conversation,
          }
        : { compacting: false },
    );
    void reloadSlot(slotId);
  } catch (e) {
    patchSlot(slotId, { compacting: false });
    toast(aiErrorMessage(e), "error");
  }
}

// ───────────────────────── catalog and history ─────────────────────────

/** Providers and the default model (Settings → AI). */
export async function loadCatalog() {
  try {
    const [providers, settings] = await Promise.all([api.ai_providers_list(), api.ai_settings_get()]);
    useAi.setState({ catalog: { providers, settings, loaded: true } });
  } catch {
    useAi.setState((st) => ({ catalog: { ...st.catalog, loaded: true } }));
  }
}

export async function loadHistory() {
  try {
    const history = await api.ai_conversations_list();
    useAi.setState({ history, historyFailed: false });
  } catch {
    useAi.setState((st) => ({ history: st.history ?? [], historyFailed: true }));
  }
}

export async function renameConversation(id: string, title: string) {
  try {
    updateConversation(await api.ai_conversation_rename(id, title));
  } catch (e) {
    toast(aiErrorMessage(e), "error");
  }
}

export async function pinConversation(id: string, pinned: boolean) {
  try {
    updateConversation(await api.ai_conversation_pin(id, pinned));
  } catch (e) {
    toast(aiErrorMessage(e), "error");
  }
}

/** AI-25: saves the conversation as a Markdown file the user picks. */
export async function exportConversation(id: string) {
  try {
    const detail = await api.ai_conversation_get(id);
    const conversation = detail.conversation;
    const host = useVaultData.getState().hosts.find((h) => h.id === conversation.host_id);
    const when = new Intl.DateTimeFormat(getLocale(), { dateStyle: "medium", timeStyle: "short" });
    const text = conversationMarkdown(
      { conversation, entries: detail.entries, host: host ? { name: host.name, target: `${host.username}@${host.address}:${host.port}` } : null },
      { t, time: (ms) => when.format(ms) },
    );
    const path = await pickSavePath(exportFileName(conversation.title, t("ai.export.fileName")), { name: "Markdown", extension: "md" });
    if (!path) return;
    await api.save_text_file(path, text);
    toast(t("ai.export.done"), "success");
  } catch (e) {
    toast(aiErrorMessage(e), "error");
  }
}

export async function deleteConversation(id: string) {
  const slotId = slotOf(id);
  if (slotId) {
    release(slotId);
    setSlot(slotId, blankSlot(defaultMode(), getSlot(slotId).draft));
  }
  try {
    await api.ai_conversation_delete(id);
    useAi.setState((st) => ({ history: st.history?.filter((c) => c.id !== id) ?? null }));
  } catch (e) {
    toast(aiErrorMessage(e), "error");
  }
}

// ───────────────────────── the panel ─────────────────────────

/** `selectAll`: select the draft, so typing replaces a suggested question. */
export function focusInput(selectAll = false) {
  useAi.setState((st) => ({ focusTick: st.focusTick + 1, focusSelectAll: selectAll }));
}

/** Shows or hides the panel (Ctrl+Shift+A / ⌘⇧A); the open state is a device preference (§9). */
export function toggleAiPanel(open?: boolean) {
  const app = useApp.getState();
  const next = open ?? !app.prefs.ai_panel_open;
  if (next === app.prefs.ai_panel_open) {
    if (next) focusInput();
    return;
  }
  void app.setPrefs({ ...app.prefs, ai_panel_open: next }).catch(() => {});
  if (next) focusInput();
  else {
    const active = useTabs.getState().active;
    if (active !== HOME_SLOT) getSession(active)?.focus();
  }
}
