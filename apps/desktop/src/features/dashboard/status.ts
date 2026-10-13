import { errorMessage } from "@/app/errors";
import type { MessageKey, useT } from "@/i18n";
import type { HostReadout } from "./readings";

type T = ReturnType<typeof useT>;

/** The badge on the OS icon: green while it reads or answers, orange when it needs the user, red when it failed. */
export function statusColor(readout: HostReadout | undefined, leftOut: boolean): string {
  if (leftOut || !readout) return "var(--fg3)";
  switch (readout.phase) {
    case "reading":
      return "var(--green)";
    case "held":
      return "var(--orange)";
    case "failed":
      return "var(--red)";
    case "offline":
      return "var(--fg3)";
    default:
      return readout.online ? "var(--green)" : "var(--fg3)";
  }
}

/** What the host's status says in words, and whether it is a problem. */
export function statusText(t: T, readout: HostReadout | undefined): { text: string; tone: "normal" | "warn" | "error" | "dim" } {
  const r = readout;
  switch (r?.phase ?? "waiting") {
    case "waiting":
      return { text: t("dashboard.state.waiting"), tone: "dim" };
    case "connecting":
      return { text: t("dashboard.state.connecting"), tone: "dim" };
    case "reading":
      return { text: r?.tab ? t("dashboard.viaTab") : t("dashboard.state.reading"), tone: "normal" };
    case "offline":
      return { text: t("dashboard.state.offline"), tone: "dim" };
    case "unsupported":
      return {
        text: r?.system ? t("dashboard.state.unsupported", { system: r.system }) : t("dashboard.state.unsupportedUnknown"),
        tone: "dim",
      };
    case "held": {
      const text = t(`dashboard.hold.${r!.hold ?? "sign_in_failed"}` as MessageKey);
      // The server's answer to a sign-in that failed, when there was one.
      return { text: r?.error ? `${text}: ${errorMessage(t, r.error)}` : text, tone: "warn" };
    }
    case "failed":
      return { text: t("dashboard.failed", { error: r?.error ? errorMessage(t, r.error) : "" }), tone: "error" };
  }
}
