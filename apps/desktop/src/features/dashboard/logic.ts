import { percentOf, type StatsReading } from "@/features/terminal/stats";
import type { GroupView, HostView, MountView } from "@/ipc/types";
import type { HostPhase, HostReadout } from "./readings";

/** How the dashboard orders hosts within a group (HOST-14). `list` keeps the host list's order. */
export type DashboardSort = "list" | "name" | "status" | "cpu" | "memory" | "disk" | "network" | "uptime";
export const DASHBOARD_SORTS: DashboardSort[] = ["list", "name", "status", "cpu", "memory", "disk", "network", "uptime"];

/** The table's columns that can be turned off; the name always shows. */
export type DashboardColumn = "status" | "cpu" | "memory" | "disk" | "network" | "load" | "uptime";
export const DASHBOARD_COLUMNS: DashboardColumn[] = ["status", "cpu", "memory", "disk", "network", "load", "uptime"];

/** What left a host out of the dashboard on this device: its own switch, its group's, or nothing. */
export function leftOutBy(host: HostView, groups: ReadonlyMap<string, GroupView>): "host" | "group" | null {
  if (!host.dashboard) return "host";
  return host.group_id !== null && groups.get(host.group_id)?.dashboard === false ? "group" : null;
}

/** Whether the dashboard shows a host: neither it nor its group was left out on this device. */
export function shownOnDashboard(host: HostView, groups: ReadonlyMap<string, GroupView>): boolean {
  return leftOutBy(host, groups) === null;
}

export interface DashboardSection {
  /** null for the hosts without a group. */
  group: GroupView | null;
  hosts: HostView[];
}

/** A section per group, in the sidebar's order, then the hosts without a group (or in a group that is gone). */
export function groupSections(hosts: HostView[], groups: GroupView[]): DashboardSection[] {
  const known = new Set(groups.map((g) => g.id));
  const sections: DashboardSection[] = groups
    .map((group) => ({ group, hosts: hosts.filter((h) => h.group_id === group.id) }))
    .filter((s) => s.hosts.length > 0);
  const rest = hosts.filter((h) => h.group_id === null || !known.has(h.group_id));
  if (rest.length > 0) sections.push({ group: null, hosts: rest });
  return sections;
}

export function latestReading(readout: HostReadout | undefined): StatsReading | undefined {
  const history = readout?.history;
  return history && history.length > 0 ? history[history.length - 1] : undefined;
}

export function memPercent(r: StatsReading | undefined): number | null {
  return r ? percentOf(r.mem_used, r.mem_total) : null;
}

/** A filesystem's use as `df` counts it: blocks reserved for root are neither used nor available. */
export function mountPercent(m: Pick<MountView, "used" | "available">): number | null {
  return percentOf(m.used, m.used + m.available);
}

/** The root filesystem's use. */
export function rootPercent(r: StatsReading | undefined): number | null {
  if (!r || r.disk_used == null || r.disk_available == null) return null;
  return mountPercent({ used: r.disk_used, available: r.disk_available });
}

/** The fullest filesystem, the root filesystem included. */
export function fullestMount(r: StatsReading | undefined): { mount: string; percent: number } | null {
  if (!r) return null;
  let best: { mount: string; percent: number } | null = null;
  const root = rootPercent(r);
  if (root != null) best = { mount: "/", percent: root };
  for (const m of r.mounts) {
    const p = mountPercent(m);
    if (p != null && (best === null || p > best.percent)) best = { mount: m.mount_point, percent: p };
  }
  return best;
}

/** Receive and send rates together, in bytes per second. */
export function netRate(r: StatsReading | undefined): number | null {
  if (!r || (r.net_rx_rate == null && r.net_tx_rate == null)) return null;
  return (r.net_rx_rate ?? 0) + (r.net_tx_rate ?? 0);
}

/** A value over the threshold stands out; a threshold of 0 is off. */
export function pastThreshold(percent: number | null, threshold: number): boolean {
  return threshold > 0 && percent != null && percent > threshold;
}

/**
 * How long readings can be apart and still count as one run: three intervals, and room for the filesystem list,
 * which a network filesystem that stopped answering can hold up for 5 s every fifth reading.
 */
export function readingGapMs(intervalSecs: number): number {
  return intervalSecs * 3000 + 8000;
}

/** The readings are old: the host stopped answering, or none came for longer than {@link readingGapMs}. */
export function isStale(readout: HostReadout | undefined, now: number, intervalSecs: number): boolean {
  const last = latestReading(readout);
  if (!readout || !last) return false;
  return readout.phase !== "reading" || now - last.at > readingGapMs(intervalSecs);
}

/** Problems first when sorting by status. */
const STATUS_RANK: Record<HostPhase, number> = {
  offline: 0,
  failed: 1,
  held: 2,
  connecting: 3,
  waiting: 4,
  unsupported: 5,
  reading: 6,
};

/**
 * Orders `hosts`, which come in the host list's order. Values sort highest first and hosts without one last;
 * ties keep the host list's order.
 */
export function sortForDashboard(
  hosts: HostView[],
  sort: DashboardSort,
  readouts: Record<string, HostReadout>,
  locale: string,
): HostView[] {
  if (sort === "list") return hosts;
  if (sort === "name") {
    const collator = new Intl.Collator(locale, { numeric: true, sensitivity: "base" });
    return [...hosts].sort((a, b) => collator.compare(a.name, b.name));
  }
  if (sort === "status") {
    const rank = (h: HostView) => STATUS_RANK[readouts[h.id]?.phase ?? "waiting"];
    return [...hosts].sort((a, b) => rank(a) - rank(b));
  }
  const value = (h: HostView): number | null => {
    const r = latestReading(readouts[h.id]);
    switch (sort) {
      case "cpu":
        return r?.cpu_percent ?? null;
      case "memory":
        return memPercent(r);
      case "disk":
        return fullestMount(r)?.percent ?? null;
      case "network":
        return netRate(r);
      case "uptime":
        return r?.uptime_secs ?? null;
    }
  };
  const keyed = hosts.map((h) => ({ h, v: value(h) }));
  keyed.sort((a, b) => {
    if (a.v == null || b.v == null) return a.v == null ? (b.v == null ? 0 : 1) : -1;
    return b.v - a.v;
  });
  return keyed.map((k) => k.h);
}
