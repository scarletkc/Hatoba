import { hostById, useVaultData } from "@/app/data";
import { useApp } from "@/app/store";
import type { AppError, HostView, QuickTarget, SshErrorKind } from "@/ipc/types";

/** Short, familiar codes for the error card's meta line ("ETIMEDOUT · 09:44:07"). */
const SSH_CODES: Record<SshErrorKind, string> = {
  dns: "ENOTFOUND",
  refused: "ECONNREFUSED",
  timeout: "ETIMEDOUT",
  unreachable: "ENETUNREACH",
  auth_failed: "EAUTH",
  host_key_rejected: "EHOSTKEY",
  key_parse: "EKEY",
  disconnected: "ECONNRESET",
  protocol: "EPROTO",
  io: "EIO",
  channel: "ECHANNEL",
  sftp: "ESFTP",
  cancelled: "ECANCELED",
  proxy_unreachable: "EPROXY",
  proxy_auth: "EPROXYAUTH",
  proxy: "EPROXY",
  proxy_missing: "EPROXY",
  other: "EFAILED",
};

export function errorCode(err: AppError): string {
  return err.code === "ssh" ? SSH_CODES[err.ssh_kind ?? "other"] : err.code.toUpperCase();
}

export function clockTime(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

/**
 * Text for "Copy Diagnostics". Technical English on purpose (it is pasted into bug reports and
 * chats) and free of secrets: no passwords, passphrases or key material.
 */
export function diagnosticsText(
  host: HostView | QuickTarget | null | undefined,
  fallbackName: string,
  err: AppError,
  at: number,
  attempts: number,
): string {
  // A quick connection (HOST-12) has a target but no saved host.
  const saved = host && "id" in host ? host : undefined;
  const jump = hostById(saved?.jump_host_id)?.name;
  const proxy = proxyLine(saved);
  const lines = [
    "Hatoba connection diagnostics",
    `Host: ${saved?.name ?? fallbackName}${host ? ` (${host.address}:${host.port})` : ""}`,
    host && `User: ${host.username}`,
    host && `Auth: ${saved ? saved.auth_kind : "quick connect (agent, then password or keyboard-interactive)"}`,
    jump && `Jump host: ${jump}`,
    proxy && `Proxy: ${proxy}`,
    `Error: ${errorCode(err)} (${err.code}${err.ssh_kind ? `/${err.ssh_kind}` : ""})`,
    `Detail: ${err.detail}`,
    `Attempts: ${attempts}`,
    `Time: ${new Date(at).toISOString()}`,
  ];
  return lines.filter(Boolean).join("\n");
}

/**
 * The proxy a connection goes through (SSH-13), for the diagnostics: the choice of its first hop,
 * which is the outermost jump host or the host itself. No credentials.
 */
function proxyLine(host: HostView | undefined): string | null {
  const seen = new Set<string>();
  let first = host;
  while (first?.jump_host_id && !seen.has(first.jump_host_id) && seen.size < 8) {
    seen.add(first.jump_host_id);
    const next = hostById(first.jump_host_id);
    if (!next) break;
    first = next;
  }
  const mode = first?.proxy_mode ?? "device_default";
  if (mode === "direct") return null;
  const id = mode === "proxy" ? first?.proxy_id : useApp.getState().prefs.default_proxy_id;
  if (!id) return null;
  const p = useVaultData.getState().proxies.find((x) => x.id === id);
  const via = mode === "proxy" ? "" : " (device default)";
  return p ? `${p.kind} ${p.address}:${p.port}${via}` : `deleted${via}`;
}
