import type { MessageKey, Params } from "@/i18n";

type T = (key: MessageKey, params?: Params) => string;

/** "90 seconds", "2 minutes", "1 hour", "4 hours", or Never for 0. */
export function durationLabel(t: T, seconds: number): string {
  if (seconds === 0) return t("settings.sec.never");
  if (seconds === 3600) return t("settings.sec.hour");
  if (seconds % 3600 === 0) return t("settings.sec.hours", { n: seconds / 3600 });
  if (seconds % 60 === 0) return t("settings.sec.minutes", { n: seconds / 60 });
  return t("settings.sec.seconds", { n: seconds });
}
