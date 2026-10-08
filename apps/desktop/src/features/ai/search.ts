/** History search (AI-24). */

/** How long the search field waits after the last keystroke before it asks Rust. */
export const SEARCH_DEBOUNCE_MS = 250;

export interface TextPart {
  text: string;
  match: boolean;
}

/** `text` split into the parts that match `query` (case-insensitive) and the parts between them. */
export function highlightParts(text: string, query: string): TextPart[] {
  const q = query.trim();
  if (!q || !text) return text ? [{ text, match: false }] : [];
  const re = new RegExp(q.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "giu");
  const parts: TextPart[] = [];
  let last = 0;
  for (const m of text.matchAll(re)) {
    const at = m.index ?? 0;
    if (!m[0]) break;
    if (at > last) parts.push({ text: text.slice(last, at), match: false });
    parts.push({ text: m[0], match: true });
    last = at + m[0].length;
  }
  if (last < text.length) parts.push({ text: text.slice(last), match: false });
  return parts;
}

/** A snippet on one line: whitespace runs, including line breaks, become one space. */
export function oneLine(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}
