import type { MessageKey, Params } from "@/i18n";
import type { ForwardView, RunningForwardView } from "@/ipc/types";

/** `host:port`, with IPv6 literals bracketed. */
export function hostPort(host: string, port: number | string): string {
  return `${host.includes(":") ? `[${host}]` : host}:${port}`;
}

/**
 * A forward as one line: where it listens (on the server for a remote one) and where it leads, or that it is a SOCKS
 * proxy. `port` is the port it listens on while it runs.
 */
export function forwardRule(
  t: (key: MessageKey, params?: Params) => string,
  f: ForwardView,
  port: number | null = null,
): string {
  const bind = hostPort(f.bind_address, port ?? (f.bind_port || t("hosts.fwd.autoPort")));
  switch (f.kind) {
    case "local":
      return `${bind} → ${forwardDest(t, f)}`;
    case "remote":
      return `${t("terminal.fwd.onServer", { addr: bind })} → ${hostPort(f.dest_host, f.dest_port)}`;
    case "dynamic":
      return `${bind} (${t("terminal.fwd.socks")})`;
  }
}

/** What a forward is called where one line names it: its name, else its rule. */
export function forwardLabel(t: (key: MessageKey, params?: Params) => string, f: ForwardView): string {
  return f.name || forwardRule(t, f);
}

/**
 * Where a forward leads. A local one's loopback destination is the server's own, named so beside the address it listens
 * on here, which is often the same `127.0.0.1:port`. A running forward is named as it started, before any edit to its
 * rule.
 */
export function forwardDest(
  t: (key: MessageKey, params?: Params) => string,
  f: ForwardView,
  running: RunningForwardView | null = null,
): string {
  const kind = running?.kind ?? f.kind;
  const dest = running?.dest ?? hostPort(f.dest_host, f.dest_port);
  const host = running ? running.dest.replace(/:\d+$/, "").replace(/^\[(.*)\]$/, "$1") : f.dest_host;
  return kind === "local" && isLoopback(host) ? t("terminal.fwd.onServer", { addr: dest }) : dest;
}

function isLoopback(host: string): boolean {
  const h = host.trim().toLowerCase();
  return h === "localhost" || h === "::1" || /^127\./.test(h);
}

/**
 * Where this computer reaches a forward listening on `bindAddress`: `localhost` for the address it names or a wildcard
 * one, else that address, as one on `127.0.1.1` takes nothing on `127.0.0.1`.
 */
export function reachableAddress(bindAddress: string, port: number): string {
  const a = bindAddress.trim().toLowerCase();
  const local = a === "" || a === "localhost" || a === "0.0.0.0" || a === "::" || a === "::1" || a === "127.0.0.1";
  return local ? `localhost:${port}` : hostPort(bindAddress.trim(), port);
}
