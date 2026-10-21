/** `host:port`, with IPv6 literals bracketed. */
export function hostPort(host: string, port: number | string): string {
  return `${host.includes(":") ? `[${host}]` : host}:${port}`;
}

/**
 * Where this computer reaches a forward listening on `bindAddress`: `localhost` for a loopback or wildcard address,
 * else that address, which is one interface's.
 */
export function reachableAddress(bindAddress: string, port: number): string {
  const a = bindAddress.trim().toLowerCase();
  const local = a === "" || a === "localhost" || a === "0.0.0.0" || a === "::" || a === "::1" || a.startsWith("127.");
  return local ? `localhost:${port}` : hostPort(bindAddress.trim(), port);
}
