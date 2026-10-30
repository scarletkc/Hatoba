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

type Tone = "normal" | "warn" | "error" | "dim";

/** What the host's status says in words, and whether it is a problem. */
export function statusText(t: T, readout: HostReadout | undefined): { text: string; tone: Tone } {
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
        text: r?.system
          ? t("dashboard.state.unsupported", { system: r.system })
          : t("dashboard.state.unsupportedUnknown"),
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

/**
 * The status as a card shows it in place of readings: an icon (none while connecting, which spins), a short
 * title, and the reason apart from it.
 */
export function statusParts(
  t: T,
  readout: HostReadout | undefined,
): { icon: string | null; title: string; detail: string | null; tone: Tone } {
  const r = readout;
  const reason = r?.error ? errorMessage(t, r.error) : null;
  switch (r?.phase ?? "waiting") {
    case "connecting":
      return { icon: null, title: t("dashboard.state.connecting"), detail: null, tone: "dim" };
    case "offline":
      return { icon: "cloud-slash", title: t("dashboard.state.offline"), detail: null, tone: "dim" };
    case "unsupported":
      return { icon: "info", title: statusText(t, r).text, detail: null, tone: "dim" };
    case "held": {
      const hold = r!.hold ?? "sign_in_failed";
      return {
        icon: hold.startsWith("host_key") ? "fingerprint" : "lock-key",
        title: t(`dashboard.hold.${hold}` as MessageKey),
        detail: reason,
        tone: "warn",
      };
    }
    case "failed":
      return { icon: "warning-circle", title: t("dashboard.state.failed"), detail: reason, tone: "error" };
    default:
      // Waiting, or connected before the first reading.
      return { icon: "hourglass-medium", title: statusText(t, r).text, detail: null, tone: "dim" };
  }
}
