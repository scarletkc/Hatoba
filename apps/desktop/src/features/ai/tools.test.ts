import { describe, expect, it } from "vitest";
import type { AiAllowance, McpToolInfo } from "@/ipc/types";
import {
  allowanceOf,
  allowanceScope,
  callAllowed,
  callMode,
  callFileChange,
  callSummary,
  exitStatusOf,
  grantKeys,
  isFileTool,
  keySequence,
  mcpOffResult,
  mustAsk,
  needsApproval,
  needsSession,
  parseArgs,
  parseReadTerminal,
  parseSendInput,
  proxyArguments,
  proxyCall,
  runsInFrontend,
  savedFor,
  shownCall,
  shownText,
  splitMcpName,
  toolKind,
  toolLabel,
  urlDomain,
  type HostRef,
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
    expect(needsApproval("read_file", "manual")).toBe(false);
    expect(needsApproval("web_search", "manual")).toBe(false);
    expect(needsApproval("read_skill", "manual")).toBe(false);
    expect(needsApproval("run_command", "manual")).toBe(true);
    expect(needsApproval("send_input", "manual")).toBe(true);
    expect(needsApproval("fetch_url", "manual")).toBe(true);
    expect(needsApproval("mcp", "manual")).toBe(true);
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

describe("the mode a call runs in (§13.5)", () => {
  it("keeps a turn that started in manual approval asking, and asks at once after a switch to it", () => {
    expect(callMode("manual", "bypass")).toBe("manual");
    expect(callMode("bypass", "manual")).toBe("manual");
    expect(callMode("manual", "manual")).toBe("manual");
    expect(callMode("bypass", "bypass")).toBe("bypass");
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
    expect(mustAsk({ kind: "mcp", mode: "manual", allowed: false, mcp: info(false, false) })).toBe(true);
    expect(mustAsk({ kind: "mcp", mode: "manual", allowed: false, mcp: info(true, false) })).toBe(false);
    expect(mustAsk({ kind: "mcp", mode: "manual", allowed: false, mcp: info(true, true) })).toBe(false);
  });

  it("asks for MCP tools in bypass mode only when the server is set to Always ask", () => {
    expect(mustAsk({ kind: "mcp", mode: "bypass", allowed: false, mcp: info(false, false) })).toBe(false);
    expect(mustAsk({ kind: "mcp", mode: "bypass", allowed: false, mcp: info(true, true) })).toBe(true);
  });

  it("asks in both modes for an MCP call whose tool cannot be named", () => {
    expect(mustAsk({ kind: "mcp", mode: "manual", allowed: false, mcp: null })).toBe(true);
    expect(mustAsk({ kind: "mcp", mode: "bypass", allowed: false, mcp: null })).toBe(true);
  });

  it("remembers Allow for this conversation per MCP server and tool, not per called name", () => {
    const scope = allowanceScope("mcp", {}, info(false, false));
    expect(scope && grantKeys(scope)).toEqual(["mcp:s1:list_issues"]);
    // The same name can mean another server in a later request (cleaned names collide, the tools menu
    // changes the suffixes), and then it is another grant.
    const other = { ...info(false, false), server_id: "s2" };
    expect(callAllowed("mcp", {}, other, ["mcp:s1:list_issues"], [], null)).toBe(false);
    expect(callAllowed("mcp", {}, info(false, false), ["mcp:s1:list_issues"], [], null)).toBe(true);
    expect(allowanceScope("mcp", {}, null)).toBeNull();
  });

  it("allows a call through call_mcp_tool as the tool it names (AI-45)", () => {
    const kind = toolKind("call_mcp_tool");
    const args = parseArgs('{"tool":"mcp__github__list_issues","arguments":{}}') ?? {};
    const scope = allowanceScope(kind, args, info(false, false));
    expect(scope && grantKeys(scope)).toEqual(["mcp:s1:list_issues"]);
    expect(scope && savedFor(scope, { host_id: "h1", target: null })).toEqual([]);
    expect(callAllowed(kind, args, info(false, false), ["mcp:s1:list_issues"], [], null)).toBe(true);
    expect(mustAsk({ kind, mode: "manual", allowed: false, mcp: info(true, false) })).toBe(false);
  });

  it("tells the model when a server was switched off during the turn (AI-30)", () => {
    expect(mcpOffResult("github")).toBe('The MCP server "github" was switched off for this conversation, so the tool did not run.');
  });

  it("runs any call an approval covers without asking", () => {
    expect(mustAsk({ kind: "run_command", mode: "manual", allowed: true })).toBe(false);
    expect(mustAsk({ kind: "mcp", mode: "bypass", allowed: true, mcp: info(false, true) })).toBe(false);
    expect(mustAsk({ kind: "run_command", mode: "manual", allowed: false })).toBe(true);
    expect(mustAsk({ kind: "web_search", mode: "manual", allowed: false })).toBe(false);
  });

  it("allows nothing ahead from a file change's card that shows why it cannot apply (AI-39, AI-40)", () => {
    const refusal = "/etc/motd has not been read in this conversation, so edit_file did not change it.";
    // The runner: a decision with a refusal stores no allowance, whatever it asked for.
    expect(allowanceOf({ allow: "conversation" })).toBe("conversation");
    expect(allowanceOf({ allow: "conversation", refusal })).toBeUndefined();
    expect(allowanceOf({ allow: "always", refusal })).toBeUndefined();
    expect(allowanceOf({ refusal })).toBeUndefined();
  });

  it("labels built-in, MCP and unknown tools", () => {
    const t = (key: string, params?: Record<string, string | number>) => (params ? `${key}:${JSON.stringify(params)}` : key);
    expect(toolLabel(t, "run_command")).toBe("ai.tool.run_command");
    expect(toolLabel(t, "mcp__github__list_issues")).toBe('ai.tool.mcp:{"server":"github","tool":"list_issues"}');
    expect(toolLabel(t, "rm_rf")).toBe("rm_rf");
  });
});

describe("approvals of commands, pages and tools (AI-19)", () => {
  const web: HostRef = { host_id: "h-web", target: null };
  const quick: HostRef = { host_id: null, target: "root@10.0.0.5:22" };
  const saved: AiAllowance[] = [
    { kind: "command", host_id: "h-web", target: null, value: "systemctl restart" },
    { kind: "command", host_id: null, target: "root@10.0.0.5:22", value: "apt install" },
    { kind: "tool", host_id: "h-web", target: null, value: "edit_file" },
    { kind: "domain", host_id: null, target: null, value: "docs.example.com" },
  ];
  const cmd = (command: string) => ({ command });

  it("runs commands that only read, and those a prefix of the tab's host or the conversation allows", () => {
    expect(callAllowed("run_command", cmd("ls -la && git status"), null, [], [], null)).toBe(true);
    expect(callAllowed("run_command", cmd("systemctl restart nginx"), null, [], saved, web)).toBe(true);
    expect(callAllowed("run_command", cmd("systemctl restart nginx"), null, [], saved, quick)).toBe(false);
    expect(callAllowed("run_command", cmd("apt install -y jq"), null, [], saved, quick)).toBe(true);
    expect(callAllowed("run_command", cmd("systemctl stop nginx"), null, [], saved, web)).toBe(false);
    expect(callAllowed("run_command", cmd("git pull"), null, ["run_command:git pull"], [], null)).toBe(true);
    // A tool allowed for the conversation does not allow commands.
    expect(callAllowed("run_command", cmd("rm -rf /tmp/x"), null, ["run_command", "edit_file"], [], web)).toBe(false);
  });

  it("allows a page by its exact host name", () => {
    expect(callAllowed("fetch_url", { url: "https://docs.example.com/a" }, null, [], saved, null)).toBe(true);
    expect(callAllowed("fetch_url", { url: "https://evil.docs.example.com.attacker.net/" }, null, [], saved, null)).toBe(false);
    expect(callAllowed("fetch_url", { url: "https://example.com/" }, null, [], saved, null)).toBe(false);
    expect(callAllowed("fetch_url", { url: "https://example.com/x" }, null, ["fetch_url:example.com"], [], null)).toBe(true);
    expect(callAllowed("fetch_url", { url: "not a url" }, null, ["fetch_url"], [], null)).toBe(false);
  });

  it("allows edits, writes and input per tool and host", () => {
    expect(callAllowed("edit_file", {}, null, [], saved, web)).toBe(true);
    expect(callAllowed("write_file", {}, null, [], saved, web)).toBe(false);
    expect(callAllowed("edit_file", {}, null, [], saved, quick)).toBe(false);
    expect(callAllowed("send_input", {}, null, ["send_input"], [], quick)).toBe(true);
  });

  it("names what a card's approvals cover", () => {
    expect(allowanceScope("run_command", cmd("cd /srv && git pull && systemctl restart app"), null)).toEqual({
      kind: "command",
      prefixes: ["git pull", "systemctl restart"],
      savable: true,
    });
    // Interpreters, deleting and network transfers are allowed for the conversation only, also when
    // named with a directory or a version.
    for (const line of [
      "rm -rf build",
      "/bin/rm -rf build",
      "python3 deploy.py",
      "/usr/bin/python3 deploy.py",
      "python3.12 deploy.py",
      "curl -X POST https://example.com",
      "/usr/bin/curl -X POST https://example.com",
      "mkfs.ext4 /dev/sdb1",
      "/sbin/mkfs.ext4 /dev/sdb1",
      "git pull && rm x",
    ])
      expect(allowanceScope("run_command", cmd(line), null), line).toMatchObject({ kind: "command", savable: false });
    expect(allowanceScope("run_command", cmd("/usr/bin/git pull"), null)).toMatchObject({ kind: "command", savable: true });
    expect(allowanceScope("run_command", cmd("sudo reboot"), null)).toBeNull();
    expect(allowanceScope("fetch_url", { url: "https://Docs.Example.com/x?y" }, null)).toEqual({ kind: "domain", domain: "docs.example.com" });
    expect(allowanceScope("fetch_url", { url: "ftp://x" }, null)).toBeNull();
    expect(allowanceScope("write_file", {}, null)).toEqual({ kind: "tool", tool: "write_file" });
    expect(allowanceScope("read_file", {}, null)).toBeNull();
    expect(grantKeys({ kind: "command", prefixes: ["git pull", "make"], savable: true })).toEqual(["run_command:git pull", "run_command:make"]);
    expect(grantKeys({ kind: "domain", domain: "a.com" })).toEqual(["fetch_url:a.com"]);
  });

  it("saves commands and tools for the host, and domains for every tab", () => {
    expect(savedFor({ kind: "command", prefixes: ["git pull"], savable: true }, quick)).toEqual([
      { kind: "command", host_id: null, target: "root@10.0.0.5:22", value: "git pull" },
    ]);
    expect(savedFor({ kind: "command", prefixes: ["rm"], savable: false }, quick)).toEqual([]);
    expect(savedFor({ kind: "tool", tool: "send_input" }, web)).toEqual([{ kind: "tool", host_id: "h-web", target: null, value: "send_input" }]);
    expect(savedFor({ kind: "domain", domain: "a.com" }, web)).toEqual([{ kind: "domain", host_id: null, target: null, value: "a.com" }]);
    expect(savedFor({ kind: "mcp", key: "mcp:s1:x" }, web)).toEqual([]);
    expect(urlDomain(" http://[::1]:8080/x ")).toBe("[::1]");
  });
});

describe("the change of a file call from its arguments (AI-41)", () => {
  it("shows an edit's replaced text and a write's content, and nothing for other tools", () => {
    const edit = callFileChange("edit_file", JSON.stringify({ path: "/etc/a.conf", old_string: "port 80\n", new_string: "port 8080\n" }));
    expect(edit).toEqual({ path: "/etc/a.conf", before: "port 80\n", after: "port 8080\n", error: null, crlf: false });
    // Whether the write created the file is not in its arguments: the content shows as added, not as a new file.
    expect(callFileChange("write_file", JSON.stringify({ path: "b", content: "x\n" }))).toMatchObject({ before: "", after: "x\n" });
    expect(callFileChange("read_file", JSON.stringify({ path: "b" }))).toBeNull();
    expect(callFileChange("edit_file", "{")).toBeNull();
  });
});

describe("calls through call_mcp_tool (AI-45)", () => {
  it("are MCP calls, and search_tools runs without asking", () => {
    expect(toolKind("call_mcp_tool")).toBe("mcp");
    expect(toolKind("search_tools")).toBe("search_tools");
    expect(needsApproval(toolKind("call_mcp_tool"), "manual")).toBe(true);
    expect(needsApproval(toolKind("search_tools"), "manual")).toBe(false);
    expect(runsInFrontend(toolKind("search_tools"))).toBe(false);
    expect(callSummary("search_tools", '{"query":"issues"}')).toBe("issues");
  });

  it("name the tool and its arguments, an object or a string that holds one, as Rust reads them", () => {
    expect(proxyCall("call_mcp_tool", '{"tool":"mcp__gh__x","arguments":{"a":1}}')).toEqual({ tool: "mcp__gh__x", arguments: '{"a":1}' });
    expect(proxyCall("call_mcp_tool", '{"tool":" mcp__gh__x ","arguments":"{\\"a\\": 1}"}')).toEqual({ tool: "mcp__gh__x", arguments: '{"a":1}' });
    expect(proxyCall("call_mcp_tool", '{"tool":"mcp__gh__x"}')).toEqual({ tool: "mcp__gh__x", arguments: "{}" });
    expect(proxyCall("call_mcp_tool", '{"arguments":{}}')).toBeNull();
    expect(proxyCall("mcp__gh__x", '{"tool":"mcp__gh__y"}')).toBeNull();
    // Only an MCP tool: Rust does not run a built-in tool through it.
    expect(proxyCall("call_mcp_tool", '{"tool":"run_command","arguments":{"command":"rm -rf /"}}')).toBeNull();
  });

  it("show as the tool they name, and an edit changes only that tool's arguments", () => {
    const call = { id: "c1", name: "call_mcp_tool", arguments: '{"tool":"mcp__gh__create_issue","arguments":{"title":"x"}}' };
    const shown = shownCall(call);
    expect(shown).toEqual({ id: "c1", name: "mcp__gh__create_issue", arguments: '{"title":"x"}' });
    expect(toolLabel((key, params) => `${key}:${params?.server}/${params?.tool}`, shown.name)).toBe("ai.tool.mcp:gh/create_issue");
    expect(shownCall({ id: "c2", name: "run_command", arguments: "{}" }).name).toBe("run_command");
    // A call that names a built-in tool shows as the call it is, not as that tool.
    const builtin = { id: "c3", name: "call_mcp_tool", arguments: '{"tool":"write_file","arguments":{"path":"/etc/hosts"}}' };
    expect(shownCall(builtin)).toEqual(builtin);
    expect(JSON.parse(proxyArguments(call.arguments, '{"title":"y"}'))).toEqual({ tool: "mcp__gh__create_issue", arguments: { title: "y" } });
  });
});
