import {
  memo,
  useCallback,
  useState,
  type CSSProperties,
  type MouseEvent,
  type ReactNode,
  type RefCallback,
} from "react";
import { Button, Card, Chip, Icon, IconButton, LinkButton, Spinner, Tag } from "@hatoba/ui";
import { formatLoad, formatPercent, formatRate, formatUptime, type StatsReading } from "@/features/terminal/stats";
import { formatBytes, formatMessageTime, useT, type MessageKey } from "@/i18n";
import type { GroupView, HostView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { OsIcon } from "@/lib/OsIcon";
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
import { statusColor, statusParts, statusText } from "./status";
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
  /** Some listed host has tags, so every card keeps a line for them and the cards line up. */
  tagRow: boolean;
  connectLabel: string;
  /** A click on a host, whose Shift and Ctrl (⌘ on macOS) pick several. */
  onSelect(e: MouseEvent, id: string): void;
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

/** **Retry** as an icon in a table row, for a host that failed or is held. */
function RetryIcon({ host, readout, t }: { host: HostView; readout: HostReadout | undefined; t: T }) {
  if (!canRetry(readout)) return null;
  return (
    <span className={s.actions} onClick={(e) => e.stopPropagation()} onDoubleClick={(e) => e.stopPropagation()}>
      <IconButton icon="arrow-clockwise" label={t("btn.retry")} onClick={() => retryHost(host.id)} />
    </span>
  );
}

/** A text button on a card, which neither selects nor connects the card. */
function CardLink({ label, onClick }: { label: string; onClick(): void }) {
  return (
    <LinkButton
      className={s.cardLink}
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      onDoubleClick={(e) => e.stopPropagation()}
    >
      {label}
    </LinkButton>
  );
}

/** The status, and for old readings when they were read; the full text on hover. */
function statusLine(
  t: T,
  readout: HostReadout | undefined,
  reading: StatsReading | undefined,
  stale: boolean,
  now: number,
) {
  const status = statusText(t, readout);
  const read = stale && reading ? t("dashboard.readAt", { time: formatMessageTime(t.locale, reading.at, now) }) : null;
  return { ...status, full: read ? `${status.text} · ${read}` : status.text };
}

/**
 * The Connect button a row or card shows under the pointer or while selected, and always while the host needs the
 * user to sign in.
 */
function ConnectButton({ id, view, shown, className }: { id: string; view: View; shown: boolean; className: string }) {
  return (
    <Button
      size="xs"
      variant="primary"
      tabIndex={-1}
      className={cx(className, shown && s.connectShown)}
      onClick={(e) => {
        e.stopPropagation();
        view.onConnect(id);
      }}
    >
      {view.connectLabel}
    </Button>
  );
}

/** The root filesystem, or the fullest one once it is past the threshold. */
function shownDisk(reading: StatsReading | undefined, threshold: number): { mount: string; percent: number | null } {
  const fullest = fullestMount(reading);
  return fullest && pastThreshold(fullest.percent, threshold) ? fullest : { mount: "/", percent: rootPercent(reading) };
}

/** Every filesystem, for the disk's tooltip. */
function mountsTitle(r: StatsReading): string {
  const lines = r.mounts.map((m) => {
    const p = mountPercent(m);
    return `${m.mount_point}  ${p == null ? "—" : formatPercent(p)}  (${formatBytes(m.used)} / ${formatBytes(m.total)})`;
  });
  return lines.join("\n");
}

function rates(r: StatsReading): string | null {
  if (r.net_rx_rate == null && r.net_tx_rate == null) return null;
  return `↓ ${r.net_rx_rate != null ? formatRate(r.net_rx_rate) : "—"} ↑ ${r.net_tx_rate != null ? formatRate(r.net_tx_rate) : "—"}`;
}

// ───────────────────────── Cards ─────────────────────────

/** A labelled meter that turns red past its threshold: a row of the meters' grid. */
function Meter({
  label,
  sub,
  percent,
  alert,
  title,
}: {
  label: string;
  sub?: string;
  percent: number | null;
  alert: boolean;
  title?: string;
}) {
  return (
    <>
      <span className={s.meterLabel} title={title}>
        {label}
        {sub && <span className={s.meterSub}>{sub}</span>}
      </span>
      <span
        className={cx(s.meter, alert && s.alert)}
        title={title}
        role="meter"
        aria-label={sub ? `${label} ${sub}` : label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent == null ? undefined : Math.round(percent)}
      >
        <span className={s.meterFill} style={{ width: `${percent ?? 0}%` }} />
      </span>
      <span className={cx(s.meterValue, alert && s.alert)} title={title}>
        {alert && <Icon name="warning" fill />}
        {percent == null ? "—" : formatPercent(percent)}
      </span>
    </>
  );
}

/** CPU, memory, and the disk the table shows, as meters. */
function Meters({ reading, alerts, t }: { reading: StatsReading; alerts: Alerts; t: T }) {
  const mem = memPercent(reading);
  const disk = shownDisk(reading, alerts.disk);
  const over = (n: number) => t("dashboard.alert", { n });
  return (
    <div className={s.meters}>
      <Meter
        label={t("terminal.stats.cpu")}
        percent={reading.cpu_percent}
        alert={pastThreshold(reading.cpu_percent, alerts.cpu)}
        title={pastThreshold(reading.cpu_percent, alerts.cpu) ? over(alerts.cpu) : undefined}
      />
      <Meter
        label={t("terminal.stats.mem")}
        percent={mem}
        alert={pastThreshold(mem, alerts.mem)}
        title={pastThreshold(mem, alerts.mem) ? over(alerts.mem) : undefined}
      />
      <Meter
        label={t("terminal.stats.disk")}
        sub={disk.mount}
        percent={disk.percent}
        alert={pastThreshold(disk.percent, alerts.disk)}
        title={mountsTitle(reading) || undefined}
      />
    </div>
  );
}

/** Under the meters: the rates, then the load and uptime. */
function DetailLines({ reading, t }: { reading: StatsReading; t: T }) {
  const load = formatLoad(reading.load);
  const details = [
    load ? t("terminal.stats.load", { load }) : null,
    reading.uptime_secs != null
      ? t("terminal.stats.uptime", { time: formatUptime(t.locale, reading.uptime_secs) })
      : null,
  ].filter(Boolean);
  return (
    <>
      <div className={s.line}>
        <span className={s.lineText}>{rates(reading)}</span>
      </div>
      <div className={s.line}>
        <span className={s.lineText}>{details.join(" · ")}</span>
      </div>
    </>
  );
}

/** Under readings from before the host stopped answering: what happened, then when they were read, and Retry. */
function StaleLines({
  host,
  readout,
  reading,
  t,
  now,
}: {
  host: HostView;
  readout: HostReadout | undefined;
  reading: StatsReading;
  t: T;
  now: number;
}) {
  const status = statusText(t, readout);
  const { icon } = statusParts(t, readout);
  return (
    <>
      <div className={cx(s.line, s[status.tone])} title={status.text}>
        {readout?.phase === "connecting" ? <Spinner size={10} /> : icon && <Icon name={icon} />}
        <span className={s.lineText}>{status.text}</span>
      </div>
      <div className={s.line}>
        <span className={s.lineText}>
          {t("dashboard.readAt", { time: formatMessageTime(t.locale, reading.at, now) })}
        </span>
        {canRetry(readout) && <CardLink label={t("btn.retry")} onClick={() => retryHost(host.id)} />}
      </div>
    </>
  );
}

/** The readings' place on a card without readings: what stands in their way, its reason, and what to do. */
function CardState({
  icon,
  title,
  detail,
  tone,
  action,
}: {
  icon: string | null;
  title: string;
  detail?: string | null;
  tone: string;
  action?: ReactNode;
}) {
  return (
    <div className={s.state}>
      {icon ? <Icon name={icon} className={cx(s.stateIcon, s[tone])} /> : <Spinner size={16} />}
      <div className={cx(s.stateTitle, s[tone])} title={title}>
        {title}
      </div>
      {detail && (
        <div className={s.stateDetail} title={detail}>
          {detail}
        </div>
      )}
      {action}
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

/**
 * A host as a card: its readings while resource usage shows, else when it last connected. Every card has the same
 * parts at the same heights, whatever the host's state, so the cards of a row line up.
 */
export const HostCard = memo(function HostCard({ host, selected, readout, leftOut, list, view }: ItemProps) {
  const { t, now, interval, alerts, stats } = view;
  const read = stats && !leftOut;
  const reading = read ? latestReading(readout) : undefined;
  const stale = read && isStale(readout, now, interval);
  const target = `${host.username ? `${host.username}@` : ""}${host.address}:${host.port}`;
  const body = () => {
    if (leftOut) {
      const include = dashboardSwitch(host, view.groups);
      return (
        <CardState
          icon="eye-slash"
          title={t("dashboard.leftOutBadge")}
          tone="dim"
          action={<CardLink label={t(include.key)} onClick={include.run} />}
        />
      );
    }
    if (reading) {
      return (
        <>
          <div className={cx(stale && s.stale)}>
            <Meters reading={reading} alerts={alerts} t={t} />
          </div>
          {stale ? (
            <StaleLines host={host} readout={readout} reading={reading} t={t} now={now} />
          ) : (
            <DetailLines reading={reading} t={t} />
          )}
        </>
      );
    }
    const status = statusParts(t, readout);
    const retry = canRetry(readout) ? (
      <CardLink label={t("btn.retry")} onClick={() => retryHost(host.id)} />
    ) : undefined;
    return <CardState {...status} action={retry} />;
  };
  return (
    <Card
      interactive
      role="option"
      aria-selected={selected}
      data-id={host.id}
      className={cx(s.card, selected && s.cardSel, stats && leftOut && s.cardLeftOut)}
      onClick={(e) => view.onSelect(e, host.id)}
      onDoubleClick={() => view.onConnect(host.id)}
      onContextMenu={(e) => view.onContext(e, host)}
    >
      <div className={s.cardHead}>
        <OsIcon os={(stats && readout?.os) || host.os} badge={stats ? statusColor(readout, !!leftOut) : list.dot} />
        <span className={s.name}>{host.name}</span>
        {host.favorite && <Icon name="star" fill className={s.fav} />}
        <ConnectButton id={host.id} view={view} shown={read && readout?.phase === "held"} className={s.connect} />
      </div>
      <div className={s.target}>{target}</div>
      {view.tagRow && (
        <div className={s.tags} title={host.tags.join(", ") || undefined}>
          {host.tags.map((tag) => (
            <Tag key={tag} name={tag} />
          ))}
        </div>
      )}
      {stats ? (
        <div className={s.body}>{body()}</div>
      ) : (
        <div className={cx(s.cardLast, list.failed && s.error)}>{list.last}</div>
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
  return {
    gridTemplateColumns: [...widths.map(([least, most]) => `minmax(${least}px, ${most})`), `${ACTION_WIDTH}px`].join(
      " ",
    ),
  };
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

/** The table's head, with **Columns** in the place of Connect, so turning resource usage on moves nothing above the table. */
export function StatsHead({
  columns,
  t,
  measure,
  columnsOpen,
  onColumns,
}: {
  columns: DashboardColumn[];
  t: T;
  measure: RefCallback<HTMLElement>;
  columnsOpen: boolean;
  onColumns(anchor: HTMLElement): void;
}) {
  return (
    <div ref={measure} className={cx(s.tableHead, s.grid)} style={gridOf(columns)}>
      <span className={s.headName}>{t("dashboard.col.name")}</span>
      {columns.map((c) => (
        <span key={c}>{t(`dashboard.col.${c}` as MessageKey)}</span>
      ))}
      <span className={s.action}>
        <IconButton
          small
          icon="columns"
          label={t("dashboard.columns")}
          aria-haspopup="menu"
          aria-expanded={columnsOpen}
          onClick={(e) => onColumns(e.currentTarget)}
        />
      </span>
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

/** A host's row with its readings, a column per value. */
export const StatsRow = memo(function StatsRow({
  host,
  selected,
  readout,
  leftOut,
  columns,
  view,
}: Omit<ItemProps, "list"> & { columns: DashboardColumn[] }) {
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
            <RetryIcon host={host} readout={readout} t={t} />
          </span>
        );
      case "cpu":
        return (
          <MiniMeter
            percent={reading?.cpu_percent ?? null}
            alert={pastThreshold(reading?.cpu_percent ?? null, alerts.cpu)}
          />
        );
      case "memory": {
        const p = memPercent(reading);
        return <MiniMeter percent={p} alert={pastThreshold(p, alerts.mem)} />;
      }
      case "disk": {
        const disk = shownDisk(reading, alerts.disk);
        return (
          <MiniMeter
            percent={disk.percent}
            alert={pastThreshold(disk.percent, alerts.disk)}
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
      onClick={(e) => view.onSelect(e, host.id)}
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
        <ConnectButton id={host.id} view={view} shown={!leftOut && readout?.phase === "held"} className={s.connect} />
      </span>
    </div>
  );
});
