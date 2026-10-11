import { describe, expect, it } from "vitest";
import { translate, type Locale, type MessageKey, type Params } from "@/i18n";
import { composeMessage, fileRevertNote, parseMessage } from "./attachments";
import { noteLabel, noteTitle } from "./notes";

const tr = (locale: Locale) => (key: MessageKey, params?: Params) => translate(locale, key, params);

describe("notes as dividers (AI-05, AI-09)", () => {
  it("say that the user stopped the reply before the message (§13.1)", () => {
    const note = { kind: "interrupted" } as const;
    expect(noteLabel(tr("en"), note)).toBe("Previous reply stopped");
    expect(noteTitle(tr("en"), note)).toBe("Previous reply stopped");
    expect(noteLabel(tr("zh-CN"), note)).toBe("上一条回复已被停止");
    expect(noteLabel(tr("ja"), note)).toBe("前の返信は停止されました");
  });

  it("say where the conversation moved and where it was", () => {
    const note = { kind: "host_change", from: "staging-web", to: "prod-db" } as const;
    expect(noteLabel(tr("zh-CN"), note)).toBe("已转到 prod-db（之前在 staging-web）");
    expect(noteLabel(tr("en"), note)).toBe("Moved to prod-db (was on staging-web)");
    expect(noteTitle(tr("en"), note)).toBe("Moved to prod-db (was on staging-web)");
    // The earlier host was deleted.
    expect(noteLabel(tr("en"), { ...note, from: "" })).toBe("Moved to prod-db (was on Deleted host)");
  });

  it("name models by their names, with the IDs in the tooltip", () => {
    const note = { kind: "model_change", from: "Claude Sonnet 5.5 (claude-sonnet-5-5)", to: "Claude Opus 5.5 (claude-opus-5-5)" } as const;
    expect(noteLabel(tr("zh-CN"), note)).toBe("已切换到 Claude Opus 5.5（之前是 Claude Sonnet 5.5）");
    expect(noteLabel(tr("en"), note)).toBe("Switched to Claude Opus 5.5 (was Claude Sonnet 5.5)");
    expect(noteTitle(tr("en"), note)).toBe("Switched to Claude Opus 5.5 (claude-opus-5-5) (was Claude Sonnet 5.5 (claude-sonnet-5-5))");
    expect(noteLabel(tr("en"), { ...note, from: "qwen3:8b" })).toBe("Switched to Claude Opus 5.5 (was qwen3:8b)");
  });

  it("say whether a terminal became available or went away", () => {
    const attached = { kind: "terminal_change", to: "attached" } as const;
    const detached = { kind: "terminal_change", to: "detached" } as const;
    expect(noteLabel(tr("en"), attached)).toBe("Terminal now available");
    expect(noteLabel(tr("en"), detached)).toBe("Terminal no longer available");
    expect(noteTitle(tr("en"), detached)).toBe("Terminal no longer available");
    expect(noteLabel(tr("zh-CN"), attached)).toBe("终端已可用");
    expect(noteLabel(tr("zh-CN"), detached)).toBe("终端不再可用");
    expect(noteLabel(tr("ja"), attached)).toBe("ターミナルを使えるようになりました");
    expect(noteLabel(tr("ja"), detached)).toBe("ターミナルを使えなくなりました");
  });

  it("name the file whose change the user reverted (AI-41)", () => {
    const note = fileRevertNote("/etc/nginx/nginx.conf", "toolu_01", "edit_file", false);
    expect(noteLabel(tr("en"), note)).toBe("Reverted the change to /etc/nginx/nginx.conf");
    expect(noteLabel(tr("zh-CN"), note)).toBe("已撤销对 /etc/nginx/nginx.conf 的修改");
    expect(noteLabel(tr("ja"), note)).toBe("/etc/nginx/nginx.conf への変更を元に戻しました");
    expect(noteTitle(tr("en"), note)).toBe("Reverted the change to /etc/nginx/nginx.conf");
  });

  it("come out of a stored message in order, apart from its bubble", () => {
    const stored = composeMessage("Why?", [{ kind: "paste", lines: 1, text: "x" }], [
      { kind: "model_change", from: "A (a)", to: "B (b)" },
      { kind: "terminal_change", to: "attached" },
      { kind: "host_change", from: "one", to: "two" },
      { kind: "interrupted" },
    ]);
    const parts = parseMessage(stored);
    expect(parts.notes.map((n) => noteLabel(tr("en"), n))).toEqual([
      "Previous reply stopped",
      "Moved to two (was on one)",
      "Terminal now available",
      "Switched to B (was A)",
    ]);
    expect(parts.typed).toBe("Why?");
    expect(parts.attachments).toHaveLength(1);
  });
});

describe("the list of deferred MCP tools (AI-45)", () => {
  // `deferred::list_block` in crates/hatoba-ai/src/mcp/deferred.rs.
  const stored =
    '<mcp_tools servers="2" tools="3">\nThe MCP tools you can call with call_mcp_tool from this message on, by server.\n- files (2 tools): mcp__files__echo, mcp__files__env\n- web (1 tool): mcp__web__lookup\n</mcp_tools>\n\nhello';

  it("is a note before the message, read back with its counts", () => {
    const parts = parseMessage(stored);
    expect(parts.typed).toBe("hello");
    expect(parts.notes).toEqual([
      {
        kind: "mcp_tools",
        servers: 2,
        tools: 3,
        body: "The MCP tools you can call with call_mcp_tool from this message on, by server.\n- files (2 tools): mcp__files__echo, mcp__files__env\n- web (1 tool): mcp__web__lookup",
      },
    ]);
    expect(composeMessage("hello", [], parts.notes)).toBe(stored);
    // Counts that are not numbers make it typed text.
    expect(parseMessage(stored.replace('tools="3"', 'tools="x"')).notes).toEqual([]);
  });

  it("comes after the other notes", () => {
    const list = { kind: "mcp_tools", servers: 0, tools: 0, body: "No MCP tools are available from this message on." } as const;
    const text = composeMessage("q", [], [list, { kind: "interrupted" }]);
    expect(text.startsWith("<interrupted>")).toBe(true);
    expect(parseMessage(text).notes.map((n) => n.kind)).toEqual(["interrupted", "mcp_tools"]);
  });

  it("says how many tools it lists", () => {
    const note = (tools: number) => ({ kind: "mcp_tools", servers: 1, tools, body: "" }) as const;
    expect(noteLabel(tr("en"), note(45))).toBe("Listed 45 MCP tools");
    expect(noteLabel(tr("en"), note(1))).toBe("Listed 1 MCP tool");
    expect(noteLabel(tr("en"), note(0))).toBe("No MCP tools available");
    expect(noteLabel(tr("zh-CN"), note(45))).toBe("列出了 45 个 MCP 工具");
    expect(noteLabel(tr("ja"), note(0))).toBe("使える MCP ツールはありません");
  });
});
