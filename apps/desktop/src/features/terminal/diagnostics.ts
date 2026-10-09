import { hostById } from "@/app/data";
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
  const lines = [
    "Hatoba connection diagnostics",
    `Host: ${saved?.name ?? fallbackName}${host ? ` (${host.address}:${host.port})` : ""}`,
    host && `User: ${host.username}`,
    host && `Auth: ${saved ? saved.auth_kind : "quick connect (agent, then password or keyboard-interactive)"}`,
    jump && `Jump host: ${jump}`,
    `Error: ${errorCode(err)} (${err.code}${err.ssh_kind ? `/${err.ssh_kind}` : ""})`,
    `Detail: ${err.detail}`,
    `Attempts: ${attempts}`,
    `Time: ${new Date(at).toISOString()}`,
  ];
  return lines.filter(Boolean).join("\n");
}
