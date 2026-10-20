/**
 * AI-10, AI-35, AI-50, AI-51: what goes with a message. Each attachment is stored in the user's
 * entry as a leading attachment block, so neither the entry format nor the IPC contract changes:
 *
 *   <terminal_selection host="prod-api" lines="3">
 *   …the selected text…
 *   </terminal_selection>
 *
 *   <file name="nginx.conf" lines="40">
 *   …the file…
 *   </file>
 *
 *   the typed text
 *
 * Before them, Rust stores the notes Hatoba writes itself when a message follows a reply the user
 * stopped, moves the conversation to another host, changes whether a terminal is attached, goes to
 * another model, is sent in another permission mode than the model was told of, starts a conversation
 * with the memories, follows a revert of a file tool's change, is sent with a connected terminal, or
 * comes with another list of deferred MCP tools (`interrupted`, `host_change`, `terminal_change`,
 * `model_change`, `permission_mode`, `memory`, `file_revert`, `terminal_state`, `mcp_tools`); the panel
 * shows the terminal's screen as a collapsed block, hides the memories, and shows the others as dividers. The block
 * format and its escaping are specified once, in docs/hatoba-spec.md §13.3
 * ("Attachment blocks"); Rust's `title_of` (src-tauri/src/ai.rs) reads the same blocks to title a
 * conversation.
 */

import type { AiEntryView, AiPermissionMode, DroppedFile } from "@/ipc/types";
import { estimateTokens } from "./meter";

/** As for tool results (§13.4): over this many characters, the first 7,500 and the last 22,500 are kept. */
export const SELECTION_MAX_CHARS = 30_000;
const HEAD_CHARS = 7_500;
const TAIL_CHARS = 22_500;

/** A paste this long (characters or lines) becomes an attachment instead of going into the input (AI-35). */
export const LONG_PASTE_CHARS = 2_000;
export const LONG_PASTE_LINES = 30;
/** Text files over this size are refused (AI-35). */
export const FILE_MAX_BYTES = 256 * 1024;
/** All attachments of one message together, in UTF-8 bytes (AI-35). */
export const MESSAGE_MAX_BYTES = 512 * 1024;

/** The diagnostics of a failed connection, attached from the terminal's error card (AI-10). */
export interface DiagnosticsAttachment {
  kind: "diagnostics";
  /** The host's display name. */
  host: string;
  text: string;
}

/** The tab's terminal selection (AI-10). */
export interface SelectionAttachment {
  kind: "selection";
  /** The tab host's display name. */
  host: string;
  /** Lines in the selection as the user made it. */
  lines: number;
  /** The selected text, clipped to the limit. */
  text: string;
  truncated: boolean;
}

/** A long paste (AI-35), kept whole: the user means the model to see all of it. */
export interface PasteAttachment {
  kind: "paste";
  lines: number;
  text: string;
}

/** A UTF-8 text file (AI-35). */
export interface FileAttachment {
  kind: "file";
  /** The file's base name, never a path: a path can reveal the local user name. */
  name: string;
  lines: number;
  text: string;
}

/** A text file on the tab's host, picked in the `@` menu (AI-51). */
export interface HostFileAttachment {
  kind: "host_file";
  /** The tab host's display name. */
  host: string;
  /** The path as the host resolved it. */
  path: string;
  lines: number;
  text: string;
}

/** A directory's entries on the tab's host, picked in the `@` menu (AI-51): one per line, a `/` after each directory. */
export interface HostDirectoryAttachment {
  kind: "host_directory";
  host: string;
  path: string;
  entries: number;
  /** Entries were left out: the listing passed its size limit. */
  truncated: boolean;
  text: string;
}

/** A resource of an MCP server, picked in the `@` menu (AI-51). */
export interface McpResourceAttachment {
  kind: "mcp_resource";
  /** The server's name. */
  server: string;
  uri: string;
  lines: number;
  text: string;
}

/** The `SKILL.md` of a skill the user ran with `/<name>` (AI-50). */
export interface SkillAttachment {
  kind: "skill";
  name: string;
  text: string;
}

/** A prompt of an MCP server the user ran with `/mcp__<server>__<prompt>` (AI-50): its messages as text. */
export interface McpPromptAttachment {
  kind: "mcp_prompt";
  server: string;
  /** The prompt's name on its server. */
  name: string;
  text: string;
}

export type Attachment =
  | DiagnosticsAttachment
  | SelectionAttachment
  | PasteAttachment
  | FileAttachment
  | HostFileAttachment
  | HostDirectoryAttachment
  | McpResourceAttachment
  | SkillAttachment
  | McpPromptAttachment;

// ───────────── making attachments ─────────────

/** Terminal text without trailing spaces on its lines, and without leading and trailing blank lines. */
export function cleanTerminalText(text: string): string {
  return text
    .replace(/\r\n?/g, "\n")
    .split("\n")
    .map((line) => line.replace(/\s+$/, ""))
    .join("\n")
    .replace(/^\n+/, "")
    .replace(/\n+$/, "");
}

/** Like Rust's `truncate_result`: counted in characters, with a line saying how many were left out. */
export function clipText(text: string): { text: string; truncated: boolean } {
  const chars = [...text];
  if (chars.length <= SELECTION_MAX_CHARS) return { text, truncated: false };
  const leftOut = chars.length - HEAD_CHARS - TAIL_CHARS;
  const head = chars.slice(0, HEAD_CHARS).join("");
  const tail = chars.slice(chars.length - TAIL_CHARS).join("");
  return { text: `${head}\n\n[… ${leftOut} characters left out …]\n\n${tail}`, truncated: true };
}

/** Lines of a text; a final line break does not start another line. */
export function lineCount(text: string): number {
  if (!text) return 0;
  return text.replace(/\n$/, "").split("\n").length;
}

/** What a selection attaches, or null when it is empty or only whitespace. */
export function makeAttachment(host: string, selection: string): SelectionAttachment | null {
  const clean = cleanTerminalText(selection);
  if (!clean.trim()) return null;
  return { kind: "selection", host, lines: clean.split("\n").length, ...clipText(clean) };
}

/** What diagnostics attach, clipped like a selection, or null when there is no text. */
export function makeDiagnostics(host: string, text: string): DiagnosticsAttachment | null {
  const clean = text.replace(/\r\n?/g, "\n").trim();
  return clean ? { kind: "diagnostics", host, text: clipText(clean).text } : null;
}

/** Whether a paste is long enough to become an attachment rather than go into the input. */
export function isLongPaste(text: string): boolean {
  return text.length >= LONG_PASTE_CHARS || lineCount(text.replace(/\r\n?/g, "\n")) >= LONG_PASTE_LINES;
}

/** A pasted text as an attachment, whole, or null when it is only whitespace. */
export function makePaste(text: string): PasteAttachment | null {
  const clean = text.replace(/\r\n?/g, "\n");
  return clean.trim() ? { kind: "paste", lines: lineCount(clean), text: clean } : null;
}

/** A file's base name: what follows the last `/` or `\`. */
export function baseName(name: string): string {
  return name.split(/[\\/]/).pop() || name;
}

const IMAGE_EXT = /\.(png|jpe?g|gif|webp|bmp|ico|svg|heic|heif|avif|tiff?)$/i;

/** Why a file is not attached: the checks here, and `unreadable` when it could not be read at all. */
export type FileRefusal = "image" | "too_large" | "binary" | "unreadable";

export type FileCheck = { ok: true; file: FileAttachment } | { ok: false; reason: Exclude<FileRefusal, "unreadable"> };

/** Whether a file is an image, by its type or name; images are not supported yet. */
export function isImage(name: string, type: string): boolean {
  return type.startsWith("image/") || IMAGE_EXT.test(name);
}

/** Before reading: images and files over the size cap are refused (AI-35). */
export function checkFile(name: string, type: string, size: number): FileCheck | null {
  if (isImage(name, type)) return { ok: false, reason: "image" };
  if (size > FILE_MAX_BYTES) return { ok: false, reason: "too_large" };
  return null;
}

/** A file's bytes as an attachment: UTF-8 text only, so NUL bytes or invalid UTF-8 refuse it. */
export function textFile(name: string, bytes: Uint8Array): FileCheck {
  if (bytes.length > FILE_MAX_BYTES) return { ok: false, reason: "too_large" };
  if (bytes.includes(0)) return { ok: false, reason: "binary" };
  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return { ok: false, reason: "binary" };
  }
  return { ok: true, file: fileAttachment(name, text) };
}

/** A file's text as an attachment: line breaks as `\n`, named by its base name. */
function fileAttachment(name: string, text: string): FileAttachment {
  const clean = text.replace(/\r\n?/g, "\n");
  return { kind: "file", name: baseName(name), lines: lineCount(clean), text: clean };
}

/**
 * A file dropped on the panel in the desktop app, as Rust read it (`ai_read_dropped_files`, with the
 * checks of `checkFile` and `textFile`), made into an attachment the same way, or the reason it is
 * refused. The size cap is checked again here.
 */
export function droppedFile(result: DroppedFile): { ok: true; file: FileAttachment } | { ok: false; name: string; reason: FileRefusal } {
  const name = baseName(result.name);
  if (result.status === "refused") return { ok: false, name, reason: result.reason };
  if (new TextEncoder().encode(result.text).length > FILE_MAX_BYTES) return { ok: false, name, reason: "too_large" };
  return { ok: true, file: fileAttachment(name, result.text) };
}

/** The UTF-8 size of an attachment's text, which counts toward the per-message cap. */
export function attachmentBytes(a: Attachment): number {
  return new TextEncoder().encode(a.text).length;
}

/** Whether adding `next` keeps the message's attachments within the cap. */
export function fitsMessage(current: readonly Attachment[], next: Attachment): boolean {
  return current.reduce((n, a) => n + attachmentBytes(a), 0) + attachmentBytes(next) <= MESSAGE_MAX_BYTES;
}

/** The context meter's estimate for an attachment (AI-20). */
export function attachmentTokens(a: Attachment): number {
  return estimateTokens(a.text);
}

/** The last part of a path: what follows its last `/`, ignoring a trailing one. */
export function pathName(path: string): string {
  return path.replace(/\/+$/, "").split("/").pop() || path;
}

/** How a chip names a folder: its last part and a `/`, or `/` for the root. */
export function folderName(path: string): string {
  const name = pathName(path);
  return name.endsWith("/") ? name : `${name}/`;
}

/** A conversation's title when its first message has nothing typed and starts with `a`, as Rust's `title_of` makes it (AI-23). */
export function blockTitle(a: Attachment): string {
  switch (a.kind) {
    case "diagnostics":
      return "Connection diagnostics";
    case "selection":
      return "Terminal selection";
    case "paste":
      return "Pasted text";
    case "file":
      return a.name;
    case "host_file":
      return pathName(a.path);
    case "host_directory":
      return a.path;
    case "mcp_resource":
      return a.uri;
    case "skill":
      return `/${a.name}`;
    case "mcp_prompt":
      return a.name;
  }
}

// ───────────── the stored blocks ─────────────

/**
 * A note Hatoba writes before a message (§13.1, AI-05, AI-09, AI-41), never the user: the reply before
 * it was stopped by the user, or with this message the conversation moved to another host, a terminal
 * became attached or stopped being attached, it went to another model, the permission mode the model
 * works in changed (AI-16), or the user reverted the change a file tool call made, or this message
 * shows the attached terminal's screen (AI-46) or lists the deferred MCP tools (AI-45). `from` of a host change is empty when the earlier host no longer exists;
 * a model is named as `Name (id)`, or by its ID alone. A revert's `path` is the file's path as the call
 * resolved it, and `body` its text for the model; the screen counts its lines, and `alternate` says a
 * full-screen program showed it; the MCP tool list counts its servers and tools, and `body` lists them.
 */
export type Note =
  | { kind: "interrupted" }
  | { kind: "host_change" | "model_change"; from: string; to: string }
  | { kind: "terminal_change"; to: "attached" | "detached" }
  | { kind: "permission_mode"; mode: AiPermissionMode }
  | { kind: "memory"; entries: number; host?: string; body: string }
  | { kind: "file_revert"; path: string; toolCallId: string; body: string }
  | { kind: "terminal_state"; host: string; alternate: boolean; lines: number; body: string }
  | { kind: "mcp_tools"; servers: number; tools: number; body: string };

/** The order Hatoba writes its notes in; a `file_revert` comes once for each reverted call. */
const NOTE_ORDER = [
  "interrupted",
  "host_change",
  "terminal_change",
  "model_change",
  "permission_mode",
  "memory",
  "file_revert",
  "terminal_state",
  "mcp_tools",
] as const satisfies readonly Note["kind"][];

const attr = (s: string) => s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
const unattr = (s: string) => s.replace(/&quot;/g, '"').replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&");

/**
 * A closing tag of the block's kind inside the text gets a backslash after its `<` (and one more on
 * each that already has some), so the text can never end the block early, and `unescapeBody` undoes
 * it exactly. `tag` is a fixed tag name, never user text.
 */
function escapeBody(text: string, tag: string): string {
  return text.replace(new RegExp(`<(\\\\*)/${tag}`, "gi"), (m) => `<\\${m.slice(1)}`);
}

function unescapeBody(text: string, tag: string): string {
  return text.replace(new RegExp(`<\\\\(\\\\*/${tag})`, "gi"), "<$1");
}

const TAGS = {
  diagnostics: "connection_diagnostics",
  selection: "terminal_selection",
  paste: "pasted_text",
  file: "file",
  host_file: "host_file",
  host_directory: "host_directory",
  mcp_resource: "mcp_resource",
  skill: "skill",
  mcp_prompt: "mcp_prompt",
} as const satisfies Record<Attachment["kind"], string>;

/** One line of a name inside a note: control characters as spaces, trimmed, as Rust writes it. */
const noteText = (s: string) => s.replace(/\p{Cc}/gu, " ").trim();

function writeBlock(tag: string, attrs: [string, string][], body: string): string {
  const list = attrs.map(([name, value]) => ` ${name}="${attr(value)}"`).join("");
  return `<${tag}${list}>\n${escapeBody(body, tag)}\n</${tag}>`;
}

/** An attachment's attributes, in the order its block writes them. */
function attributes(a: Attachment): [string, string][] {
  switch (a.kind) {
    case "diagnostics":
      return [["host", a.host]];
    case "selection":
      return [["host", a.host], ["lines", String(a.lines)], ...(a.truncated ? [["truncated", "true"] as [string, string]] : [])];
    case "paste":
      return [["lines", String(a.lines)]];
    case "file":
      return [["name", a.name], ["lines", String(a.lines)]];
    case "host_file":
      return [["host", a.host], ["path", a.path], ["lines", String(a.lines)]];
    case "host_directory":
      return [["host", a.host], ["path", a.path], ["entries", String(a.entries)], ...(a.truncated ? [["truncated", "true"] as [string, string]] : [])];
    case "mcp_resource":
      return [["server", a.server], ["uri", a.uri], ["lines", String(a.lines)]];
    case "skill":
      return [["name", a.name]];
    case "mcp_prompt":
      return [["server", a.server], ["name", a.name]];
  }
}

function block(a: Attachment): string {
  return writeBlock(TAGS[a.kind], attributes(a), a.text);
}

const TERMINAL_BODY = {
  attached:
    "A terminal is attached from this message on. Before this message none was, so replies that said they could not read a screen or run commands were right at the time.",
  detached: "No terminal is attached from this message on. Screens and command output before this message came from the terminal that was attached then.",
} as const;

const PERMISSION_BODY = {
  manual: "From this message on, the permission mode is manual approval.",
  auto: "From this message on, the permission mode is auto.",
  bypass: "From this message on, the permission mode is bypass.",
} as const satisfies Record<AiPermissionMode, string>;

const INTERRUPTED_BODY =
  "The user stopped your previous reply before it was finished. It ends where it was cut off; anything you were still writing, tool calls included, was not kept.";

/**
 * A note as Rust writes it (`interrupted_block`, `host_change_block`, `terminal_change_block`,
 * `model_change_block`, `permission_mode_block`, `memory_block`, `file_revert_block`, `terminal_state_block` and
 * `deferred::list_block` in crates/hatoba-ai). The panel never sends one; Rust adds them when it stores a message, and the mock
 * does the same.
 */
export function noteBlock(n: Note): string {
  if (n.kind === "mcp_tools") return writeBlock(n.kind, [["servers", String(n.servers)], ["tools", String(n.tools)]], n.body);
  if (n.kind === "memory")
    return writeBlock(n.kind, [["entries", String(n.entries)], ...(n.host === undefined ? [] : [["host", noteText(n.host)] as [string, string]])], n.body);
  if (n.kind === "terminal_state")
    return writeBlock(n.kind, [["host", noteText(n.host)], ["alternate_screen", String(n.alternate)], ["lines", String(n.lines)]], n.body);
  if (n.kind === "interrupted") return writeBlock(n.kind, [], INTERRUPTED_BODY);
  if (n.kind === "terminal_change") return writeBlock(n.kind, [["to", n.to]], TERMINAL_BODY[n.to]);
  if (n.kind === "permission_mode") return writeBlock(n.kind, [["mode", n.mode]], PERMISSION_BODY[n.mode]);
  if (n.kind === "file_revert") return writeBlock(n.kind, [["path", noteText(n.path)], ["tool_call_id", noteText(n.toolCallId)]], n.body);
  const from = noteText(n.from);
  const body =
    n.kind === "model_change"
      ? "Earlier replies in this conversation came from another model."
      : `The conversation moved to another host. Screens and command output before this message came from ${from ? `"${from}"` : "another host"}.`;
  return writeBlock(n.kind, [["from", from], ["to", noteText(n.to)]], body);
}

/** As Rust's `MAX_SCREEN_CHARS` (AI-46): a longer screen keeps its first quarter and the rest from its end. */
const SCREEN_MAX_CHARS = 4_000;

/**
 * The `terminal_state` note of a message sent with `screen` (AI-46), as Rust's `terminal_state_block`
 * writes it: control characters as spaces, blank lines at either end left out, and a long screen cut
 * like a tool result. Null for a blank screen.
 */
export function screenStateNote(host: string, screen: { text: string; alternate: boolean }): Note | null {
  const lines = screen.text
    .replace(/\r\n/g, "\n")
    .replace(/[^\P{Cc}\n\t]/gu, " ")
    .split("\n")
    .map((l) => l.trimEnd());
  const first = lines.findIndex((l) => l !== "");
  if (first < 0) return null;
  const shown = lines.slice(first, lines.findLastIndex((l) => l !== "") + 1);
  const chars = Array.from(shown.join("\n"));
  const head = SCREEN_MAX_CHARS / 4;
  const body =
    chars.length <= SCREEN_MAX_CHARS
      ? chars.join("")
      : `${chars.slice(0, head).join("")}\n\n[… ${chars.length - SCREEN_MAX_CHARS} characters left out …]\n\n${chars.slice(chars.length - (SCREEN_MAX_CHARS - head)).join("")}`;
  return { kind: "terminal_state", host, alternate: screen.alternate, lines: shown.length, body };
}

/** The note of a revert (AI-41) as Rust's `file_revert_block` words it; `created`: the call created the file, which the revert deleted. */
export function fileRevertNote(path: string, toolCallId: string, tool: string, created: boolean): Note {
  const [file, call] = [noteText(path), noteText(toolCallId)];
  return {
    kind: "file_revert",
    path,
    toolCallId,
    body: created
      ? `The user reverted your ${tool} call ${call}, which created ${file}, so it was deleted.`
      : `The user reverted the change your ${tool} call ${call} made to ${file}.`,
  };
}

/**
 * The order attachments are stored in: the connection diagnostics, then the terminal selection (at
 * most one of each, the reason for asking first), then the others (pasted text, files, what the `/`
 * and `@` menus attached) in the order they were added.
 */
export function orderAttachments(list: readonly Attachment[]): Attachment[] {
  const diagnostics = list.find((a) => a.kind === "diagnostics");
  const selection = list.find((a) => a.kind === "selection");
  const rest = list.filter((a) => a.kind !== "diagnostics" && a.kind !== "selection");
  return [...(diagnostics ? [diagnostics] : []), ...(selection ? [selection] : []), ...rest];
}

/** The user entry's text: Hatoba's notes in their order, the attachment blocks, then the typed text. */
export function composeMessage(typed: string, attachments: readonly Attachment[], notes: readonly Note[] = []): string {
  const ordered = NOTE_ORDER.flatMap((kind) => notes.filter((n) => n.kind === kind));
  return [...ordered.map(noteBlock), ...orderAttachments(attachments).map(block), typed].join("\n\n");
}

/** A user entry's text split into Hatoba's notes, its attachments and what the user typed. */
export interface MessageParts {
  notes: Note[];
  attachments: Attachment[];
  typed: string;
}

const OPEN =
  /^<(interrupted|host_change|terminal_change|model_change|permission_mode|memory|file_revert|terminal_state|mcp_tools|terminal_selection|connection_diagnostics|pasted_text|file|host_file|host_directory|mcp_resource|skill|mcp_prompt)((?: [a-z_]+="[^"]*")*)>\n/;
const DIGITS = /^\d+$/;

type Block = { kind: "note"; note: Note } | { kind: "attachment"; attachment: Attachment };

/** The block an opening tag describes, when its attributes are the kind's own, in order. */
function blockOf(tag: string, attrs: [string, string][], body: string): Block | null {
  const names = attrs.map(([name]) => name).join(",");
  const value = (i: number) => attrs[i][1];
  const attachment = (a: Attachment): Block => ({ kind: "attachment", attachment: a });
  switch (tag) {
    case "interrupted":
      return names === "" ? { kind: "note", note: { kind: tag } } : null;
    case "host_change":
    case "model_change":
      return names === "from,to" ? { kind: "note", note: { kind: tag, from: value(0), to: value(1) } } : null;
    case "terminal_change": {
      const to = names === "to" ? value(0) : "";
      return to === "attached" || to === "detached" ? { kind: "note", note: { kind: tag, to } } : null;
    }
    case "permission_mode": {
      const mode = names === "mode" ? value(0) : "";
      return mode === "manual" || mode === "auto" || mode === "bypass" ? { kind: "note", note: { kind: tag, mode } } : null;
    }
    case "memory":
      if (names === "entries,host" && DIGITS.test(value(0))) return { kind: "note", note: { kind: tag, entries: Number(value(0)), host: value(1), body } };
      return names === "entries" && DIGITS.test(value(0)) ? { kind: "note", note: { kind: tag, entries: Number(value(0)), body } } : null;
    case "file_revert":
      return names === "path,tool_call_id" ? { kind: "note", note: { kind: tag, path: value(0), toolCallId: value(1), body } } : null;
    case "terminal_state":
      return names === "host,alternate_screen,lines" && (value(1) === "true" || value(1) === "false") && DIGITS.test(value(2))
        ? { kind: "note", note: { kind: tag, host: value(0), alternate: value(1) === "true", lines: Number(value(2)), body } }
        : null;
    case "mcp_tools":
      return names === "servers,tools" && DIGITS.test(value(0)) && DIGITS.test(value(1))
        ? { kind: "note", note: { kind: tag, servers: Number(value(0)), tools: Number(value(1)), body } }
        : null;
    case TAGS.selection:
      if ((names === "host,lines" || (names === "host,lines,truncated" && value(2) === "true")) && DIGITS.test(value(1)))
        return attachment({ kind: "selection", host: value(0), lines: Number(value(1)), truncated: attrs.length === 3, text: body });
      return null;
    case TAGS.diagnostics:
      return names === "host" ? attachment({ kind: "diagnostics", host: value(0), text: body }) : null;
    case TAGS.paste:
      return names === "lines" && DIGITS.test(value(0)) ? attachment({ kind: "paste", lines: Number(value(0)), text: body }) : null;
    case TAGS.file:
      return names === "name,lines" && DIGITS.test(value(1)) ? attachment({ kind: "file", name: value(0), lines: Number(value(1)), text: body }) : null;
    case TAGS.host_file:
      return names === "host,path,lines" && DIGITS.test(value(2))
        ? attachment({ kind: "host_file", host: value(0), path: value(1), lines: Number(value(2)), text: body })
        : null;
    case TAGS.host_directory:
      if ((names === "host,path,entries" || (names === "host,path,entries,truncated" && value(3) === "true")) && DIGITS.test(value(2)))
        return attachment({ kind: "host_directory", host: value(0), path: value(1), entries: Number(value(2)), truncated: attrs.length === 4, text: body });
      return null;
    case TAGS.mcp_resource:
      return names === "server,uri,lines" && DIGITS.test(value(2))
        ? attachment({ kind: "mcp_resource", server: value(0), uri: value(1), lines: Number(value(2)), text: body })
        : null;
    case TAGS.skill:
      return names === "name" ? attachment({ kind: "skill", name: value(0), text: body }) : null;
    case TAGS.mcp_prompt:
      return names === "server,name" ? attachment({ kind: "mcp_prompt", server: value(0), name: value(1), text: body }) : null;
    default:
      return null;
  }
}

/**
 * The block `text` starts with, as Rust's `leading_block` reads it: the opening tag with its
 * attributes in the kind's order, a line break, the body, a line break and the closing tag, then a
 * blank line, a line break or the end. The body ends at the first closing tag that such a break or
 * the end follows.
 */
function leadingBlock(text: string): { block: Block; rest: string } | null {
  const m = OPEN.exec(text);
  if (!m) return null;
  const tag = m[1];
  const attrs = [...m[2].matchAll(/ ([a-z_]+)="([^"]*)"/g)].map((a): [string, string] => [a[1], unattr(a[2])]);
  if (!blockOf(tag, attrs, "")) return null;
  const close = `\n</${tag}>`;
  for (let from = m[0].length; ; ) {
    const at = text.indexOf(close, from);
    if (at < 0) return null;
    const after = text.slice(at + close.length);
    const rest = after.startsWith("\n\n") ? after.slice(2) : after.startsWith("\n") ? after.slice(1) : after === "" ? "" : null;
    if (rest !== null) return { block: blockOf(tag, attrs, unescapeBody(text.slice(m[0].length, at), tag))!, rest };
    from = at + 1;
  }
}

/**
 * A user entry's text split into its leading blocks and the typed text. Hatoba's notes come first,
 * at most one of each kind but `file_revert`, which comes once for each reverted call; the attachment
 * blocks after them are read in any order. A note after an attachment, a second note of another kind,
 * a second diagnostics or selection block, like anything that is not a valid block, is part of the
 * typed text.
 */
export function parseMessage(text: string): MessageParts {
  const parts: MessageParts = { notes: [], attachments: [], typed: text };
  for (;;) {
    const b = leadingBlock(parts.typed);
    if (!b) return parts;
    if (b.block.kind === "note") {
      const note = b.block.note;
      if (parts.attachments.length > 0 || (note.kind !== "file_revert" && parts.notes.some((n) => n.kind === note.kind))) return parts;
      parts.notes.push(note);
    } else {
      const a = b.block.attachment;
      const single = a.kind === "diagnostics" || a.kind === "selection";
      if (single && parts.attachments.some((x) => x.kind === a.kind)) return parts;
      parts.attachments.push(a);
    }
    parts.typed = b.rest;
  }
}

/**
 * A summary's text (AI-21) split into the `permission_mode` note Rust stores before the summary, which
 * carries the mode past the compaction, and the summary itself, without the `memory` note (AI-53) that
 * follows the mode's.
 */
export function splitPermissionNote(text: string): { mode: AiPermissionMode | null; summary: string } {
  let mode: AiPermissionMode | null = null;
  let rest = text;
  for (const m of ["manual", "auto", "bypass"] as const) {
    const note = `${noteBlock({ kind: "permission_mode", mode: m })}\n\n`;
    if (text.startsWith(note)) {
      mode = m;
      rest = text.slice(note.length);
      break;
    }
  }
  const memory = leadingBlock(rest);
  if (memory?.block.kind === "note" && memory.block.note.kind === "memory") rest = memory.rest;
  return { mode, summary: rest };
}

/**
 * AI-16: the permission mode the model was last told of, as Rust's `noted_permission` reads it from a
 * conversation's entries: the newest `permission_mode` note, or the one the newest summary starts
 * with. Null when it was told of none: before any note, and after a summary without one.
 */
export function notedPermission(entries: readonly AiEntryView[]): AiPermissionMode | null {
  for (let i = entries.length - 1; i >= 0; i--) {
    const e = entries[i];
    if (e.role === "summary") return splitPermissionNote(e.text).mode;
    if (e.role !== "user") continue;
    const note = parseMessage(e.text).notes.find((n) => n.kind === "permission_mode");
    if (note) return note.mode;
  }
  return null;
}

/** A model as a `model_change` note names it (`Name (id)`), by its name alone, for the panel. */
export function modelName(label: string): string {
  const m = /^(.+) \([^()]+\)$/.exec(label);
  return m ? m[1] : label;
}

// ───────────── the selection chip ─────────────

/** The tab's selection as the panel last saw it; `seq` moves whenever the text changes. */
export interface SelectionState {
  text: string;
  seq: number;
}

/** `renewed`: the selection was cleared on the way (a new drag), so the same text counts as a new selection. */
export function nextSelection(prev: SelectionState, text: string, renewed = false): SelectionState {
  return text === prev.text && !(renewed && text) ? prev : { text, seq: prev.seq + 1 };
}

/** The chip shows a non-empty selection unless it was removed or sent, until the selection changes. */
export function chipShown(selection: SelectionState, hidden: number | null): boolean {
  return selection.text.trim() !== "" && hidden !== selection.seq;
}
