import { useRef, useState } from "react";
import { createPortal } from "react-dom";
import { errorMessage } from "@/app/errors";
import { Icon, Spinner } from "@/components/controls";
import type { MenuAnchor } from "@/components/overlay";
import { formatBytes, useT } from "@/i18n";
import { cx } from "@/lib/cx";
import { useStatusPopover } from "./popover";
import { Sparkline } from "./Sparkline";
import {
  formatLoad,
  formatPercent,
  formatRate,
  formatUptime,
  meterLevel,
  percentOf,
  STATS_GAP_MS,
  STATS_WINDOW_MS,
  type SessionStats,
  type StatsReading,
} from "./stats";
import s from "./StatsPopover.module.css";

/** The charts span the popover's content width. */
const CHART_WIDTH = 276;
const CHART_HEIGHT = 34;
/** The network chart's scale never drops below 1 KB/s, so an idle line stays flat at the bottom. */
const MIN_RATE_SCALE = 1024;

/**
 * The status bar's resource usage popover (TERM-12): CPU and network over the last few minutes,
 * memory, swap and the root filesystem, and the switch that hides it for this host. Pointing at a
 * chart shows every value as it was at that reading.
 */
export function StatsPopover({
  anchor,
  stats,
  onRetry,
  onHide,
  onClose,
}: {
  anchor: MenuAnchor;
  stats: SessionStats | undefined;
  onRetry(): void;
  onHide(): void;
  /** `restoreFocus` is true when closed from the keyboard, so typing can continue in the terminal. */
  onClose: (restoreFocus: boolean) => void;
}) {
  const t = useT();
  const ref = useRef<HTMLDivElement>(null);
  const [picked, setPicked] = useState<number | null>(null);
  const state = stats?.state ?? { kind: "starting" };
  const history = stats?.history ?? [];
  const { pos, locked } = useStatusPopover(ref, anchor, onClose, "[data-stats-trigger]", [state.kind, history.length > 0]);

  const last = history.length - 1;
  const index = picked != null && picked <= last ? picked : last;
  const reading: StatsReading | undefined = history[index];
  const agoSecs = reading && index < last ? Math.round((history[last].at - reading.at) / 1000) : 0;

  let body;
  if (state.kind === "unsupported") {
    body = (
      <div className={s.note}>
        {state.system ? t("terminal.stats.unsupportedSystem", { system: state.system }) : t("terminal.stats.unsupported")}
      </div>
    );
  } else if (state.kind === "failed" && !reading) {
    body = <Failed message={errorMessage(t, state.error)} onRetry={onRetry} />;
  } else if (!reading) {
    body = (
      <div className={s.note}>
        <Spinner size={12} />
        {t("terminal.stats.waiting")}
      </div>
    );
  } else {
    body = (
      <>
        {state.kind === "failed" && <Failed message={errorMessage(t, state.error)} onRetry={onRetry} />}
        <CpuSection history={history} reading={reading} picked={picked} onPick={setPicked} />
        <MemorySection reading={reading} />
        <NetworkSection history={history} reading={reading} picked={picked} onPick={setPicked} />
        <DiskSection reading={reading} />
        {reading.uptime_secs != null && (
          <div className={cx(s.section, s.detail)}>{t("terminal.stats.uptime", { time: formatUptime(t.locale, reading.uptime_secs) })}</div>
        )}
      </>
    );
  }

  if (locked) return null;
  return createPortal(
    <div ref={ref} role="dialog" aria-label={t("terminal.stats")} className={s.popover} style={pos}>
      <div className={s.header}>
        <span>{t("terminal.stats")}</span>
        {agoSecs > 0 && <span className={s.ago}>{t("terminal.stats.ago", { n: agoSecs })}</span>}
      </div>
      {body}
      <div className={s.separator} />
      {state.kind !== "unsupported" && <div className={s.footnote}>{t("terminal.stats.note")}</div>}
      <button
        type="button"
        className={s.hide}
        onClick={() => {
          onClose(true);
          onHide();
        }}
      >
        <Icon name="eye-slash" />
        {t("terminal.stats.hide")}
      </button>
    </div>,
    document.body,
  );
}

function Failed({ message, onRetry }: { message: string; onRetry(): void }) {
  const t = useT();
  return (
    <div className={cx(s.note, s.noteError)}>
      <span className={s.noteText}>{t("terminal.stats.failed", { error: message })}</span>
      <button type="button" className={s.retry} onClick={onRetry}>
        {t("btn.retry")}
      </button>
    </div>
  );
}

interface ChartProps {
  history: StatsReading[];
  reading: StatsReading;
  picked: number | null;
  onPick(index: number | null): void;
}

function CpuSection({ history, reading, picked, onPick }: ChartProps) {
  const t = useT();
  const load = formatLoad(reading.load);
  const details = [reading.cpus != null ? t("terminal.stats.cores", { n: reading.cpus }) : null, load ? t("terminal.stats.load", { load }) : null];
  return (
    <div className={s.section}>
      <div className={s.row}>
        <span className={s.label}>{t("terminal.stats.cpu")}</span>
        <span className={s.value}>{reading.cpu_percent != null ? formatPercent(reading.cpu_percent) : "—"}</span>
      </div>
      <Sparkline
        series={[{ values: history.map((r) => r.cpu_percent), className: s.cpuLine }]}
        times={history.map((r) => r.at)}
        span={STATS_WINDOW_MS}
        gap={STATS_GAP_MS}
        max={100}
        width={CHART_WIDTH}
        height={CHART_HEIGHT}
        label={t("terminal.stats.chartCpu")}
        picked={picked}
        onPick={onPick}
      />
      {details.some(Boolean) && <div className={s.detail}>{details.filter(Boolean).join(" · ")}</div>}
    </div>
  );
}

function MemorySection({ reading }: { reading: StatsReading }) {
  const t = useT();
  const percent = percentOf(reading.mem_used, reading.mem_total);
  if (percent == null) return null;
  const swap =
    reading.swap_total === 0
      ? t("terminal.stats.noSwap")
      : reading.swap_total != null && reading.swap_used != null
        ? `${t("terminal.stats.swap")} ${formatBytes(reading.swap_used)} / ${formatBytes(reading.swap_total)}`
        : null;
  return (
    <div className={s.section}>
      <Usage label={t("terminal.stats.mem")} used={reading.mem_used!} total={reading.mem_total!} percent={percent} />
      {swap && <div className={s.detail}>{swap}</div>}
    </div>
  );
}

function DiskSection({ reading }: { reading: StatsReading }) {
  const t = useT();
  const { disk_used: used, disk_available: available, disk_total: total } = reading;
  if (used == null || available == null || total == null) return null;
  // As `df` counts it: blocks reserved for root are neither used nor available.
  const percent = percentOf(used, used + available);
  if (percent == null) return null;
  return (
    <div className={s.section}>
      <Usage label={t("terminal.stats.disk")} sub="/" used={used} total={total} percent={percent} />
    </div>
  );
}

/** A labelled meter: `3.1 GB / 7.8 GB  41%`, the fill turning orange and then red as it fills. */
function Usage({ label, sub, used, total, percent }: { label: string; sub?: string; used: number; total: number; percent: number }) {
  const level = meterLevel(percent);
  return (
    <>
      <div className={s.row}>
        <span className={s.label}>
          {label}
          {sub && <span className={s.sub}>{sub}</span>}
        </span>
        <span className={s.amount}>
          {formatBytes(used)} / {formatBytes(total)}
        </span>
        <span className={s.value}>
          {level !== "normal" && <Icon name="warning" fill className={s[level]} />}
          {formatPercent(percent)}
        </span>
      </div>
      <div
        className={cx(s.meter, s[level])}
        role="meter"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(percent)}
      >
        <div className={s.meterFill} style={{ width: `${percent}%` }} />
      </div>
    </>
  );
}

function NetworkSection({ history, reading, picked, onPick }: ChartProps) {
  const t = useT();
  if (reading.net_interfaces.length === 0) return null;
  const rx = history.map((r) => r.net_rx_rate);
  const tx = history.map((r) => r.net_tx_rate);
  const max = Math.max(MIN_RATE_SCALE, ...rx.map((v) => v ?? 0), ...tx.map((v) => v ?? 0));
  const rate = (v: number | null) => (v != null ? formatRate(v) : "—");
  return (
    <div className={s.section}>
      <div className={s.row}>
        <span className={s.label}>
          {t("terminal.stats.net")}
          <span className={s.sub}>{reading.net_interfaces.join(", ")}</span>
        </span>
        <span className={s.legend} aria-label={t("terminal.stats.rx", { rate: rate(reading.net_rx_rate) })}>
          <i className={cx(s.key, s.rxLine)} />↓ {rate(reading.net_rx_rate)}
        </span>
        <span className={s.legend} aria-label={t("terminal.stats.tx", { rate: rate(reading.net_tx_rate) })}>
          <i className={cx(s.key, s.txLine)} />↑ {rate(reading.net_tx_rate)}
        </span>
      </div>
      <Sparkline
        series={[
          { values: rx, className: s.rxLine },
          { values: tx, className: s.txLine },
        ]}
        times={history.map((r) => r.at)}
        span={STATS_WINDOW_MS}
        gap={STATS_GAP_MS}
        max={max}
        width={CHART_WIDTH}
        height={CHART_HEIGHT}
        label={t("terminal.stats.chartNet")}
        picked={picked}
        onPick={onPick}
      />
    </div>
  );
}
