import { Icon } from "@/components/controls";
import { formatBytes, useT } from "@/i18n";
import { api } from "@/ipc/api";
import { cx } from "@/lib/cx";
import { formatProgress, useTransfers, type TransferItem } from "./transfers";
import s from "./TransferList.module.css";

/** The "Transfers" section at the bottom of the SFTP panel (SFTP-02). */
export function TransferList({ sessionId }: { sessionId: string }) {
  const t = useT();
  const all = useTransfers((st) => st.items);
  const items = all.filter((i) => i.session_id === sessionId);
  if (items.length === 0) return null;
  return (
    <div className={s.section}>
      <div className={s.header}>{t("sftp.transfers")}</div>
      {items.map((item) => (
        <TransferRow key={item.transfer_id} item={item} />
      ))}
    </div>
  );
}

function TransferRow({ item }: { item: TransferItem }) {
  const t = useT();
  const running = item.state === "running";
  const failed = item.state === "failed";
  const pct = item.state === "done" ? 100 : item.total > 0 ? Math.min(100, Math.floor((item.bytes / item.total) * 100)) : 0;

  const eta = () => {
    const sec = Math.ceil((item.total - item.bytes) / item.bytes_per_sec);
    if (sec < 60) return t("sftp.eta.seconds", { n: Math.max(1, sec) });
    if (sec < 3600) return t("sftp.eta.minutes", { n: Math.ceil(sec / 60) });
    return t("sftp.eta.hours", { n: Math.ceil(sec / 3600) });
  };

  let line: string;
  if (failed) line = item.error ?? "";
  else if (item.state === "cancelled") line = t("sftp.transferCancelled");
  else if (item.state === "done") line = `${t("sftp.transferDone")} · ${formatBytes(item.total)}`;
  else {
    const parts = [formatProgress(item.bytes, item.total)];
    if (item.bytes_per_sec > 0) parts.push(`${formatBytes(item.bytes_per_sec)}/s`, eta());
    line = parts.join(" · ");
  }

  const close = () =>
    running ? void api.transfer_cancel(item.transfer_id).catch(() => {}) : useTransfers.getState().dismiss(item.transfer_id);

  return (
    <div className={cx(s.transfer, item.fading && s.fading)}>
      <div className={s.top}>
        <Icon name={item.direction === "upload" ? "upload-simple" : "download-simple"} size={14} color={failed ? "var(--red)" : "var(--accent)"} />
        <span className={s.name} title={item.name}>
          {item.name}
        </span>
        <span className={s.pct}>{pct}%</span>
        <button
          type="button"
          className={s.close}
          aria-label={running ? t("sftp.cancelTransfer") : t("sftp.dismissTransfer")}
          title={running ? t("sftp.cancelTransfer") : t("sftp.dismissTransfer")}
          onClick={close}
        >
          <Icon name="x" size={12} />
        </button>
      </div>
      <div className={s.track} role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
        <div className={cx(s.fill, failed && s.fillFailed)} style={{ width: `${pct}%` }} />
      </div>
      <div className={cx(s.line, failed && s.lineFailed)} title={failed ? line : undefined}>
        {line}
      </div>
    </div>
  );
}
