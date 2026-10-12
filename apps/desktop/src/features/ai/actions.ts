import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { useTabs, type SessionTab, type TabSource } from "@/app/tabs";
import { confirm, toast } from "@/components/overlay";
import { connectHost, connectTarget } from "@/features/terminal/connect";
import { getSession, type LiveSession } from "@/features/terminal/session";
import { getLocale, t, type MessageKey, type Params } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type {
  AiAutoVerdict,
  AiConversationView,
  AiEffort,
  AiEntryView,
  AiFileChange,
  AiFilePreview,
  AiFileRevert,
  AiModelRef,
  AiPermissionMode,
  AiToolCall,
  AiToolResultInput,
  AiTurnContext,
  AiTurnEndReason,
  AiTurnEvent,
  DroppedFile,
  McpToolInfo,
} from "@/ipc/types";
import { pickSavePath } from "@/lib/native";
import {
  baseName,
  checkFile,
  chipShown,
  composeMessage,
  droppedFile,
  FILE_MAX_BYTES,
  fitsMessage,
  makeAttachment,
  makeDiagnostics,
  makePaste,
  MESSAGE_MAX_BYTES,
  nextSelection,
  notedPermission,
  parseMessage,
  textFile,
  type Attachment,
  type FileAttachment,
  type FileRefusal,
  type PasteAttachment,
  type SelectionAttachment,
} from "./attachments";
import { addAllowances, hostRefOf, resetAllowances, savedAllowances, useAllowances } from "./allowances";
import { hardRule, type HardRule } from "./autoRules";
import { JUDGE_ERROR } from "./autoText";
import { askEditedFiles } from "./EditFiles";
import { confirmRevert } from "./RevertConfirm";
import { chosenEffort } from "./effort";
import { conversationMarkdown, exportFileName } from "./exportMarkdown";
import { forkModel, forkTitle } from "./fork";
import { conversationModel, judgeChosen, sameModel } from "./models";
import { joinMessages, messageAttachments, messageBack, messageText, pickInput, sendsQueued, withinCap } from "./queue";
import { approvalCall, canAskSide, newSideQuestion, reduceSideEvent, sideRunning, type SideQuestion } from "./side";
import {
  blankSlot,
  defaultMode,
  getSlot,
  HOME_SLOT,
  NO_SELECTION,
  offersHomeConversation,
  patchSlot,
  sendsScreen,
  setSlot,
  slotOf,
  updateConversation,
  useAi,
  type InputChip,
  type PendingAttachment,
  type QueuedMessage,
  type Slot,
  type TabSelection,
} from "./store";
import { screenNote, screenTail } from "./terminalText";
import { DISCONNECTED, NO_TAB, readTerminal, sendInput } from "./terminalTools";
import {
  allowanceOf,
  allowanceScope,
  CALL_MCP_TOOL,
  callAllowed,
  callMode,
  grantKeys,
  mcpOffResult,
  mustAsk,
  needsApproval,
  needsSession,
  parseArgs,
  savedFor,
  shownCall,
  toolKind,
  type HostRef,
  type Scope,
  type ToolKind,
} from "./tools";
import {
  editedFiles,
  entriesBefore,
  laterContextStart,
  laterMessages,
  LOCAL_ENTRY,
  newTurn,
  placeUserEntry,
  reduceTurnEvent,
  unansweredCalls,
  upsertEntry,
  type AutoAsk,
  type CallInfo,
  type CallState,
} from "./turn";

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

/** The thinking level the next request of a slot is made with (AI-05); null is Default. Rust fits it to the model. */
export function slotEffort(slotId: string): AiEffort | null {
  const slot = getSlot(slotId);
  return chosenEffort(slot.effort, slot.conversationId, slot.conversation, useAi.getState().catalog.settings);
}

/**
 * What a request may act on: the tab's host or quick-connect target and, when connected, its terminal
 * tools and session, whose server's identification string the system prompt states (AI-08, AI-09,
 * §13.1), the permission mode a `permission_mode` note tells the model of (§13.5), and the screen
 * a message shows while **Send Terminal Screen** is on (AI-46). The home tab names the
 * conversation's own host or target, which leaves it where it is.
 */
export function turnContext(slotId: string): AiTurnContext | null {
  const model = slotModel(slotId);
  if (!model) return null;
  const attached = attachedTab(slotId);
  const connected = !!attached && attached.tab.status === "connected";
  const slot = getSlot(slotId);
  const term = connected && sendsScreen(slot, useAi.getState().catalog.settings) ? attached.session?.term : undefined;
  const conversation = slot.conversation;
  return {
    provider_id: model.provider_id,
    model_id: model.model_id,
    effort: slotEffort(slotId),
    host_id: attached ? attached.tab.hostId : (conversation?.host_id ?? null),
    target: attached ? attached.tab.target : (conversation?.quick_target ?? null),
    tab: connected,
    session_id: connected ? (attached.tab.sessionId ?? null) : null,
    disabled_mcp_servers: slot.mcpOff,
    permission_mode: slot.mode,
    screen: term ? screenNote(term.buffer.active, term.rows) : null,
  };
}

/** Applies a streamed event to a slot; a stored summary moves the conversation's `context_start` (AI-21, AI-22). */
function applyEvent(slotId: string, ev: AiTurnEvent) {
  patchSlot(slotId, (s) => {
    const before = s.conversation?.context_start ?? null;
    const next = reduceTurnEvent({ entries: s.entries, turn: s.turn, outcome: s.outcome, contextStart: before, effortIgnored: s.effortIgnored }, ev);
    return {
      entries: next.entries,
      turn: next.turn,
      outcome: next.outcome,
      effortIgnored: next.effortIgnored ?? s.effortIgnored,
      conversation: s.conversation && next.contextStart !== before ? { ...s.conversation, context_start: next.contextStart } : s.conversation,
    };
  });
}

/** The conversation view a command returned, without moving `context_start` back past a summary the turn already stored. */
function withContext(s: Slot, view: AiConversationView): AiConversationView {
  return { ...view, context_start: laterContextStart(s.entries, s.conversation?.context_start ?? null, view.context_start) };
}

// ───────────────────────── the turn runner ─────────────────────────

/**
 * `allow`: AI-19 Allow for this conversation, or Always allow on this device (AI-19, and AI-31 for MCP
 * tools), for the call's `allowanceScope`.
 * `refusal`: the error the approval card of an `edit_file` or `write_file` call showed in place of a diff
 * (AI-39, AI-40); the call answers with it instead of running, so nothing is written that the user did
 * not see as a diff.
 */
type Decision =
  | { kind: "run"; edited: string | null; allow?: "conversation" | "always"; refusal?: string }
  | { kind: "reject"; reason: string }
  | { kind: "stop" };

/** AI-19: later calls the grants cover run without asking in this conversation until the app quits. `grants` are `grantKeys`. */
function allowInConversation(convId: string, grants: string[]) {
  useAi.setState((st) => {
    const list = st.allowed[convId] ?? [];
    const added = grants.filter((g) => !list.includes(g));
    return added.length === 0 ? {} : { allowed: { ...st.allowed, [convId]: [...list, ...added] } };
  });
}

/**
 * AI-47: what Auto mode does with a call manual approval would ask about: run it without the card, or show the
 * card, with what it says about why (null: as in manual approval). `judged`: Auto decided the call, so its
 * result is stored with the judge's verdict.
 */
interface AutoOutcome {
  run: boolean;
  ask: AutoAsk | null;
  judged: boolean;
}

/** AI-47: the calls whose verdict is asked for as soon as their response arrives (`prefetch`). */
const PREFETCHED: readonly ToolKind[] = ["run_command", "edit_file", "write_file", "fetch_url"];

/** AI-47: whether `verdict`'s judge is still the one chosen in Settings → AI, and still listed by its provider. */
async function judgeStillChosen(verdict: AiAutoVerdict): Promise<boolean> {
  try {
    const [providers, settings] = await Promise.all([api.ai_providers_list(), api.ai_settings_get()]);
    return sameModel(settings.judge_model, verdict) && judgeChosen(providers, settings);
  } catch {
    return false;
  }
}

/** AI-47: the verdict standing in for a judge that could not be asked: the card asks, saying so. */
function failedVerdict(): AiAutoVerdict {
  return { provider_id: "", model_id: "", decision: "ask", answers: [], concerns: [], failure: JUDGE_ERROR };
}

const runners = new Map<string, TurnRunner>();

/**
 * Drives one turn (§13.1) for the slot it belongs to: applies the streamed events, then handles the
 * tool calls of each response one at a time, in order, applying the permission mode (§13.5).
 * `read_terminal` and `send_input` run here; other tools run in Rust.
 */
class TurnRunner {
  /** The slot the turn's events and state go to; moves with the conversation (Connect to host, AI-09). */
  slotId: string;
  /** The tab the turn started on, the only one whose terminal its tools may use (AI-08). */
  private readonly tab: string;
  /** The permission mode the model was told of for this turn (§13.5); a switch to bypass waits for the next message. */
  private readonly mode: AiPermissionMode;
  convId: string | null;
  private ended = false;
  private disposed = false;
  private stopping = false;
  private processing = false;
  private waiter: ((d: Decision) => void) | null = null;
  /** AI-47: the judge's verdicts asked for ahead (`prefetch`), by call id, until their call takes them. */
  private readonly verdicts = new Map<string, Promise<AiAutoVerdict>>();
  /** AI-47: the user rejected a call of the response being handled, so Auto asks about the rest of its calls without the judge. */
  private rejected = false;
  private readonly abort = new AbortController();
  private readonly ready: Promise<string | null>;
  private resolveReady!: (id: string | null) => void;

  constructor(slotId: string, convId: string | null, mode: AiPermissionMode) {
    this.slotId = slotId;
    this.tab = slotId;
    this.convId = convId;
    this.mode = mode;
    this.ready = new Promise((resolve) => (this.resolveReady = resolve));
    if (convId) this.resolveReady(convId);
  }

  get active(): boolean {
    return !this.disposed && !this.ended;
  }

  /** Still handling calls: not over, and not asked to stop. */
  private get acting(): boolean {
    return this.active && !this.stopping;
  }

  /**
   * The terminal the turn's tools act on: its own tab's, and none once the conversation moved to
   * another slot, whose tab may be on another host (AI-08). Moving a running turn is refused where it
   * can be; this holds whatever moves it.
   */
  private target(): ReturnType<typeof attachedTab> {
    return this.slotId === this.tab ? attachedTab(this.tab) : null;
  }

  get gone(): boolean {
    return this.disposed;
  }

  /**
   * Rust registered a new conversation's turn under `convId` before storing its message (AI-45). A stop
   * names it from now on, so it works while the MCP servers start; one asked for before is sent now.
   */
  private opened(convId: string) {
    if (this.convId) return;
    this.convId = convId;
    if (this.stopping) void this.requestStop(convId);
  }

  /** `ai_send` stored the conversation. A stop asked for before that is sent now. */
  started(convId: string) {
    this.convId = convId;
    this.resolveReady(convId);
    if (this.stopping) void this.requestStop(convId);
  }

  onEvent = (ev: AiTurnEvent) => {
    if (this.disposed) return;
    if (ev.kind === "opened") {
      this.opened(ev.conversation_id);
      return;
    }
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
    void afterTurn(this.slotId, reason, this.stopping);
  }

  private setCall(call: CallInfo | null) {
    if (this.disposed) return;
    patchSlot(this.slotId, (s) => (s.turn ? { turn: { ...s.turn, call } } : {}));
  }

  private wait(callId: string, state: CallState, mcp?: McpToolInfo | null, auto?: AutoAsk | null): Promise<Decision> {
    if (!this.acting) return Promise.resolve({ kind: "stop" });
    this.setCall({ id: callId, state, mcp, auto });
    return new Promise((resolve) => (this.waiter = resolve));
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
      const calls = unansweredCalls(getSlot(this.slotId).entries);
      // AI-47: a call a saved approval covers never reaches the judge, so they are read (once after each
      // unlock) before any call is judged ahead.
      if (callMode(this.mode, getSlot(this.slotId).mode) === "auto") await savedAllowances().catch(() => []);
      if (!this.active || this.stopping) return;
      this.rejected = false;
      this.prefetch(convId, calls);
      for (const call of calls) {
        if (!this.active || this.stopping) return;
        await this.handleCall(convId, call);
      }
    } finally {
      this.processing = false;
      if (this.active) this.setCall(null);
    }
  }

  private async handleCall(convId: string, call: AiToolCall) {
    const kind = toolKind(call.name);
    // AI-09: with no terminal tab, a terminal tool cannot run, so it gets its error without asking first.
    if (needsSession(kind) && !this.target()) return this.answer(convId, call.id, NO_TAB);
    // AI-31: an MCP call's tool, resolved the way Rust runs the call; its Always allow and its server's
    // Always ask decide whether it asks. A `call_mcp_tool` call (AI-45) counts as the tool it names.
    let mcp: McpToolInfo | null = null;
    if (kind === "mcp") {
      if (call.name === CALL_MCP_TOOL) {
        const proxy = await api.mcp_proxy_info(convId, call.arguments).catch(() => null);
        if (!this.acting) return;
        // Nothing would run (arguments that do not match the tool's schema, a tool the conversation does
        // not have, a deleted server): Rust answers it with that error, without asking.
        if (proxy && !proxy.runs) {
          this.setCall({ id: call.id, state: "running" });
          await this.execute(convId, call, kind, null, false);
          return;
        }
        mcp = proxy?.tool ?? null;
      } else {
        mcp = await api.mcp_tool_info(convId, call.name).catch(() => null);
      }
      if (!this.acting) return;
      // AI-30: a server switched off in the tools menu runs nothing more, in this turn too.
      if (mcp && getSlot(this.slotId).mcpOff.includes(mcp.server_id)) return this.answer(convId, call.id, mcpOffResult(mcp.server_name));
    }
    // AI-19: the approvals the user chose for this conversation and saved on this device, which name the
    // tab's host for commands and tools; and §13.5's commands that only read.
    const target = this.target();
    const host = target ? hostRefOf(target.tab) : null;
    const mode = callMode(this.mode, getSlot(this.slotId).mode);
    const saved = kind !== "mcp" && needsApproval(kind, mode) ? await savedAllowances().catch(() => []) : [];
    if (!this.acting) return;
    const allowed = callAllowed(kind, parseArgs(call.arguments) ?? {}, mcp, useAi.getState().allowed[convId] ?? [], saved, host);
    let edited: string | null = null;
    let judged = false;
    if (mustAsk({ kind, mode, allowed, mcp })) {
      // AI-47: in Auto, the judge decides first whether the card shows, unless a hard rule shows it.
      const auto: AutoOutcome | null = mode === "auto" ? await this.judge(convId, call, kind, mcp) : { run: false, ask: null, judged: false };
      if (!auto || !this.acting) return;
      judged = auto.judged;
      if (!auto.run) {
        const d = await this.wait(call.id, "approval", mcp, auto.ask);
        if (!this.acting || d.kind === "stop") return;
        if (d.kind === "reject") {
          // AI-47: Auto asks about the response's later calls; their verdicts asked for ahead are dropped, so
          // that no result stores one.
          this.rejected = true;
          this.verdicts.clear();
          this.setCall({ id: call.id, state: "running" });
          await this.report(convId, call.id, { status: "rejected", content: d.reason, edited_arguments: null, judged });
          return;
        }
        edited = d.edited;
        const allow = allowanceOf(d);
        // What the card offered, for the input that runs: an edited command names its own prefixes.
        const scope = allow ? allowanceScope(kind, parseArgs(edited ?? call.arguments) ?? {}, mcp) : null;
        if (allow === "conversation" && scope) allowInConversation(convId, grantKeys(scope));
        else if (allow === "always" && scope) await this.alwaysAllow(scope, host, mcp);
        if (d.refusal !== undefined) {
          if (!this.acting) return;
          this.setCall({ id: call.id, state: "running" });
          await this.report(convId, call.id, { status: "error", content: d.refusal, edited_arguments: edited, judged });
          return;
        }
      }
    }
    if (!this.acting) return;
    this.setCall({ id: call.id, state: "running" });
    await this.execute(convId, call, kind, edited, judged);
  }

  /**
   * AI-47: what Auto mode does with a call manual approval would ask about. A hard rule, or a call the user
   * rejected earlier in the response, shows the card at once, without the judge; otherwise the call waits for
   * the judge's verdict (`judging`), asked for here or ahead (`prefetch`), runs on `run`, and shows the card with
   * the verdict on `ask`. A switch to manual approval meanwhile applies to the call (§13.5): its card shows as in
   * manual approval, and its result goes without the verdict. Null when the turn stopped meanwhile.
   */
  private async judge(convId: string, call: AiToolCall, kind: ToolKind, mcp: McpToolInfo | null): Promise<AutoOutcome | null> {
    const prefetched = this.verdicts.get(call.id);
    this.verdicts.delete(call.id);
    const screen = kind === "send_input" ? this.screenLines() : null;
    // A `call_mcp_tool` call's rules read the arguments of the tool it names (AI-45).
    const args = parseArgs(shownCall(call).arguments) ?? {};
    const rule: HardRule | null = hardRule({ kind, args, mcp, screen }) ?? (this.rejected ? { kind: "after_reject" } : null);
    if (rule) return { run: false, ask: { rule }, judged: false };
    this.setCall({ id: call.id, state: "judging", mcp });
    let verdict = await (prefetched ?? this.askJudge(convId, call.id, screen));
    if (!this.acting) return null;
    // A verdict asked for ahead may come from a judge the user has cleared or replaced since, while an earlier call
    // waited: the call is judged again, by the judge chosen now, which asks when there is none.
    if (prefetched) {
      const current = await judgeStillChosen(verdict);
      if (!this.acting) return null;
      if (!current) verdict = await this.askJudge(convId, call.id, screen);
      if (!this.acting) return null;
    }
    if (callMode(this.mode, getSlot(this.slotId).mode) !== "auto") return { run: false, ask: null, judged: false };
    return verdict.decision === "run" ? { run: true, ask: null, judged: true } : { run: false, ask: { verdict }, judged: true };
  }

  /** AI-47: asks the judge about a call; an error asking is a verdict that asks, saying so. */
  private askJudge(convId: string, callId: string, screen: string | null): Promise<AiAutoVerdict> {
    return api.ai_judge(convId, callId, screen).catch(failedVerdict);
  }

  /**
   * AI-47: in Auto, asks the judge about a response's calls as soon as it arrives, so that their answers
   * mostly overlap (the judge reads no results, so none waits for an earlier call): each `run_command`,
   * `edit_file`, `write_file` and `fetch_url` call that, as far as can be told now, will reach it (it asks, no
   * approval covers it, and no hard rule sends it to the card). Not `send_input`, whose judge and hard rules read
   * the screen as it is when the call's turn comes, nor MCP calls, whose tool is resolved first. A call that ends up not
   * taking its verdict stores its result without it.
   */
  private prefetch(convId: string, calls: readonly AiToolCall[]) {
    if (callMode(this.mode, getSlot(this.slotId).mode) !== "auto") return;
    const target = this.target();
    const host = target ? hostRefOf(target.tab) : null;
    const grants = useAi.getState().allowed[convId] ?? [];
    const saved = useAllowances.getState().list ?? [];
    for (const call of calls) {
      const kind = toolKind(call.name);
      if (!PREFETCHED.includes(kind) || this.verdicts.has(call.id) || (needsSession(kind) && !target)) continue;
      const args = parseArgs(call.arguments) ?? {};
      const allowed = callAllowed(kind, args, null, grants, saved, host);
      if (!mustAsk({ kind, mode: "auto", allowed }) || hardRule({ kind, args })) continue;
      this.verdicts.set(call.id, this.askJudge(convId, call.id, null));
    }
  }

  /** AI-47: the last lines of the turn's terminal, which a `send_input` call's judge and hard rules read. */
  private screenLines(): string | null {
    const term = this.target()?.session?.term;
    return term ? screenTail(term.buffer.active, term.rows) : null;
  }

  /** Answers a call with an error result, without asking or running it. */
  private async answer(convId: string, callId: string, content: string) {
    this.setCall({ id: callId, state: "running" });
    await this.report(convId, callId, { status: "error", content, edited_arguments: null, judged: false });
  }

  /** Always allow on the card (AI-19, AI-31): stored on this device, then the call runs. */
  private async alwaysAllow(scope: Scope, host: HostRef | null, mcp: McpToolInfo | null) {
    try {
      if (scope.kind === "mcp" && mcp) await api.mcp_set_always_allow(mcp.server_id, mcp.tool.tool, true);
      else await addAllowances(savedFor(scope, host));
    } catch (e) {
      // The user approved this call, so it runs; only the setting failed.
      toast(aiErrorMessage(e), "error");
    }
  }

  /** Runs a call; `judged`: Auto decided it (AI-47), so its result goes with the judge's verdict. */
  private async execute(convId: string, call: AiToolCall, kind: ToolKind, edited: string | null, judged: boolean) {
    const args = edited ?? call.arguments;
    const error = (content: string): AiToolResultInput => ({ status: "error", content, edited_arguments: edited, judged });
    if (kind === "unknown") return this.report(convId, call.id, error(`There is no tool named "${call.name}".`));

    const target = this.target();
    if (needsSession(kind)) {
      if (!target) return this.report(convId, call.id, error(NO_TAB));
      if (!target.session || target.session.status !== "connected" || !target.tab.sessionId) return this.report(convId, call.id, error(DISCONNECTED));
    }
    try {
      if (kind === "read_terminal") return this.report(convId, call.id, { ...readTerminal(target!.session!, args), edited_arguments: edited, judged });
      if (kind === "send_input") {
        const result = await sendInput(target!.session!, args, this.abort.signal);
        if (!result || !this.active) return;
        return this.report(convId, call.id, { ...result, edited_arguments: edited, judged });
      }
      // AI-08: tools use the tab's current session, so they follow a reconnect.
      const sessionId = target?.tab.status === "connected" ? target.tab.sessionId : null;
      const { entry, dropped_file_changes } = await api.ai_tool_run(convId, call.id, sessionId, edited, judged);
      this.addEntry(entry);
      // AI-41: a write that began keeps a snapshot, which the call's block can now revert, also when it failed
      // partway. Keeping it may have dropped older ones over the size limit, of any conversation.
      if (dropped_file_changes) refreshAllFileChanges();
      else if (kind === "edit_file" || kind === "write_file") void refreshFileChanges(this.slotId, convId);
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
  if (sendsQueued(reason, stoppedByUser)) void sendQueued(target);
  else giveBackQueued(target);
}

// ───────────────────────── turns ─────────────────────────

/** The tab's own chips (AI-10): its diagnostics, then its selection while the chip shows. */
function liveChips(slotId: string): InputChip[] {
  const diagnostics = slotId === HOME_SLOT ? undefined : useAi.getState().diagnostics[slotId];
  const selection = selectionAttachment(slotId);
  return [...(diagnostics ? [diagnostics] : []), ...(selection ? [selection] : [])];
}

/**
 * A message of the typed text with what the input area sends with it, as `pickInput` picks it (AI-10, AI-35):
 * a chip of a kind the queued message it joins (`beside`) has already stays (`left`) for a later one. For a
 * message that was not sent, `restore` puts its chips still in `kept` back in their places: the selection
 * while the tab's selection is the same, the diagnostics while the tab has none. It returns the others.
 */
function inputMessage(slotId: string, typed: string, beside: QueuedMessage | null): { message: QueuedMessage; left: PendingAttachment[] } {
  const picked = pickInput(liveChips(slotId), getSlot(slotId).extras, beside ? messageAttachments(beside) : []);
  const { seq, hidden: hiddenBefore } = tabSelection(slotId);
  const restore = (kept: readonly Attachment[]) => {
    const stray: InputChip[] = [];
    for (const chip of picked.chips) {
      if (!kept.includes(chip)) continue;
      if (chip.kind === "selection" && tabSelection(slotId).seq === seq) setTabSelection(slotId, (sel) => ({ ...sel, hidden: hiddenBefore }));
      else if (chip.kind === "diagnostics" && !useAi.getState().diagnostics[slotId]) useAi.setState((st) => ({ diagnostics: { ...st.diagnostics, [slotId]: chip } }));
      else stray.push(chip);
    }
    return stray;
  };
  return { message: { typed, chips: picked.chips, extras: picked.extras, restore }, left: picked.left };
}

/**
 * `inputMessage`, which then leaves the input area: the box empties, and the chips it took hide. Its pastes and
 * files go back to whichever slot shows the conversation by then.
 */
function takeInput(slotId: string, typed: string, beside: QueuedMessage | null = null): QueuedMessage {
  const { message, left } = inputMessage(slotId, typed, beside);
  if (message.chips.some((c) => c.kind === "selection")) hideSelection(slotId);
  if (message.chips.some((c) => c.kind === "diagnostics")) removeDiagnostics(slotId);
  patchSlot(slotId, { draft: "", extras: left });
  return message;
}

/**
 * Puts a message that was not sent back into the slot's input area, before what it holds (§13.1): its chips in
 * their places, or, where another chip of their kind took that place meanwhile, with its pastes and files.
 */
function putBack(slotId: string, message: QueuedMessage) {
  const stray = message.restore(message.chips).map((attachment) => ({ id: `x${++extraSeq}`, attachment }));
  patchSlot(slotId, (s) => messageBack(s, { ...message, extras: [...stray, ...message.extras] }));
}

/** Whether the input area's message, joined to the queued one, keeps its attachments within the cap (AI-35). */
function joinFits(slotId: string, typed: string, queued: QueuedMessage | null): boolean {
  if (!queued || !typed || withinCap(joinMessages(queued, inputMessage(slotId, typed, queued).message))) return true;
  toast(tooMuchMessage(), "error");
  return false;
}

/**
 * Enter while a turn runs (§13.1): the message waits for the turn to end, with what it takes from the input
 * area, and shows above the input box. A second one joins it, so one message goes.
 */
export function queueMessage(slotId: string, raw: string): boolean {
  const typed = raw.trim();
  const queued = getSlot(slotId).queued;
  if (!typed || !joinFits(slotId, typed, queued)) return false;
  const taken = takeInput(slotId, typed, queued);
  patchSlot(slotId, { queued: queued ? joinMessages(queued, taken) : taken });
  return true;
}

/** §13.1: the turn ended, so the slot's queued message goes; one that cannot goes back into the input area. */
async function sendQueued(slotId: string) {
  const slot = getSlot(slotId);
  if (!slot.queued || slot.turn || slot.remoteRunning || runners.has(slotId)) return;
  if (!(await sendMessage(slotId, ""))) giveBackQueued(slotId);
}

/** The slot's queued message goes back into its input area, unsent, before what that holds (§13.1). */
function giveBackQueued(slotId: string) {
  const queued = getSlot(slotId).queued;
  if (!queued) return;
  patchSlot(slotId, { queued: null });
  putBack(slotId, queued);
}

/** **Edit** on the queued message, or ↑ in the empty input box: it goes back into the box to be changed (§13.1). */
export function editQueued(slotId: string) {
  giveBackQueued(slotId);
  focusInput();
}

/** × on the queued message: it goes, with what it took from the input area. */
export function discardQueued(slotId: string) {
  patchSlot(slotId, { queued: null });
}

/** × on an attachment of the queued message. */
export function removeQueuedAttachment(slotId: string, attachment: Attachment) {
  patchSlot(slotId, (s) =>
    s.queued
      ? { queued: { ...s.queued, chips: s.queued.chips.filter((c) => c !== attachment), extras: s.queued.extras.filter((x) => x.attachment !== attachment) } }
      : {},
  );
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
  const runner = new TurnRunner(slotId, slot.conversationId, context.permission_mode);
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
    if (runner.gone) return true; // the slot moved on (or the vault locked) meanwhile
    const target = runner.slotId;
    rememberChoices(target, conv.id);
    patchSlot(target, (s) => ({ conversationId: conv.id, conversation: withContext(s, conv), entries: placeUserEntry(s.entries, started.user_entry) }));
    return true;
  } catch (e) {
    // The conversation may have moved meanwhile (Connect to host); its runner moved with it.
    const target = runner.slotId;
    runner.dispose();
    // A message queued meanwhile goes back too, after this one.
    giveBackQueued(target);
    patchSlot(target, (s) => ({ entries: s.entries.filter((x) => x.entry_id !== LOCAL_ENTRY), turn: null }));
    putBack(target, message);
    // Stopped before it was stored, while the MCP servers started (AI-45) or the context was compacted
    // (AI-22): nothing was stored, a new conversation was not created, and the message is back in the input.
    if (toAppError(e).code !== "cancelled") toast(aiErrorMessage(e), "error");
    return false;
  }
}

// ───────────────────────── side questions (AI-42) ─────────────────────────

let sideSeq = 0;

/** What each side question asked from the input area took from it (AI-42), until it is answered or replaced. */
const sideInputs = new Map<number, { raw: string; message: QueuedMessage }>();

/** Updates the side question `id` in the slot that shows its conversation now: it may have moved meanwhile. */
function patchSide(convId: string, id: number, update: (q: SideQuestion) => SideQuestion) {
  const target = slotOf(convId);
  if (target) patchSlot(target, (s) => (s.side?.id === id ? { side: update(s.side) } : {}));
}

/**
 * A side question from the input area was refused, failed or was stopped: what it took goes back, as for a
 * message that was not sent. The typed text fills the box only when it is empty, the pastes and files go
 * before those there, and the selection and diagnostics chips come back. Its bubble keeps Retry.
 */
function giveBackSideInput(convId: string, id: number) {
  const taken = sideInputs.get(id);
  if (!taken) return;
  sideInputs.delete(id);
  const { message } = taken;
  const stray = message.restore(message.chips).map((attachment) => ({ id: `x${++extraSeq}`, attachment }));
  const target = slotOf(convId);
  if (target) patchSlot(target, (s) => ({ draft: s.draft || taken.raw, extras: [...stray, ...message.extras, ...s.extras] }));
}

/**
 * AI-42: asks a side question about the slot's conversation without stopping its turn. From the input area
 * (`fromInput`) it is what the box holds, with the attachments a message would take, which leave the input
 * area; from an approval card only the card's text. The answer shows at the end of the conversation and is
 * not stored. A side question already on its way in the conversation is replaced, and what it took from the
 * input area stays out.
 */
export function askSideQuestion(slotId: string, raw: string, fromInput = true): boolean {
  const slot = getSlot(slotId);
  const typed = raw.trim();
  const convId = slot.conversationId;
  if (!convId || !canAskSide(slot, typed)) return false;
  const context = turnContext(slotId);
  if (!context) {
    toast(t("ai.err.noModel"), "error");
    return false;
  }
  if (slot.side) sideInputs.delete(slot.side.id);
  const id = ++sideSeq;
  let text = typed;
  if (fromInput) {
    const taken = takeInput(slotId, typed);
    text = messageText(taken);
    sideInputs.set(id, { raw, message: taken });
  }
  void runSide(slotId, newSideQuestion(id, convId, typed, text), context, approvalCall(slot));
  return true;
}

/** Retry on a side question that failed: the same question, with what it took from the input area. */
export function retrySideQuestion(slotId: string) {
  const slot = getSlot(slotId);
  const q = slot.side;
  if (!q || sideRunning(q) || slot.conversationId !== q.conversationId || !canAskSide(slot, q.sent)) return;
  const context = turnContext(slotId);
  if (!context) {
    toast(t("ai.err.noModel"), "error");
    return;
  }
  void runSide(slotId, newSideQuestion(++sideSeq, q.conversationId, q.question, q.sent), context, approvalCall(slot));
}

async function runSide(slotId: string, q: SideQuestion, context: AiTurnContext, approval: string | null) {
  patchSlot(slotId, { side: q });
  const update = (f: (cur: SideQuestion) => SideQuestion) => patchSide(q.conversationId, q.id, f);
  try {
    const answer = await api.ai_side_question(
      { conversation_id: q.conversationId, question_id: q.id, text: q.sent, context, approval_call_id: approval },
      (ev) => update((cur) => reduceSideEvent(cur, ev)),
    );
    sideInputs.delete(q.id);
    update((cur) => (sideRunning(cur) ? { ...cur, phase: "done", answer: answer.text, finish: answer.finish } : cur));
  } catch (e) {
    // Refused, failed, or the vault locked (a stop gave the input back already, and a newer side question
    // or a message that replaced it keeps the input area as it is).
    giveBackSideInput(q.conversationId, q.id);
    // Stopped, replaced by a newer side question, or the vault locked.
    if (toAppError(e).code === "cancelled") update((cur) => (sideRunning(cur) ? { ...cur, phase: "stopped" } : cur));
    // What streamed before the failure goes: it may be the start of an answer that was dropped. A failure
    // that comes after the user stopped the question changes nothing.
    else update((cur) => (sideRunning(cur) ? { ...cur, phase: "error", answer: "", error: aiErrorMessage(e) } : cur));
  }
}

/**
 * Stop on the side question, or Esc while it runs (AI-42): only the side question stops, never the turn,
 * and what it took from the input area goes back there.
 */
export function stopSideQuestion(slotId: string) {
  const q = getSlot(slotId).side;
  if (!q || !sideRunning(q)) return;
  patchSide(q.conversationId, q.id, (cur) => ({ ...cur, phase: "stopped" }));
  giveBackSideInput(q.conversationId, q.id);
  void api.ai_side_stop(q.conversationId, q.id).catch(() => {});
}

/** × on the side question: it goes, and stops first when its answer is still on its way. */
export function dismissSideQuestion(slotId: string) {
  stopSideQuestion(slotId);
  patchSlot(slotId, { side: null });
}

/**
 * A message discards the slot's side question (AI-42), whose bubble would otherwise stay below the new turn.
 * One still on its way stops first; what it took from the input area stays out, since it was asked.
 */
function discardSideQuestion(slotId: string) {
  const q = getSlot(slotId).side;
  if (!q) return;
  sideInputs.delete(q.id);
  if (sideRunning(q)) void api.ai_side_stop(q.conversationId, q.id).catch(() => {});
  patchSlot(slotId, { side: null });
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
  // mode its messages named.
  const runner = new TurnRunner(slotId, id, notedPermission(slot.entries));
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
        slotBusy(slotId) ? Promise.resolve<AiFileRevert>({ status: "failed", reason: "failed", detail: t("ai.revert.busy") }) : revertChange(slotId, id, change),
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

  const runner = new TurnRunner(slotId, id, context.permission_mode);
  runners.set(slotId, runner);
  // Rust keeps the notes of the message it replaces (AI-05, AI-09), so its dividers stay while it is sent;
  // the screen and the permission mode it works out again (AI-46, §13.5).
  const original = now.entries.find((e) => e.entry_id === entryId);
  const kept = original?.role === "user" ? parseMessage(original.text).notes.filter((n) => n.kind !== "terminal_state" && n.kind !== "permission_mode") : [];
  const local: AiEntryView = { role: "user", entry_id: LOCAL_ENTRY, created_at: Date.now(), text: composeMessage(text, [], kept) };
  patchSlot(slotId, (s) => ({ entries: [...entriesBefore(s.entries, entryId), local], turn: newTurn(), outcome: null, effortIgnored: false }));
  try {
    const started = await api.ai_edit_resend(id, entryId, text, context, runner.onEvent);
    if (useAi.getState().history) void loadHistory();
    if (runner.gone) return true;
    patchSlot(runner.slotId, (s) => ({ conversation: withContext(s, started.conversation), entries: placeUserEntry(s.entries, started.user_entry) }));
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

/** Puts a message that was not sent back into the slot's input, before what it holds: its typed text, and its pastes and files. */
function restoreDraft(slotId: string, text: string) {
  const { attachments, typed } = parseMessage(text);
  const extras: PendingAttachment[] = attachments.flatMap((a) => (a.kind === "paste" || a.kind === "file" ? [{ id: `x${++extraSeq}`, attachment: a }] : []));
  patchSlot(slotId, (s) => messageBack(s, { typed, chips: [], extras, restore: () => [] }));
}

/** The slots whose turn an earlier page started the user is stopping: its end sends the queued message (§13.1). */
const remoteStops = new Set<string>();

/** Stop, or Esc in the panel (AI-18); the queued message goes once the turn has stopped (§13.1). */
export async function stopTurn(slotId: string) {
  const runner = runners.get(slotId);
  if (runner) return runner.stop();
  const slot = getSlot(slotId);
  if (slot.remoteRunning && slot.conversationId) {
    remoteStops.add(slotId);
    try {
      await api.ai_stop(slot.conversationId).catch(() => {});
      patchSlot(slotId, { remoteRunning: false });
      await reloadSlot(slotId);
    } finally {
      remoteStops.delete(slotId);
    }
    await sendQueued(slotId);
  }
}

/** AI-41: reads again the file changes of every open conversation, as the size limit spans them all. */
function refreshAllFileChanges() {
  for (const [id, s] of Object.entries(useAi.getState().slots)) if (s.conversationId) void refreshFileChanges(id, s.conversationId);
}

/** AI-41: reads again which changes of the conversation's file tool calls this device keeps. */
export async function refreshFileChanges(slotId: string, convId: string) {
  try {
    const fileChanges = await api.ai_file_changes(convId);
    // The lock cleared them meanwhile: they name servers and paths.
    if (unlocked() && getSlot(slotId).conversationId === convId) patchSlot(slotId, { fileChanges });
  } catch {
    // Locked or deleted meanwhile; the next load reads them again.
  }
}

/** AI-41: what a call changed, from the snapshot this device keeps; null once it was reverted or dropped. Rejects when it cannot be read. */
export async function fileChangeDiff(slotId: string, change: Pick<AiFileChange, "response_id" | "tool_call_id">): Promise<AiFilePreview | null> {
  const convId = getSlot(slotId).conversationId;
  if (!convId) return null;
  return api.ai_file_change_diff(convId, change.response_id, change.tool_call_id);
}

/**
 * AI-41: reverts the change of a file tool call on the slot's tab, once the conversation is idle. The user
 * confirms first, with the diff the revert writes back; a file the call created is deleted. With `restore`
 * (Restore anyway), it writes over what the diff of the last revert showed, which the user confirms too;
 * `recreate`: that diff showed the file missing, so it is created again. `onStart` runs once the revert is under way. Null when nothing was asked of Rust or it refused; otherwise
 * what the revert did, for the call's block to show.
 */
export async function revertFileChange(
  slotId: string,
  change: AiFileChange,
  options: { restore?: boolean; recreate?: boolean; onStart?: () => void } = {},
): Promise<AiFileRevert | null> {
  const convId = getSlot(slotId).conversationId;
  if (!convId || slotBusy(slotId)) return null;
  const path = change.path;
  const ok = options.restore
    ? await confirm({
        ...(change.created
          ? { title: t("ai.revert.deleteAnywayTitle", { path }), body: t("ai.revert.deleteAnywayBody"), confirmLabel: t("ai.revert.deleteAnyway") }
          : { title: t("ai.revert.restoreTitle", { path }), body: t(options.recreate ? "ai.revert.recreateBody" : "ai.revert.restoreBody"), confirmLabel: t("ai.revert.restore") }),
        danger: true,
      })
    : await fileChangeDiff(slotId, change).then(
        (preview) => confirmRevert(change, preview),
        // The confirmation promises the diff it writes back: without it, nothing is asked or written.
        (e: unknown) => {
          toast(aiErrorMessage(e), "error");
          return false;
        },
      );
  if (!ok || getSlot(slotId).conversationId !== convId || slotBusy(slotId)) return null;
  options.onStart?.();
  patchSlot(slotId, { reverting: true });
  try {
    const outcome = await api.ai_file_revert(convId, change.response_id, change.tool_call_id, revertSession(slotId), !!options.restore);
    if (outcome.status === "reverted") {
      markReverted(slotId, convId, change);
      // Nothing to write: a revert whose result was lost did it already, or the write never changed the file.
      const done = outcome.wrote ? "ai.revert.done" : change.created ? "ai.revert.alreadyGone" : "ai.revert.already";
      toast(t(done, { path }), "success");
    }
    return outcome;
  } catch (e) {
    toast(aiErrorMessage(e), "error");
    // The size limit may have dropped the change meanwhile, or another device's edit deleted its call.
    void refreshFileChanges(slotId, convId);
    return null;
  } finally {
    patchSlot(slotId, { reverting: false });
  }
}

/** The vault is unlocked: what a command returned may go into the store. */
function unlocked(): boolean {
  return useApp.getState().phase === "unlocked";
}

/** AI-41: a revert of the slot's conversation runs, so nothing is sent until it ends (Rust refuses it too). */
function reverting(slotId: string): boolean {
  if (!getSlot(slotId).reverting) return false;
  toast(t("ai.revert.wait"), "error");
  return true;
}

/** The session a revert runs on: the slot's tab, when it is connected. */
function revertSession(slotId: string): string | null {
  const target = attachedTab(slotId);
  return target?.tab.status === "connected" ? (target.tab.sessionId ?? null) : null;
}

function markReverted(slotId: string, convId: string, change: AiFileChange) {
  const same = (c: AiFileChange) => c.response_id === change.response_id && c.tool_call_id === change.tool_call_id;
  patchSlot(slotId, (s) => (s.conversationId === convId ? { fileChanges: s.fileChanges.map((c) => (same(c) ? { ...c, state: "reverted" as const } : c)) } : {}));
}

/** AI-26, AI-41: reverts a change an edit is about to delete the call of; an error is a failure the edit's dialog lists. */
async function revertChange(slotId: string, convId: string, change: AiFileChange): Promise<AiFileRevert> {
  try {
    const outcome = await api.ai_file_revert(convId, change.response_id, change.tool_call_id, revertSession(slotId), false);
    if (outcome.status === "reverted") markReverted(slotId, convId, change);
    return outcome;
  } catch (e) {
    void refreshFileChanges(slotId, convId);
    return { status: "failed", reason: "failed", detail: aiErrorMessage(e) };
  }
}

/** The user's answer to an approval card (AI-17). */
export function decideCall(slotId: string, decision: Decision) {
  runners.get(slotId)?.decide(decision);
}

/**
 * AI-39, AI-40: the change an `edit_file` or `write_file` call waiting on the slot's approval card would
 * make, read from the host of the slot's tab, with the arguments as the user edited them on the card.
 */
export function previewFileCall(slotId: string, callId: string, edited: string | null): Promise<AiFilePreview> {
  const convId = getSlot(slotId).conversationId;
  if (!convId) return Promise.reject(new Error("the conversation is not stored yet"));
  const target = attachedTab(slotId);
  const sessionId = target?.tab.status === "connected" ? target.tab.sessionId : null;
  return api.ai_file_preview(convId, callId, sessionId, edited);
}

export type { Decision };

// ───────────────────────── conversations in slots ─────────────────────────

/** The slot's conversation leaves it: its turn and its side question (AI-42) stop, and its queued message goes back into the input area (§13.1). */
function release(slotId: string) {
  runners.get(slotId)?.abandon();
  stopSideQuestion(slotId);
  giveBackQueued(slotId);
}

/** New conversation (AI-07): stored when its first message is sent. */
export function newConversation(slotId: string) {
  release(slotId);
  setSlot(slotId, blankSlot(defaultMode(), getSlot(slotId).draft, getSlot(slotId).extras));
  focusInput();
}

/** Reads the slot's conversation again from Rust (after unlock, a sync, or a stopped turn). */
export async function reloadSlot(slotId: string) {
  const id = getSlot(slotId).conversationId;
  if (!id) return;
  try {
    const detail = await api.ai_conversation_get(id);
    if (getSlot(slotId).conversationId !== id) return;
    // §13.1: a turn an earlier page started ended on its own, and nothing tells how, so its queued message goes
    // back. A poll that sees the end of one the user is stopping leaves it to the stop, which sends it.
    const remoteEnded = getSlot(slotId).remoteRunning && !detail.running && !remoteStops.has(slotId);
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
      setSlot(slotId, blankSlot(defaultMode(), getSlot(slotId).draft, getSlot(slotId).extras)); // deleted on another device
      return;
    }
    patchSlot(slotId, { loading: false, loadFailed: true });
  }
}

/** Whether a slot's conversation is busy: a turn running (here or in Rust), waiting for approval, compacting, or reverting a file change (AI-41). */
function slotBusy(slotId: string): boolean {
  const s = getSlot(slotId);
  return !!s.turn || s.remoteRunning || s.compacting || s.reverting || runners.has(slotId);
}

/** Whether a conversation is busy in the slot that shows it (`slotBusy`); one no slot shows is not. */
export function conversationBusy(id: string): boolean {
  const slotId = slotOf(id);
  return !!slotId && slotBusy(slotId);
}

/**
 * Moves a slot's conversation to another slot; the source gets a new conversation. A running turn moves
 * with it (Connect to host moves the home tab's), but it never uses the terminal of the slot it moved to:
 * its runner stays pinned to the tab it started on (AI-08).
 */
function moveSlot(from: string, to: string) {
  const source = getSlot(from);
  const runner = runners.get(from);
  const target = getSlot(to);
  // The input area of the slot the conversation moves to wins when it holds something, a tab's selection or
  // diagnostics chip included (AI-10); the source's then stays.
  const keep = !!target.draft || pendingAttachments(to).length > 0;
  setSlot(to, { ...source, draft: keep ? target.draft : source.draft, extras: keep ? target.extras : source.extras });
  setSlot(from, keep ? blankSlot(defaultMode(), source.draft, source.extras) : blankSlot());
  if (runner) {
    runners.delete(from);
    runner.slotId = to;
    runners.set(to, runner);
  }
}

/**
 * Opens a conversation from history in a slot (AI-09); a conversation shown elsewhere moves here. One
 * that is busy in another tab (a turn running or waiting for approval, or a compaction) stays there,
 * since its turn acts on that tab's terminal (AI-08): its tab comes to the front instead.
 * `reveal`: an entry to scroll to and highlight, from a history search hit (AI-24).
 */
export async function openConversation(slotId: string, id: string, reveal: string | null = null) {
  const current = getSlot(slotId);
  if (current.conversationId === id) {
    if (reveal) patchSlot(slotId, { reveal });
    return;
  }
  const from = slotOf(id);
  if (from && from !== slotId && slotBusy(from)) {
    useTabs.getState().activate(from);
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

/** AI-09 "Connect to host": opens a tab on the host or quick-connect target (HOST-12) and attaches the home tab's conversation to it. */
export async function connectConversation(source: TabSource) {
  const before = new Set(useTabs.getState().tabs.map((x) => x.id));
  await (source.hostId !== null ? connectHost(source.hostId) : connectTarget(source.target));
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
  resetAllowances();
  // §13.1: queued messages go back into the input areas, unsent.
  for (const id of Object.keys(useAi.getState().slots)) giveBackQueued(id);
  useAi.setState((st) => ({
    history: null,
    slots: Object.fromEntries(
      Object.entries(st.slots).map(([id, s]) => [
        id,
        // File changes name servers and paths: conversation plaintext too.
        { ...s, conversation: null, entries: [], fileChanges: [], reverting: false, turn: null, outcome: null, effortIgnored: false, remoteRunning: false, compacting: false, side: null, loading: !!s.conversationId },
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
  patchSlot(slotId, { model, effortIgnored: false });
}

/** AI-05: the thinking level of the slot's next messages; the conversation keeps it once one is sent. */
export function setSlotEffort(slotId: string, effort: AiEffort | "default") {
  patchSlot(slotId, { effort, effortIgnored: false });
}

/** AI-30: switches an MCP server off (or back on) for the slot's conversation until the app quits. */
export function setMcpServerOff(slotId: string, serverId: string, off: boolean) {
  const slot = getSlot(slotId);
  const mcpOff = off ? [...slot.mcpOff.filter((x) => x !== serverId), serverId] : slot.mcpOff.filter((x) => x !== serverId);
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

// ───────────────────────── long pastes and text files (AI-35) ─────────────────────────

let extraSeq = 0;

/** Everything that would go with the slot's next message, for the size cap. */
export function pendingAttachments(slotId: string): Attachment[] {
  const extras = getSlot(slotId).extras;
  return [...pickInput(liveChips(slotId), extras).chips, ...extras.map((x) => x.attachment)];
}

const kb = (bytes: number) => `${bytes / 1024} KB`;

/** The toast for an attachment that would pass the per-message cap. */
export function tooMuchMessage(): string {
  return t("ai.attach.total", { size: kb(MESSAGE_MAX_BYTES) });
}

/** Adds a paste or a file to the slot's next message, unless the message would pass the cap. */
function addExtra(slotId: string, attachment: PasteAttachment | FileAttachment): boolean {
  if (!fitsMessage(pendingAttachments(slotId), attachment)) {
    toast(tooMuchMessage(), "error");
    return false;
  }
  patchSlot(slotId, (s) => ({ extras: [...s.extras, { id: `x${++extraSeq}`, attachment }] }));
  return true;
}

/** A long paste into the input box becomes an attachment. */
export function addPaste(slotId: string, text: string): boolean {
  const paste = makePaste(text);
  return paste ? addExtra(slotId, paste) : false;
}

export function removeExtra(slotId: string, id: string) {
  patchSlot(slotId, (s) => ({ extras: s.extras.filter((x) => x.id !== id) }));
}

function refusal(reason: FileRefusal, name: string): string {
  if (reason === "image") return t("ai.attach.image", { name });
  if (reason === "too_large") return t("ai.attach.tooLarge", { name, size: kb(FILE_MAX_BYTES) });
  if (reason === "unreadable") return t("ai.attach.readFailed", { name });
  return t("ai.attach.binary", { name });
}

/**
 * Reads files the user picked or dropped, in the WebView. UTF-8 text files come back; an image, a
 * file over the size cap, or one with NUL bytes or invalid UTF-8 is refused with a toast naming it.
 */
export async function readTextFiles(files: readonly File[]): Promise<FileAttachment[]> {
  const read: FileAttachment[] = [];
  for (const file of files) {
    const name = baseName(file.name);
    const early = checkFile(name, file.type, file.size);
    if (early && !early.ok) {
      toast(refusal(early.reason, name), "error");
      continue;
    }
    let bytes: Uint8Array;
    try {
      bytes = new Uint8Array(await file.arrayBuffer());
    } catch {
      toast(t("ai.attach.readFailed", { name }), "error");
      continue;
    }
    const result = textFile(name, bytes);
    if (result.ok) read.push(result.file);
    else toast(refusal(result.reason, name), "error");
  }
  return read;
}

/** Add Text Files in the + menu, or files dropped on the panel: they go with the slot's next message. */
export async function attachFiles(slotId: string, files: readonly File[]) {
  for (const file of await readTextFiles(files)) if (!addExtra(slotId, file)) break;
  focusInput();
}

/**
 * Files dropped on the panel in the desktop app, whose webview hands over paths only: Rust reads them
 * (only paths of the window's last drop) with the same rules, and they go with the slot's next message.
 */
export async function attachDroppedPaths(slotId: string, paths: readonly string[]) {
  let results: DroppedFile[];
  try {
    results = await api.ai_read_dropped_files([...paths]);
  } catch (e) {
    toast(aiErrorMessage(e), "error");
    return;
  }
  for (const result of results) {
    const read = droppedFile(result);
    if (!read.ok) toast(refusal(read.reason, read.name), "error");
    else if (!addExtra(slotId, read.file)) break;
  }
  focusInput();
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
  if (!id || slot.turn || slot.compacting || reverting(slotId)) return;
  const context = turnContext(slotId);
  if (!context) {
    toast(t("ai.err.noModel"), "error");
    return;
  }
  patchSlot(slotId, { compacting: true });
  try {
    const entry = await api.ai_compact(id, context);
    // The slot that shows the conversation now: it may have moved meanwhile (Connect to host).
    const target = slotOf(id);
    if (!target) return;
    patchSlot(target, (s) => ({
      compacting: false,
      entries: upsertEntry(s.entries, entry),
      conversation: s.conversation ? { ...s.conversation, context_start: entry.entry_id } : s.conversation,
    }));
    void reloadSlot(target);
  } catch (e) {
    const target = slotOf(id);
    if (target) patchSlot(target, { compacting: false });
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

/**
 * AI-43: copies a conversation up to the entry `entryId` (all of it with null) into a new conversation, and
 * opens that in the slot as History's Open does (AI-09). Not while the conversation is busy, or while a fork
 * of it is being made. The fork stays in History when opening it would stop or replace something: the slot
 * is busy (a turn running or waiting for approval, or a compaction), or was when Fork was chosen, or it moved
 * on while Rust copied (another conversation, a message, a turn or a compaction, even one that has ended since).
 */
export async function forkConversation(slotId: string, id: string, entryId: string | null) {
  if (conversationBusy(id) || useAi.getState().forking.includes(id)) return;
  useAi.setState((st) => ({ forking: [...st.forking, id] }));
  const shown = slotOf(id);
  const source = shown ? getSlot(shown) : null;
  const original = source?.conversation ?? useAi.getState().history?.find((c) => c.id === id) ?? null;
  // A message the original's next reply answered with another model goes on with that model (AI-05).
  const model = entryId && source ? forkModel(source.entries, entryId) : null;
  // The stored slot itself: one never used is not stored, and getSlot makes a new blank one each time.
  const before = useAi.getState().slots[slotId];
  const runner = runners.get(slotId);
  const busy = slotBusy(slotId);
  try {
    // The number in the title skips those History has (AI-43).
    if (!useAi.getState().history) await loadHistory();
    const taken = (useAi.getState().history ?? []).map((c) => c.title);
    const fork = await api.ai_conversation_fork(id, entryId, forkTitle(t, original?.title ?? "", taken));
    updateConversation(fork);
    // Every message, stored entry and reload gives the slot a new entries array, so the same array means
    // nothing happened in it meanwhile, however it ended.
    const now = useAi.getState().slots[slotId];
    const movedOn = now?.conversationId !== before?.conversationId || now?.entries !== before?.entries || runners.get(slotId) !== runner || now?.compacting !== before?.compacting;
    if (busy || slotBusy(slotId) || movedOn) {
      toast(t("ai.fork.inHistory"), "success");
      return;
    }
    await openConversation(slotId, fork.id);
    if (model && getSlot(slotId).conversationId === fork.id) setSlotModel(slotId, model);
    toast(t("ai.fork.done"), "success");
    focusInput();
  } catch (e) {
    toast(aiErrorMessage(e), "error");
  } finally {
    useAi.setState((st) => ({ forking: st.forking.filter((x) => x !== id) }));
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
    setSlot(slotId, blankSlot(defaultMode(), getSlot(slotId).draft, getSlot(slotId).extras));
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
