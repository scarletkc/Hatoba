import { describe, expect, it } from "vitest";
import {
  callSummary,
  exitStatusOf,
  keySequence,
  needsApproval,
  needsSession,
  parseArgs,
  parseReadTerminal,
  parseSendInput,
  runsInFrontend,
  splitMcpName,
  toolKind,
} from "./tools";

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
  });

  it("runs everything in bypass mode", () => {
    for (const kind of ["read_terminal", "run_command", "send_input", "web_search", "fetch_url", "read_skill", "mcp"] as const) {
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
  });

  it("finds the exit status in a run_command result", () => {
    expect(exitStatusOf("exit status: 0\nstdout:\nok")).toBe(0);
    expect(exitStatusOf("Exit code 127")).toBe(127);
    expect(exitStatusOf("no status here")).toBeNull();
  });
});
