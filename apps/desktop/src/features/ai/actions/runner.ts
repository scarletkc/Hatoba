/**
 * The turn runner (§13.1): one for each running turn, by slot, and what the calls of a tab's conversations share: its
 * terminal and its room for exec channels (AI-07).
 */

import { toast } from "@hatoba/ui";
import { api, toAppError } from "@/ipc/api";
import type {
  AiAutoVerdict,
  AiEntryView,
  AiPermissionMode,
  AiToolCall,
  AiToolResultInput,
  AiTurnEndReason,
  AiTurnEvent,
  McpToolInfo,
} from "@/ipc/types";
import { judgeChosen, sameModel } from "../models";
import { notifyAi } from "../notify";
import { addAllowances, hostRefOf, savedAllowances, useAllowances } from "../rules/allowances";
import { hardRule, type HardRule } from "../rules/autoRules";
import { JUDGE_ERROR } from "../rules/autoText";
import { dialectOf } from "../rules/commandRules";
import { getSlot, patchSlot, slotOf, tabOf, useAi } from "../store";
import { CallRoom, nextBatch, runConcurrently } from "../batches";
import { screenTail } from "../terminal/terminalText";
import { DISCONNECTED, NO_TAB, readTerminal, sendInput, terminalInUse } from "../terminal/terminalTools";
import {
  allowanceOf,
  allowanceScope,
  CALL_MCP_TOOL,
  callAllowed,
  callMode,
  concurrencySafe,
  grantKeys,
  limitOf,
  mcpOffResult,
  mustAsk,
  needsApproval,
  needsSession,
  parseArgs,
  savedFor,
  searchesNatively,
  shownCall,
  toolKind,
  type HostRef,
  type Scope,
  type ToolKind,
} from "../tools";
import { reduceTurnEvent, unansweredCalls, withCall, type AutoAsk, type CallInfo, type CallState } from "../turn";
import { aiErrorMessage, attachedTab } from "./context";
import { refreshAllFileChanges, refreshFileChanges } from "./fileChanges";

/** Applies a streamed event to a slot; a stored summary moves the conversation's `context_start` (AI-21, AI-22). */
function applyEvent(slotId: string, ev: AiTurnEvent) {
  patchSlot(slotId, (s) => {
    const before = s.conversation?.context_start ?? null;
    const next = reduceTurnEvent(
      { entries: s.entries, turn: s.turn, outcome: s.outcome, contextStart: before, effortIgnored: s.effortIgnored },
      ev,
    );
    return {
      entries: next.entries,
      turn: next.turn,
      outcome: next.outcome,
      effortIgnored: next.effortIgnored ?? s.effortIgnored,
      conversation:
        s.conversation && next.contextStart !== before
          ? { ...s.conversation, context_start: next.contextStart }
          : s.conversation,
    };
  });
}

/**
 * `allow`: AI-19 Allow for this conversation, or Always allow on this device (AI-19, and AI-31 for MCP
 * tools), for the call's `allowanceScope`.
 * `refusal`: the error the approval card of an `edit_file` or `write_file` call showed in place of a diff
 * (AI-39, AI-40); the call answers with it instead of running, so nothing is written that the user did
 * not see as a diff.
 */
export type Decision =
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

export const runners = new Map<string, TurnRunner>();

/**
 * AI-43: when each slot last changed by the user's doing, by a sequence number: a turn or a compaction started in
 * it, or another conversation shown. A fork can tell from it that the slot moved on while it was made, also when
 * what started has ended since, or the slot went back to the conversation it showed.
 */
export const lastChange = new Map<string, number>();
let changeSeq = 0;

export function noteChange(slotId: string) {
  lastChange.set(slotId, ++changeSeq);
}

/**
 * AI-07: the turn that holds each tab's terminal for `send_input`, by tab. A tab's conversations share its one
 * interactive shell, so the first turn that types into it holds it until that turn ends, and another turn's input
 * never lands in the middle of its work. Reading the screen, `run_command` (its own exec channel) and the file
 * tools need no hold.
 */
const terminalHolders = new Map<string, TurnRunner>();

/**
 * §13.1, AI-07: the room each tab has for the calls that open an exec channel on its SSH connection or search the
 * web, by tab. Its conversations, also those in its background, share the connection, so its limits hold for all
 * of their calls together.
 */
export const rooms = new Map<string, CallRoom>();

function roomOf(tab: string): CallRoom {
  let room = rooms.get(tab);
  if (!room) rooms.set(tab, (room = new CallRoom()));
  return room;
}

/** What follows the end of a turn in its slot (`afterTurn`); `stoppedByUser`: Stop or Esc. */
type TurnEnd = (slotId: string, reason: AiTurnEndReason, stoppedByUser: boolean) => Promise<void>;

/**
 * Drives one turn (§13.1) for the slot it belongs to: applies the streamed events, then handles the
 * tool calls of each response in order, applying the permission mode (§13.5): a run of calls that only
 * read and need no approval card runs together, and every other call runs alone (`nextBatch`).
 * `read_terminal` and `send_input` run here; other tools run in Rust.
 */
export class TurnRunner {
  /**
   * The slot the turn's events and state go to; moves with the conversation (Connect to host, AI-09), and
   * into the background of its tab and back (AI-07).
   */
  slotId: string;
  /** The tab the turn started on, the only one whose terminal its tools may use (AI-08), also from the background (AI-07). */
  readonly tab: string;
  /** The permission mode the model was told of for this turn (§13.5); a switch to bypass waits for the next message. */
  private readonly mode: AiPermissionMode;
  private readonly onEnd: TurnEnd;
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
  /** §13.1: what frees the exec channel of a call that runs with others, by call id, once its tool ended (`tool_ran`). */
  private readonly releases = new Map<string, () => void>();
  /** §13.1: the room of the turn's tab, which its calls share with the tab's other conversations (AI-07). */
  private readonly room: CallRoom;
  private readonly abort = new AbortController();
  private readonly ready: Promise<string | null>;
  private resolveReady!: (id: string | null) => void;

  constructor(slotId: string, convId: string | null, mode: AiPermissionMode, onEnd: TurnEnd) {
    this.slotId = slotId;
    this.tab = tabOf(slotId);
    this.room = roomOf(this.tab);
    noteChange(slotId);
    this.convId = convId;
    this.mode = mode;
    this.onEnd = onEnd;
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
   * The terminal the turn's tools act on: its own tab's, while that tab shows the conversation or holds it in
   * the background (AI-07), and none once the conversation moved to another tab, which may be on another
   * host (AI-08). Moving a running turn is refused where it can be; this holds whatever moves it.
   */
  private target(): ReturnType<typeof attachedTab> {
    return tabOf(this.slotId) === this.tab ? attachedTab(this.tab) : null;
  }

  /** AI-07: this turn holds the tab's terminal from now on; the function returned lets go of a hold it took just now. */
  private holdTerminal(): () => void {
    const held = terminalHolders.get(this.tab) === this;
    terminalHolders.set(this.tab, this);
    return () => {
      if (!held && terminalHolders.get(this.tab) === this) terminalHolders.delete(this.tab);
    };
  }

  /**
   * AI-07: the error result of a `send_input` call while another turn holds the tab's terminal
   * (`terminalHolders`), naming that turn's conversation; null when this turn may type into it.
   */
  private terminalTaken(): string | null {
    const holder = terminalHolders.get(this.tab);
    if (!holder || holder === this || !holder.active) return null;
    return terminalInUse(getSlot(holder.slotId).conversation?.title ?? "");
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
    // §13.1: the call's tool ended, while its result may wait for an earlier call's.
    if (ev.kind === "tool_ran") {
      this.releases.get(ev.tool_call_id)?.();
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

  /** The conversation is closed with its tab, or deleted: stop the turn in Rust and forget it here. */
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
    if (terminalHolders.get(this.tab) === this) terminalHolders.delete(this.tab);
    // Calls of the turn that wait for room give up.
    this.room.wake();
  }

  private finish(reason: AiTurnEndReason) {
    this.ended = true;
    this.release();
    if (runners.get(this.slotId) === this) runners.delete(this.slotId);
    void this.onEnd(this.slotId, reason, this.stopping);
  }

  /** Shows what a call being handled waits for, in place of what it showed before. */
  private setCall(call: CallInfo) {
    this.patchCalls((calls) => withCall(calls, call));
  }

  /** The call is handled: its result shows, if it has one. */
  private endCall(callId: string) {
    this.patchCalls((calls) => calls.filter((c) => c.id !== callId));
  }

  private patchCalls(change: (calls: CallInfo[]) => CallInfo[]) {
    // Once the turn ended, the slot's turn is another one's, or none.
    if (!this.active) return;
    patchSlot(this.slotId, (s) => (s.turn ? { turn: { ...s.turn, calls: change(s.turn.calls) } } : {}));
  }

  private wait(callId: string, state: CallState, mcp?: McpToolInfo | null, auto?: AutoAsk | null): Promise<Decision> {
    if (!this.acting) return Promise.resolve({ kind: "stop" });
    this.setCall({ id: callId, state, mcp, auto });
    if (state === "approval") void notifyAi(this.slotId, "approval");
    return new Promise((resolve) => (this.waiter = resolve));
  }

  private addEntry(entry: AiEntryView) {
    if (this.disposed) return;
    // Once the turn ended, the runner no longer moves with its conversation (AI-07): later entries go to the slot
    // that holds the conversation then.
    const target = !this.ended ? this.slotId : this.convId ? slotOf(this.convId) : undefined;
    if (target) applyEvent(target, { kind: "entry", entry });
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
      for (let rest = calls; rest.length > 0;) {
        // §13.1: decided as each batch starts, with the mode and the approvals the calls before it left.
        const joins = await this.joins(convId);
        if (!this.acting) return;
        const batch = nextBatch(rest, joins);
        let done = 0;
        if (batch.length > 1) {
          const native = searchesNatively(useAi.getState().catalog.settings);
          const limit = (call: AiToolCall) => limitOf(toolKind(call.name), native);
          // A call that waited for room starts only while it still needs no card: a switch to a stricter mode
          // applies to the turn's next call (§13.5), and the calls left run as the next batches.
          const starts = (call: AiToolCall) => this.acting && this.joinsNow(convId)(call);
          done = await runConcurrently(
            batch,
            limit,
            (call, release) => this.runAlongside(convId, call, release),
            starts,
            this.room,
          );
        }
        if (done === 0) {
          if (!this.acting) return;
          try {
            await this.handleCall(convId, rest[0]);
          } finally {
            this.endCall(rest[0].id);
          }
          done = 1;
        }
        rest = rest.slice(done);
      }
    } finally {
      this.processing = false;
      if (this.active) this.patchCalls(() => []);
    }
  }

  /**
   * §13.1: whether a call runs together with the calls beside it: it only reads (`concurrencySafe`), and in the mode
   * of now and with the approvals of now, it neither shows an approval card nor goes to Auto's judge, which may
   * show one. A call that asks runs alone.
   */
  private async joins(convId: string): Promise<(call: AiToolCall) => boolean> {
    if (callMode(this.mode, getSlot(this.slotId).mode) !== "bypass") await savedAllowances().catch(() => []);
    return this.joinsNow(convId);
  }

  /** `joins` with the saved approvals as read so far. */
  private joinsNow(convId: string): (call: AiToolCall) => boolean {
    const mode = callMode(this.mode, getSlot(this.slotId).mode);
    const saved = useAllowances.getState().list ?? [];
    const target = this.target();
    const host = target ? hostRefOf(target.tab) : null;
    const dialect = dialectOf(target?.tab.local?.kind);
    const grants = useAi.getState().allowed[convId] ?? [];
    return (call) => {
      const kind = toolKind(call.name);
      const args = parseArgs(call.arguments) ?? {};
      return (
        concurrencySafe(kind, args, dialect) &&
        !mustAsk({ kind, mode, allowed: callAllowed(kind, args, null, grants, saved, host, dialect) })
      );
    };
  }

  /**
   * Runs a call of a batch whose calls run together (§13.1), which needs no approval card. `release` frees its exec
   * channel for the batch once Rust says its tool ended (`tool_ran`).
   */
  private async runAlongside(convId: string, call: AiToolCall, release: () => void) {
    this.releases.set(call.id, release);
    try {
      const kind = toolKind(call.name);
      if (needsSession(kind) && !this.target()) return await this.answer(convId, call.id, NO_TAB);
      if (!this.acting) return;
      this.setCall({ id: call.id, state: "running" });
      await this.execute(convId, call, kind, null, false);
    } finally {
      this.releases.delete(call.id);
      this.endCall(call.id);
    }
  }

  private async handleCall(convId: string, call: AiToolCall) {
    const kind = toolKind(call.name);
    // AI-09: with no terminal tab, a terminal tool cannot run, so it gets its error without asking first.
    if (needsSession(kind) && !this.target()) return this.answer(convId, call.id, NO_TAB);
    // AI-07: nor can `send_input` while another conversation's turn holds the tab's terminal.
    const taken = kind === "send_input" ? this.terminalTaken() : null;
    if (taken) return this.answer(convId, call.id, taken);
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
      if (mcp && getSlot(this.slotId).mcpOff.includes(mcp.server_id))
        return this.answer(convId, call.id, mcpOffResult(mcp.server_name));
    }
    // AI-19: the approvals the user chose for this conversation and saved on this device, which name the
    // tab's host for commands and tools; and §13.5's commands that only read, in the tab's shell's syntax.
    const target = this.target();
    const host = target ? hostRefOf(target.tab) : null;
    const dialect = dialectOf(target?.tab.local?.kind);
    const mode = callMode(this.mode, getSlot(this.slotId).mode);
    const saved = kind !== "mcp" && needsApproval(kind, mode) ? await savedAllowances().catch(() => []) : [];
    if (!this.acting) return;
    const allowed = callAllowed(
      kind,
      parseArgs(call.arguments) ?? {},
      mcp,
      useAi.getState().allowed[convId] ?? [],
      saved,
      host,
      dialect,
    );
    let edited: string | null = null;
    let judged = false;
    if (mustAsk({ kind, mode, allowed, mcp })) {
      // AI-47: in Auto, the judge decides first whether the card shows, unless a hard rule shows it.
      const auto: AutoOutcome | null =
        mode === "auto" ? await this.judge(convId, call, kind, mcp) : { run: false, ask: null, judged: false };
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
        const scope = allow ? allowanceScope(kind, parseArgs(edited ?? call.arguments) ?? {}, mcp, dialect) : null;
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
    const leave = await this.enterRoom(call.id, kind);
    if (!leave) return;
    this.setCall({ id: call.id, state: "running" });
    try {
      await this.execute(convId, call, kind, edited, judged);
    } finally {
      leave();
    }
  }

  /**
   * §13.1: room on the tab for a call that runs alone, when it opens an exec channel or searches the web. The tab's
   * other conversations may hold all of it with their own calls (AI-07); the call then waits, queued, and no longer
   * shows the card it was approved on. Resolves with what gives the room back once the call ends, or null when the
   * turn stopped meanwhile.
   */
  private async enterRoom(callId: string, kind: ToolKind): Promise<(() => void) | null> {
    const limit = limitOf(kind, searchesNatively(useAi.getState().catalog.settings));
    if (!limit) return () => {};
    if (this.room.full(limit)) this.endCall(callId);
    return this.room.enter(limit, () => this.acting);
  }

  /**
   * AI-47: what Auto mode does with a call manual approval would ask about. A hard rule, or a call the user
   * rejected earlier in the response, shows the card at once, without the judge; otherwise the call waits for
   * the judge's verdict (`judging`), asked for here or ahead (`prefetch`), runs on `run`, and shows the card with
   * the verdict on `ask`. A switch to manual approval meanwhile applies to the call (§13.5): its card shows as in
   * manual approval, and its result goes without the verdict. Null when the turn stopped meanwhile.
   */
  private async judge(
    convId: string,
    call: AiToolCall,
    kind: ToolKind,
    mcp: McpToolInfo | null,
  ): Promise<AutoOutcome | null> {
    const prefetched = this.verdicts.get(call.id);
    this.verdicts.delete(call.id);
    const screen = kind === "send_input" ? this.screenLines() : null;
    // A `call_mcp_tool` call's rules read the arguments of the tool it names (AI-45).
    const args = parseArgs(shownCall(call).arguments) ?? {};
    const dialect = dialectOf(this.target()?.tab.local?.kind);
    const rule: HardRule | null =
      hardRule({ kind, args, mcp, screen, dialect }) ?? (this.rejected ? { kind: "after_reject" } : null);
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
    return verdict.decision === "run"
      ? { run: true, ask: null, judged: true }
      : { run: false, ask: { verdict }, judged: true };
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
    const dialect = dialectOf(target?.tab.local?.kind);
    const grants = useAi.getState().allowed[convId] ?? [];
    const saved = useAllowances.getState().list ?? [];
    for (const call of calls) {
      const kind = toolKind(call.name);
      if (!PREFETCHED.includes(kind) || this.verdicts.has(call.id) || (needsSession(kind) && !target)) continue;
      const args = parseArgs(call.arguments) ?? {};
      const allowed = callAllowed(kind, args, null, grants, saved, host, dialect);
      if (!mustAsk({ kind, mode: "auto", allowed }) || hardRule({ kind, args, dialect })) continue;
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
    const error = (content: string): AiToolResultInput => ({
      status: "error",
      content,
      edited_arguments: edited,
      judged,
    });
    if (kind === "unknown") return this.report(convId, call.id, error(`There is no tool named "${call.name}".`));

    const target = this.target();
    if (needsSession(kind)) {
      if (!target) return this.report(convId, call.id, error(NO_TAB));
      if (!target.session || target.session.status !== "connected" || !target.tab.sessionId)
        return this.report(convId, call.id, error(DISCONNECTED));
    }
    try {
      if (kind === "read_terminal")
        return this.report(convId, call.id, {
          ...readTerminal(target!.session!, args),
          edited_arguments: edited,
          judged,
        });
      if (kind === "send_input") {
        // Another turn may have taken the terminal while this call waited for approval. This turn takes it once it
        // types, so a call that types nothing (invalid arguments, a write that failed) leaves it as it was.
        const taken = this.terminalTaken();
        if (taken) return this.report(convId, call.id, error(taken));
        const result = await sendInput(target!.session!, args, this.abort.signal, () => this.holdTerminal());
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
      else if (kind === "edit_file" || kind === "write_file") void refreshFileChanges(convId);
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

/** The user's answer to an approval card (AI-17). */
export function decideCall(slotId: string, decision: Decision) {
  runners.get(slotId)?.decide(decision);
}
