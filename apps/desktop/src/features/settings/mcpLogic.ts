import type {
  McpSecretInput,
  McpServerInput,
  McpServerState,
  McpServerStatus,
  McpServerView,
  McpToolAnnotations,
  McpToolView,
  McpTransportView,
} from "@/ipc/types";
import { parseBaseUrl, type KeyDraft } from "./aiLogic";

/* Pure logic behind the MCP servers section of Settings → AI (spec §13.9, AI-29 to AI-33): no React, no i18n. */

// ───────────────────────── Command lines (AI-29) ─────────────────────────

const CONTROL = /[\u0000-\u001f\u007f]/g;
const PLAIN_ARG = /^[A-Za-z0-9_@%+=:,./\\-]+$/;

const ESCAPES: Record<string, string> = { "\n": "\\n", "\r": "\\r", "\t": "\\t" };
const escapeControl = (text: string): string =>
  text.replace(CONTROL, (c) => ESCAPES[c] ?? `\\x${c.charCodeAt(0).toString(16).padStart(2, "0")}`);

/**
 * One argument as it is shown in a command line: unchanged when it has only plain characters, otherwise in double
 * quotes with `"` escaped. Control characters show as escapes, so nothing hides in the line. For display only: a
 * backslash is left alone, so a Windows path reads as it is.
 */
export function quoteArg(arg: string): string {
  if (PLAIN_ARG.test(arg)) return arg;
  return `"${escapeControl(arg.replace(/"/g, '\\"'))}"`;
}

/** The command and its arguments as one line, which the user confirms before a `stdio` server is saved. */
export function formatCommandLine(command: string, args: readonly string[]): string {
  return [escapeControl(command), ...args.map(quoteArg)].join(" ");
}

/**
 * Splits a pasted command line into words the way a shell does for quotes: single and double quotes group words,
 * and a backslash escapes a double quote inside double quotes, and a quote or space outside quotes. Any other
 * backslash stays, so Windows paths survive. Null when a quote is not closed.
 */
export function splitCommandLine(text: string): string[] | null {
  const words: string[] = [];
  let word = "";
  let started = false;
  let quote: '"' | "'" | null = null;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quote) {
      if (c === quote) quote = null;
      else if (c === "\\" && quote === '"' && text[i + 1] === '"') word += text[++i];
      else word += c;
    } else if (c === '"' || c === "'") {
      quote = c;
      started = true;
    } else if (/\s/.test(c)) {
      if (started || word) words.push(word);
      word = "";
      started = false;
    } else if (c === "\\" && (text[i + 1] === '"' || text[i + 1] === "'" || text[i + 1] === " ")) {
      word += text[++i];
      started = true;
    } else {
      word += c;
      started = true;
    }
  }
  if (quote) return null;
  if (started || word) words.push(word);
  return words;
}

/**
 * Whether the command field holds a whole command line (`npx -y server`) instead of a program: it has a space and
 * the first word is not a path. A program path with spaces, such as `C:\Program Files\node.exe`, does not count.
 */
export function looksLikeCommandLine(command: string): boolean {
  const text = command.trim();
  if (!/\s/.test(text)) return false;
  if (/^["']/.test(text)) return false;
  return !/[\\/]/.test(text.split(/\s+/)[0]);
}

// ───────────────────────── Environment and header rows (AI-29) ─────────────────────────

/**
 * An environment variable or request header while it is being edited. A saved value never reaches the page
 * (`saved`), so the row keeps it or replaces it, like an API key (AI-01).
 */
export interface SecretRow {
  uid: number;
  key: string;
  saved: boolean;
  draft: KeyDraft;
}

let nextUid = 1;
export const newUid = (): number => nextUid++;

export const newSecretRow = (): SecretRow => ({ uid: newUid(), key: "", saved: false, draft: { mode: "replace", value: "" } });

export const savedSecretRows = (keys: readonly string[]): SecretRow[] =>
  keys.map((key) => ({ uid: newUid(), key, saved: true, draft: { mode: "keep", value: "" } }));

export type SecretKind = "env" | "header";

export interface SecretRowProblem {
  key?: "empty" | "invalid" | "duplicate";
  value?: "missing";
}

/** A row the user added and left empty: it is dropped when saving. */
export const isBlankRow = (row: SecretRow): boolean => !row.saved && row.key.trim() === "" && row.draft.value.trim() === "";

// A name has no `=`, no whitespace, and no control character; a header name is an HTTP token (RFC 9110).
const ENV_NAME = /^[^=\s\u0000-\u001f\u007f]+$/;
const HEADER_NAME = /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/;

/** The problems of each row (null when it is fine), index for index with `rows`. Blank rows are fine. */
export function validateSecretRows(rows: readonly SecretRow[], kind: SecretKind): (SecretRowProblem | null)[] {
  const seen = new Set<string>();
  return rows.map((row) => {
    if (isBlankRow(row)) return null;
    const problem: SecretRowProblem = {};
    const key = row.key.trim();
    if (key === "") problem.key = "empty";
    else if (!(kind === "env" ? ENV_NAME : HEADER_NAME).test(key)) problem.key = "invalid";
    else if (seen.has(key.toLowerCase())) problem.key = "duplicate";
    seen.add(key.toLowerCase());
    if (!row.saved && row.draft.value.trim() === "") problem.value = "missing";
    return Object.keys(problem).length ? problem : null;
  });
}

/**
 * The `env` or `headers` of `McpTransportInput`: a saved value that is kept or replaced with nothing is `null`, which
 * keeps the saved value for that name; a row that is gone drops its value.
 */
export function secretInputs(rows: readonly SecretRow[]): McpSecretInput[] {
  return rows
    .filter((row) => !isBlankRow(row))
    .map((row): McpSecretInput => {
      const typed = row.draft.value.trim();
      const replaced = row.draft.mode === "replace" && typed !== "";
      return { key: row.key.trim(), value: !row.saved || replaced ? typed : null };
    });
}

// ───────────────────────── The edit form (AI-29) ─────────────────────────

export interface ArgRow {
  uid: number;
  value: string;
}

export type TransportKind = "stdio" | "http";

export interface McpForm {
  name: string;
  kind: TransportKind;
  command: string;
  args: ArgRow[];
  env: SecretRow[];
  url: string;
  headers: SecretRow[];
  alwaysAsk: boolean;
}

export const argRows = (values: readonly string[]): ArgRow[] => values.map((value) => ({ uid: newUid(), value }));

export function initialMcpForm(server: McpServerView | null): McpForm {
  const t = server?.transport;
  return {
    name: server?.name ?? "",
    kind: t?.kind ?? "stdio",
    command: t?.kind === "stdio" ? t.command : "",
    args: t?.kind === "stdio" ? argRows(t.args) : [],
    env: t?.kind === "stdio" ? savedSecretRows(t.env_keys) : [],
    url: t?.kind === "http" ? t.url : "",
    headers: t?.kind === "http" ? savedSecretRows(t.header_keys) : [],
    alwaysAsk: server?.always_ask ?? false,
  };
}

export interface McpFormProblems {
  name: "empty" | "taken" | null;
  command: "empty" | null;
  url: "empty" | "invalid" | null;
  /** Index for index with the rows of the active transport. */
  secrets: (SecretRowProblem | null)[];
}

/** `otherNames` are the names of the other saved servers. */
export function validateMcpForm(form: McpForm, otherNames: readonly string[] = []): McpFormProblems {
  const name = form.name.trim();
  const url = form.url.trim();
  return {
    name: name === "" ? "empty" : otherNames.some((n) => n.trim().toLowerCase() === name.toLowerCase()) ? "taken" : null,
    command: form.kind === "stdio" && form.command.trim() === "" ? "empty" : null,
    url: form.kind === "http" ? (url === "" ? "empty" : parseBaseUrl(url) ? null : "invalid") : null,
    secrets: validateSecretRows(form.kind === "stdio" ? form.env : form.headers, form.kind === "stdio" ? "env" : "header"),
  };
}

export function hasMcpProblems(p: McpFormProblems): boolean {
  return !!p.name || !!p.command || !!p.url || p.secrets.some(Boolean);
}

/** What `mcp_server_save` takes. Arguments that are empty are dropped. */
export function mcpFormToInput(id: string | null, form: McpForm): McpServerInput {
  return {
    id,
    name: form.name.trim(),
    transport:
      form.kind === "stdio"
        ? { kind: "stdio", command: form.command.trim(), args: form.args.map((a) => a.value).filter((v) => v !== ""), env: secretInputs(form.env) }
        : { kind: "http", url: form.url.trim(), headers: secretInputs(form.headers) },
    always_ask: form.alwaysAsk,
  };
}

/** What the confirmation before saving a `stdio` server shows: the full command line and the names of its variables. */
export function commandPreview(form: McpForm): { line: string; envNames: string[] } {
  return {
    line: formatCommandLine(form.command.trim(), form.args.map((a) => a.value).filter((v) => v !== "")),
    envNames: form.env.filter((r) => !isBlankRow(r)).map((r) => r.key.trim()),
  };
}

// ───────────────────────── Servers and their state (AI-31, AI-32) ─────────────────────────

/** A server's transport on one line: the command line, or the URL. */
export function transportSummary(t: McpTransportView): string {
  return t.kind === "stdio" ? formatCommandLine(t.command, t.args) : t.url;
}

export const serverState = (status: McpServerStatus | undefined): McpServerState => status?.state ?? "stopped";

/** Whether a tool runs without asking in manual mode on this device: on its own, or because the whole server is. */
export const toolAllowed = (server: McpServerView, tool: McpToolView): boolean =>
  server.always_allow || server.always_allow_tools.includes(tool.tool);

/** The server after Always allow changed on this device: one tool (the server's own name), or all with `tool` null. */
export function withAlwaysAllow(server: McpServerView, tool: string | null, allow: boolean): McpServerView {
  if (tool === null) return { ...server, always_allow: allow };
  const rest = server.always_allow_tools.filter((x) => x !== tool);
  return { ...server, always_allow_tools: allow ? [...rest, tool] : rest };
}

export type AnnotationKey = "read_only" | "destructive" | "idempotent" | "open_world";

/** The hints a tool sets to true, in a fixed order. They are shown and never change whether a call asks. */
export function annotationKeys(a: McpToolAnnotations): AnnotationKey[] {
  const keys: AnnotationKey[] = [];
  if (a.read_only_hint) keys.push("read_only");
  if (a.destructive_hint) keys.push("destructive");
  if (a.idempotent_hint) keys.push("idempotent");
  if (a.open_world_hint) keys.push("open_world");
  return keys;
}

/** Replaces the server with the same ID, or adds it. */
export function upsertServer(list: readonly McpServerView[], server: McpServerView): McpServerView[] {
  return list.some((s) => s.id === server.id) ? list.map((s) => (s.id === server.id ? server : s)) : [...list, server];
}

/** The last stderr lines as one text block, without trailing empty lines. */
export const stderrText = (lines: readonly string[]): string => lines.join("\n").replace(/\s+$/, "");
