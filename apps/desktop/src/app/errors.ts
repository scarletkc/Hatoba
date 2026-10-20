import type { MessageKey, Params } from "@/i18n";
import { toAppError } from "@/ipc/api";
import type { AppError } from "@/ipc/types";

type T = (key: MessageKey, params?: Params) => string;
/** What a message may name: the SSH target, and the biometric unlock of this platform. */
type Context = { host?: string; port?: number; timeoutSec?: number; biometric?: string };

/** Localised, user-facing text for any command failure (SSH-05 readable errors). */
export function errorMessage(t: T, e: unknown, ctx: Context = {}): string {
  const err: AppError = toAppError(e);
  switch (err.code) {
    case "throttled": {
      const s = err.retry_at ? Math.max(1, Math.ceil((err.retry_at - Date.now()) / 1000)) : 1;
      return t("err.throttled", { s });
    }
    case "wrong_pin":
      return err.attempts_left ? t("err.wrong_pin", { n: err.attempts_left }) : t("err.pin_locked");
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
    case "biometric_stale":
      // Only the lock screen's biometric unlock returns this, and it passes the name.
      return t("err.biometric_stale", { name: ctx.biometric ?? "Touch ID" });
    case "ai":
      // `detail` is the provider's own message (spec §13.2), which is not translated.
      return err.http_status
        ? t("err.ai_status", { status: err.http_status, detail: err.detail })
        : t("err.ai", { detail: err.detail });
    default:
      return t(`err.${err.code}` as MessageKey);
  }
}

export function sshErrorMessage(t: T, err: AppError, ctx: { host?: string; port?: number; timeoutSec?: number } = {}): string {
  const kind = err.ssh_kind ?? "other";
  return t(`ssh.err.${kind}` as MessageKey, { host: ctx.host ?? "", port: ctx.port ?? 22, s: ctx.timeoutSec ?? 15 });
}
