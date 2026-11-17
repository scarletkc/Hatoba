export const CALL_MCP_TOOL = "call_mcp_tool";

/** `mcp__server__tool` → `["server", "tool"]`. */
export function splitMcpName(name: string): [string, string] | null {
  const m = /^mcp__(.+?)__(.+)$/.exec(name);
  return m ? [m[1], m[2]] : null;
}

/** The call's arguments as an object. An empty string counts as `{}` (some providers send it for no arguments). */
export function parseArgs(json: string): Record<string, unknown> | null {
  if (!json.trim()) return {};
  try {
    const v: unknown = JSON.parse(json);
    return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

/**
 * AI-45: the MCP tool a `call_mcp_tool` call names, and that tool's arguments as JSON: an object, or a
 * string that holds one, `{}` when left out, as Rust reads them. Null for any other call, or one whose
 * arguments name no MCP tool: Rust runs no other tool through it (a call that names `run_command`
 * returns an error), so such a call is shown as it is.
 */
export function proxyCall(name: string, json: string): { tool: string; arguments: string } | null {
  if (name !== CALL_MCP_TOOL) return null;
  const args = parseArgs(json);
  const tool = typeof args?.tool === "string" ? args.tool.trim() : "";
  if (!args || !splitMcpName(tool)) return null;
  const inner = args.arguments;
  if (inner === undefined || inner === null || (typeof inner === "string" && !inner.trim()))
    return { tool, arguments: "{}" };
  if (typeof inner === "string") {
    const parsed = parseArgs(inner);
    return { tool, arguments: parsed ? JSON.stringify(parsed) : inner };
  }
  return { tool, arguments: JSON.stringify(inner) };
}

/** A call as the panel and the export show it: a `call_mcp_tool` call as the MCP tool it names, with that tool's arguments (AI-45). */
export function shownCall<T extends { name: string; arguments: string }>(call: T): T {
  const proxy = proxyCall(call.name, call.arguments);
  return proxy ? { ...call, name: proxy.tool, arguments: proxy.arguments } : call;
}

/** The arguments a `call_mcp_tool` call runs with after the user edited the tool's own on its card (AI-17, AI-45). */
export function proxyArguments(json: string, edited: string): string {
  const args = parseArgs(json) ?? {};
  return JSON.stringify({ ...args, arguments: JSON.parse(edited) as unknown });
}
