import type { AiEntryView } from "@/ipc/types";

export interface Replies {
  /**
   * Each reply by its last assistant entry, which shows the reply's time and Copy: the reply's text,
   * empty when it has none.
   */
  texts: Map<string, string>;
  /** The last assistant entry of the reply at the end of the list, which a running turn may still add to. */
  open: string | null;
}

/**
 * The replies in a conversation, the assistant entries between two of the user's messages. Copy
 * takes their Markdown, joined by blank lines, without reasoning or tool calls.
 */
export function replies(entries: AiEntryView[]): Replies {
  const texts = new Map<string, string>();
  let parts: string[] = [];
  let last: string | null = null;
  const close = () => {
    if (last) texts.set(last, parts.join("\n\n"));
    parts = [];
    last = null;
  };
  for (const e of entries) {
    if (e.role === "user") close();
    else if (e.role === "assistant") {
      last = e.entry_id;
      const text = e.text.trim();
      if (text) parts.push(text);
    }
  }
  const open = last;
  close();
  return { texts, open };
}
