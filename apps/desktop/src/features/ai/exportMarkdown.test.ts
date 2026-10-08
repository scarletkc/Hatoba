import { describe, expect, it } from "vitest";
import { translate, type MessageKey, type Params } from "@/i18n";
import type { AiConversationView, AiEntryView } from "@/ipc/types";
import { conversationMarkdown, exportFileName } from "./exportMarkdown";
import { composeMessage, makeAttachment, makeDiagnostics, makePaste, textFile } from "./attachments";

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

  it("shows a terminal selection sent with a message as a labelled fenced block (AI-10)", () => {
    const text = composeMessage("What failed?", [makeAttachment("prod-api", "$ make\nmain.c: error ``` here")!]);
    const out = conversationMarkdown({ conversation, entries: [{ role: "user", entry_id: "u9", created_at: 1, text }], host: null }, { t, time });
    expect(out).toContain("## You\n\n**Selection · 2 lines** · `prod-api`\n\n````\n$ make\nmain.c: error ``` here\n````\n\nWhat failed?");
  });

  it("shows connection diagnostics before a selection, each as a labelled fenced block", () => {
    const text = composeMessage("Why?", [makeAttachment("staging-web-02", "attempt 3")!, makeDiagnostics("staging-web-02", "Error: ETIMEDOUT (ssh/timeout)")!]);
    const out = conversationMarkdown({ conversation, entries: [{ role: "user", entry_id: "u9", created_at: 1, text }], host: null }, { t, time });
    expect(out).toContain(
      "## You\n\n**Connection diagnostics · staging-web-02**\n\n```\nError: ETIMEDOUT (ssh/timeout)\n```\n\n**Selection · 1 line** · `staging-web-02`\n\n```\nattempt 3\n```\n\nWhy?",
    );
  });

  it("shows pasted text and files with their labels, the file named (AI-35)", () => {
    const file = textFile("/home/kc/nginx.conf", new TextEncoder().encode("server {\n  listen 80;\n}\n"));
    if (!file.ok) throw new Error(file.reason);
    const text = composeMessage("Check these", [makePaste("a\nb\nc")!, file.file]);
    const out = conversationMarkdown({ conversation, entries: [{ role: "user", entry_id: "u9", created_at: 1, text }], host: null }, { t, time });
    expect(out).toContain("## You\n\n**Pasted text · 3 lines**\n\n```\na\nb\nc\n```\n\n**nginx.conf · 3 lines**\n\n```\nserver {\n  listen 80;\n}\n\n```\n\nCheck these");
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
