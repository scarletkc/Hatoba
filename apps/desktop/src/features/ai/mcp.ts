import { create } from "zustand";
import { api } from "@/ipc/api";
import type { McpServerState, McpServerStatus, McpServerView, McpTransportView } from "@/ipc/types";

/** The MCP servers the panel's tools menu shows (AI-30), with their live state (AI-32). */
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

/** One server in the tools menu. */
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

/** The tools menu button: how many servers the conversation uses, and whether one of them failed. */
export function toolsSummary(rows: ToolsMenuRow[]): { on: number; total: number; failed: boolean } {
  const used = rows.filter((r) => r.on);
  return { on: used.length, total: rows.length, failed: used.some((r) => r.state === "failed") };
}

/** An argument as one word: in double quotes when it is empty or has spaces or quotes. */
function shellWord(arg: string): string {
  if (arg && !/[\s"']/.test(arg)) return arg;
  return `"${arg.replace(/"/g, '\\"')}"`;
}

/** The transport in one line: the command line of a `stdio` server or the URL of an `http` one. */
export function transportLabel(t: McpTransportView): string {
  return t.kind === "stdio" ? [t.command, ...t.args].map(shellWord).join(" ") : t.url;
}
