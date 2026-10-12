import { create } from "zustand";
import { formatCommandLine } from "@/features/settings/mcpLogic";
import { api } from "@/ipc/api";
import type { McpServerState, McpServerStatus, McpServerView, McpTransportView } from "@/ipc/types";
import type { PromptCommand, ResourceItem } from "./commands";

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
    .sort((a, b) => a.name.localeCompare(b.name))
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

/** How many servers the conversation uses, and those of them that failed, which the input area names (AI-32). */
export function toolsSummary(rows: ToolsMenuRow[]): { on: number; total: number; failed: ToolsMenuRow[] } {
  const used = rows.filter((r) => r.on);
  return { on: used.length, total: rows.length, failed: used.filter((r) => r.state === "failed") };
}

/**
 * AI-50, AI-51: the prompts the `/` menu and the resources the `@` menu offer, of the running servers enabled on this
 * device that the conversation has not switched off, servers by name.
 */
export function menuSources(
  servers: readonly McpServerView[],
  status: Record<string, McpServerStatus>,
  off: readonly string[],
): { prompts: PromptCommand[]; resources: ResourceItem[] } {
  const prompts: PromptCommand[] = [];
  const resources: ResourceItem[] = [];
  for (const row of toolsMenuRows([...servers], status, off)) {
    const st = status[row.server.id];
    if (!row.on || row.state !== "running" || !st) continue;
    for (const prompt of st.prompts) prompts.push({ serverId: row.server.id, server: row.server.name, prompt });
    for (const resource of st.resources) resources.push({ serverId: row.server.id, server: row.server.name, resource });
  }
  return { prompts, resources };
}

/** The transport in one line: the command line of a `stdio` server, as Settings → AI shows it, or the URL of an `http` one. */
export function transportLabel(t: McpTransportView): string {
  return t.kind === "stdio" ? formatCommandLine(t.command, t.args) : t.url;
}
