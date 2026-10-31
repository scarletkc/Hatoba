/** A run of a server message's text, or a web link in it. */
export interface MessagePart {
  text: string;
  /** Set for a link: the address it opens. */
  href?: string;
}

const LINK = /https?:\/\/[^\s<>"'`]+/g;
/** Punctuation that ends a sentence after a link rather than belonging to it. */
const TRAILING = /[.,;:!?)\]}]+$/;

/** `text` split into plain runs and the http(s) links in it. */
export function messageParts(text: string): MessagePart[] {
  const parts: MessagePart[] = [];
  let at = 0;
  for (const match of text.matchAll(LINK)) {
    const href = match[0].replace(TRAILING, "");
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
