import { create } from "zustand";
import { formatCommandLine } from "@/features/settings/mcpLogic";
import { api } from "@/ipc/api";
import type { McpServerState, McpServerStatus, McpServerView, McpTransportView } from "@/ipc/types";
import { commandNames, type PromptCommand, type ResourceItem } from "./commands";

/** The MCP servers that `/mcp` shows (AI-30), with their live state (AI-32), prompts and resources (AI-50, AI-51). */
interface McpState {
  /** null until the list has been read. */
  servers: McpServerView[] | null;
  /** By server id, from `mcp_server_status` and the `ai://mcp-status` event. */
  status: Record<string, McpServerStatus>;
  failed: boolean;
}

export const useMcp = create<McpState>(() => ({ servers: null, status: {}, failed: false }));

let loading: Promise<void> | null = null;

/** Reads the servers and the state of each one enabled on this device. */
export function loadMcp(): Promise<void> {
  loading ??= (async () => {
    try {
      const servers = await api.mcp_servers_list();
      useMcp.setState({ servers, failed: false });
      const statuses = await Promise.all(servers.filter((s) => s.enabled).map((s) => api.mcp_server_status(s.id).catch(() => null)));
      useMcp.setState((st) => {
        const status = { ...st.status };
        for (const x of statuses) if (x) status[x.server_id] = x;
        return { status };
      });
    } catch {
      useMcp.setState((st) => ({ servers: st.servers ?? [], failed: true }));
    } finally {
      loading = null;
    }
  })();
  return loading;
}

/** `ai://mcp-status`: a server started, stopped, failed or relisted its tools. */
export function applyMcpStatus(status: McpServerStatus) {
  const known = useMcp.getState().servers;
  useMcp.setState((st) => ({ status: { ...st.status, [status.server_id]: status } }));
  // A server added since the list was read (in Settings or by a sync).
  if (known && !known.some((s) => s.id === status.server_id)) void loadMcp();
}

/** Locking stops every server and drops what the vault holds. */
export function resetMcp() {
  useMcp.setState({ servers: null, status: {}, failed: false });
}

/** One server in `/mcp`. */
export interface ToolsMenuRow {
  server: McpServerView;
  state: McpServerState;
  error: string | null;
  stderr: string[];
  tools: number;
  /** Not switched off for this conversation. */
  on: boolean;
}

/** AI-30: the servers enabled on this device, by name, with their state and whether the conversation uses them. */
export function toolsMenuRows(servers: McpServerView[], status: Record<string, McpServerStatus>, off: readonly string[]): ToolsMenuRow[] {
  return servers
    .filter((s) => s.enabled)
    .sort((a, b) => a.name.localeCompare(b.name) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
    .map((server) => {
      const st = status[server.id];
      return {
        server,
        state: st?.state ?? "stopped",
        error: st?.error ?? null,
        stderr: st?.stderr ?? [],
        tools: st?.tools.length ?? 0,
        on: !off.includes(server.id),
      };
    });
}

/** The servers the conversation uses that failed, which the input area names (AI-32). */
export function failedServers(rows: ToolsMenuRow[]): ToolsMenuRow[] {
  return rows.filter((r) => r.on && r.state === "failed");
}

/**
 * AI-50, AI-51: the prompts the `/` menu and the resources the `@` menu offer, of the running servers enabled on this
 * device that the conversation has not switched off, servers by name, then id. The prompts' commands are named across
 * those servers at once, so two servers of one name never offer the same command.
 */
export function menuSources(
  servers: readonly McpServerView[],
  status: Record<string, McpServerStatus>,
  off: readonly string[],
): { prompts: PromptCommand[]; resources: ResourceItem[] } {
  const running = toolsMenuRows([...servers], status, off).filter((row) => row.on && row.state === "running" && status[row.server.id]);
  const names = commandNames(running.map((row) => [row.server.name, status[row.server.id].prompts.map((p) => p.name)] as const));
  const prompts: PromptCommand[] = [];
  const resources: ResourceItem[] = [];
  running.forEach((row, i) => {
    const st = status[row.server.id];
    st.prompts.forEach((prompt, j) => prompts.push({ serverId: row.server.id, server: row.server.name, prompt, command: names[i][j] }));
    for (const resource of st.resources) resources.push({ serverId: row.server.id, server: row.server.name, resource });
  });
  return { prompts, resources };
}

/** The transport in one line: the command line of a `stdio` server, as Settings → AI shows it, or the URL of an `http` one. */
export function transportLabel(t: McpTransportView): string {
  return t.kind === "stdio" ? formatCommandLine(t.command, t.args) : t.url;
}
