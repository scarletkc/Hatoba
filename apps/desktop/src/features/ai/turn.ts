import type { AiEntryView, AiToolCall, AiTurnEvent, AiUsage, McpToolInfo } from "@/ipc/types";

/** A response while it streams, before Rust stores it as an entry (§13.1 step 2). */
export interface LiveResponse {
  text: string;
  reasoning: string;
  toolCalls: AiToolCall[];
  usage: AiUsage | null;
}

/** What the current tool call is waiting for. */
export type CallState = "approval" | "running" | "limit";

/** The tool call being handled. */
export interface CallInfo {
  id: string;
  state: CallState;
  /** An MCP call's tool, for its approval card (AI-31); null when no running server offers it. */
  mcp?: McpToolInfo | null;
}

export interface TurnState {
  /** `starting`: `ai_send` / `ai_retry` is on its way. `waiting`: a request is sent, nothing streamed yet. */
  phase: "starting" | "waiting" | "streaming" | "tools";
  live: LiveResponse | null;
  call: CallInfo | null;
  /** Tool calls handled since the turn started or the user chose Continue (AI-18). */
  toolCount: number;
  /** An `error` event, reported when the turn ends. */
  error: { status: number | null; message: string } | null;
}

/** How the last turn ended, when the panel has something to say about it. */
export type TurnOutcome =
  | { reason: "stopped" | "length" | "refused" }
  | { reason: "error"; status: number | null; message: string };

export interface TurnSnapshot {
  entries: AiEntryView[];
  turn: TurnState | null;
  outcome: TurnOutcome | null;
  /** The conversation's `context_start` (AI-21). */
  contextStart: string | null;
}

export function newTurn(): TurnState {
  return { phase: "starting", live: null, call: null, toolCount: 0, error: null };
}

const emptyLive = (): LiveResponse => ({ text: "", reasoning: "", toolCalls: [], usage: null });

/** Adds an entry, or replaces the one with the same id (entries arrive both as events and as command results). */
export function upsertEntry(entries: AiEntryView[], entry: AiEntryView): AiEntryView[] {
  const i = entries.findIndex((e) => e.entry_id === entry.entry_id);
  if (i < 0) return [...entries, entry];
  const next = entries.slice();
  next[i] = entry;
  return next;
}

/** Applies one streamed event (§13.1, the IPC contract's turn protocol). */
export function reduceTurnEvent(s: TurnSnapshot, ev: AiTurnEvent): TurnSnapshot {
  const turn = s.turn;
  switch (ev.kind) {
    case "request_started":
      return { ...s, outcome: null, turn: { ...(turn ?? newTurn()), phase: "waiting", live: emptyLive(), error: null } };
    case "text":
    case "reasoning":
    case "tool_call":
    case "usage": {
      if (!turn) return s;
      const live = turn.live ?? emptyLive();
      const next: LiveResponse =
        ev.kind === "text"
          ? { ...live, text: live.text + ev.delta }
          : ev.kind === "reasoning"
            ? { ...live, reasoning: live.reasoning + ev.delta }
            : ev.kind === "tool_call"
              ? { ...live, toolCalls: [...live.toolCalls, { id: ev.id, name: ev.name, arguments: ev.arguments }] }
              : { ...live, usage: { input_tokens: ev.input_tokens, output_tokens: ev.output_tokens, estimated: ev.estimated } };
      return { ...s, turn: { ...turn, phase: "streaming", live: next } };
    }
    case "entry": {
      const entries = upsertEntry(s.entries, ev.entry);
      // A stored summary is where the context starts now: Compact, or automatic compaction (AI-22).
      if (ev.entry.role === "summary") return { ...s, entries, contextStart: ev.entry.entry_id };
      if (!turn || ev.entry.role !== "assistant") return { ...s, entries };
      return { ...s, entries, turn: { ...turn, live: null } };
    }
    case "done":
      if (!turn) return s;
      return { ...s, turn: { ...turn, live: null, phase: ev.finish === "tool_calls" ? "tools" : turn.phase } };
    case "error":
      if (!turn) return { ...s, outcome: { reason: "error", status: ev.status, message: ev.message } };
      return { ...s, turn: { ...turn, error: { status: ev.status, message: ev.message } } };
    case "turn_ended": {
      let outcome: TurnOutcome | null;
      switch (ev.reason) {
        case "completed":
          outcome = null;
          break;
        case "error":
          outcome = turn?.error
            ? { reason: "error", ...turn.error }
            : s.outcome?.reason === "error"
              ? s.outcome
              : { reason: "error", status: null, message: "" };
          break;
        default:
          outcome = { reason: ev.reason };
      }
      return { ...s, turn: null, outcome };
    }
  }
}

/**
 * Of two `context_start` values, the one further down the conversation. A command's conversation view
 * can be older than a summary its turn already stored (AI-22), so it must not move the start back.
 */
export function laterContextStart(entries: AiEntryView[], a: string | null, b: string | null): string | null {
  if (!a || !b || a === b) return a ?? b;
  const ia = entries.findIndex((e) => e.entry_id === a);
  const ib = entries.findIndex((e) => e.entry_id === b);
  if (ia < 0) return b;
  if (ib < 0) return a;
  return ia > ib ? a : b;
}

/** The entries before `entryId` (AI-26: everything from the edited message on is replaced). */
export function entriesBefore(entries: AiEntryView[], entryId: string): AiEntryView[] {
  const i = entries.findIndex((e) => e.entry_id === entryId);
  return i < 0 ? entries : entries.slice(0, i);
}

/** How many messages follow `entryId`: the user's, the assistant's and summaries (tool results belong to their call). */
export function laterMessages(entries: AiEntryView[], entryId: string): number {
  const i = entries.findIndex((e) => e.entry_id === entryId);
  return i < 0 ? 0 : entries.slice(i + 1).filter((e) => e.role !== "tool").length;
}

/** The tool calls of the newest assistant entry that have no result yet, in order. */
export function unansweredCalls(entries: AiEntryView[]): AiToolCall[] {
  for (let i = entries.length - 1; i >= 0; i--) {
    const e = entries[i];
    if (e.role !== "assistant") continue;
    const answered = new Set(entries.slice(i + 1).flatMap((x) => (x.role === "tool" ? [x.tool_call_id] : [])));
    return e.tool_calls.filter((c) => !answered.has(c.id));
  }
  return [];
}
