import type { AiEntryView } from "@/ipc/types";

/** What shows under a finished reply. */
export interface ReplyEnd {
  /** What Copy takes: the reply's Markdown, empty when it has no text. */
  text: string;
  /** When its last assistant message or tool result was stored. */
  at: number;
}

export interface Replies {
  /** Each reply by its last assistant entry, which shows the reply's time and Copy. */
  ends: Map<string, ReplyEnd>;
  /** The last assistant entry of the reply at the end of the list, which a running turn may still add to. */
  open: string | null;
}

/**
 * The replies in a conversation, the assistant entries between two of the user's messages. Copy
 * takes their Markdown, joined by blank lines, without reasoning or tool calls. A message keeps its
 * leading indentation, which can make a code block, and loses only the blank lines around it.
 */
export function replies(entries: AiEntryView[]): Replies {
  const ends = new Map<string, ReplyEnd>();
  let parts: string[] = [];
  let last: string | null = null;
  let at = 0;
  const close = () => {
    if (last) ends.set(last, { text: parts.join("\n\n"), at });
    parts = [];
    last = null;
    at = 0;
  };
  for (const e of entries) {
    if (e.role === "user") close();
    else if (e.role === "assistant") {
      last = e.entry_id;
      at = Math.max(at, e.created_at);
      if (e.text.trim()) parts.push(e.text.replace(/^(?:[ \t]*\n)+/, "").trimEnd());
    } else if (e.role === "tool" && last) at = Math.max(at, e.created_at);
  }
  const open = last;
  close();
  return { ends, open };
}
