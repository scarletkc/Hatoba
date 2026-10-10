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

interface Reply {
  last: string;
  parts: string[];
  at: number;
}

/**
 * The replies in a conversation, the assistant entries between two of the user's messages. Copy
 * takes their Markdown, joined by blank lines, without reasoning or tool calls. A message keeps its
 * leading indentation, which can make a code block, and loses only the blank lines around it.
 *
 * A call's result counts for its reply, even when sync stored it after the user's next message. Calls
 * and results pair as in Rust's `pair_results`: a call takes the first result with its ID stored after
 * it that no earlier call took, so a reused ID or a duplicate result doesn't move another reply's time.
 */
export function replies(entries: AiEntryView[]): Replies {
  const results = new Map<string, number[]>();
  for (const [i, e] of entries.entries()) {
    if (e.role !== "tool") continue;
    const queue = results.get(e.tool_call_id);
    if (queue) queue.push(i);
    else results.set(e.tool_call_id, [i]);
  }

  const list: Reply[] = [];
  let reply: Reply | null = null;
  for (const [i, e] of entries.entries()) {
    if (e.role === "user") reply = null;
    if (e.role !== "assistant") continue;
    if (!reply) list.push((reply = { last: e.entry_id, parts: [], at: 0 }));
    reply.last = e.entry_id;
    reply.at = Math.max(reply.at, e.created_at);
    if (e.text.trim()) reply.parts.push(e.text.replace(/^(?:[ \t]*\n)+/, "").trimEnd());
    for (const call of e.tool_calls) {
      const queue = results.get(call.id);
      while (queue?.length && queue[0] < i) queue.shift();
      const answer = queue?.shift();
      if (answer !== undefined) reply.at = Math.max(reply.at, entries[answer].created_at);
    }
  }
  return {
    ends: new Map(list.map((r) => [r.last, { text: r.parts.join("\n\n"), at: r.at }])),
    open: reply?.last ?? null,
  };
}
