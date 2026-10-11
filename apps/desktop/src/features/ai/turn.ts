import type { AiEntryView, AiFileChange, AiToolCall, AiTurnEvent, AiUsage, McpToolInfo } from "@/ipc/types";

/** A response while it streams, before Rust stores it as an entry (§13.1 step 2). */
export interface LiveResponse {
  text: string;
  reasoning: string;
  toolCalls: AiToolCall[];
  usage: AiUsage | null;
}

/** What the current tool call is waiting for. */
export type CallState = "approval" | "running";

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
  /** An `error` event, reported when the turn ends. */
  error: { status: number | null; message: string } | null;
}

/** How the last turn ended, when the panel has something to say about it. `empty`: the reply had no answer (§13.1). */
export type TurnOutcome =
  | { reason: "stopped" | "length" | "refused" | "empty" }
  | { reason: "error"; status: number | null; message: string };

export interface TurnSnapshot {
  entries: AiEntryView[];
  turn: TurnState | null;
  outcome: TurnOutcome | null;
  /** The conversation's `context_start` (AI-21). */
  contextStart: string | null;
  /** The provider refused the thinking level, and the turn went on without it (AI-05). */
  effortIgnored?: boolean;
}

export function newTurn(): TurnState {
  return { phase: "starting", live: null, call: null, error: null };
}

const emptyLive = (): LiveResponse => ({ text: "", reasoning: "", toolCalls: [], usage: null });

/** The id of the user's message the panel shows until `ai_send` / `ai_edit_resend` returns the stored entry. */
export const LOCAL_ENTRY = "~local";

/** An entry the panel shows before Rust stored it; it stays after every stored entry. */
const isPending = (e: AiEntryView) => e.entry_id.startsWith("~");

/**
 * Adds an entry, or replaces the one with the same id (entries arrive both as events and as command
 * results). A new entry goes where its id puts it: stored entries are ordered by `entry_id` (UUIDv7,
 * §13.7), and what Rust stores before the user's pending message (a stopped turn's cancelled results, a
 * summary of AI-22) comes before it, whichever reaches the panel first.
 */
export function upsertEntry(entries: AiEntryView[], entry: AiEntryView): AiEntryView[] {
  const i = entries.findIndex((e) => e.entry_id === entry.entry_id);
  if (i >= 0) {
    const next = entries.slice();
    next[i] = entry;
    return next;
  }
  if (isPending(entry)) return [...entries, entry];
  const at = entries.findIndex((e) => isPending(e) || e.entry_id > entry.entry_id);
  return at < 0 ? [...entries, entry] : [...entries.slice(0, at), entry, ...entries.slice(at)];
}

/** The stored user entry in place of the one shown while the command was on its way, in its stored order. */
export function placeUserEntry(entries: AiEntryView[], entry: AiEntryView): AiEntryView[] {
  return upsertEntry(
    entries.filter((e) => e.entry_id !== LOCAL_ENTRY),
    entry,
  );
}

/** Applies one streamed event (§13.1, the IPC contract's turn protocol). */
export function reduceTurnEvent(s: TurnSnapshot, ev: AiTurnEvent): TurnSnapshot {
  const turn = s.turn;
  switch (ev.kind) {
    case "opened":
      return s;
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
    case "effort_ignored":
      return { ...s, effortIgnored: true };
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

/** What an edit of the user's message would do to files the assistant changed (AI-26, AI-41). */
export interface EditedFiles {
  /** The changes of the calls the edit deletes that this device can still revert, oldest call first. */
  revert: AiFileChange[];
  /**
   * The `edit_file` and `write_file` calls the edit deletes that wrote, by their result, and whose change this
   * device does not keep: another device ran them, or the size limit dropped it.
   */
  untracked: number;
}

/** The file changes an edit of `entryId` deletes the calls of: those this device can revert, and how many others stay on the host. */
export function editedFiles(entries: AiEntryView[], changes: AiFileChange[], entryId: string): EditedFiles {
  const i = entries.findIndex((e) => e.entry_id === entryId);
  if (i < 0) return { revert: [], untracked: 0 };
  const deleted = entries.slice(i + 1);
  const responses = new Set(deleted.flatMap((e) => (e.role === "assistant" ? [e.entry_id] : [])));
  const kept = changes.filter((c) => responses.has(c.response_id));
  const keyOf = (c: AiFileChange) => `${c.response_id}:${c.tool_call_id}`;
  const known = new Set(kept.filter((c) => c.state !== "dropped").map(keyOf));
  // A dropped change stays on the host too, also one whose write failed partway (its result is an error).
  const staying = new Set(kept.filter((c) => c.state === "dropped").map(keyOf));
  let response: (AiEntryView & { role: "assistant" }) | null = null;
  for (const e of deleted) {
    if (e.role === "assistant") response = e;
    if (e.role !== "tool" || e.status !== "ok" || !response) continue;
    const call = response.tool_calls.find((c) => c.id === e.tool_call_id);
    const key = `${response.entry_id}:${e.tool_call_id}`;
    if ((call?.name === "edit_file" || call?.name === "write_file") && !known.has(key)) staying.add(key);
  }
  return { revert: kept.filter((c) => c.state === "written" || c.state === "unfinished"), untracked: staying.size };
}

/** The `data-reveal` key of a call's block: by its response too, since responses may reuse call ids. */
export function callRevealKey(responseId: string, callId: string): string {
  return `call:${responseId}:${callId}`;
}

/**
 * What a reveal of `entry` scrolls to and highlights (AI-24, AI-41): the entry, or for a tool result the block of
 * its call, in the newest response before it with that call id. Null for a result without one.
 */
export function revealKey(entries: AiEntryView[], entry: AiEntryView): string | null {
  if (entry.role !== "tool") return `entry:${entry.entry_id}`;
  const at = entries.indexOf(entry);
  const response = entries.slice(0, at < 0 ? entries.length : at).findLast((e) => e.role === "assistant" && e.tool_calls.some((c) => c.id === entry.tool_call_id));
  return response ? callRevealKey(response.entry_id, entry.tool_call_id) : null;
}

/**
 * The tool calls of the newest assistant entry that have no result yet, in order. Results answer the
 * calls with their id in order, as Rust pairs them, so of calls that share an id the first ones are
 * answered first.
 */
export function unansweredCalls(entries: AiEntryView[]): AiToolCall[] {
  for (let i = entries.length - 1; i >= 0; i--) {
    const e = entries[i];
    if (e.role !== "assistant") continue;
    const results = new Map<string, number>();
    for (const x of entries.slice(i + 1)) if (x.role === "tool") results.set(x.tool_call_id, (results.get(x.tool_call_id) ?? 0) + 1);
    return e.tool_calls.filter((c) => {
      const left = results.get(c.id) ?? 0;
      if (left > 0) results.set(c.id, left - 1);
      return left === 0;
    });
  }
  return [];
}
