import { describe, expect, it } from "vitest";
import type { McpServerStatus, McpServerView } from "@/ipc/types";
import { toolsMenuRows, toolsSummary, transportLabel } from "./mcp";

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
});

describe("tools menu (AI-30)", () => {
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

  it("counts the servers the conversation uses and flags a failed one among them", () => {
    const st = { a: status("a", "failed"), b: status("b", "running", 1) };
    expect(toolsSummary(toolsMenuRows(servers, st, []))).toEqual({ on: 2, total: 2, failed: true });
    expect(toolsSummary(toolsMenuRows(servers, st, ["a"]))).toEqual({ on: 1, total: 2, failed: false });
    expect(toolsSummary([])).toEqual({ on: 0, total: 0, failed: false });
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
