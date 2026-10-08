/**
 * AI-10: the terminal selection that goes with a message. It is stored in the user's entry as a
 * leading attachment block, so neither the entry format nor the IPC contract changes:
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

const TAG = "terminal_selection";

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

/** The user entry's text: the selection block, then the typed text. */
export function composeMessage(typed: string, attachment: SelectionAttachment | null): string {
  if (!attachment) return typed;
  const truncated = attachment.truncated ? ' truncated="true"' : "";
  return `<${TAG} host="${attr(attachment.host)}" lines="${attachment.lines}"${truncated}>\n${escapeBody(attachment.text, TAG)}\n</${TAG}>\n\n${typed}`;
}

const BLOCK = /^<terminal_selection host="([^"]*)" lines="(\d+)"( truncated="true")?>\n([\s\S]*?)\n<\/terminal_selection>(?:\n\n|\n|$)/;

/** A user entry's text split into its selection block, when it starts with one, and the typed text. */
export function parseMessage(text: string): { attachment: SelectionAttachment | null; typed: string } {
  const m = BLOCK.exec(text);
  if (!m) return { attachment: null, typed: text };
  return {
    attachment: { host: unattr(m[1]), lines: Number(m[2]), truncated: !!m[3], text: unescapeBody(m[4], TAG) },
    typed: text.slice(m[0].length),
  };
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
