/**
 * AI-10, AI-35: what goes with a message. Each attachment is stored in the user's entry as a
 * leading attachment block, so neither the entry format nor the IPC contract changes:
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
 * The block format and its escaping are specified once, in docs/hatoba-spec.md §13.3 ("Attachment
 * blocks"); Rust's `title_of` (src-tauri/src/ai.rs) reads the same blocks to title a conversation.
 */

import { estimateTokens } from "./meter";

/** As for tool results (§13.4): over this many characters, the first 4,000 and the last 12,000 are kept. */
export const SELECTION_MAX_CHARS = 16_000;
const HEAD_CHARS = 4_000;
const TAIL_CHARS = 12_000;

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

export type Attachment = DiagnosticsAttachment | SelectionAttachment | PasteAttachment | FileAttachment;

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

export type FileCheck = { ok: true; file: FileAttachment } | { ok: false; reason: "image" | "too_large" | "binary" };

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
  const clean = text.replace(/\r\n?/g, "\n");
  return { ok: true, file: { kind: "file", name: baseName(name), lines: lineCount(clean), text: clean } };
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

// ───────────── the stored blocks ─────────────

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
} as const satisfies Record<Attachment["kind"], string>;

function block(a: Attachment): string {
  const attrs: [string, string][] =
    a.kind === "diagnostics"
      ? [["host", a.host]]
      : a.kind === "selection"
        ? [["host", a.host], ["lines", String(a.lines)], ...(a.truncated ? [["truncated", "true"] as [string, string]] : [])]
        : a.kind === "paste"
          ? [["lines", String(a.lines)]]
          : [["name", a.name], ["lines", String(a.lines)]];
  const tag = TAGS[a.kind];
  const list = attrs.map(([name, value]) => ` ${name}="${attr(value)}"`).join("");
  return `<${tag}${list}>\n${escapeBody(a.text, tag)}\n</${tag}>`;
}

/**
 * The order attachments are stored in: the connection diagnostics, then the terminal selection (at
 * most one of each, the reason for asking first), then pasted text and files in the order the user
 * added them.
 */
export function orderAttachments(list: readonly Attachment[]): Attachment[] {
  const diagnostics = list.find((a) => a.kind === "diagnostics");
  const selection = list.find((a) => a.kind === "selection");
  const rest = list.filter((a) => a.kind === "paste" || a.kind === "file");
  return [...(diagnostics ? [diagnostics] : []), ...(selection ? [selection] : []), ...rest];
}

/** The user entry's text: the attachment blocks, then the typed text. */
export function composeMessage(typed: string, attachments: readonly Attachment[]): string {
  return [...orderAttachments(attachments).map(block), typed].join("\n\n");
}

/** A user entry's text split into its attachments and what the user typed. */
export interface MessageParts {
  attachments: Attachment[];
  typed: string;
}

const OPEN = /^<(terminal_selection|connection_diagnostics|pasted_text|file)((?: [a-z_]+="[^"]*")*)>\n/;
const DIGITS = /^\d+$/;

/** The attachment an opening tag describes, when its attributes are the kind's own, in order. */
function attachmentOf(tag: string, attrs: [string, string][], body: string): Attachment | null {
  const names = attrs.map(([name]) => name).join(",");
  const value = (i: number) => attrs[i][1];
  switch (tag) {
    case TAGS.selection:
      if ((names === "host,lines" || (names === "host,lines,truncated" && value(2) === "true")) && DIGITS.test(value(1)))
        return { kind: "selection", host: value(0), lines: Number(value(1)), truncated: attrs.length === 3, text: body };
      return null;
    case TAGS.diagnostics:
      return names === "host" ? { kind: "diagnostics", host: value(0), text: body } : null;
    case TAGS.paste:
      return names === "lines" && DIGITS.test(value(0)) ? { kind: "paste", lines: Number(value(0)), text: body } : null;
    case TAGS.file:
      return names === "name,lines" && DIGITS.test(value(1)) ? { kind: "file", name: value(0), lines: Number(value(1)), text: body } : null;
    default:
      return null;
  }
}

/**
 * The attachment block `text` starts with, as Rust's `leading_attachment` reads it: the opening tag
 * with its attributes in the kind's order, a line break, the body, a line break and the closing tag,
 * then a blank line, a line break or the end. The body ends at the first closing tag that such a
 * break or the end follows.
 */
function leadingBlock(text: string): { attachment: Attachment; rest: string } | null {
  const m = OPEN.exec(text);
  if (!m) return null;
  const tag = m[1];
  const attrs = [...m[2].matchAll(/ ([a-z_]+)="([^"]*)"/g)].map((a): [string, string] => [a[1], unattr(a[2])]);
  if (!attachmentOf(tag, attrs, "")) return null;
  const close = `\n</${tag}>`;
  for (let from = m[0].length; ; ) {
    const at = text.indexOf(close, from);
    if (at < 0) return null;
    const after = text.slice(at + close.length);
    const rest = after.startsWith("\n\n") ? after.slice(2) : after.startsWith("\n") ? after.slice(1) : after === "" ? "" : null;
    if (rest !== null) return { attachment: attachmentOf(tag, attrs, unescapeBody(text.slice(m[0].length, at), tag))!, rest };
    from = at + 1;
  }
}

/**
 * A user entry's text split into its leading attachment blocks and the typed text. Blocks are read
 * in any order; a second diagnostics or selection block, like anything that is not a valid block,
 * is part of the typed text.
 */
export function parseMessage(text: string): MessageParts {
  const parts: MessageParts = { attachments: [], typed: text };
  for (;;) {
    const b = leadingBlock(parts.typed);
    if (!b) return parts;
    const single = b.attachment.kind === "diagnostics" || b.attachment.kind === "selection";
    if (single && parts.attachments.some((a) => a.kind === b.attachment.kind)) return parts;
    parts.attachments.push(b.attachment);
    parts.typed = b.rest;
  }
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
