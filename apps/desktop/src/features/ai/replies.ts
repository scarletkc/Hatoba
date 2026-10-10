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
 * leading indentation, which can make a code block, and loses only the blank lines around it. A tool
 * result counts for the reply whose call it answers, even when sync stored it after the user's next
 * message.
 */
export function replies(entries: AiEntryView[]): Replies {
  const list: Reply[] = [];
  const byCall = new Map<string, Reply>();
  let reply: Reply | null = null;
  for (const e of entries) {
    if (e.role === "user") reply = null;
    else if (e.role === "assistant") {
      if (!reply) list.push((reply = { last: e.entry_id, parts: [], at: 0 }));
      reply.last = e.entry_id;
      reply.at = Math.max(reply.at, e.created_at);
      if (e.text.trim()) reply.parts.push(e.text.replace(/^(?:[ \t]*\n)+/, "").trimEnd());
      for (const call of e.tool_calls) byCall.set(call.id, reply);
    } else if (e.role === "tool") {
      const owner = byCall.get(e.tool_call_id);
      if (owner) owner.at = Math.max(owner.at, e.created_at);
    }
  }
  return {
    ends: new Map(list.map((r) => [r.last, { text: r.parts.join("\n\n"), at: r.at }])),
    open: reply?.last ?? null,
  };
}
