import type { MessageKey, Params } from "@/i18n";
import { toAppError } from "@/ipc/api";
import type { AppError } from "@/ipc/types";

type T = (key: MessageKey, params?: Params) => string;

/** Localised, user-facing text for any command failure (SSH-05 readable errors). */
export function errorMessage(t: T, e: unknown, ctx: { host?: string; port?: number; timeoutSec?: number } = {}): string {
  const err: AppError = toAppError(e);
  switch (err.code) {
    case "throttled": {
      const s = err.retry_at ? Math.max(1, Math.ceil((err.retry_at - Date.now()) / 1000)) : 1;
      return t("err.throttled", { s });
    }
    case "ssh":
      return sshErrorMessage(t, err, ctx);
    case "key_parse":
      return err.key_kind ? t(`key.err.${err.key_kind}` as MessageKey) : t("err.key_parse");
    case "cloudflare_permission":
      // The dashboard's names for the permissions, which are not translated.
      return t("err.cloudflare_permission", {
        permission: err.permission === "d1" ? "Account · D1 · Edit" : "Account · Workers Scripts · Edit",
      });
    case "cloudflare":
      return err.cf_code ? t("err.cloudflare_code", { code: err.cf_code }) : t("err.cloudflare");
    default:
      return t(`err.${err.code}` as MessageKey);
  }
}

export function sshErrorMessage(t: T, err: AppError, ctx: { host?: string; port?: number; timeoutSec?: number } = {}): string {
  const kind = err.ssh_kind ?? "other";
  return t(`ssh.err.${kind}` as MessageKey, { host: ctx.host ?? "", port: ctx.port ?? 22, s: ctx.timeoutSec ?? 15 });
}
