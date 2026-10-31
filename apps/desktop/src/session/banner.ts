/** A run of a server message's text, or a web link in it. */
export interface MessagePart {
  text: string;
  /** Set for a link: the address it opens. */
  href?: string;
}

const LINK = /https?:\/\/[^\s<>"'`]+/gi;
/** Punctuation that ends a sentence after a link rather than belonging to it. */
const TRAILING = ".,;:!?)]}";

/**
 * `link` without the punctuation after it. A `)` or `]` that closes a `(` or `[` before it in the
 * link stays, as the `]` of an IPv6 address does.
 */
function trimLink(link: string): string {
  const open = { ")": 0, "]": 0 };
  /** The indexes of the brackets that close one. */
  const closing = new Set<number>();
  for (let i = 0; i < link.length; i++) {
    const c = link[i];
    if (c === "(") open[")"]++;
    else if (c === "[") open["]"]++;
    else if ((c === ")" || c === "]") && open[c] > 0) {
      open[c]--;
      closing.add(i);
    }
  }
  let end = link.length;
  while (end > 0 && TRAILING.includes(link[end - 1]) && !closing.has(end - 1)) end--;
  return link.slice(0, end);
}

/** `text` split into plain runs and the http(s) links in it. */
export function messageParts(text: string): MessagePart[] {
  const parts: MessagePart[] = [];
  let at = 0;
  for (const match of text.matchAll(LINK)) {
    const href = trimLink(match[0]);
    const start = match.index;
    if (start > at) parts.push({ text: text.slice(at, start) });
    parts.push({ text: href, href });
    at = start + href.length;
  }
  if (at < text.length) parts.push({ text: text.slice(at) });
  return parts;
}

/** The first http(s) link in `text`. */
export function firstLink(text: string): string | null {
  return messageParts(text).find((p) => p.href)?.href ?? null;
}
