import { describe, expect, it } from "vitest";
import { translate, type MessageKey, type Params } from "@/i18n";
import type { AiConversationView, AiEntryView } from "@/ipc/types";
import { conversationMarkdown, exportFileName } from "./exportMarkdown";

const t = (key: MessageKey, params?: Params) => translate("en", key, params);
const time = (ms: number) => `T${ms}`;

const conversation: AiConversationView = {
  id: "c1",
  title: "Disk on prod",
  host_id: "h1",
  pinned: false,
  context_start: "s1",
  created_at: 1000,
  updated_at: 1000,
  last_activity: 5000,
};

const entries: AiEntryView[] = [
  { role: "user", entry_id: "u1", created_at: 1, text: "How full is the disk?" },
  {
    role: "assistant",
    entry_id: "a1",
    created_at: 2,
    provider_id: "p",
    model_id: "claude-sonnet-5-5",
    text: "Let me check.",
    reasoning: "Run df.\nThen answer.",
    tool_calls: [
      { id: "c1", name: "run_command", arguments: '{"command":"df -h /"}' },
      { id: "c2", name: "mcp__github__create_issue", arguments: '{"title":"disk"}' },
      { id: "c3", name: "fetch_url", arguments: '{"url":"https://example.com"}' },
    ],
    finish: "tool_calls",
    usage: null,
  },
  { role: "tool", entry_id: "t1", created_at: 3, tool_call_id: "c1", status: "ok", content: "exit status: 0\n```\n48% used" },
  { role: "tool", entry_id: "t2", created_at: 3, tool_call_id: "c2", status: "rejected", content: "not now" },
  { role: "summary", entry_id: "s1", created_at: 4, text: "The disk is 48% full." },
  { role: "user", entry_id: "u2", created_at: 5, text: "Thanks" },
];

describe("Markdown export (AI-25)", () => {
  const md = conversationMarkdown({ conversation, entries, host: { name: "prod-api", target: "deploy@10.0.0.4:22" } }, { t, time });

  it("starts with the title, the host and the creation time", () => {
    expect(md.startsWith("# Disk on prod\n\n- Host: prod-api (`deploy@10.0.0.4:22`)\n- Created: T1000\n\n")).toBe(true);
  });

  it("puts the user's and the assistant's messages under headings, with reasoning as a quote", () => {
    expect(md).toContain("## You\n\nHow full is the disk?");
    expect(md).toContain("## Assistant · claude-sonnet-5-5\n\n> **Reasoning**\n>\n> Run df.\n> Then answer.\n\nLet me check.");
  });

  it("shows each call with its input and result in fenced blocks that the content cannot close", () => {
    expect(md).toContain('**Run command** · Exit 0\n\nInput:\n\n```json\n{\n  "command": "df -h /"\n}\n```\n\nOutput:\n\n````\nexit status: 0\n```\n48% used\n````');
    expect(md).toContain("**github · create_issue** · Rejected");
    expect(md).toContain("Reason for rejecting:\n\n```\nnot now\n```");
    expect(md).toContain("**Fetch page** · Not run");
  });

  it("marks the summary and where the context starts", () => {
    expect(md).toContain("---\n\n*Everything above is outside the context*\n\n## Summary of the compacted conversation\n\nThe disk is 48% full.");
    expect(md.endsWith("## You\n\nThanks\n")).toBe(true);
  });

  it("names a deleted host and an untitled conversation", () => {
    const out = conversationMarkdown({ conversation: { ...conversation, title: "" }, entries: [], host: null }, { t, time });
    expect(out).toBe("# Untitled conversation\n\n- Host: Deleted host\n- Created: T1000\n");
  });
});

describe("export file name", () => {
  it("drops characters file systems refuse", () => {
    expect(exportFileName('nginx: 502 / "upstream"?', "conversation")).toBe("nginx 502 upstream.md");
    expect(exportFileName("  ...  ", "conversation")).toBe("conversation.md");
    expect(exportFileName("trailing dots...", "c")).toBe("trailing dots.md");
    expect(exportFileName("排查 prod-api 上的 nginx 502", "对话")).toBe("排查 prod-api 上的 nginx 502.md");
  });
});
