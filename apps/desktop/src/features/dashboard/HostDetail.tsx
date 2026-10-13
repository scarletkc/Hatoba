import { useState } from "react";
import { Button, Icon } from "@/components/controls";
import { FooterSpacer, Sheet } from "@/components/overlay";
import { OsIcon } from "@/features/hosts/OsIcon";
import { Sparkline } from "@/features/terminal/Sparkline";
import { formatLoad, formatPercent, formatRate, formatUptime, percentOf, type StatsReading } from "@/features/terminal/stats";
import { formatBytes, formatMessageTime, useT } from "@/i18n";
import type { GroupView, HostView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { dashboardSwitch, openTerminal, retryHost } from "./actions";
import { statusColor, statusText } from "./status";
import { isStale, memPercent, readingGapMs, mountPercent, pastThreshold, rootPercent } from "./logic";
import type { HostReadout } from "./readings";
import s from "./HostDetail.module.css";

/** The charts span the sheet's content width. */
const CHART_WIDTH = 484;
const CHART_HEIGHT = 44;
/** The charts show the readings of the last 3 minutes. */
const SPAN_MS = 180_000;
/** The network chart's scale never drops below 1 KB/s. */
const MIN_RATE_SCALE = 1024;

type T = ReturnType<typeof useT>;

/**
 * A host's detail on the dashboard (HOST-14): CPU and network over the last minutes, memory and swap, every
 * filesystem, the load and uptime, as the terminal's resource usage popover shows them (TERM-12). **Open
 * Terminal** brings the host's tab to the front or opens one.
 */
export function HostDetail({
  host,
  readout,
  shown,
  groups,
  interval,
  alerts,
  onClose,
}: {
  host: HostView;
  readout: HostReadout | undefined;
  /** Not left out of the dashboard. */
  shown: boolean;
  groups: ReadonlyMap<string, GroupView>;
  interval: number;
  alerts: { cpu: number; mem: number; disk: number };
  onClose(): void;
}) {
  const t = useT();
  const [picked, setPicked] = useState<number | null>(null);
  const history = shown ? (readout?.history ?? []) : [];
  const last = history.length - 1;
  const index = picked != null && picked <= last ? picked : last;
  const reading: StatsReading | undefined = history[index];
  const now = Date.now();
  const stale = shown && isStale(readout, now, interval);
  const status = statusText(t, readout);
  const target = `${host.username ? `${host.username}@` : ""}${host.address}:${host.port}`;
  const gap = readingGapMs(interval);
  const times = history.map((r) => r.at);
  const phase = readout?.phase;
  const toggle = dashboardSwitch(host, groups);

  const footer = (
    <>
      <Button
        icon={toggle.icon}
        onClick={() => {
          toggle.run();
          if (shown) onClose();
        }}
      >
        {t(toggle.key)}
      </Button>
      <FooterSpacer />
      {shown && (phase === "failed" || phase === "offline" || (phase === "held" && readout?.hold !== "password")) && (
        <Button onClick={() => retryHost(host.id)}>{t("btn.retry")}</Button>
      )}
      <Button onClick={onClose}>{t("btn.close")}</Button>
      <Button
        variant="primary"
        icon="terminal-window"
        onClick={() => {
          onClose();
          openTerminal(host.id);
        }}
      >
        {t("dashboard.openTerminal")}
      </Button>
    </>
  );

  return (
    <Sheet onClose={onClose} footer={footer} width={540}>
      <div className={s.head}>
        <OsIcon os={readout?.os ?? host.os} badge={statusColor(readout, !shown)} />
        <div className={s.headText}>
          <div className={s.title}>{host.name}</div>
          <div className={s.target}>{target}</div>
        </div>
      </div>
      <div className={cx(s.status, s[status.tone])}>
        {!shown ? t("dashboard.leftOutBadge") : status.text}
        {stale && reading && ` · ${t("dashboard.readAt", { time: formatMessageTime(t.locale, history[last].at, now) })}`}
        {picked != null && index < last && ` · ${t("terminal.stats.ago", { n: Math.round((history[last].at - reading!.at) / 1000) })}`}
      </div>
      {reading && (
        <div className={cx(s.body, stale && s.stale)}>
          <Section label={t("terminal.stats.cpu")} value={reading.cpu_percent != null ? formatPercent(reading.cpu_percent) : "—"} alert={pastThreshold(reading.cpu_percent, alerts.cpu)}>
            <Sparkline
              series={[{ values: history.map((r) => r.cpu_percent), className: s.cpuLine }]}
              times={times}
              span={SPAN_MS}
              gap={gap}
              max={100}
              width={CHART_WIDTH}
              height={CHART_HEIGHT}
              label={t("terminal.stats.chartCpu")}
              picked={picked}
              onPick={setPicked}
            />
            <CpuDetail reading={reading} t={t} />
          </Section>
          <MemorySection reading={reading} alert={alerts.mem} t={t} />
          <NetworkSection history={history} reading={reading} gap={gap} picked={picked} onPick={setPicked} t={t} />
          <FilesystemsSection reading={reading} alert={alerts.disk} t={t} />
        </div>
      )}
    </Sheet>
  );
}

function Section({ label, value, alert, children }: { label: string; value?: string; alert?: boolean; children?: React.ReactNode }) {
  return (
    <section className={s.section} aria-label={label}>
      <div className={s.row}>
        <span className={s.label}>{label}</span>
        {value && (
          <span className={cx(s.value, alert && s.alert)}>
            {alert && <Icon name="warning" fill />}
            {value}
          </span>
        )}
      </div>
      {children}
    </section>
  );
}

function CpuDetail({ reading, t }: { reading: StatsReading; t: T }) {
  const load = formatLoad(reading.load);
  const parts = [
    reading.cpus != null ? t("terminal.stats.cores", { n: reading.cpus }) : null,
    load ? t("terminal.stats.load", { load }) : null,
    reading.uptime_secs != null ? t("terminal.stats.uptime", { time: formatUptime(t.locale, reading.uptime_secs) }) : null,
  ].filter(Boolean);
  return parts.length > 0 ? <div className={s.detail}>{parts.join(" · ")}</div> : null;
}

/** `3.1 GB / 7.8 GB` and a meter, red past the threshold. A filesystem is labelled with its mount point (`mono`). */
function Usage({ label, mono, used, total, percent, alert }: { label: string; mono?: boolean; used: number; total: number; percent: number; alert: boolean }) {
  return (
    <div className={cx(s.usage, alert && s.alert)}>
      <div className={s.row}>
        <span className={cx(s.usageLabel, mono && s.mono)} title={label}>
          {label}
        </span>
        <span className={s.amount}>
          {formatBytes(used)} / {formatBytes(total)}
        </span>
        <span className={s.value}>
          {alert && <Icon name="warning" fill />}
          {formatPercent(percent)}
        </span>
      </div>
      <div className={s.meter} role="meter" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(percent)}>
        <div className={s.meterFill} style={{ width: `${percent}%` }} />
      </div>
    </div>
  );
}

function MemorySection({ reading, alert, t }: { reading: StatsReading; alert: number; t: T }) {
  const percent = memPercent(reading);
  if (percent == null) return null;
  const swapPercent = percentOf(reading.swap_used, reading.swap_total);
  return (
    <section className={s.section} aria-label={t("terminal.stats.mem")}>
      <Usage label={t("terminal.stats.mem")} used={reading.mem_used!} total={reading.mem_total!} percent={percent} alert={pastThreshold(percent, alert)} />
      {reading.swap_total === 0 ? (
        <div className={s.detail}>{t("terminal.stats.noSwap")}</div>
      ) : (
        swapPercent != null && (
          <Usage label={t("terminal.stats.swap")} used={reading.swap_used!} total={reading.swap_total!} percent={swapPercent} alert={false} />
        )
      )}
    </section>
  );
}

function NetworkSection({
  history,
  reading,
  gap,
  picked,
  onPick,
  t,
}: {
  history: StatsReading[];
  reading: StatsReading;
  gap: number;
  picked: number | null;
  onPick(i: number | null): void;
  t: T;
}) {
  if (reading.net_interfaces.length === 0) return null;
  const rx = history.map((r) => r.net_rx_rate);
  const tx = history.map((r) => r.net_tx_rate);
  const max = Math.max(MIN_RATE_SCALE, ...rx.map((v) => v ?? 0), ...tx.map((v) => v ?? 0));
  const rate = (v: number | null) => (v != null ? formatRate(v) : "—");
  return (
    <section className={s.section} aria-label={t("terminal.stats.net")}>
      <div className={s.row}>
        <span className={s.label}>
          {t("terminal.stats.net")}
          <span className={s.sub}>{reading.net_interfaces.join(", ")}</span>
        </span>
        <span className={s.legend}>
          <i className={cx(s.key, s.rxLine)} />↓ {rate(reading.net_rx_rate)}
        </span>
        <span className={s.legend}>
          <i className={cx(s.key, s.txLine)} />↑ {rate(reading.net_tx_rate)}
        </span>
      </div>
      <Sparkline
        series={[
          { values: rx, className: s.rxLine },
          { values: tx, className: s.txLine },
        ]}
        times={history.map((r) => r.at)}
        span={SPAN_MS}
        gap={gap}
        max={max}
        width={CHART_WIDTH}
        height={CHART_HEIGHT}
        label={t("terminal.stats.chartNet")}
        picked={picked}
        onPick={onPick}
      />
      {reading.net_rx_total != null && reading.net_tx_total != null && (
        <div className={s.detail}>{t("dashboard.netTotal", { rx: formatBytes(reading.net_rx_total), tx: formatBytes(reading.net_tx_total) })}</div>
      )}
    </section>
  );
}

function FilesystemsSection({ reading, alert, t }: { reading: StatsReading; alert: number; t: T }) {
  // The root filesystem leads, from its own reading when `df` listed no `/` (or listed none at all).
  const mounts = reading.mounts.some((m) => m.mount_point === "/")
    ? reading.mounts
    : reading.disk_total != null && reading.disk_used != null && reading.disk_available != null
      ? [{ source: "", mount_point: "/", total: reading.disk_total, used: reading.disk_used, available: reading.disk_available }, ...reading.mounts]
      : reading.mounts;
  if (mounts.length === 0) return null;
  const root = rootPercent(reading);
  return (
    <section className={s.section} aria-label={t("dashboard.filesystems")}>
      <div className={s.row}>
        <span className={s.label}>{t("dashboard.filesystems")}</span>
      </div>
      {mounts.map((m) => {
        const percent = (m.mount_point === "/" ? root : null) ?? mountPercent(m);
        if (percent == null) return null;
        return (
          <Usage
            key={`${m.source}|${m.mount_point}`}
            label={m.mount_point}
            mono
            used={m.used}
            total={m.total}
            percent={percent}
            alert={pastThreshold(percent, alert)}
          />
        );
      })}
    </section>
  );
}
