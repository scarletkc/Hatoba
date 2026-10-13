import { describe, expect, it } from "vitest";
import type { McpServerStatus, McpServerView } from "@/ipc/types";
import { failedServers, menuSources, toolsMenuRows, transportLabel } from "./mcp";

const server = (id: string, name: string, enabled = true): McpServerView => ({
  id,
  name,
  transport: { kind: "stdio", command: "npx", args: ["-y", "@modelcontextprotocol/server-github"], env_keys: ["GITHUB_TOKEN"] },
  always_ask: false,
  enabled,
  always_allow: false,
  always_allow_tools: [],
  updated_at: 0,
});

const status = (server_id: string, state: McpServerStatus["state"], tools = 0): McpServerStatus => ({
  server_id,
  state,
  error: state === "failed" ? "exited with status 1" : null,
  stderr: state === "failed" ? ["npm ERR! 404"] : [],
  tools: Array.from({ length: tools }, (_, i) => ({
    name: `mcp__${server_id}__t${i}`,
    tool: `t${i}`,
    description: "",
    annotations: { title: null, read_only_hint: null, destructive_hint: null, idempotent_hint: null, open_world_hint: null },
    always_allow: false,
  })),
  prompts: [],
  resources: [],
});

describe("/mcp (AI-30)", () => {
  const servers = [server("b", "github"), server("a", "filesystem"), server("c", "local", false)];

  it("lists the servers enabled on this device by name, with their state and tools", () => {
    const rows = toolsMenuRows(servers, { b: status("b", "running", 3), a: status("a", "failed") }, []);
    expect(rows.map((r) => r.server.name)).toEqual(["filesystem", "github"]);
    expect(rows[0]).toMatchObject({ state: "failed", error: "exited with status 1", stderr: ["npm ERR! 404"], tools: 0, on: true });
    expect(rows[1]).toMatchObject({ state: "running", tools: 3, on: true });
  });

  it("treats a server without a status as stopped, and marks the ones switched off", () => {
    const rows = toolsMenuRows(servers, {}, ["b"]);
    expect(rows.map((r) => [r.server.id, r.state, r.on])).toEqual([
      ["a", "stopped", true],
      ["b", "stopped", false],
    ]);
  });

  it("names the failed servers among those the conversation uses", () => {
    const st = { a: status("a", "failed"), b: status("b", "running", 1) };
    expect(failedServers(toolsMenuRows(servers, st, [])).map((r) => r.server.name)).toEqual(["filesystem"]);
    expect(failedServers(toolsMenuRows(servers, st, ["a"]))).toEqual([]);
    expect(failedServers([])).toEqual([]);
  });

  it("offers the prompts and resources of the running servers the conversation uses (AI-50, AI-51)", () => {
    const prompt = { name: "review", title: null, description: "", arguments: [] };
    const resource = { uri: "repo://x/README.md", name: "README.md", title: null, description: "", mime_type: null };
    const running = { ...status("b", "running"), prompts: [prompt], resources: [resource] };
    const failed = { ...status("a", "failed"), prompts: [{ ...prompt, name: "x" }] };
    const st = { a: failed, b: running, c: { ...running, server_id: "c" } };
    expect(menuSources(servers, st, [])).toEqual({
      prompts: [{ serverId: "b", server: "github", prompt, command: "mcp__github__review" }],
      resources: [{ serverId: "b", server: "github", resource }],
    });
    // Two running servers of one name offer two commands, numbered by id.
    const twins = [server("y", "docs"), server("x", "docs")];
    const both = { x: { ...running, server_id: "x" }, y: { ...running, server_id: "y" } };
    expect(menuSources(twins, both, []).prompts.map((p) => [p.serverId, p.command])).toEqual([
      ["x", "mcp__docs__review"],
      ["y", "mcp__docs__review_2"],
    ]);
    expect(menuSources(servers, st, ["b"])).toEqual({ prompts: [], resources: [] });
  });

  it("shows the full command line or the URL", () => {
    expect(transportLabel({ kind: "stdio", command: "npx", args: ["-y", "@scope/pkg", "--root", "C:\\My Files"], env_keys: [] })).toBe(
      'npx -y @scope/pkg --root "C:\\My Files"',
    );
    expect(transportLabel({ kind: "stdio", command: "uvx", args: ['say "hi"', ""], env_keys: [] })).toBe('uvx "say \\"hi\\"" ""');
    expect(transportLabel({ kind: "stdio", command: "node", args: ["server.js\r--evil"], env_keys: [] })).toBe('node "server.js\\r--evil"');
    expect(transportLabel({ kind: "http", url: "https://mcp.example.com/mcp", header_keys: ["Authorization"] })).toBe("https://mcp.example.com/mcp");
  });
});
