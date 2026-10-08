import type { MessageKey, Params } from "@/i18n";
import type { AiPermissionMode, McpToolInfo } from "@/ipc/types";

/** Built-in tool names (§13.4, §13.8); MCP tools are `mcp__<server>__<tool>` (§13.9). */
export const READ_TERMINAL = "read_terminal";
export const RUN_COMMAND = "run_command";
export const SEND_INPUT = "send_input";
export const WEB_SEARCH = "web_search";
export const FETCH_URL = "fetch_url";
export const READ_SKILL = "read_skill";

export type ToolKind = "read_terminal" | "run_command" | "send_input" | "web_search" | "fetch_url" | "read_skill" | "mcp" | "unknown";

const BUILTIN: readonly string[] = [READ_TERMINAL, RUN_COMMAND, SEND_INPUT, WEB_SEARCH, FETCH_URL, READ_SKILL];

export function toolKind(name: string): ToolKind {
  if (BUILTIN.includes(name)) return name as ToolKind;
  if (/^mcp__.+__.+/.test(name)) return "mcp";
  return "unknown";
}

/** `read_terminal` and `send_input` need the tab's xterm instance, so the frontend runs them (§13.1). */
export function runsInFrontend(kind: ToolKind): boolean {
  return kind === "read_terminal" || kind === "send_input";
}

/** Tools that act on the tab's SSH session and return an error while it is disconnected (AI-08). */
export function needsSession(kind: ToolKind): boolean {
  return kind === "read_terminal" || kind === "send_input" || kind === "run_command";
}

/**
 * §13.5: in manual approval mode `read_terminal`, `web_search` and `read_skill` run without asking and
 * everything else waits for the user; bypass runs everything. Unknown tools never run, so never ask.
 */
export function needsApproval(kind: ToolKind, mode: AiPermissionMode): boolean {
  if (mode === "bypass") return false;
  return kind === "run_command" || kind === "send_input" || kind === "fetch_url" || kind === "mcp";
}

/** What decides whether a call waits for the user. */
export interface AskInput {
  kind: ToolKind;
  mode: AiPermissionMode;
  /** AI-19: the user chose Allow for this conversation on an earlier call of this tool. */
  allowedHere: boolean;
  /** An MCP call's tool, or null when no running server offers it. */
  mcp?: McpToolInfo | null;
}

/**
 * §13.5 with AI-19 and AI-31. Allow for this conversation wins, since the user chose it on a card of
 * this very tool. An MCP tool asks in manual mode unless it is set to Always allow on this device, and
 * in bypass mode only when its server is set to Always ask. An MCP name that no running server offers
 * never asks: Rust answers it with an error result.
 */
export function mustAsk({ kind, mode, allowedHere, mcp }: AskInput): boolean {
  if (allowedHere) return false;
  if (kind !== "mcp") return needsApproval(kind, mode);
  if (!mcp) return false;
  return mode === "bypass" ? mcp.always_ask : !mcp.tool.always_allow;
}

/** `mcp__server__tool` → `["server", "tool"]`. */
export function splitMcpName(name: string): [string, string] | null {
  const m = /^mcp__(.+?)__(.+)$/.exec(name);
  return m ? [m[1], m[2]] : null;
}

const TOOL_LABEL: Record<Exclude<ToolKind, "mcp" | "unknown">, MessageKey> = {
  read_terminal: "ai.tool.read_terminal",
  run_command: "ai.tool.run_command",
  send_input: "ai.tool.send_input",
  web_search: "ai.tool.web_search",
  fetch_url: "ai.tool.fetch_url",
  read_skill: "ai.tool.read_skill",
};

/** The name a call shows with: the built-in tool's label, `server · tool` for MCP, or the raw name. */
export function toolLabel(t: (key: MessageKey, params?: Params) => string, name: string): string {
  const kind = toolKind(name);
  if (kind === "mcp") {
    const [server, tool] = splitMcpName(name) ?? ["", name];
    return t("ai.tool.mcp", { server, tool });
  }
  return kind === "unknown" ? name : t(TOOL_LABEL[kind]);
}

// ───────────── arguments ─────────────

export type Parsed<T> = { ok: true; value: T } | { ok: false; error: string };

/** The call's arguments as an object. An empty string counts as `{}` (some providers send it for no arguments). */
export function parseArgs(json: string): Record<string, unknown> | null {
  if (!json.trim()) return {};
  try {
    const v: unknown = JSON.parse(json);
    return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

/** An integer argument: a number or a numeric string, rounded down and clamped; `fallback` when absent. */
function intArg(v: unknown, fallback: number, min: number, max: number): number | null {
  if (v === undefined || v === null || v === "") return fallback;
  const n = typeof v === "number" ? v : typeof v === "string" && v.trim() !== "" ? Number(v) : NaN;
  if (!Number.isFinite(n)) return null;
  return Math.min(max, Math.max(min, Math.floor(n)));
}

export const READ_TERMINAL_DEFAULT_LINES = 100;
export const READ_TERMINAL_MAX_LINES = 1000;

export interface ReadTerminalArgs {
  /** Scrollback lines above the visible screen. */
  lines: number;
}

/** AI-11: `{ lines?: integer }`, 100 by default and at most 1,000. */
export function parseReadTerminal(json: string): Parsed<ReadTerminalArgs> {
  const args = parseArgs(json);
  if (!args) return { ok: false, error: "The arguments are not a JSON object." };
  const lines = intArg(args.lines, READ_TERMINAL_DEFAULT_LINES, 0, READ_TERMINAL_MAX_LINES);
  if (lines === null) return { ok: false, error: '"lines" must be an integer.' };
  return { ok: true, value: { lines } };
}

export const SEND_KEYS = ["enter", "tab", "esc", "ctrl_c", "ctrl_d", "up", "down", "left", "right"] as const;
export type SendKey = (typeof SEND_KEYS)[number];

export const SEND_INPUT_DEFAULT_WAIT = 10;
export const SEND_INPUT_MAX_WAIT = 120;

export interface SendInputArgs {
  text: string;
  key: SendKey | null;
  /** How long to wait for the output to go quiet, in seconds. */
  waitSeconds: number;
}

/** AI-13: `{ text: string, key?: SendKey, wait_seconds?: integer }`, waiting 10 s by default and at most 120 s. */
export function parseSendInput(json: string): Parsed<SendInputArgs> {
  const args = parseArgs(json);
  if (!args) return { ok: false, error: "The arguments are not a JSON object." };
  if (args.text !== undefined && args.text !== null && typeof args.text !== "string") return { ok: false, error: '"text" must be a string.' };
  const text = typeof args.text === "string" ? args.text : "";
  let key: SendKey | null = null;
  if (args.key !== undefined && args.key !== null && args.key !== "") {
    const k = typeof args.key === "string" ? args.key.toLowerCase() : "";
    if (!(SEND_KEYS as readonly string[]).includes(k)) return { ok: false, error: `"key" must be one of ${SEND_KEYS.join(", ")}.` };
    key = k as SendKey;
  }
  if (!text && !key) return { ok: false, error: 'Nothing to send: give "text", "key", or both.' };
  const waitSeconds = intArg(args.wait_seconds, SEND_INPUT_DEFAULT_WAIT, 1, SEND_INPUT_MAX_WAIT);
  if (waitSeconds === null) return { ok: false, error: '"wait_seconds" must be an integer.' };
  return { ok: true, value: { text, key, waitSeconds } };
}

/** The bytes a key sends. Arrow keys follow the application cursor mode (DECCKM) like xterm does. */
export function keySequence(key: SendKey, applicationCursor: boolean): string {
  const arrow = (c: string) => (applicationCursor ? `\x1bO${c}` : `\x1b[${c}`);
  switch (key) {
    case "enter":
      return "\r";
    case "tab":
      return "\t";
    case "esc":
      return "\x1b";
    case "ctrl_c":
      return "\x03";
    case "ctrl_d":
      return "\x04";
    case "up":
      return arrow("A");
    case "down":
      return arrow("B");
    case "right":
      return arrow("C");
    case "left":
      return arrow("D");
  }
}

// ───────────── display ─────────────

/** One line that says what a call does, for the collapsed tool block. */
export function callSummary(name: string, json: string): string {
  const args = parseArgs(json) ?? {};
  const str = (v: unknown) => (typeof v === "string" ? v : v === undefined || v === null ? "" : JSON.stringify(v));
  switch (toolKind(name)) {
    case "run_command":
      return firstLine(str(args.command));
    case "send_input":
      return [str(args.text).replace(/\r?\n/g, "⏎"), args.key ? `[${str(args.key)}]` : ""].filter(Boolean).join(" ");
    case "web_search":
      return str(args.query);
    case "fetch_url":
      return str(args.url);
    case "read_skill":
      return [str(args.name), str(args.path)].filter(Boolean).join(" / ");
    case "read_terminal":
      return args.lines !== undefined ? str(args.lines) : "";
    default: {
      const text = json.trim();
      return text === "{}" ? "" : firstLine(text);
    }
  }
}

function firstLine(s: string): string {
  const lines = s.split(/\r?\n/);
  return lines.length > 1 ? `${lines[0]} …` : lines[0];
}

/** Arguments as indented JSON for display, or the raw text when they are not JSON. */
export function prettyArgs(json: string): string {
  try {
    return JSON.stringify(JSON.parse(json), null, 2);
  } catch {
    return json;
  }
}

/** The exit status a `run_command` result reports, when its text names one. */
export function exitStatusOf(content: string): number | null {
  const m = /exit (?:status|code)\s*[:=]?\s*(-?\d+)/i.exec(content.slice(0, 400));
  return m ? Number(m[1]) : null;
}
