import type { MessageKey, Params } from "@/i18n";
import type { ForwardView } from "@/ipc/types";

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
      return `${bind} → ${hostPort(f.dest_host, f.dest_port)}`;
    case "remote":
      return `${t("terminal.fwd.onServer", { addr: bind })} → ${hostPort(f.dest_host, f.dest_port)}`;
    case "dynamic":
      return `${bind} (${t("terminal.fwd.socks")})`;
  }
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
