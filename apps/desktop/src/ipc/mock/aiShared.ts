import { blockTitle, parseMessage } from "@/lib/ai/attachments";
import { appError, type HatobaApi } from "../api";
import type {
  AiAutoVerdict,
  AiConversationView,
  AiEntryView,
  AiFinish,
  AiToolCall,
  AiTurnContext,
  AiTurnEvent,
  AppError,
  HostView,
  LocalShellKind,
  QuickTarget,
} from "../types";
import type { AiMemoryMock } from "./aiMemory";
import { mockLocale } from "./visitor";

export type AiApi = Pick<
  HatobaApi,
  | "ai_conversations_list"
  | "ai_conversation_get"
  | "ai_conversation_rename"
  | "ai_conversation_pin"
  | "ai_conversation_delete"
  | "ai_conversation_fork"
  | "ai_send"
  | "ai_retry"
  | "ai_tool_result"
  | "ai_tool_run"
  | "ai_judge"
  | "ai_file_preview"
  | "ai_file_changes"
  | "ai_file_change_diff"
  | "ai_file_revert"
  | "ai_session_server"
  | "ai_stop"
  | "ai_compact"
  | "ai_side_question"
  | "ai_side_stop"
  | "ai_search"
  | "ai_edit_resend"
  | "ai_read_dropped_files"
>;

/** What the conversation mock reads from the settings and MCP mocks. */
export interface AiMockDeps {
  providers: HatobaApi["ai_providers_list"];
  settings: HatobaApi["ai_settings_get"];
  /** The hosts mock's list, whose names the notes of a move to another host carry (AI-09). */
  hosts?: () => Promise<HostView[]>;
  /** The server a live session of the shell mock is connected to, as file changes name it (AI-41). */
  sessionServer?: (sessionId: string) => string | null;
  /** The MCP servers of the extensions mock, whose tools the fake model calls. */
  mcp?: {
    servers: HatobaApi["mcp_servers_list"];
    status: HatobaApi["mcp_server_status"];
    start: HatobaApi["mcp_server_start"];
    toolInfo: HatobaApi["mcp_tool_info"];
  };
  /** The memory mock's `memory` tool (AI-53), in a conversation on `place`. */
  memory?: AiMemoryMock["memory_tool"];
}

export type Assistant = Extract<AiEntryView, { role: "assistant" }>;
export type ToolEntry = Extract<AiEntryView, { role: "tool" }>;

export interface Conv {
  view: Omit<AiConversationView, "last_activity">;
  entries: AiEntryView[];
  /** `?ai=running`: a turn "runs in Rust" until then, started by an earlier page. */
  runningUntil: number;
}

/**
 * Like Rust's `EntryPlace` (AI-43), which a user entry records and the view does not carry: where the conversation
 * was when the message was stored, before its move.
 */
export type Recorded = Pick<AiConversationView, "host_id" | "quick_target" | "origin" | "local">;

export const placeOf = (c: Recorded): Recorded => ({
  host_id: c.host_id,
  quick_target: c.quick_target,
  origin: c.origin,
  local: c.local,
});

/** Like Rust's `Place` (AI-09): where a message moves its conversation, and where from (`moveOf`). */
export interface Move {
  from: string | null;
  host_id: string | null;
  quick_target: QuickTarget | null;
  /** §7.5: the computer of a local tab. */
  local: string | null;
}

export interface Turn {
  convId: string;
  emit: (ev: AiTurnEvent) => void;
  context: AiTurnContext;
  ended: boolean;
  timers: Set<number>;
  /** Pending `sleep`s, which end (with false) when the turn ends. */
  sleepers: Set<() => void>;
  /** AI-22 ran once in this turn. */
  compacted: boolean;
  /** What the response being streamed has sent so far, which a stop stores (§13.1), and the MCP servers its request offered directly. */
  streamed: {
    provider_id: string;
    model_id: string;
    text: string;
    calls: AiToolCall[];
    servers: string[] | null;
  } | null;
}

export interface Plan {
  reasoning: string;
  text: string;
  calls: AiToolCall[];
  finish: AiFinish;
}

export const MIN = 60_000;
export const HOUR = 60 * MIN;
export const DAY = 24 * HOUR;
/** The file the "cutshort" write leaves cut short (AI-41). */
export const CUT_SHORT = "/srv/app/conf.d/cache.conf";
/** The file "write" creates, which "symlink" puts a link in the place of (AI-41). */
export const MAINTENANCE = "/srv/app/public/maintenance.html";
/** What "disk" runs, in the syntax of the tab's shell (§7.5); SSH tabs are `unix`. */
export const DISK_COMMAND: Record<LocalShellKind, string> = {
  unix: "df -h /",
  git_bash: "df -h /",
  wsl: "df -h /",
  powershell: "Get-PSDrive -PSProvider FileSystem | Format-Table Name, Used, Free",
  cmd: "dir C:\\ | findstr bytes",
};

export function chunks(text: string): string[] {
  const out: string[] = [];
  for (let i = 0; i < text.length;) {
    const n = 3 + Math.floor(Math.random() * 7);
    out.push(text.slice(i, i + n));
    i += n;
  }
  return out;
}

export function lastUser(c: Conv): { text: string; index: number } {
  for (let i = c.entries.length - 1; i >= 0; i--) {
    const e = c.entries[i];
    if (e.role === "user") return { text: e.text, index: i };
  }
  return { text: "", index: -1 };
}

/**
 * Like Rust's `title_of` (AI-23): the first non-empty line typed after the attachment blocks
 * (AI-10), or the first block's kind when nothing was typed, cut to 60 characters.
 */
export function titleOf(text: string): string {
  const { attachments, typed } = parseMessage(text);
  const block = attachments[0];
  const fallback = block ? blockTitle(block) : "";
  const line =
    typed
      .split(/\r?\n/)
      .find((l) => l.trim())
      ?.trim() ?? fallback;
  return [...line].slice(0, 60).join("");
}

/** About 40 characters before the first match of `q` (lowercase) in `text` and 100 after, on one line. */
export function snippetAround(text: string, q: string): string {
  const flat = text.replace(/\s+/g, " ").trim();
  const at = flat.toLowerCase().indexOf(q);
  const start = Math.max(0, at - 40);
  const end = Math.min(flat.length, Math.max(at, 0) + q.length + 100);
  return `${start > 0 ? "…" : ""}${flat.slice(start, end)}${end < flat.length ? "…" : ""}`;
}

export function createAiShared(deps: AiMockDeps) {
  const flags = new Set((new URLSearchParams(location.search).get("ai") ?? "").split(",").map((x) => x.trim()));
  const zh = mockLocale() === "zh-CN";
  const tw = mockLocale() === "zh-TW";
  const ja = mockLocale() === "ja";
  /** The text in the mock's language. */
  const say = (en: string, zhText: string, jaText: string, twText: string) =>
    zh ? zhText : tw ? twText : ja ? jaText : en;
  /** The text of a state that has no Japanese, which stays in English there. */
  const sayZh = (en: string, zhText: string, twText: string) => (zh ? zhText : tw ? twText : en);
  const convs = new Map<string, Conv>();
  const turns = new Map<string, Turn>();
  /** Conversations being compacted (AI-21, AI-22), which refuse a fork (AI-43) and a side question (AI-42). */
  const compacting = new Set<string>();
  /** By reply entry id: whether its request offered the terminal tools (AI-09). */
  const replyTerminal = new Map<string, boolean>();
  /** By reply entry id: the local shell whose terminal tools its request offered (§7.5). */
  const replyShell = new Map<string, string>();
  /** By reply entry id: the servers whose MCP tools its request offered directly (AI-30). */
  const replyServers = new Map<string, string[]>();
  /** By user entry id: where the entries before it came from (`Recorded`). */
  const cameFrom = new Map<string, Recorded>();
  /** Like Rust's `AssistantEntry.interrupted`, which the view does not carry: replies the user stopped (§13.1). */
  const interrupted = new Set<string>();
  /** AI-42: the side question on its way in each conversation, by the panel's number, and what stops it. */
  const sides = new Map<string, { question: number; stop: () => void }>();
  /** AI-47: by call id, the verdict `ai_judge` gave, which a result stored as `judged` keeps, as Rust does. */
  const verdicts = new Map<string, AiAutoVerdict>();
  let lastMs = 0;
  let seq = 0;

  /** Increasing ids, like the UUIDv7 entry ids Rust makes. */
  const newId = (ms = Date.now()) => {
    lastMs = Math.max(ms, lastMs + 1);
    return `${lastMs.toString(16).padStart(12, "0")}-7${(++seq).toString(16).padStart(5, "0")}`;
  };
  const fail = (code: AppError["code"], detail: string, extra: Partial<AppError> = {}): never => {
    throw appError(code, detail, extra);
  };
  const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));

  function need(id: string): Conv {
    return convs.get(id) ?? fail("not_found", "conversation not found");
  }

  function view(c: Conv): AiConversationView {
    const last = c.entries.reduce((m, e) => Math.max(m, e.created_at), 0);
    return { ...c.view, last_activity: Math.max(c.view.updated_at, last) };
  }

  function answered(c: Conv): Set<string> {
    return new Set(c.entries.flatMap((e) => (e.role === "tool" ? [e.tool_call_id] : [])));
  }

  function unanswered(c: Conv): AiToolCall[] {
    const done = answered(c);
    return c.entries.flatMap((e) => (e.role === "assistant" ? e.tool_calls.filter((x) => !done.has(x.id)) : []));
  }

  function store(c: Conv, entry: AiEntryView) {
    c.entries.push(entry);
    turns.get(c.view.id)?.emit({ kind: "entry", entry });
  }

  /** Resolves false once the turn has ended, also when it ends while sleeping. */
  function sleep(turn: Turn, ms: number): Promise<boolean> {
    return new Promise((resolve) => {
      if (turn.ended) return resolve(false);
      const wake = () => resolve(false);
      const t = window.setTimeout(() => {
        turn.timers.delete(t);
        turn.sleepers.delete(wake);
        resolve(!turn.ended);
      }, ms);
      turn.timers.add(t);
      turn.sleepers.add(wake);
    });
  }

  const call = (name: string, args: Record<string, unknown>): AiToolCall => ({
    id: `call_${(++seq).toString(36)}`,
    name,
    arguments: JSON.stringify(args),
  });

  /** Like Rust: the call must belong to the newest response and have no result yet. */
  function openCall(c: Conv, callId: string): AiToolCall {
    const newest = [...c.entries].reverse().find((e): e is Assistant => e.role === "assistant");
    const x = newest?.tool_calls.find((y) => y.id === callId) ?? fail("not_found", "tool call not found");
    if (answered(c).has(callId)) fail("invalid_input", "the tool call already has a result", { field: "tool_call_id" });
    return x;
  }

  return {
    deps,
    flags,
    say,
    sayZh,
    convs,
    turns,
    compacting,
    replyTerminal,
    replyShell,
    replyServers,
    cameFrom,
    interrupted,
    sides,
    verdicts,
    newId,
    fail,
    delay,
    need,
    view,
    answered,
    unanswered,
    store,
    sleep,
    call,
    openCall,
  };
}

export type AiMockCtx = ReturnType<typeof createAiShared>;
