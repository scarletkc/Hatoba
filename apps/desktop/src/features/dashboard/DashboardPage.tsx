import { memo, useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type MouseEvent, type RefCallback } from "react";
import { useVaultData } from "@/app/data";
import { useApp, type HostFilter } from "@/app/store";
import { Button, Icon, IconButton, LinkButton, Segmented, Spinner } from "@/components/controls";
import { EmptyState, PageHeader, Tag } from "@/components/layout";
import { Menu, PopupSelect, useMenu, type MenuEntry } from "@/components/overlay";
import { OsIcon } from "@/features/hosts/OsIcon";
import { applyFilter, sortHosts } from "@/features/hosts/order";
import { usePrefs } from "@/features/settings/shared";
import { useHostsUi } from "@/features/hosts/ui";
import { formatLoad, formatPercent, formatRate, formatUptime, useDocumentVisible, type StatsReading } from "@/features/terminal/stats";
import { connectHost } from "@/features/terminal/connect";
import { formatBytes, formatMessageTime, useT, type MessageKey } from "@/i18n";
import type { GroupView, HostView, LocalPrefs } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { dashboardSwitch, openTerminal, retryHost, setShownOnDashboard, useDashboardUi } from "./actions";
import { HostDetail } from "./HostDetail";
import {
  COLUMN_WIDTHS,
  DASHBOARD_COLUMNS,
  DASHBOARD_SORTS,
  NAME_WIDTH,
  fitColumns,
  fullestMount,
  groupSections,
  isStale,
  latestReading,
  leftOutBy,
  memPercent,
  mountPercent,
  pastThreshold,
  rootPercent,
  shownOnDashboard,
  sortForDashboard,
  type DashboardColumn,
  type DashboardSort,
} from "./logic";
import { useDashboardWatch, useReadoutMap, type HostReadout } from "./readings";
import { statusColor, statusText } from "./status";
import s from "./DashboardPage.module.css";

type T = ReturnType<typeof useT>;

/** The thresholds past which a value stands out (a setting), in percent; 0 is off. */
interface Alerts {
  cpu: number;
  mem: number;
  disk: number;
}

/** What a card or row needs besides its host. */
interface View {
  t: T;
  now: number;
  interval: number;
  alerts: Alerts;
  groups: ReadonlyMap<string, GroupView>;
  onOpen(id: string): void;
  onContext(e: MouseEvent, host: HostView): void;
}

/** The filter's value in the page header's popup. */
function filterKey(f: HostFilter): string {
  switch (f.kind) {
    case "group":
      return `group:${f.id}`;
    case "tag":
      return `tag:${f.name}`;
    default:
      return f.kind;
  }
}

function filterOf(key: string): HostFilter {
  if (key.startsWith("group:")) return { kind: "group", id: key.slice(6) };
  if (key.startsWith("tag:")) return { kind: "tag", name: key.slice(4) };
  return key === "favorites" ? { kind: "favorites" } : { kind: "all" };
}

/** Re-renders every `ms`, for relative times and readings that grow old. */
function useNow(ms: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), ms);
    return () => window.clearInterval(timer);
  }, [ms]);
  return now;
}

/**
 * The dashboard (HOST-14): the online status and resource usage of every host the filter shows, as cards or a
 * table, a section per group. The hosts are read in the background while the page is shown and the window is
 * visible; the page unmounts behind the lock screen.
 */
export function DashboardPage({ filter }: { filter: HostFilter }) {
  const t = useT();
  const [prefs, savePrefs] = usePrefs();
  const navigate = useApp((st) => st.navigate);
  const hosts = useVaultData((st) => st.hosts);
  const groups = useVaultData((st) => st.groups);
  const tags = useVaultData((st) => st.tags);
  const listSort = useHostsUi((st) => st.sort);
  const { sort, setSort, showLeftOut, setShowLeftOut } = useDashboardUi();
  const readouts = useReadoutMap();
  const visible = useDocumentVisible();
  const now = useNow(5000);
  const [detail, setDetail] = useState<string | null>(null);
  const [ctxHost, setCtxHost] = useState<HostView | null>(null);
  const [ctxGroup, setCtxGroup] = useState<GroupView | null>(null);
  const hostMenu = useMenu();
  const groupMenu = useMenu();
  const sortMenu = useMenu();
  const columnsMenu = useMenu();
  const sortButton = useRef<HTMLButtonElement>(null);
  const columnsButton = useRef<HTMLButtonElement>(null);

  const interval = prefs.dashboard_interval_secs;
  const layout = prefs.dashboard_layout;
  const hidden = prefs.dashboard_hidden_columns;
  const { dashboard_cpu_alert: cpuAlert, dashboard_mem_alert: memAlert, dashboard_disk_alert: diskAlert } = prefs;
  const alerts: Alerts = useMemo(() => ({ cpu: cpuAlert, mem: memAlert, disk: diskAlert }), [cpuAlert, memAlert, diskAlert]);
  const [measureTable, tableWidth] = useWidth();
  const columns = useMemo(() => {
    const shownColumns = DASHBOARD_COLUMNS.filter((c) => !hidden.includes(c));
    return tableWidth === null ? shownColumns : fitColumns(shownColumns, tableWidth);
  }, [hidden, tableWidth]);

  const groupMap = useMemo(() => new Map(groups.map((g) => [g.id, g])), [groups]);
  // The host list's order, which "Host List Order" keeps and the other sorts break ties with.
  const inView = useMemo(() => sortHosts(applyFilter(hosts, filter), listSort, t.locale), [hosts, filter, listSort, t.locale]);
  const shown = useMemo(() => inView.filter((h) => shownOnDashboard(h, groupMap)), [inView, groupMap]);
  const leftOut = inView.length - shown.length;
  const shownIds = useMemo(() => shown.map((h) => h.id), [shown]);
  useDashboardWatch(shownIds, interval, visible);

  const listed = showLeftOut ? inView : shown;
  const sections = useMemo(
    () => groupSections(sortForDashboard(listed, sort, readouts, t.locale), groups),
    [listed, sort, readouts, t.locale, groups],
  );

  const title = t("dashboard.title");
  const filterOptions = [
    { value: "all", label: t("sidebar.all"), icon: "hard-drives" },
    { value: "favorites", label: t("sidebar.favorites"), icon: "star" },
    ...groups.map((g) => ({ value: `group:${g.id}`, label: g.name, icon: "folder-simple" })),
    ...tags.map((tag) => ({ value: `tag:${tag.name}`, label: tag.name, icon: "tag" })),
  ];
  const sortLabel = (k: DashboardSort) => t(`dashboard.sort.${k}` as MessageKey);
  const toggleColumn = (c: DashboardColumn) =>
    savePrefs({ dashboard_hidden_columns: hidden.includes(c) ? hidden.filter((x) => x !== c) : [...hidden, c] });

  const openHostMenu = hostMenu.openAt;
  const onContext = useCallback(
    (e: MouseEvent, host: HostView) => {
      e.preventDefault();
      setCtxHost(host);
      openHostMenu({ x: e.clientX, y: e.clientY });
    },
    [openHostMenu],
  );
  // Stable between readings, so only the cards and rows whose host changed render again.
  const view: View = useMemo(() => ({ t, now, interval, alerts, groups: groupMap, onOpen: setDetail, onContext }), [t, now, interval, alerts, groupMap, onContext]);

  const hostEntries = (host: HostView): MenuEntry[] => {
    const r = readouts[host.id];
    const included = shownOnDashboard(host, groupMap);
    const toggle = dashboardSwitch(host, groupMap);
    return [
      { label: t("dashboard.openTerminal"), icon: "terminal-window", onSelect: () => openTerminal(host.id) },
      {
        label: t("hosts.menu.edit"),
        icon: "pencil-simple",
        onSelect: () => navigate({ kind: "host-edit", hostId: host.id, groupId: null, back: filter, fromDashboard: true }),
      },
      ...((canRetry(r) || r?.phase === "offline") && included ? [{ label: t("btn.retry"), icon: "arrow-clockwise", onSelect: () => retryHost(host.id) }] : []),
      { kind: "separator" },
      { label: t(toggle.key), icon: toggle.icon, onSelect: toggle.run },
    ];
  };

  const header = (
    <PageHeader title={title} count={t("hosts.count", { n: shown.length })}>
      <PopupSelect<string>
        ariaLabel={t("dashboard.filter")}
        value={filterKey(filter)}
        options={filterOptions}
        minWidth={150}
        onChange={(key) => navigate({ kind: "dashboard", filter: filterOf(key) })}
      />
      <Button
        ref={sortButton}
        trailingIcon="caret-down"
        aria-haspopup="menu"
        aria-expanded={!!sortMenu.anchor}
        aria-label={t("dashboard.sort")}
        onClick={() => sortButton.current && sortMenu.openBelow(sortButton.current, true)}
      >
        {sortLabel(sort)}
      </Button>
      {layout === "table" && (
        <Button
          ref={columnsButton}
          icon="columns"
          className={s.columnsButton}
          title={t("dashboard.columns")}
          aria-label={t("dashboard.columns")}
          aria-haspopup="menu"
          aria-expanded={!!columnsMenu.anchor}
          onClick={() => columnsButton.current && columnsMenu.openBelow(columnsButton.current, true)}
        >
          <span>{t("dashboard.columns")}</span>
        </Button>
      )}
      <Segmented<LocalPrefs["dashboard_layout"]>
        ariaLabel={t("dashboard.layout")}
        className={s.layoutSwitch}
        value={layout}
        options={[
          { value: "cards", label: t("dashboard.layout.cards"), title: t("dashboard.layout.cards"), icon: "squares-four" },
          { value: "table", label: t("dashboard.layout.table"), title: t("dashboard.layout.table"), icon: "rows" },
        ]}
        onChange={(dashboard_layout) => savePrefs({ dashboard_layout })}
      />
    </PageHeader>
  );

  let body;
  if (hosts.length === 0) {
    body = (
      <EmptyState
        icon="gauge"
        title={t("dashboard.empty.title")}
        body={t("dashboard.empty.body")}
        actions={
          <Button variant="primary" icon="plus" onClick={() => navigate({ kind: "host-edit", hostId: null, groupId: null, back: filter, fromDashboard: true })}>
            {t("hosts.newHost")}
          </Button>
        }
      />
    );
  } else if (inView.length === 0) {
    body = (
      <EmptyState
        icon="gauge"
        title={t("dashboard.emptyFilter.title")}
        body={t("dashboard.emptyFilter.body")}
        actions={<Button onClick={() => navigate({ kind: "dashboard", filter: { kind: "all" } })}>{t("dashboard.emptyFilter.all")}</Button>}
      />
    );
  } else if (listed.length === 0) {
    body = (
      <EmptyState
        icon="eye-slash"
        title={t("dashboard.leftOut", { n: leftOut })}
        actions={<Button onClick={() => setShowLeftOut(true)}>{t("dashboard.showLeftOut")}</Button>}
      />
    );
  } else {
    body = (
      <div className={s.scroll}>
        {layout === "table" && <TableHead columns={columns} t={t} measure={measureTable} />}
        {sections.map((section) => (
          <section key={section.group?.id ?? "none"} className={s.section} aria-label={section.group?.name ?? t("dashboard.noGroup")}>
            <div className={s.sectionHead}>
              <Icon name="folder-simple" className={s.sectionIcon} />
              <span className={cx(s.sectionTitle, section.group && !section.group.dashboard && s.leftOutText)}>
                {section.group?.name ?? t("dashboard.noGroup")}
              </span>
              <span className={s.sectionCount}>{section.hosts.length}</span>
              {section.group && (
                <IconButton
                  icon="dots-three"
                  label={t("dashboard.groupMenu")}
                  onClick={(e) => {
                    setCtxGroup(section.group);
                    groupMenu.openBelow(e.currentTarget);
                  }}
                />
              )}
            </div>
            {layout === "cards" ? (
              <div className={s.cards}>
                {section.hosts.map((h) => (
                  <HostCard key={h.id} host={h} readout={readouts[h.id]} leftOut={leftOutBy(h, groupMap)} view={view} />
                ))}
              </div>
            ) : (
              <div className={s.rows} role="list">
                {section.hosts.map((h) => (
                  <HostRow key={h.id} host={h} readout={readouts[h.id]} leftOut={leftOutBy(h, groupMap)} columns={columns} view={view} />
                ))}
              </div>
            )}
          </section>
        ))}
        {leftOut > 0 && (
          <div className={s.leftOutNote}>
            <Icon name="eye-slash" />
            <span>{t("dashboard.leftOut", { n: leftOut })}</span>
            <LinkButton onClick={() => setShowLeftOut(!showLeftOut)}>
              {showLeftOut ? t("dashboard.hideLeftOut") : t("dashboard.showLeftOut")}
            </LinkButton>
          </div>
        )}
      </div>
    );
  }

  const detailHost = detail ? hosts.find((h) => h.id === detail) : undefined;
  return (
    <div className={s.page}>
      {header}
      {body}
      {sortMenu.anchor && (
        <Menu
          anchor={sortMenu.anchor}
          onClose={sortMenu.close}
          minWidth={160}
          entries={DASHBOARD_SORTS.map((k) => ({ label: sortLabel(k), checked: sort === k, onSelect: () => setSort(k) }))}
        />
      )}
      {columnsMenu.anchor && (
        <Menu
          anchor={columnsMenu.anchor}
          onClose={columnsMenu.close}
          minWidth={150}
          entries={DASHBOARD_COLUMNS.map((c) => ({
            label: t(`dashboard.col.${c}` as MessageKey),
            checked: !hidden.includes(c),
            onSelect: () => toggleColumn(c),
          }))}
        />
      )}
      {hostMenu.anchor && ctxHost && <Menu anchor={hostMenu.anchor} onClose={hostMenu.close} entries={hostEntries(ctxHost)} />}
      {groupMenu.anchor && ctxGroup && (
        <Menu
          anchor={groupMenu.anchor}
          onClose={groupMenu.close}
          entries={[
            ctxGroup.dashboard
              ? { label: t("dashboard.leaveOutGroup"), icon: "eye-slash", onSelect: () => void setShownOnDashboard(ctxGroup.id, false) }
              : { label: t("dashboard.includeGroup"), icon: "eye", onSelect: () => void setShownOnDashboard(ctxGroup.id, true) },
          ]}
        />
      )}
      {detailHost && (
        <HostDetail
          host={detailHost}
          readout={readouts[detailHost.id]}
          shown={shownOnDashboard(detailHost, groupMap)}
          groups={groupMap}
          interval={interval}
          alerts={alerts}
          onClose={() => setDetail(null)}
        />
      )}
    </div>
  );
}

// ───────────────────────── Status ─────────────────────────

/** Whether **Retry** applies: a failed host, or one held for anything but a password asked every time. */
function canRetry(readout: HostReadout | undefined): boolean {
  return readout?.phase === "failed" || (readout?.phase === "held" && readout.hold !== "password");
}

/** **Connect** for a host that needs the user, **Retry** for one that failed or is held; icons in a table row. */
function StatusActions({ host, readout, t, compact }: { host: HostView; readout: HostReadout | undefined; t: T; compact?: boolean }) {
  const held = readout?.phase === "held";
  const retry = canRetry(readout);
  if (!retry && !held) return null;
  return (
    <span className={s.actions} onClick={(e) => e.stopPropagation()}>
      {held &&
        (compact ? (
          <IconButton icon="plugs-connected" label={t("hosts.connect")} className={s.rowAction} onClick={() => void connectHost(host.id)} />
        ) : (
          <Button size="xs" variant="primary" onClick={() => void connectHost(host.id)}>
            {t("hosts.connect")}
          </Button>
        ))}
      {retry &&
        (compact ? (
          <IconButton icon="arrow-clockwise" label={t("btn.retry")} className={s.rowAction} onClick={() => retryHost(host.id)} />
        ) : (
          <Button size="xs" onClick={() => retryHost(host.id)}>
            {t("btn.retry")}
          </Button>
        ))}
    </span>
  );
}

/** The host's name as the button that opens its detail; the rest of the card or row opens it on a click too. */
function NameButton({ host, onOpen }: { host: HostView; onOpen(id: string): void }) {
  return (
    <button
      type="button"
      className={s.nameButton}
      onClick={(e) => {
        e.stopPropagation();
        onOpen(host.id);
      }}
    >
      {host.name}
    </button>
  );
}

/** **Include in Dashboard**, or **Include Group in Dashboard** for a host whose group is left out. */
function IncludeButton({ host, groups, t }: { host: HostView; groups: ReadonlyMap<string, GroupView>; t: T }) {
  const toggle = dashboardSwitch(host, groups);
  return (
    <span className={s.actions} onClick={(e) => e.stopPropagation()}>
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
  readout: HostReadout | undefined;
  /** What left the host out of the dashboard, if anything. */
  leftOut: "host" | "group" | null;
  view: View;
}

const HostCard = memo(function HostCard({ host, readout, leftOut, view }: ItemProps) {
  const { t, now, interval, alerts } = view;
  const reading = leftOut ? undefined : latestReading(readout);
  const stale = !leftOut && isStale(readout, now, interval);
  const status = statusLine(t, readout, reading, stale, now);
  const target = `${host.username ? `${host.username}@` : ""}${host.address}:${host.port}`;
  return (
    <div
      className={cx(s.card, leftOut && s.cardLeftOut)}
      onClick={() => view.onOpen(host.id)}
      onContextMenu={(e) => view.onContext(e, host)}
    >
      <div className={s.cardHead}>
        <OsIcon os={readout?.os ?? host.os} badge={statusColor(readout, !!leftOut)} />
        <NameButton host={host} onOpen={view.onOpen} />
        {host.favorite && <Icon name="star" fill className={s.fav} />}
      </div>
      <div className={s.target}>{target}</div>
      {host.tags.length > 0 && (
        <div className={s.tags}>
          {host.tags.map((tag) => (
            <Tag key={tag} name={tag} />
          ))}
        </div>
      )}
      {leftOut ? (
        <div className={s.status}>
          <span className={s.badge}>{t("dashboard.leftOutBadge")}</span>
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
    </div>
  );
});

// ───────────────────────── Table ─────────────────────────

/** The grid of the table's head and rows, the columns fitColumns left room for. */
function gridOf(columns: DashboardColumn[]): CSSProperties {
  const widths = [NAME_WIDTH, ...columns.map((c) => COLUMN_WIDTHS[c])];
  return { gridTemplateColumns: widths.map(([least, most]) => `minmax(${least}px, ${most})`).join(" ") };
}

/** The width of the element given the returned ref, kept current as it resizes; null before it mounts. */
function useWidth(): [RefCallback<HTMLElement>, number | null] {
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

function TableHead({ columns, t, measure }: { columns: DashboardColumn[]; t: T; measure: RefCallback<HTMLElement> }) {
  return (
    <div ref={measure} className={cx(s.tableHead, s.grid)} style={gridOf(columns)}>
      <span className={s.headName}>{t("dashboard.col.name")}</span>
      {columns.map((c) => (
        <span key={c}>{t(`dashboard.col.${c}` as MessageKey)}</span>
      ))}
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

const HostRow = memo(function HostRow({ host, readout, leftOut, columns, view }: ItemProps & { columns: DashboardColumn[] }) {
  const { t, now, interval, alerts } = view;
  const reading = leftOut ? undefined : latestReading(readout);
  const stale = !leftOut && isStale(readout, now, interval);
  const status = statusLine(t, readout, reading, stale, now);
  const cell = (c: DashboardColumn) => {
    switch (c) {
      case "status":
        if (leftOut) return <span className={s.badge}>{t("dashboard.leftOutBadge")}</span>;
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
      role="listitem"
      className={cx(s.row, s.grid, leftOut && s.cardLeftOut, stale && s.staleRow)}
      style={gridOf(columns)}
      onClick={() => view.onOpen(host.id)}
      onContextMenu={(e) => view.onContext(e, host)}
    >
      <span className={s.rowName}>
        <OsIcon os={readout?.os ?? host.os} badge={statusColor(readout, !!leftOut)} />
        <NameButton host={host} onOpen={view.onOpen} />
        {host.favorite && <Icon name="star" fill className={s.fav} />}
      </span>
      {columns.map((c) => (
        <span key={c} className={cx(s.cell, c !== "status" && s.valueCell)}>
          {cell(c)}
        </span>
      ))}
    </div>
  );
});
