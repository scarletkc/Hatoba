/**
 * AI-10: what goes with a message, the terminal selection or the diagnostics of a failed
 * connection. Each is stored in the user's entry as a leading attachment block, so neither the
 * entry format nor the IPC contract changes:
 *
 *   <terminal_selection host="prod-api" lines="3">
 *   …the selected text…
 *   </terminal_selection>
 *
 *   the typed text
 *
 * The block format and its escaping are specified once, in docs/hatoba-spec.md §13.3 ("Attachment
 * blocks"); Rust's `title_of` (src-tauri/src/ai.rs) reads the same blocks to title a conversation.
 */

/** As for tool results (§13.4): over this many characters, the first 4,000 and the last 12,000 are kept. */
export const SELECTION_MAX_CHARS = 16_000;
const HEAD_CHARS = 4_000;
const TAIL_CHARS = 12_000;

export interface SelectionAttachment {
  /** The tab host's display name. */
  host: string;
  /** Lines in the selection as the user made it. */
  lines: number;
  /** The selected text, clipped to the limit. */
  text: string;
  truncated: boolean;
}

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

/** What a selection attaches, or null when it is empty or only whitespace. */
export function makeAttachment(host: string, selection: string): SelectionAttachment | null {
  const clean = cleanTerminalText(selection);
  if (!clean.trim()) return null;
  return { host, lines: clean.split("\n").length, ...clipText(clean) };
}

// ───────────── the stored block ─────────────

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

/**
 * The diagnostics of a failed connection, attached from the terminal's error card. They are already
 * free of secrets (`diagnosticsText` in features/terminal/diagnostics.ts) and hold the host's
 * address and user name, so they reach the provider only when the user attaches them.
 */
export interface DiagnosticsAttachment {
  /** The host's display name. */
  host: string;
  text: string;
}

/** What diagnostics attach, clipped like a selection, or null when there is no text. */
export function makeDiagnostics(host: string, text: string): DiagnosticsAttachment | null {
  const clean = text.replace(/\r\n?/g, "\n").trim();
  return clean ? { host, text: clipText(clean).text } : null;
}

/** A user entry's text split into its attachment blocks and what the user typed. */
export interface MessageParts {
  diagnostics: DiagnosticsAttachment | null;
  selection: SelectionAttachment | null;
  typed: string;
}

const SELECTION_TAG = "terminal_selection";
const DIAGNOSTICS_TAG = "connection_diagnostics";

function block(tag: string, attrs: [string, string][], body: string): string {
  const list = attrs.map(([name, value]) => ` ${name}="${attr(value)}"`).join("");
  return `<${tag}${list}>\n${escapeBody(body, tag)}\n</${tag}>`;
}

/**
 * The user entry's text: the attachment blocks, then the typed text. A message carries at most one
 * block of each kind, the diagnostics before the selection, so the reason for asking comes first.
 */
export function composeMessage(typed: string, { diagnostics = null, selection = null }: Partial<Omit<MessageParts, "typed">>): string {
  const blocks: string[] = [];
  if (diagnostics) blocks.push(block(DIAGNOSTICS_TAG, [["host", diagnostics.host]], diagnostics.text));
  if (selection) {
    const attrs: [string, string][] = [["host", selection.host], ["lines", String(selection.lines)]];
    if (selection.truncated) attrs.push(["truncated", "true"]);
    blocks.push(block(SELECTION_TAG, attrs, selection.text));
  }
  return [...blocks, typed].join("\n\n");
}

const OPEN = /^<(terminal_selection|connection_diagnostics)((?: [a-z_]+="[^"]*")*)>\n/;

/**
 * The attachment block `text` starts with, as Rust's `leading_attachment` reads it: the opening tag
 * with its attributes in the kind's order, a line break, the body, a line break and the closing tag,
 * then a blank line, a line break or the end. The body ends at the first closing tag that such a
 * break or the end follows.
 */
function leadingBlock(text: string): { tag: string; attrs: [string, string][]; body: string; rest: string } | null {
  const m = OPEN.exec(text);
  if (!m) return null;
  const tag = m[1];
  const attrs = [...m[2].matchAll(/ ([a-z_]+)="([^"]*)"/g)].map((a): [string, string] => [a[1], unattr(a[2])]);
  const names = attrs.map(([name]) => name).join(",");
  const valid =
    tag === SELECTION_TAG
      ? (names === "host,lines" || (names === "host,lines,truncated" && attrs[2][1] === "true")) && /^\d+$/.test(attrs[1][1])
      : names === "host";
  if (!valid) return null;
  const close = `\n</${tag}>`;
  for (let from = m[0].length; ; ) {
    const at = text.indexOf(close, from);
    if (at < 0) return null;
    const after = text.slice(at + close.length);
    const rest = after.startsWith("\n\n") ? after.slice(2) : after.startsWith("\n") ? after.slice(1) : after === "" ? "" : null;
    if (rest !== null) return { tag, attrs, body: unescapeBody(text.slice(m[0].length, at), tag), rest };
    from = at + 1;
  }
}

/** A user entry's text split into its leading attachment blocks, at most one of each kind, and the typed text. */
export function parseMessage(text: string): MessageParts {
  const parts: MessageParts = { diagnostics: null, selection: null, typed: text };
  for (;;) {
    const b = leadingBlock(parts.typed);
    if (!b) return parts;
    if (b.tag === SELECTION_TAG) {
      if (parts.selection) return parts;
      parts.selection = { host: b.attrs[0][1], lines: Number(b.attrs[1][1]), truncated: b.attrs.length === 3, text: b.body };
    } else {
      if (parts.diagnostics) return parts;
      parts.diagnostics = { host: b.attrs[0][1], text: b.body };
    }
    parts.typed = b.rest;
  }
}

// ───────────── the chip ─────────────

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
