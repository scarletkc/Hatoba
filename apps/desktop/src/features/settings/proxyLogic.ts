import type { ProxyKind } from "@/ipc/types";

/** The port a new proxy of each kind starts with. */
export const DEFAULT_PORT: Record<ProxyKind, number> = { socks5: 1080, http: 8080 };

export const KIND_LABEL: Record<ProxyKind, string> = { socks5: "SOCKS5", http: "HTTP" };

/** The longest SOCKS5 username or password in bytes (RFC 1929); HTTP proxies get the same limit. */
export const MAX_CREDENTIAL_BYTES = 255;

/** `host:port`, with an IPv6 address in brackets. */
export function authority(address: string, port: number): string {
  return address.includes(":") ? `[${address}]:${port}` : `${address}:${port}`;
}

/** What a proxy row and a picker show under the name: the type and where it is. */
export function proxySummary(p: { kind: ProxyKind; address: string; port: number; username: string }): string {
  return `${KIND_LABEL[p.kind]} · ${p.username ? `${p.username}@` : ""}${authority(p.address, p.port)}`;
}

export interface ParsedProxyUrl {
  kind: ProxyKind;
  address: string;
  port: number | null;
  username: string;
  password: string;
}

/**
 * Reads a proxy URL pasted into the address field, such as `socks5://127.0.0.1:7890` or
 * `http://user:pass@proxy.example.com:3128`. `null` for anything else, including schemes Hatoba
 * cannot use (`https://` proxies, SOCKS4).
 */
export function parseProxyUrl(text: string): ParsedProxyUrl | null {
  const m = /^(socks5h?|socks|http):\/\/(?:([^:@/]*)(?::([^@/]*))?@)?(\[[^\]]+\]|[^:/@[\]]+)(?::(\d{1,5}))?\/?$/i.exec(text.trim());
  if (!m) return null;
  const decode = (s: string | undefined) => {
    try {
      return decodeURIComponent(s ?? "");
    } catch {
      return s ?? "";
    }
  };
  const port = m[5] ? Number(m[5]) : null;
  if (port !== null && (port < 1 || port > 65535)) return null;
  return {
    kind: m[1].toLowerCase() === "http" ? "http" : "socks5",
    address: m[4].replace(/^\[(.*)\]$/, "$1"),
    port,
    username: decode(m[2]),
    password: decode(m[3]),
  };
}

export type ProxyField = "name" | "address" | "port" | "username" | "password";

export interface ProxyFormValues {
  name: string;
  kind: ProxyKind;
  address: string;
  port: string;
  username: string;
  /** The password being typed; empty when the saved one is kept. */
  password: string;
}

/** Which field is wrong and why, as message keys (the backend checks the same rules). */
export function validateProxy(
  f: ProxyFormValues,
): Partial<Record<ProxyField, "nameRequired" | "addressRequired" | "addressInvalid" | "portInvalid" | "usernameColon" | "tooLong">> {
  const e: ReturnType<typeof validateProxy> = {};
  if (!f.name.trim()) e.name = "nameRequired";
  const address = f.address.trim().replace(/^\[(.*)\]$/, "$1");
  if (!address) e.address = "addressRequired";
  else if (/[\s@/\\]/.test(address)) e.address = "addressInvalid";
  const port = Number(f.port);
  if (!/^\d+$/.test(f.port.trim()) || port < 1 || port > 65535) e.port = "portInvalid";
  const username = f.username.trim();
  const bytes = (s: string) => new TextEncoder().encode(s).length;
  if (f.kind === "http" && username.includes(":")) e.username = "usernameColon";
  else if (bytes(username) > MAX_CREDENTIAL_BYTES) e.username = "tooLong";
  if (bytes(f.password) > MAX_CREDENTIAL_BYTES) e.password = "tooLong";
  return e;
}
