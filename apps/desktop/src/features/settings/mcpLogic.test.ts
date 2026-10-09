import { describe, expect, it } from "vitest";
import type { McpServerView, McpToolAnnotations, McpToolView } from "@/ipc/types";
import {
  annotationKeys,
  argRows,
  commandPreview,
  formatCommandLine,
  hasMcpProblems,
  initialMcpForm,
  isBlankRow,
  looksLikeCommandLine,
  mcpFormToInput,
  newSecretRow,
  quoteArg,
  savedSecretRows,
  secretInputs,
  serverState,
  splitCommandLine,
  stderrText,
  toolAllowed,
  transportSummary,
  upsertServer,
  validateMcpForm,
  validateSecretRows,
  withAlwaysAllow,
  type McpForm,
  type SecretRow,
} from "./mcpLogic";

const stdioServer = (extra: Partial<McpServerView> = {}): McpServerView => ({
  id: "m1",
  name: "filesystem",
  transport: { kind: "stdio", command: "npx", args: ["-y", "@modelcontextprotocol/server-filesystem", "C:\\Users\\kc\\My Docs"], env_keys: ["API_TOKEN"] },
  always_ask: false,
  enabled: true,
  always_allow: false,
  always_allow_tools: [],
  updated_at: 1,
  ...extra,
});

const httpServer = (): McpServerView => ({
  ...stdioServer(),
  id: "m2",
  name: "github",
  transport: { kind: "http", url: "https://api.example.com/mcp", header_keys: ["Authorization"] },
});

const row = (key: string, value: string, extra: Partial<SecretRow> = {}): SecretRow => ({
  ...newSecretRow(),
  key,
  draft: { mode: "replace", value },
  ...extra,
});

describe("quoteArg and formatCommandLine", () => {
  it("leaves plain arguments alone", () => {
    expect(quoteArg("-y")).toBe("-y");
    expect(quoteArg("@modelcontextprotocol/server-filesystem")).toBe("@modelcontextprotocol/server-filesystem");
    expect(quoteArg("C:\\Users\\kc\\docs")).toBe("C:\\Users\\kc\\docs");
    expect(quoteArg("--port=8080")).toBe("--port=8080");
  });

  it("quotes empty arguments and ones with spaces or shell characters", () => {
    expect(quoteArg("")).toBe('""');
    expect(quoteArg("My Docs")).toBe('"My Docs"');
    expect(quoteArg("a;rm -rf /")).toBe('"a;rm -rf /"');
    expect(quoteArg("$HOME")).toBe('"$HOME"');
  });

  it("escapes quotes and shows control characters", () => {
    expect(quoteArg('say "hi"')).toBe('"say \\"hi\\""');
    expect(quoteArg("a\nb")).toBe('"a\\nb"');
    expect(quoteArg("a\u001bb")).toBe('"a\\x1bb"');
  });

  it("joins the command and its arguments", () => {
    expect(formatCommandLine("npx", ["-y", "server", "My Docs"])).toBe('npx -y server "My Docs"');
    expect(formatCommandLine("node", [])).toBe("node");
    expect(formatCommandLine("a\nb", [])).toBe("a\\nb");
  });
});

describe("splitCommandLine", () => {
  it("splits on whitespace", () => {
    expect(splitCommandLine("npx -y  @scope/server")).toEqual(["npx", "-y", "@scope/server"]);
    expect(splitCommandLine("  node  ")).toEqual(["node"]);
    expect(splitCommandLine("")).toEqual([]);
  });

  it("groups quoted words", () => {
    expect(splitCommandLine('node "My Docs/server.js" \'two words\'')).toEqual(["node", "My Docs/server.js", "two words"]);
    expect(splitCommandLine('echo ""')).toEqual(["echo", ""]);
    expect(splitCommandLine('echo "say \\"hi\\""')).toEqual(["echo", 'say "hi"']);
    expect(splitCommandLine('a"b c"d')).toEqual(["ab cd"]);
  });

  it("keeps Windows paths", () => {
    expect(splitCommandLine("node C:\\tools\\server.js")).toEqual(["node", "C:\\tools\\server.js"]);
    expect(splitCommandLine('"\\\\host\\share\\x.exe" --flag')).toEqual(["\\\\host\\share\\x.exe", "--flag"]);
    expect(splitCommandLine("node My\\ Docs")).toEqual(["node", "My Docs"]);
  });

  it("gives up on an unclosed quote", () => {
    expect(splitCommandLine('node "oops')).toBeNull();
  });

  it("reads back what formatCommandLine wrote", () => {
    const args = ["-y", "My Docs", 'say "hi"', "", "plain"];
    expect(splitCommandLine(formatCommandLine("npx", args))).toEqual(["npx", ...args]);
  });
});

describe("looksLikeCommandLine", () => {
  it("spots a whole command line in the command field", () => {
    expect(looksLikeCommandLine("npx -y server")).toBe(true);
    expect(looksLikeCommandLine("uvx mcp-server-git --repository .")).toBe(true);
  });

  it("accepts a program or a path, even with spaces", () => {
    expect(looksLikeCommandLine("npx")).toBe(false);
    expect(looksLikeCommandLine("")).toBe(false);
    expect(looksLikeCommandLine("C:\\Program Files\\nodejs\\node.exe")).toBe(false);
    expect(looksLikeCommandLine("/usr/local/bin/my server")).toBe(false);
    expect(looksLikeCommandLine('"C:\\Program Files\\x.exe" --a')).toBe(false);
  });
});

describe("environment and header rows", () => {
  it("flags missing, invalid, and repeated names and a missing value", () => {
    const rows = [row("TOKEN", "x"), row("token", "y"), row("", "z"), row("A=B", "v"), row("NEW", "")];
    expect(validateSecretRows(rows, "env")).toEqual([null, { key: "duplicate" }, { key: "empty" }, { key: "invalid" }, { value: "missing" }]);
  });

  it("checks header names as HTTP tokens", () => {
    expect(validateSecretRows([row("Authorization", "Bearer x"), row("X-Api-Key", "k")], "header")).toEqual([null, null]);
    expect(validateSecretRows([row("Bad Header", "x"), row("a:b", "x")], "header")).toEqual([{ key: "invalid" }, { key: "invalid" }]);
  });

  it("lets a saved row stay without a value", () => {
    expect(validateSecretRows(savedSecretRows(["TOKEN"]), "env")).toEqual([null]);
  });

  it("ignores blank rows", () => {
    expect(isBlankRow(newSecretRow())).toBe(true);
    expect(validateSecretRows([newSecretRow()], "env")).toEqual([null]);
    expect(secretInputs([newSecretRow(), row("A", "1")])).toEqual([{ key: "A", value: "1" }]);
  });

  it("keeps a saved value with null and replaces it only with a typed value", () => {
    const [kept, replaced, emptied] = savedSecretRows(["KEEP", "REPLACE", "EMPTY"]);
    replaced.draft = { mode: "replace", value: " new " };
    emptied.draft = { mode: "replace", value: "" };
    expect(secretInputs([kept, replaced, emptied])).toEqual([
      { key: "KEEP", value: null },
      { key: "REPLACE", value: "new" },
      { key: "EMPTY", value: null },
    ]);
  });

  it("sends a new row's value and trims the name", () => {
    expect(secretInputs([row(" TOKEN ", " abc ")])).toEqual([{ key: "TOKEN", value: "abc" }]);
  });
});

describe("the form", () => {
  it("starts empty as a stdio server", () => {
    const f = initialMcpForm(null);
    expect(f).toMatchObject({ name: "", kind: "stdio", command: "", args: [], env: [], url: "", headers: [], alwaysAsk: false });
  });

  it("is filled from a stdio server, with its saved variables as Saved rows", () => {
    const f = initialMcpForm(stdioServer({ always_ask: true }));
    expect(f).toMatchObject({ name: "filesystem", kind: "stdio", command: "npx", alwaysAsk: true });
    expect(f.args.map((a) => a.value)).toEqual(["-y", "@modelcontextprotocol/server-filesystem", "C:\\Users\\kc\\My Docs"]);
    expect(f.env).toMatchObject([{ key: "API_TOKEN", saved: true, draft: { mode: "keep" } }]);
    expect(f.headers).toEqual([]);
  });

  it("is filled from an http server", () => {
    const f = initialMcpForm(httpServer());
    expect(f).toMatchObject({ kind: "http", url: "https://api.example.com/mcp" });
    expect(f.headers).toMatchObject([{ key: "Authorization", saved: true }]);
    expect(f.env).toEqual([]);
  });

  const valid = (extra: Partial<McpForm> = {}): McpForm => ({ ...initialMcpForm(null), name: "fs", command: "npx", ...extra });

  it("accepts a complete stdio server and a complete http server", () => {
    expect(hasMcpProblems(validateMcpForm(valid()))).toBe(false);
    expect(hasMcpProblems(validateMcpForm(valid({ kind: "http", url: "https://example.com/mcp" })))).toBe(false);
  });

  it("flags a missing or taken name, case-insensitively", () => {
    expect(validateMcpForm(valid({ name: " " })).name).toBe("empty");
    expect(validateMcpForm(valid({ name: "GitHub" }), ["github"]).name).toBe("taken");
    expect(validateMcpForm(valid({ name: "fs" }), ["github"]).name).toBeNull();
  });

  it("checks only the transport that is chosen", () => {
    expect(validateMcpForm(valid({ command: "", url: "nonsense" })).command).toBe("empty");
    expect(validateMcpForm(valid({ command: "", url: "nonsense" })).url).toBeNull();
    const http = validateMcpForm(valid({ kind: "http", command: "", url: "nonsense" }));
    expect(http.command).toBeNull();
    expect(http.url).toBe("invalid");
    expect(validateMcpForm(valid({ kind: "http", url: "" })).url).toBe("empty");
  });

  it("checks the rows of the chosen transport", () => {
    const f = valid({ env: [row("A=B", "x")], headers: [row("Bad Name", "x")] });
    expect(validateMcpForm(f).secrets).toEqual([{ key: "invalid" }]);
    expect(validateMcpForm({ ...f, kind: "http", url: "https://x.test" }).secrets).toEqual([{ key: "invalid" }]);
    expect(hasMcpProblems(validateMcpForm(valid({ env: [row("OK", "x")] })))).toBe(false);
  });

  it("builds the input for a stdio server, dropping empty arguments and blank rows", () => {
    const f = valid({ command: " npx ", args: argRows(["-y", "", "server"]), env: [row("TOKEN", "abc"), newSecretRow()], alwaysAsk: true });
    expect(mcpFormToInput(null, f)).toEqual({
      id: null,
      name: "fs",
      transport: { kind: "stdio", command: "npx", args: ["-y", "server"], env: [{ key: "TOKEN", value: "abc" }] },
      always_ask: true,
    });
  });

  it("builds the input for an http server and keeps saved header values", () => {
    const f = { ...initialMcpForm(httpServer()), url: " https://api.example.com/mcp " };
    expect(mcpFormToInput("m2", f)).toEqual({
      id: "m2",
      name: "github",
      transport: { kind: "http", url: "https://api.example.com/mcp", headers: [{ key: "Authorization", value: null }] },
      always_ask: false,
    });
  });

  it("keeps the other transport's rows out of the input", () => {
    const f = { ...initialMcpForm(stdioServer()), kind: "http" as const, url: "https://x.test/mcp" };
    expect(mcpFormToInput("m1", f).transport).toEqual({ kind: "http", url: "https://x.test/mcp", headers: [] });
  });

  it("previews the command line and the variable names, never the values", () => {
    const f = valid({ command: "npx", args: argRows(["-y", "My Docs"]), env: [row("TOKEN", "super-secret"), savedSecretRows(["OLD"])[0]] });
    const p = commandPreview(f);
    expect(p).toEqual({ line: 'npx -y "My Docs"', envNames: ["TOKEN", "OLD"] });
    expect(JSON.stringify(p)).not.toContain("super-secret");
  });
});

describe("servers, state, and Always allow", () => {
  const tool = (name: string, extra: Partial<McpToolView> = {}): McpToolView => ({
    name: `mcp__fs__${name}`,
    tool: name,
    description: "",
    annotations: { title: null, read_only_hint: null, destructive_hint: null, idempotent_hint: null, open_world_hint: null },
    always_allow: false,
    ...extra,
  });

  it("summarizes the transport on one line", () => {
    expect(transportSummary(stdioServer().transport)).toBe('npx -y @modelcontextprotocol/server-filesystem "C:\\Users\\kc\\My Docs"');
    expect(transportSummary(httpServer().transport)).toBe("https://api.example.com/mcp");
  });

  it("treats a server without a status as stopped", () => {
    expect(serverState(undefined)).toBe("stopped");
    expect(serverState({ server_id: "m1", state: "failed", error: "x", stderr: [], tools: [] })).toBe("failed");
  });

  it("allows a tool on its own or with the whole server", () => {
    expect(toolAllowed(stdioServer(), tool("read"))).toBe(false);
    expect(toolAllowed(stdioServer({ always_allow_tools: ["read"] }), tool("read"))).toBe(true);
    expect(toolAllowed(stdioServer({ always_allow_tools: ["read"] }), tool("write"))).toBe(false);
    expect(toolAllowed(stdioServer({ always_allow: true }), tool("write"))).toBe(true);
  });

  it("changes Always allow of one tool or of the server", () => {
    const s = withAlwaysAllow(stdioServer(), "read", true);
    expect(s.always_allow_tools).toEqual(["read"]);
    expect(withAlwaysAllow(s, "read", true).always_allow_tools).toEqual(["read"]);
    expect(withAlwaysAllow(s, "read", false).always_allow_tools).toEqual([]);
    expect(withAlwaysAllow(s, null, true)).toMatchObject({ always_allow: true, always_allow_tools: ["read"] });
  });

  it("lists the hints that are true", () => {
    const a = (extra: Partial<McpToolAnnotations>): McpToolAnnotations => ({
      title: null,
      read_only_hint: null,
      destructive_hint: null,
      idempotent_hint: null,
      open_world_hint: null,
      ...extra,
    });
    expect(annotationKeys(a({}))).toEqual([]);
    expect(annotationKeys(a({ read_only_hint: true, destructive_hint: false }))).toEqual(["read_only"]);
    expect(annotationKeys(a({ open_world_hint: true, destructive_hint: true, idempotent_hint: true, read_only_hint: true }))).toEqual([
      "read_only",
      "destructive",
      "idempotent",
      "open_world",
    ]);
  });

  it("replaces a server in place or adds it", () => {
    const list = [stdioServer(), httpServer()];
    expect(upsertServer(list, { ...httpServer(), name: "gh" }).map((s) => s.name)).toEqual(["filesystem", "gh"]);
    expect(upsertServer(list, { ...stdioServer(), id: "m3", name: "new" })).toHaveLength(3);
  });

  it("drops trailing empty lines of stderr", () => {
    expect(stderrText(["a", "b", "", ""])).toBe("a\nb");
    expect(stderrText([])).toBe("");
  });
});
