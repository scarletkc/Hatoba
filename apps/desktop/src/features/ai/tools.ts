import type { MessageKey, Params } from "@/i18n";
import type { AiFilePreview, AiPermissionMode, McpToolInfo } from "@/ipc/types";

/**
 * Built-in tool names (§13.4, §13.8); MCP tools are `mcp__<server>__<tool>` (§13.9), called directly or,
 * once a conversation's MCP tools are deferred, through `call_mcp_tool` (AI-45).
 */
export const READ_TERMINAL = "read_terminal";
export const RUN_COMMAND = "run_command";
export const SEND_INPUT = "send_input";
export const READ_FILE = "read_file";
export const EDIT_FILE = "edit_file";
export const WRITE_FILE = "write_file";
export const WEB_SEARCH = "web_search";
export const FETCH_URL = "fetch_url";
export const READ_SKILL = "read_skill";
export const SEARCH_TOOLS = "search_tools";
export const CALL_MCP_TOOL = "call_mcp_tool";

export type ToolKind =
  | "read_terminal"
  | "run_command"
  | "send_input"
  | "read_file"
  | "edit_file"
  | "write_file"
  | "web_search"
  | "fetch_url"
  | "read_skill"
  | "search_tools"
  | "mcp"
  | "unknown";

const BUILTIN: readonly string[] = [READ_TERMINAL, RUN_COMMAND, SEND_INPUT, READ_FILE, EDIT_FILE, WRITE_FILE, WEB_SEARCH, FETCH_URL, READ_SKILL, SEARCH_TOOLS];

/** A call's kind; a `call_mcp_tool` call (AI-45) is an MCP call, approved as the tool it names. */
export function toolKind(name: string): ToolKind {
  if (BUILTIN.includes(name)) return name as ToolKind;
  if (name === CALL_MCP_TOOL || /^mcp__.+__.+/.test(name)) return "mcp";
  return "unknown";
}

/**
 * AI-45: the MCP tool a `call_mcp_tool` call names, and that tool's arguments as JSON: an object, or a
 * string that holds one, `{}` when left out, as Rust reads them. Null for any other call, or one whose
 * arguments name no MCP tool: Rust runs no other tool through it (a call that names `run_command`
 * returns an error), so such a call is shown as it is.
 */
export function proxyCall(name: string, json: string): { tool: string; arguments: string } | null {
  if (name !== CALL_MCP_TOOL) return null;
  const args = parseArgs(json);
  const tool = typeof args?.tool === "string" ? args.tool.trim() : "";
  if (!args || !splitMcpName(tool)) return null;
  const inner = args.arguments;
  if (inner === undefined || inner === null || (typeof inner === "string" && !inner.trim())) return { tool, arguments: "{}" };
  if (typeof inner === "string") {
    const parsed = parseArgs(inner);
    return { tool, arguments: parsed ? JSON.stringify(parsed) : inner };
  }
  return { tool, arguments: JSON.stringify(inner) };
}

/** A call as the panel and the export show it: a `call_mcp_tool` call as the MCP tool it names, with that tool's arguments (AI-45). */
export function shownCall<T extends { name: string; arguments: string }>(call: T): T {
  const proxy = proxyCall(call.name, call.arguments);
  return proxy ? { ...call, name: proxy.tool, arguments: proxy.arguments } : call;
}

/** The arguments a `call_mcp_tool` call runs with after the user edited the tool's own on its card (AI-17, AI-45). */
export function proxyArguments(json: string, edited: string): string {
  const args = parseArgs(json) ?? {};
  return JSON.stringify({ ...args, arguments: JSON.parse(edited) as unknown });
}

/** `read_terminal` and `send_input` need the tab's xterm instance, so the frontend runs them (§13.1). */
export function runsInFrontend(kind: ToolKind): boolean {
  return kind === "read_terminal" || kind === "send_input";
}

/** The file tools (AI-38…40), which run in Rust over an exec channel of the tab's connection. */
export function isFileTool(kind: ToolKind): kind is "read_file" | "edit_file" | "write_file" {
  return kind === "read_file" || kind === "edit_file" || kind === "write_file";
}

/** Tools that act on the tab's SSH session and return an error while it is disconnected (AI-08). */
export function needsSession(kind: ToolKind): boolean {
  return kind === "read_terminal" || kind === "send_input" || kind === "run_command" || isFileTool(kind);
}

/**
 * §13.5: in manual approval mode `read_terminal`, `web_search`, `read_skill` and `search_tools` (AI-45) run without asking and
 * everything else waits for the user, `read_file` included (it can send `~/.ssh` or `.env` to the
 * provider); bypass runs everything. Unknown tools never run, so never ask.
 */
export function needsApproval(kind: ToolKind, mode: AiPermissionMode): boolean {
  if (mode === "bypass") return false;
  return kind === "run_command" || kind === "send_input" || isFileTool(kind) || kind === "fetch_url" || kind === "mcp";
}

/** What decides whether a call waits for the user. */
export interface AskInput {
  kind: ToolKind;
  mode: AiPermissionMode;
  /** AI-19: the user chose Allow for this conversation on an earlier call of this tool. */
  allowedHere: boolean;
  /** An MCP call's tool (`mcp_tool_info`), or null when no offer or running server has it. */
  mcp?: McpToolInfo | null;
}

/**
 * §13.5 with AI-19 and AI-31. Allow for this conversation wins, since the user chose it on a card of
 * this very tool. An MCP tool asks in manual mode unless it is set to Always allow on this device, and
 * in bypass mode only when its server is set to Always ask. An MCP call whose tool `mcp_tool_info`
 * cannot name asks in both modes: nothing says which server it would reach.
 */
export function mustAsk({ kind, mode, allowedHere, mcp }: AskInput): boolean {
  if (allowedHere) return false;
  if (kind !== "mcp") return needsApproval(kind, mode);
  if (!mcp) return true;
  return mode === "bypass" ? mcp.always_ask : !mcp.tool.always_allow;
}

/**
 * What AI-19 Allow for this conversation remembers for a call: a built-in tool by name, an MCP tool by
 * its server and the server's own tool name (the name the model calls depends on the servers a request
 * offered, so it can mean another server later). Null for an MCP call whose tool is unknown, which
 * cannot be allowed ahead.
 */
export function grantKey(kind: ToolKind, name: string, mcp: McpToolInfo | null | undefined): string | null {
  if (kind !== "mcp") return name;
  return mcp ? `mcp:${mcp.server_id}:${mcp.tool.tool}` : null;
}

/**
 * Whether an approval card offers Allow for this conversation (AI-19). Not for an MCP call whose tool is
 * unknown, which cannot be allowed ahead, nor for an edit or write whose card shows `refusal`, why it
 * cannot apply (AI-39, AI-40): the user saw no change of the tool's.
 */
export function offersAllowHere(kind: ToolKind, mcp: McpToolInfo | null | undefined, refusal: string | undefined): boolean {
  return (kind !== "mcp" || !!mcp) && refusal === undefined;
}

/**
 * The allowance a Run decision stores (AI-19, AI-31): none when it carries a refusal, so no path grants a
 * tool from a card that showed no change.
 */
export function allowanceOf(decision: { allow?: "conversation" | "always"; refusal?: string }): "conversation" | "always" | undefined {
  return decision.refusal === undefined ? decision.allow : undefined;
}

/** The result of a call whose server was switched off in the tools menu during the turn (AI-30); the model reads it. */
export function mcpOffResult(serverName: string): string {
  return `The MCP server "${serverName}" was switched off for this conversation, so the tool did not run.`;
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
  read_file: "ai.tool.read_file",
  edit_file: "ai.tool.edit_file",
  write_file: "ai.tool.write_file",
  web_search: "ai.tool.web_search",
  fetch_url: "ai.tool.fetch_url",
  read_skill: "ai.tool.read_skill",
  search_tools: "ai.tool.search_tools",
};

/**
 * The name a call shows with: the built-in tool's label, `server · tool` for MCP, or the raw name. A
 * `call_mcp_tool` call shows as the tool it names: pass it through `shownCall` first.
 */
export function toolLabel(t: (key: MessageKey, params?: Params) => string, name: string): string {
  const kind = toolKind(name);
  if (name === CALL_MCP_TOOL) return name;
  if (kind === "mcp") {
    const [server, tool] = splitMcpName(name) ?? ["", name];
    return t("ai.tool.mcp", { server, tool });
  }
  return kind === "unknown" ? name : t(TOOL_LABEL[kind]);
}

// ───────────── arguments ─────────────

export type Parsed<T> = { ok: true; value: T } | { ok: false; error: string };

/**
 * AI-41: what an `edit_file` or `write_file` call changes, from its arguments alone, for its block when this
 * device keeps no snapshot of it: the replaced text and its replacement, or the content written. Null for other
 * tools, or arguments that do not parse.
 */
export function callFileChange(name: string, json: string): AiFilePreview | null {
  const kind = toolKind(name);
  if (kind !== "edit_file" && kind !== "write_file") return null;
  const args = parseArgs(json);
  if (!args) return null;
  const text = (v: unknown) => (typeof v === "string" ? v : "");
  // Whether write_file created the file is not in its arguments, so the content shows as added, without "new file".
  const [before, after] = kind === "edit_file" ? [text(args.old_string), text(args.new_string)] : ["", text(args.content)];
  return { path: text(args.path), before, after, error: null, crlf: false };
}

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
    case "search_tools":
      return str(args.query);
    case "read_file":
      return args.offset !== undefined && args.offset !== null ? `${str(args.path)}:${str(args.offset)}` : str(args.path);
    case "edit_file":
    case "write_file":
      return firstLine(str(args.path));
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

/**
 * Characters that are invisible or reorder what follows: C0 and C1 controls but the line break, and
 * the bidirectional and zero-width format characters.
 */
const HIDDEN = /[\u0000-\u0009\u000b-\u001f\u007f-\u009f\u061c\u200b-\u200f\u202a-\u202e\u2060-\u2069\ufeff]/g;

const SHOWN: Record<string, string> = { "\t": "\\t", "\r": "\\r" };

/** Text as the approval card shows it (AI-17): its lines, with hidden characters as visible escapes. */
export interface ShownText {
  /** The lines, split at line breaks; a final line break leaves an empty last line. */
  lines: string[];
  /** Some character was replaced by an escape. */
  escaped: boolean;
}

/**
 * The input of a call for the approval card, where nothing may hide: a tab, a carriage return or an
 * escape sequence typed into the shell, or a bidirectional override that shows the text in another
 * order than it runs, each shows as an escape (`\t`, `\r`, `\x1b`, `\u{202e}`), as command lines do in
 * Settings → AI (`formatCommandLine`).
 */
export function shownText(text: string): ShownText {
  let escaped = false;
  const shown = text.replace(HIDDEN, (c) => {
    escaped = true;
    const code = c.charCodeAt(0);
    return SHOWN[c] ?? (code <= 0xff ? `\\x${code.toString(16).padStart(2, "0")}` : `\\u{${code.toString(16)}}`);
  });
  return { lines: shown.split("\n"), escaped };
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
