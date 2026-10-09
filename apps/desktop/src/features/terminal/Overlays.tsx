import { errorMessage } from "@/app/errors";
import { Button, Icon, Spinner } from "@/components/controls";
import { toast } from "@/components/overlay";
import { useT } from "@/i18n";
import type { AppError, HostView, QuickTarget } from "@/ipc/types";
import { writeClipboard } from "./clipboard";
import { clockTime, diagnosticsText, errorCode } from "./diagnostics";
import s from "./Overlays.module.css";

/** Shown over the terminal while the SSH handshake and authentication run. */
export function ConnectingOverlay({ name }: { name: string }) {
  const t = useT();
  return (
    <div className={s.connecting} role="status">
      <div className={s.connectingPill}>
        <Spinner size={14} />
        {t("terminal.connectingTo", { host: name })}
      </div>
    </div>
  );
}

/** Connection failed (SSH-05): what happened, in words, plus retry (design §03b). */
export function ErrorCard({
  name,
  host,
  target,
  error,
  at,
  attempts,
  onRetry,
  onEdit,
  onAskAi,
}: {
  name: string;
  host: HostView | undefined;
  /** What a quick connection connects to (HOST-12). */
  target: QuickTarget | null;
  error: AppError | null;
  at: number | null;
  attempts: number;
  onRetry(): void;
  onEdit(): void;
  /** Hands the diagnostics to the AI panel for this tab (AI-10). */
  onAskAi(diagnostics: string): void;
}) {
  const t = useT();
  const err: AppError = error ?? { code: "internal", detail: "" };
  const when = clockTime(at ?? Date.now());
  const meta = attempts > 1
    ? t("terminal.err.metaRetries", { code: errorCode(err), n: attempts - 1, time: when })
    : t("terminal.err.meta", { code: errorCode(err), time: when });

  const copyDiagnostics = async () => {
    try {
      await writeClipboard(diagnosticsText(host ?? target, name, err, at ?? Date.now(), attempts));
      toast(t("terminal.err.diagCopied"), "success");
    } catch {
      toast(t("terminal.clipboardDenied"), "error");
    }
  };

  return (
    <div className={s.scrim}>
      <div role="alertdialog" aria-labelledby="term-err-title" className={s.card}>
        <div className={s.cardHead}>
          <Icon name="plugs" size={20} color="#ff6b61" />
          <div id="term-err-title" className={s.cardTitle}>
            {t("terminal.err.title", { name })}
          </div>
        </div>
        <div className={s.cardBody}>
          {errorMessage(t, err, { host: (host ?? target)?.address, port: (host ?? target)?.port, timeoutSec: 15 })}
        </div>
        <div className={s.meta}>{meta}</div>
        <div className={s.cardActions}>
          <button type="button" className={s.linkButton} onClick={() => void copyDiagnostics()}>
            <Icon name="copy" size={13} />
            {t("terminal.err.copyDiag")}
          </button>
          <div className={s.grow} />
          <button type="button" className={s.darkButton} onClick={() => onAskAi(diagnosticsText(host ?? target, name, err, at ?? Date.now(), attempts))}>
            <Icon name="sparkle" size={13} />
            {t("terminal.err.askAi")}
          </button>
          {host && (
            <button type="button" className={s.darkButton} onClick={onEdit}>
              {t("terminal.err.editHost")}
            </button>
          )}
          <button type="button" className={s.primaryButton} onClick={onRetry}>
            <Icon name="arrow-clockwise" size={13} />
            {t("btn.retry")}
          </button>
        </div>
      </div>
    </div>
  );
}

/** SSH-07: the session ended; reconnecting is always an explicit action. */
export function DisconnectedBanner({ reason, onReconnect }: { reason: string | null; onReconnect(): void }) {
  const t = useT();
  return (
    <div className={s.banner} role="status">
      <Icon name="plugs" size={16} color="var(--orange)" />
      <span className={s.bannerText}>
        {reason ? t("terminal.disconnectedReason", { reason }) : t("terminal.disconnected")}
      </span>
      <Button variant="primary" size="sm" icon="arrow-clockwise" onClick={onReconnect}>
        {t("terminal.reconnect")}
      </Button>
    </div>
  );
}
