import { memo, useCallback, useState, type CSSProperties, type MouseEvent, type RefCallback } from "react";
import { Button, Icon, IconButton, Spinner } from "@/components/controls";
import { Tag } from "@/components/layout";
import { Card, Chip } from "@/components/patterns";
import { OsIcon } from "@/features/hosts/OsIcon";
import { formatLoad, formatPercent, formatRate, formatUptime, type StatsReading } from "@/features/terminal/stats";
import { connectHost } from "@/features/terminal/connect";
import { formatBytes, formatMessageTime, useT, type MessageKey } from "@/i18n";
import type { GroupView, HostView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { dashboardSwitch, retryHost } from "./actions";
import {
  COLUMN_GAP,
  COLUMN_WIDTHS,
  NAME_WIDTH,
  fitColumns,
  fullestMount,
  isStale,
  latestReading,
  memPercent,
  mountPercent,
  pastThreshold,
  rootPercent,
  type DashboardColumn,
} from "./logic";
import type { HostReadout } from "./readings";
import { statusColor, statusText } from "./status";
import s from "./views.module.css";

type T = ReturnType<typeof useT>;

/** The thresholds past which a value stands out (a setting), in percent; 0 is off. */
export interface Alerts {
  cpu: number;
  mem: number;
  disk: number;
}

/** What a card or row needs besides its host. Stable between readings, so only the hosts that changed render again. */
export interface View {
  t: T;
  now: number;
  interval: number;
  alerts: Alerts;
  groups: ReadonlyMap<string, GroupView>;
  /** Resource usage shows (HOST-14); without it a card shows the last connection. */
  stats: boolean;
  connectLabel: string;
  onSelect(id: string): void;
  onConnect(id: string): void;
  onContext(e: MouseEvent, host: HostView): void;
}

/** The host list's look of a host that is not read: the badge and the last connection, worked out by the page. */
export interface ListState {
  /** The OS icon's badge: connected, failed, answering or unknown (HOST-10). */
  dot: string;
  /** The last connection, or that the last attempt failed. */
  last: string;
  failed: boolean;
}

// ───────────────────────── Status ─────────────────────────

/** Whether **Retry** applies: a failed host, or one held for anything but a password asked every time. */
export function canRetry(readout: HostReadout | undefined): boolean {
  return readout?.phase === "failed" || (readout?.phase === "held" && readout.hold !== "password");
}

/** **Connect** for a host that needs the user, **Retry** for one that failed or is held; icons in a table row. */
function StatusActions({ host, readout, t, compact }: { host: HostView; readout: HostReadout | undefined; t: T; compact?: boolean }) {
  const held = readout?.phase === "held";
  const retry = canRetry(readout);
  if (!retry && !held) return null;
  return (
    <span className={s.actions} onClick={(e) => e.stopPropagation()} onDoubleClick={(e) => e.stopPropagation()}>
      {held &&
        (compact ? (
          <IconButton icon="plugs-connected" label={t("hosts.connect")} onClick={() => void connectHost(host.id)} />
        ) : (
          <Button size="xs" variant="primary" onClick={() => void connectHost(host.id)}>
            {t("hosts.connect")}
          </Button>
        ))}
      {retry &&
        (compact ? (
          <IconButton icon="arrow-clockwise" label={t("btn.retry")} onClick={() => retryHost(host.id)} />
        ) : (
          <Button size="xs" onClick={() => retryHost(host.id)}>
            {t("btn.retry")}
          </Button>
        ))}
    </span>
  );
}

/** **Watch Resource Usage**, or the group's, for a host that is left out. */
function IncludeButton({ host, groups, t }: { host: HostView; groups: ReadonlyMap<string, GroupView>; t: T }) {
  const toggle = dashboardSwitch(host, groups);
  return (
    <span className={s.actions} onClick={(e) => e.stopPropagation()} onDoubleClick={(e) => e.stopPropagation()}>
      <Button size="xs" onClick={toggle.run}>
        {t(toggle.key)}
      </Button>
    </span>
  );
}

/** The status, and for old readings when they were read; the full text on hover. */
function statusLine(t: T, readout: HostReadout | undefined, reading: StatsReading | undefined, stale: boolean, now: number) {
  const status = statusText(t, readout);
  const read = stale && reading ? t("dashboard.readAt", { time: formatMessageTime(t.locale, reading.at, now) }) : null;
  return { ...status, full: read ? `${status.text} · ${read}` : status.text };
}

/** The Connect button a row or card shows under the pointer or while selected. */
function ConnectButton({ id, view, className }: { id: string; view: View; className: string }) {
  return (
    <Button
      size="xs"
      variant="primary"
      tabIndex={-1}
      className={className}
      onClick={(e) => {
        e.stopPropagation();
        view.onConnect(id);
      }}
    >
      {view.connectLabel}
    </Button>
  );
}

// ───────────────────────── Cards ─────────────────────────

/** A labelled meter that turns red past its threshold. */
function Meter({ label, sub, percent, alert, alertLabel }: { label: string; sub?: string; percent: number | null; alert: boolean; alertLabel: string }) {
  return (
    <div className={cx(s.meterRow, alert && s.alert)} title={alert ? alertLabel : undefined}>
      <span className={s.meterLabel}>
        {label}
        {sub && <span className={s.meterSub}>{sub}</span>}
      </span>
      <span className={s.meter} role="meter" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent == null ? undefined : Math.round(percent)}>
        <span className={s.meterFill} style={{ width: `${percent ?? 0}%` }} />
      </span>
      <span className={s.meterValue}>
        {alert && <Icon name="warning" fill />}
        {percent == null ? "—" : formatPercent(percent)}
      </span>
    </div>
  );
}

function rates(r: StatsReading): string | null {
  if (r.net_rx_rate == null && r.net_tx_rate == null) return null;
  return `↓ ${r.net_rx_rate != null ? formatRate(r.net_rx_rate) : "—"} ↑ ${r.net_tx_rate != null ? formatRate(r.net_tx_rate) : "—"}`;
}

/** CPU, memory and the root filesystem as meters, then the rates, load and uptime. */
function Readings({ reading, alerts, t }: { reading: StatsReading; alerts: Alerts; t: T }) {
  const mem = memPercent(reading);
  const root = rootPercent(reading);
  // Every other filesystem over the threshold, beside the root filesystem's bar.
  const overs = reading.mounts
    .filter((m) => m.mount_point !== "/")
    .map((m) => ({ mount: m.mount_point, percent: mountPercent(m) }))
    .filter((m) => pastThreshold(m.percent, alerts.disk));
  const load = formatLoad(reading.load);
  const net = rates(reading);
  const details = [
    load ? t("terminal.stats.load", { load }) : null,
    reading.uptime_secs != null ? t("terminal.stats.uptime", { time: formatUptime(t.locale, reading.uptime_secs) }) : null,
  ].filter(Boolean);
  return (
    <div className={s.readings}>
      <Meter
        label={t("terminal.stats.cpu")}
        percent={reading.cpu_percent}
        alert={pastThreshold(reading.cpu_percent, alerts.cpu)}
        alertLabel={t("dashboard.alert", { n: alerts.cpu })}
      />
      <Meter label={t("terminal.stats.mem")} percent={mem} alert={pastThreshold(mem, alerts.mem)} alertLabel={t("dashboard.alert", { n: alerts.mem })} />
      <Meter
        label={t("terminal.stats.disk")}
        sub="/"
        percent={root}
        alert={pastThreshold(root, alerts.disk)}
        alertLabel={t("dashboard.alert", { n: alerts.disk })}
      />
      {overs.length > 0 && (
        <div className={cx(s.detail, s.alertText)} title={overs.map((m) => `${m.mount} ${formatPercent(m.percent!)}`).join("\n")}>
          <Icon name="warning" fill /> {overs.map((m) => `${m.mount} ${formatPercent(m.percent!)}`).join(" · ")}
        </div>
      )}
      {net && <div className={s.detail}>{net}</div>}
      {details.length > 0 && <div className={s.detail}>{details.join(" · ")}</div>}
    </div>
  );
}

interface ItemProps {
  host: HostView;
  selected: boolean;
  /** The readings, while resource usage shows. */
  readout: HostReadout | undefined;
  /** What left the host out of the readings, if anything. */
  leftOut: "host" | "group" | null;
  list: ListState;
  view: View;
}

/** A host as a card: its readings while resource usage shows, else when it last connected. */
export const HostCard = memo(function HostCard({ host, selected, readout, leftOut, list, view }: ItemProps) {
  const { t, now, interval, alerts, stats } = view;
  const reading = stats && !leftOut ? latestReading(readout) : undefined;
  const stale = stats && !leftOut && isStale(readout, now, interval);
  const status = statusLine(t, readout, reading, stale, now);
  const target = `${host.username ? `${host.username}@` : ""}${host.address}:${host.port}`;
  return (
    <Card
      interactive
      role="option"
      aria-selected={selected}
      data-id={host.id}
      className={cx(s.card, selected && s.cardSel, stats && leftOut && s.cardLeftOut)}
      onClick={() => view.onSelect(host.id)}
      onDoubleClick={() => view.onConnect(host.id)}
      onContextMenu={(e) => view.onContext(e, host)}
    >
      <div className={s.cardHead}>
        <OsIcon os={(stats && readout?.os) || host.os} badge={stats ? statusColor(readout, !!leftOut) : list.dot} />
        <span className={s.name}>{host.name}</span>
        {host.favorite && <Icon name="star" fill className={s.fav} />}
        <ConnectButton id={host.id} view={view} className={s.connect} />
      </div>
      <div className={s.target}>{target}</div>
      {host.tags.length > 0 && (
        <div className={s.tags}>
          {host.tags.map((tag) => (
            <Tag key={tag} name={tag} />
          ))}
        </div>
      )}
      {!stats ? (
        <div className={cx(s.status, s.cardLast, list.failed && s.error)}>{list.last}</div>
      ) : leftOut ? (
        <div className={s.status}>
          <Chip>{t("dashboard.leftOutBadge")}</Chip>
          <IncludeButton host={host} groups={view.groups} t={t} />
        </div>
      ) : (
        <>
          {reading && (
            <div className={cx(stale && s.stale)}>
              <Readings reading={reading} alerts={alerts} t={t} />
            </div>
          )}
          {(!reading || stale) && (
            <div className={s.status}>
              {readout?.phase === "connecting" && <Spinner size={12} />}
              <span className={cx(s.statusText, s[status.tone])} title={status.full}>
                {status.full}
              </span>
              <StatusActions host={host} readout={readout} t={t} />
            </div>
          )}
        </>
      )}
    </Card>
  );
});

// ───────────────────────── Table ─────────────────────────

/** The width of the trailing column, which holds Connect. */
const ACTION_WIDTH = 64;

/** The grid of the table's head and rows: the name, the columns fitColumns left room for, and Connect. */
function gridOf(columns: DashboardColumn[]): CSSProperties {
  const widths = [NAME_WIDTH, ...columns.map((c) => COLUMN_WIDTHS[c])];
  return { gridTemplateColumns: [...widths.map(([least, most]) => `minmax(${least}px, ${most})`), `${ACTION_WIDTH}px`].join(" ") };
}

/** The columns a table `width` pixels wide has room for besides the name and Connect. */
export function fitStatsColumns(columns: DashboardColumn[], width: number | null): DashboardColumn[] {
  return width === null ? columns : fitColumns(columns, width - ACTION_WIDTH - COLUMN_GAP);
}

/** The width of the element given the returned ref, kept current as it resizes; null before it mounts. */
export function useWidth(): [RefCallback<HTMLElement>, number | null] {
  const [width, setWidth] = useState<number | null>(null);
  const ref = useCallback((el: HTMLElement | null) => {
    if (!el) return;
    const update = () => setWidth(el.clientWidth);
    update();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(update);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  return [ref, width];
}

export function StatsHead({ columns, t, measure }: { columns: DashboardColumn[]; t: T; measure: RefCallback<HTMLElement> }) {
  return (
    <div ref={measure} className={cx(s.tableHead, s.grid)} style={gridOf(columns)}>
      <span className={s.headName}>{t("dashboard.col.name")}</span>
      {columns.map((c) => (
        <span key={c}>{t(`dashboard.col.${c}` as MessageKey)}</span>
      ))}
      <span />
    </div>
  );
}

/** A small meter and its percentage, red past the threshold. */
function MiniMeter({ percent, alert, title }: { percent: number | null; alert: boolean; title?: string }) {
  if (percent == null) return <span className={s.dim}>—</span>;
  return (
    <span className={cx(s.mini, alert && s.alert)} title={title}>
      <span className={s.meter}>
        <span className={s.meterFill} style={{ width: `${percent}%` }} />
      </span>
      <span className={s.miniValue}>{formatPercent(percent)}</span>
    </span>
  );
}

/** Every filesystem, for the disk cell's tooltip. */
function mountsTitle(r: StatsReading): string {
  const lines = r.mounts.map((m) => {
    const p = mountPercent(m);
    return `${m.mount_point}  ${p == null ? "—" : formatPercent(p)}  (${formatBytes(m.used)} / ${formatBytes(m.total)})`;
  });
  return lines.join("\n");
}

/** A host's row with its readings, a column per value. */
export const StatsRow = memo(function StatsRow({ host, selected, readout, leftOut, columns, view }: Omit<ItemProps, "list"> & { columns: DashboardColumn[] }) {
  const { t, now, interval, alerts } = view;
  const reading = leftOut ? undefined : latestReading(readout);
  const stale = !leftOut && isStale(readout, now, interval);
  const status = statusLine(t, readout, reading, stale, now);
  const cell = (c: DashboardColumn) => {
    switch (c) {
      case "status":
        if (leftOut) return <Chip>{t("dashboard.leftOutBadge")}</Chip>;
        return (
          <span className={s.cellStatus}>
            <span className={cx(s.statusText, s[status.tone])} title={status.full}>
              {status.full}
            </span>
            <StatusActions host={host} readout={readout} t={t} compact />
          </span>
        );
      case "cpu":
        return <MiniMeter percent={reading?.cpu_percent ?? null} alert={pastThreshold(reading?.cpu_percent ?? null, alerts.cpu)} />;
      case "memory": {
        const p = memPercent(reading);
        return <MiniMeter percent={p} alert={pastThreshold(p, alerts.mem)} />;
      }
      case "disk": {
        const fullest = fullestMount(reading);
        // The root filesystem, or the fullest one once it is past the threshold.
        const shownMount = fullest && pastThreshold(fullest.percent, alerts.disk) ? fullest : { mount: "/", percent: rootPercent(reading) };
        return (
          <MiniMeter
            percent={shownMount.percent}
            alert={pastThreshold(shownMount.percent, alerts.disk)}
            title={reading ? mountsTitle(reading) || undefined : undefined}
          />
        );
      }
      case "network":
        return <span className={s.mono}>{reading ? (rates(reading) ?? "—") : "—"}</span>;
      case "load":
        return <span className={s.mono}>{(reading && formatLoad(reading.load)) ?? "—"}</span>;
      case "uptime":
        return <span>{reading?.uptime_secs != null ? formatUptime(t.locale, reading.uptime_secs) : "—"}</span>;
    }
  };
  return (
    <div
      role="option"
      aria-selected={selected}
      data-id={host.id}
      className={cx(s.row, s.grid, selected && s.rowSel, leftOut && s.cardLeftOut, stale && s.staleRow)}
      style={gridOf(columns)}
      onClick={() => view.onSelect(host.id)}
      onDoubleClick={() => view.onConnect(host.id)}
      onContextMenu={(e) => view.onContext(e, host)}
    >
      <span className={s.rowName}>
        <OsIcon os={readout?.os ?? host.os} badge={statusColor(readout, !!leftOut)} />
        <span className={s.name}>{host.name}</span>
        {host.favorite && <Icon name="star" fill className={s.fav} />}
      </span>
      {columns.map((c) => (
        <span key={c} className={cx(s.cell, c !== "status" && s.valueCell)}>
          {cell(c)}
        </span>
      ))}
      <span className={s.action}>
        <ConnectButton id={host.id} view={view} className={s.connect} />
      </span>
    </div>
  );
});
