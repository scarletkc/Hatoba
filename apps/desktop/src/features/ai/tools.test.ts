import { describe, expect, it } from "vitest";
import type { McpToolInfo } from "@/ipc/types";
import {
  callSummary,
  exitStatusOf,
  grantKey,
  isFileTool,
  keySequence,
  mcpOffResult,
  mustAsk,
  needsApproval,
  needsSession,
  parseArgs,
  parseReadTerminal,
  parseSendInput,
  runsInFrontend,
  shownText,
  splitMcpName,
  toolKind,
  toolLabel,
} from "./tools";

describe("the approval card's view of an input (AI-17)", () => {
  it("keeps plain text as it is, one entry per line", () => {
    expect(shownText("df -h /")).toEqual({ lines: ["df -h /"], escaped: false });
    expect(shownText("cd /srv\nmake deploy")).toEqual({ lines: ["cd /srv", "make deploy"], escaped: false });
    // A final line break (it presses Enter) leaves an empty last line, so it can be marked.
    expect(shownText("uptime\n").lines).toEqual(["uptime", ""]);
    expect(shownText("日本語 é 😀").escaped).toBe(false);
  });

  it("shows control characters as escapes", () => {
    expect(shownText("ls\tx")).toEqual({ lines: ["ls\\tx"], escaped: true });
    expect(shownText("echo hi\r\nrm -rf ~").lines).toEqual(["echo hi\\r", "rm -rf ~"]);
    expect(shownText("printf '\x1b[2J'").lines).toEqual(["printf '\\x1b[2J'"]);
    expect(shownText("a\x00b\x7fc\x85").lines).toEqual(["a\\x00b\\x7fc\\x85"]);
  });

  it("shows bidirectional and zero-width characters as escapes", () => {
    // `rm -rf /` written as `echo safe` with a right-to-left override.
    expect(shownText("echo \u202e/ fr- mr").lines).toEqual(["echo \\u{202e}/ fr- mr"]);
    expect(shownText("a\u200bb\u2066c\u2069d\ufeff").lines).toEqual(["a\\u{200b}b\\u{2066}c\\u{2069}d\\u{feff}"]);
    expect(shownText("\u061c").escaped).toBe(true);
  });
});

describe("tool kinds", () => {
  it("names built-in, MCP and unknown tools", () => {
    expect(toolKind("read_terminal")).toBe("read_terminal");
    expect(toolKind("run_command")).toBe("run_command");
    expect(toolKind("mcp__github__create_issue")).toBe("mcp");
    expect(toolKind("mcp__broken")).toBe("unknown");
    expect(toolKind("rm_rf")).toBe("unknown");
    expect(splitMcpName("mcp__git_hub__list__all")).toEqual(["git_hub", "list__all"]);
  });

  it("runs the xterm tools in the frontend and needs a session for terminal tools", () => {
    expect(runsInFrontend("read_terminal")).toBe(true);
    expect(runsInFrontend("send_input")).toBe(true);
    expect(runsInFrontend("run_command")).toBe(false);
    expect(needsSession("run_command")).toBe(true);
    expect(needsSession("fetch_url")).toBe(false);
    expect(needsSession("web_search")).toBe(false);
  });

  it("runs the file tools in Rust on the tab's session (AI-38…40)", () => {
    for (const name of ["read_file", "edit_file", "write_file"]) {
      const kind = toolKind(name);
      expect(kind).toBe(name);
      expect(isFileTool(kind)).toBe(true);
      expect(runsInFrontend(kind)).toBe(false);
      expect(needsSession(kind)).toBe(true);
    }
    expect(isFileTool("run_command")).toBe(false);
  });
});

describe("permission modes (§13.5)", () => {
  it("asks in manual mode for everything but reading and searching", () => {
    expect(needsApproval("read_terminal", "manual")).toBe(false);
    expect(needsApproval("web_search", "manual")).toBe(false);
    expect(needsApproval("read_skill", "manual")).toBe(false);
    expect(needsApproval("run_command", "manual")).toBe(true);
    expect(needsApproval("send_input", "manual")).toBe(true);
    expect(needsApproval("fetch_url", "manual")).toBe(true);
    expect(needsApproval("mcp", "manual")).toBe(true);
    // read_file asks too: it can send ~/.ssh or .env to the provider.
    expect(needsApproval("read_file", "manual")).toBe(true);
    expect(needsApproval("edit_file", "manual")).toBe(true);
    expect(needsApproval("write_file", "manual")).toBe(true);
  });

  it("runs everything in bypass mode", () => {
    for (const kind of ["read_terminal", "run_command", "send_input", "read_file", "edit_file", "write_file", "web_search", "fetch_url", "read_skill", "mcp"] as const) {
      expect(needsApproval(kind, "bypass")).toBe(false);
    }
  });
});

describe("read_terminal arguments (AI-11)", () => {
  it("defaults to 100 lines and clamps to 0..1000", () => {
    expect(parseReadTerminal("{}")).toEqual({ ok: true, value: { lines: 100 } });
    expect(parseReadTerminal("")).toEqual({ ok: true, value: { lines: 100 } });
    expect(parseReadTerminal('{"lines": 5000}')).toEqual({ ok: true, value: { lines: 1000 } });
    expect(parseReadTerminal('{"lines": -3}')).toEqual({ ok: true, value: { lines: 0 } });
    expect(parseReadTerminal('{"lines": "40"}')).toEqual({ ok: true, value: { lines: 40 } });
    expect(parseReadTerminal('{"lines": 12.7}')).toEqual({ ok: true, value: { lines: 12 } });
  });

  it("rejects malformed arguments", () => {
    expect(parseReadTerminal("[1]").ok).toBe(false);
    expect(parseReadTerminal("{oops").ok).toBe(false);
    expect(parseReadTerminal('{"lines": "many"}').ok).toBe(false);
  });
});

describe("send_input arguments (AI-13)", () => {
  it("parses text, key and wait with defaults and clamps", () => {
    expect(parseSendInput('{"text":"ls -la","key":"enter"}')).toEqual({ ok: true, value: { text: "ls -la", key: "enter", waitSeconds: 10 } });
    expect(parseSendInput('{"text":"q","wait_seconds":500}')).toEqual({ ok: true, value: { text: "q", key: null, waitSeconds: 120 } });
    expect(parseSendInput('{"key":"CTRL_C","wait_seconds":0}')).toEqual({ ok: true, value: { text: "", key: "ctrl_c", waitSeconds: 1 } });
  });

  it("rejects bad keys, bad types and empty input", () => {
    expect(parseSendInput('{"text":"x","key":"f5"}').ok).toBe(false);
    expect(parseSendInput('{"text":42}').ok).toBe(false);
    expect(parseSendInput("{}").ok).toBe(false);
    expect(parseSendInput('{"text":"x","wait_seconds":"soon"}').ok).toBe(false);
  });

  it("maps keys, following application cursor mode for arrows", () => {
    expect(keySequence("enter", false)).toBe("\r");
    expect(keySequence("tab", false)).toBe("\t");
    expect(keySequence("esc", false)).toBe("\x1b");
    expect(keySequence("ctrl_c", false)).toBe("\x03");
    expect(keySequence("ctrl_d", false)).toBe("\x04");
    expect(keySequence("up", false)).toBe("\x1b[A");
    expect(keySequence("up", true)).toBe("\x1bOA");
    expect(keySequence("left", true)).toBe("\x1bOD");
    expect(keySequence("right", false)).toBe("\x1b[C");
    expect(keySequence("down", false)).toBe("\x1b[B");
  });
});

describe("display helpers", () => {
  it("parses only JSON objects", () => {
    expect(parseArgs('{"a":1}')).toEqual({ a: 1 });
    expect(parseArgs("null")).toBeNull();
    expect(parseArgs("  ")).toEqual({});
  });

  it("summarizes calls in one line", () => {
    expect(callSummary("run_command", '{"command":"df -h\\nfree -m"}')).toBe("df -h …");
    expect(callSummary("send_input", '{"text":"y","key":"enter"}')).toBe("y [enter]");
    expect(callSummary("fetch_url", '{"url":"https://example.com"}')).toBe("https://example.com");
    expect(callSummary("mcp__x__y", "{}")).toBe("");
    expect(callSummary("read_file", '{"path":"/etc/hosts"}')).toBe("/etc/hosts");
    expect(callSummary("read_file", '{"path":"/var/log/syslog","offset":200}')).toBe("/var/log/syslog:200");
    expect(callSummary("edit_file", '{"path":"~/.bashrc","old_string":"a","new_string":"b"}')).toBe("~/.bashrc");
    expect(callSummary("write_file", '{"path":"/srv/new.txt","content":"x"}')).toBe("/srv/new.txt");
  });

  it("finds the exit status in a run_command result", () => {
    expect(exitStatusOf("exit status: 0\nstdout:\nok")).toBe(0);
    expect(exitStatusOf("Exit code 127")).toBe(127);
    expect(exitStatusOf("no status here")).toBeNull();
  });
});

describe("asking with MCP tools and Allow for this conversation (AI-19, AI-31)", () => {
  const info = (always_allow: boolean, always_ask: boolean): McpToolInfo => ({
    server_id: "s1",
    server_name: "github",
    always_ask,
    tool: {
      name: "mcp__github__list_issues",
      tool: "list_issues",
      description: "Lists issues.",
      annotations: { title: null, read_only_hint: true, destructive_hint: null, idempotent_hint: null, open_world_hint: null },
      always_allow,
    },
  });

  it("asks for MCP tools in manual mode unless the tool is always allowed on this device", () => {
    expect(mustAsk({ kind: "mcp", mode: "manual", allowedHere: false, mcp: info(false, false) })).toBe(true);
    expect(mustAsk({ kind: "mcp", mode: "manual", allowedHere: false, mcp: info(true, false) })).toBe(false);
    expect(mustAsk({ kind: "mcp", mode: "manual", allowedHere: false, mcp: info(true, true) })).toBe(false);
  });

  it("asks for MCP tools in bypass mode only when the server is set to Always ask", () => {
    expect(mustAsk({ kind: "mcp", mode: "bypass", allowedHere: false, mcp: info(false, false) })).toBe(false);
    expect(mustAsk({ kind: "mcp", mode: "bypass", allowedHere: false, mcp: info(true, true) })).toBe(true);
  });

  it("asks in both modes for an MCP call whose tool cannot be named", () => {
    expect(mustAsk({ kind: "mcp", mode: "manual", allowedHere: false, mcp: null })).toBe(true);
    expect(mustAsk({ kind: "mcp", mode: "bypass", allowedHere: false, mcp: null })).toBe(true);
  });

  it("remembers Allow for this conversation per MCP server and tool, not per called name", () => {
    expect(grantKey("run_command", "run_command", null)).toBe("run_command");
    expect(grantKey("mcp", "mcp__github__list_issues", info(false, false))).toBe("mcp:s1:list_issues");
    // The same name can mean another server in a later request (cleaned names collide, the tools menu
    // changes the suffixes), and then it is another grant.
    const other = { ...info(false, false), server_id: "s2" };
    expect(grantKey("mcp", "mcp__github__list_issues", other)).toBe("mcp:s2:list_issues");
    expect(grantKey("mcp", "mcp__github__list_issues", null)).toBeNull();
  });

  it("tells the model when a server was switched off during the turn (AI-30)", () => {
    expect(mcpOffResult("github")).toBe('The MCP server "github" was switched off for this conversation, so the tool did not run.');
  });

  it("runs any tool allowed for this conversation without asking", () => {
    expect(mustAsk({ kind: "run_command", mode: "manual", allowedHere: true })).toBe(false);
    expect(mustAsk({ kind: "mcp", mode: "bypass", allowedHere: true, mcp: info(false, true) })).toBe(false);
    expect(mustAsk({ kind: "run_command", mode: "manual", allowedHere: false })).toBe(true);
    expect(mustAsk({ kind: "web_search", mode: "manual", allowedHere: false })).toBe(false);
    // AI-19 per tool: allowing edit_file leaves write_file asking.
    expect(grantKey("edit_file", "edit_file", null)).toBe("edit_file");
    expect(grantKey("write_file", "write_file", null)).not.toBe(grantKey("edit_file", "edit_file", null));
  });

  it("labels built-in, MCP and unknown tools", () => {
    const t = (key: string, params?: Record<string, string | number>) => (params ? `${key}:${JSON.stringify(params)}` : key);
    expect(toolLabel(t, "run_command")).toBe("ai.tool.run_command");
    expect(toolLabel(t, "mcp__github__list_issues")).toBe('ai.tool.mcp:{"server":"github","tool":"list_issues"}');
    expect(toolLabel(t, "rm_rf")).toBe("rm_rf");
  });
});
