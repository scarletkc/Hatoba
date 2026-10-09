import { controlStyles, Icon, IconButton, StatusDot } from "@/components/controls";
import { useT } from "@/i18n";
import type { TabStatus } from "@/app/tabs";
import { cx } from "@/lib/cx";
import { formatPercent, formatRate, percentOf, type SessionStats } from "./stats";
import s from "./StatusBar.module.css";

const DOT: Record<TabStatus, string> = {
  connecting: "var(--orange)",
  connected: "var(--green)",
  failed: "var(--red)",
  disconnected: "var(--fg3)",
};

interface Props {
  status: TabStatus;
  /** `user@host:port`, when the host still exists. */
  target: string | null;
  /** Name of the jump host (SSH-10). */
  via: string | null;
  latencyMs: number | null;
  findOpen: boolean;
  sftpOpen: boolean;
  /** The tab has saved port forwards to offer (FWD-01): not for a quick connection. */
  forwards: boolean;
  /** Forwards running on this session (FWD-01); shown as a count on the button. */
  forwardCount: number;
  forwardsOpen: boolean;
  /** The menu of the ⋯ button is open. */
  moreOpen: boolean;
  /** The server's resource usage shows in this tab (TERM-12). */
  statsOn: boolean;
  stats: SessionStats | undefined;
  statsOpen: boolean;
  findHint: string;
  onFind(): void;
  onShowStats(): void;
  onStats(button: HTMLElement): void;
  onForwards(button: HTMLElement): void;
  onToggleSftp(): void;
  onMore(button: HTMLElement): void;
}

/** The 36px bar above a terminal (design §03 / §03b). */
export function StatusBar({
  status,
  target,
  via,
  latencyMs,
  findOpen,
  sftpOpen,
  forwards,
  forwardCount,
  forwardsOpen,
  moreOpen,
  statsOn,
  stats,
  statsOpen,
  findHint,
  onFind,
  onShowStats,
  onStats,
  onForwards,
  onToggleSftp,
  onMore,
}: Props) {
  const t = useT();
  const connected = status === "connected";
  return (
    <div className={s.bar}>
      <span className={s.state} role="status">
        <StatusDot color={DOT[status]} />
        {t(`terminal.status.${status}`)}
      </span>
      <span className={s.sep} />
      {target && <span className={s.target}>{target}</span>}
      {via && (
        <span className={s.via}>
          <Icon name="path" size={12} />
          {t("terminal.via")} {via}
        </span>
      )}
      {connected && latencyMs != null && <span className={s.latency}>{latencyMs} ms</span>}
      <div className={s.spacer} />
      {statsOn && connected ? (
        <StatsReadout stats={stats} open={statsOpen} onClick={onStats} />
      ) : (
        <IconButton icon="gauge" label={t("terminal.stats.show")} disabled={!connected} onClick={onShowStats} />
      )}
      <IconButton icon="magnifying-glass" label={`${t("terminal.find")} (${findHint})`} active={findOpen} onClick={onFind} />
      {forwards && (
        <button
          type="button"
          className={cx(controlStyles.iconButton, forwardsOpen && controlStyles.iconButtonActive, s.fwd)}
          disabled={!connected}
          data-forwards-trigger=""
          aria-haspopup="dialog"
          aria-expanded={forwardsOpen}
          aria-label={forwardCount > 0 ? `${t("terminal.fwd")} (${t("terminal.fwd.running", { n: forwardCount })})` : t("terminal.fwd")}
          title={forwardCount > 0 ? `${t("terminal.fwd")} · ${t("terminal.fwd.running", { n: forwardCount })}` : t("terminal.fwd")}
          onClick={(e) => onForwards(e.currentTarget)}
        >
          <Icon name="arrows-left-right" />
          {forwardCount > 0 && <span className={s.fwdCount}>{forwardCount}</span>}
        </button>
      )}
      <button
        type="button"
        className={cx(s.sftp, sftpOpen && connected && s.sftpOn)}
        disabled={!connected}
        aria-pressed={sftpOpen && connected}
        title={t("terminal.sftpToggle")}
        onClick={onToggleSftp}
      >
        <Icon name="folder-simple" size={14} />
        {t("terminal.sftp")}
      </button>
      <IconButton
        icon="dots-three"
        size={16}
        label={t("terminal.more")}
        active={moreOpen}
        aria-haspopup="menu"
        aria-expanded={moreOpen}
        onClick={(e) => onMore(e.currentTarget)}
      />
    </div>
  );
}

/**
 * The resource usage readout (TERM-12): CPU, memory and network rates, which open the details. A
 * narrow bar drops the rates, then everything but the icon.
 */
function StatsReadout({ stats, open, onClick }: { stats: SessionStats | undefined; open: boolean; onClick(button: HTMLElement): void }) {
  const t = useT();
  const kind = stats?.state.kind ?? "starting";
  const latest = stats?.history.at(-1);
  const mem = latest ? percentOf(latest.mem_used, latest.mem_total) : null;
  const value = (v: number | null | undefined, format: (v: number) => string) => (v != null ? format(v) : "—");
  return (
    <button
      type="button"
      className={cx(s.stats, open && s.statsOpen)}
      data-stats-trigger=""
      aria-haspopup="dialog"
      aria-expanded={open}
      title={t("terminal.stats")}
      onClick={(e) => onClick(e.currentTarget)}
    >
      <Icon name={kind === "failed" ? "warning-circle" : "gauge"} size={14} className={cx(kind === "failed" && s.statsFailed)} />
      {kind !== "failed" && kind !== "unsupported" && (
        <span className={s.statsText}>
          <span>
            {t("terminal.stats.cpu")} <b className={s.pct}>{value(latest?.cpu_percent, formatPercent)}</b>
          </span>
          <span>
            {t("terminal.stats.mem")} <b className={s.pct}>{value(mem, formatPercent)}</b>
          </span>
          {latest && latest.net_interfaces.length > 0 && (
            <span className={s.statsNet}>
              ↓ <b className={s.rate}>{value(latest.net_rx_rate, formatRate)}</b> ↑ <b className={s.rate}>{value(latest.net_tx_rate, formatRate)}</b>
            </span>
          )}
        </span>
      )}
    </button>
  );
}
