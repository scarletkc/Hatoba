import { describe, expect, it } from "vitest";
import type { StatsReading } from "@/features/terminal/stats";
import type { DashboardEvent, GroupView, HostView, ServerStatsView } from "@/ipc/types";
import {
  DASHBOARD_COLUMNS,
  fitColumns,
  fullestMount,
  groupSections,
  isStale,
  leftOutBy,
  netRate,
  pastThreshold,
  rootPercent,
  shownOnDashboard,
  sortForDashboard,
} from "./logic";
import { applyEvent, NO_READOUT, type HostReadout } from "./readings";

const GIB = 1024 ** 3;

const stats = (patch: Partial<ServerStatsView> = {}): ServerStatsView => ({
  cpu_percent: 10,
  cpus: 4,
  load: [0.5, 0.4, 0.3],
  mem_total: 8 * GIB,
  mem_used: 2 * GIB,
  swap_total: 0,
  swap_used: 0,
  net_rx_rate: 100,
  net_tx_rate: 50,
  net_interfaces: ["eth0"],
  disk_total: 100 * GIB,
  disk_used: 40 * GIB,
  disk_available: 55 * GIB,
  uptime_secs: 3600,
  net_rx_total: 1000,
  net_tx_total: 500,
  mounts: [],
  ...patch,
});

const reading = (at: number, patch: Partial<ServerStatsView> = {}): StatsReading => ({ ...stats(patch), at });

const host = (id: string, patch: Partial<HostView> = {}): HostView => ({
  id,
  name: id,
  address: `${id}.example`,
  port: 22,
  username: "ops",
  auth_kind: "key",
  has_password: false,
  key_id: null,
  group_id: null,
  tags: [],
  favorite: false,
  jump_host_id: null,
  proxy_mode: "device_default",
  proxy_id: null,
  note: "",
  ai_notes: "",
  updated_at: 0,
  last_connected_at: null,
  os: null,
  show_stats: false,
  dashboard: true,
  env: [],
  ...patch,
});

const group = (id: string, sort: number, dashboard = true): GroupView => ({ id, name: id, parent_id: null, sort, dashboard });

const reads = (...readings: StatsReading[]): HostReadout => ({ ...NO_READOUT, phase: "reading", online: true, history: readings });

describe("dashboard events (HOST-14)", () => {
  const T0 = Date.UTC(2026, 9, 12, 12);
  const apply = (events: DashboardEvent[], start?: HostReadout) =>
    events.reduce<HostReadout | undefined>((r, e, i) => applyEvent(r, e, T0 + i * 3000), start);

  it("follows a host from the probe to its readings", () => {
    const r = apply([
      { kind: "probe", host_id: "h", online: true },
      { kind: "connecting", host_id: "h" },
      { kind: "reading", host_id: "h", os: "ubuntu", tab: false },
      { kind: "stats", host_id: "h", stats: stats({ cpu_percent: null }) },
      { kind: "stats", host_id: "h", stats: stats({ cpu_percent: 42 }) },
    ])!;
    expect(r.phase).toBe("reading");
    expect(r.online).toBe(true);
    expect(r.os).toBe("ubuntu");
    expect(r.history.map((x) => x.cpu_percent)).toEqual([null, 42]);
    expect(r.history[1].at).toBe(T0 + 4 * 3000);
  });

  it("keeps the readings when the host stops answering", () => {
    const r = apply([
      { kind: "stats", host_id: "h", stats: stats() },
      { kind: "failed", host_id: "h", error: { code: "ssh", detail: "reset", ssh_kind: "disconnected" } },
      { kind: "probe", host_id: "h", online: false },
    ])!;
    expect(r.phase).toBe("offline");
    expect(r.online).toBe(false);
    expect(r.history).toHaveLength(1);
    // Answering again: waiting for the next step, the readings still there.
    const back = applyEvent(r, { kind: "probe", host_id: "h", online: true }, T0 + 60_000);
    expect(back.phase).toBe("waiting");
    expect(back.history).toHaveLength(1);
  });

  it("drops the readings of a host whose connection settings changed", () => {
    const r = apply([
      { kind: "reading", host_id: "h", os: "ubuntu", tab: false },
      { kind: "stats", host_id: "h", stats: stats() },
      { kind: "reset", host_id: "h" },
    ])!;
    expect(r.history).toHaveLength(0);
    expect(r.os).toBeNull();
  });

  it("holds a host that needs the user and clears the hold once it reads", () => {
    const held = apply([
      { kind: "probe", host_id: "h", online: true },
      { kind: "held", host_id: "h", hold: "sign_in_failed", error: { code: "ssh", detail: "denied", ssh_kind: "auth_failed" } },
    ])!;
    expect(held.phase).toBe("held");
    expect(held.hold).toBe("sign_in_failed");
    const reading = applyEvent(held, { kind: "reading", host_id: "h", os: null, tab: true }, T0);
    expect(reading).toMatchObject({ phase: "reading", hold: null, error: null, tab: true });
  });

  it("drops readings older than the charts' window", () => {
    let r: HostReadout | undefined;
    for (let i = 0; i < 100; i++) r = applyEvent(r, { kind: "stats", host_id: "h", stats: stats() }, T0 + i * 3000);
    // 3 minutes of readings every 3 s, both ends included.
    expect(r!.history).toHaveLength(61);
  });
});

describe("dashboard view", () => {
  it("leaves out hosts and groups turned off on this device", () => {
    const groups = new Map([
      ["on", group("on", 0)],
      ["off", group("off", 1, false)],
    ]);
    expect(shownOnDashboard(host("a"), groups)).toBe(true);
    expect(shownOnDashboard(host("b", { dashboard: false }), groups)).toBe(false);
    expect(shownOnDashboard(host("c", { group_id: "on" }), groups)).toBe(true);
    expect(shownOnDashboard(host("d", { group_id: "off" }), groups)).toBe(false);
    // A group that is gone does not hide its former hosts.
    expect(shownOnDashboard(host("e", { group_id: "gone" }), groups)).toBe(true);
    // Including a host whose group is left out is the group's switch, not the host's.
    expect(leftOutBy(host("d", { group_id: "off" }), groups)).toBe("group");
    expect(leftOutBy(host("f", { group_id: "off", dashboard: false }), groups)).toBe("host");
    expect(leftOutBy(host("c", { group_id: "on" }), groups)).toBeNull();
  });

  it("puts each group's hosts in a section, in the sidebar's order, then the rest", () => {
    const hosts = [host("a", { group_id: "g2" }), host("b"), host("c", { group_id: "g1" }), host("d", { group_id: "gone" }), host("e", { group_id: "g2" })];
    const sections = groupSections(hosts, [group("g1", 0), group("g2", 1), group("empty", 2)]);
    expect(sections.map((s) => [s.group?.id ?? null, s.hosts.map((h) => h.id)])).toEqual([
      ["g1", ["c"]],
      ["g2", ["a", "e"]],
      [null, ["b", "d"]],
    ]);
  });

  it("sorts by a value, highest first, hosts without one last and ties in list order", () => {
    const hosts = [host("a"), host("b"), host("c"), host("d")];
    const readouts: Record<string, HostReadout> = {
      a: reads(reading(1, { cpu_percent: 20 })),
      b: reads(reading(1, { cpu_percent: 80 })),
      c: { ...NO_READOUT, phase: "offline", online: false },
      d: reads(reading(1, { cpu_percent: 20 })),
    };
    const order = (sort: Parameters<typeof sortForDashboard>[1]) => sortForDashboard(hosts, sort, readouts, "en").map((h) => h.id);
    expect(order("cpu")).toEqual(["b", "a", "d", "c"]);
    expect(order("list")).toEqual(["a", "b", "c", "d"]);
    // Problems first.
    expect(order("status")[0]).toBe("c");
  });

  it("finds the fullest filesystem and counts use as df does", () => {
    const r = reading(1, {
      mounts: [
        { source: "/dev/sda1", mount_point: "/", total: 100 * GIB, used: 40 * GIB, available: 55 * GIB },
        { source: "/dev/sdb1", mount_point: "/data", total: 100 * GIB, used: 93 * GIB, available: 2 * GIB },
      ],
    });
    // 40 used of the 95 that are used or available.
    expect(rootPercent(r)).toBeCloseTo((40 / 95) * 100);
    expect(fullestMount(r)?.mount).toBe("/data");
    expect(fullestMount(r)?.percent).toBeCloseTo((93 / 95) * 100);
    expect(fullestMount(undefined)).toBeNull();
    expect(netRate(r)).toBe(150);
    expect(netRate(reading(1, { net_rx_rate: null, net_tx_rate: null }))).toBeNull();
  });

  it("applies the thresholds, with 0 turning one off", () => {
    expect(pastThreshold(91, 90)).toBe(true);
    expect(pastThreshold(90.1, 90)).toBe(true);
    expect(pastThreshold(90, 90)).toBe(false);
    expect(pastThreshold(100, 0)).toBe(false);
    expect(pastThreshold(null, 90)).toBe(false);
  });

  it("dims readings once three intervals and the filesystems' 8 s pass without one, or the host stops answering", () => {
    const at = 1_000_000;
    const live = reads(reading(at));
    expect(isStale(live, at + 12_000, 3)).toBe(false);
    expect(isStale(live, at + 18_000, 3)).toBe(true);
    expect(isStale({ ...live, phase: "failed" }, at + 1_000, 3)).toBe(true);
    expect(isStale({ ...NO_READOUT, phase: "offline" }, at, 3)).toBe(false);
  });

  it("drops the table's last columns that a narrow row has no room for", () => {
    expect(fitColumns(DASHBOARD_COLUMNS, 884)).toEqual(DASHBOARD_COLUMNS);
    expect(fitColumns(DASHBOARD_COLUMNS, 883)).toEqual(["status", "cpu", "memory", "disk", "network", "load"]);
    // Beside the widest AI panel, and in the smallest window.
    expect(fitColumns(DASHBOARD_COLUMNS, 422)).toEqual(["status", "cpu", "memory"]);
    expect(fitColumns(DASHBOARD_COLUMNS, 274)).toEqual(["status"]);
    expect(fitColumns(DASHBOARD_COLUMNS, 200)).toEqual([]);
    // Only the shown columns count, and none after the first that does not fit.
    expect(fitColumns(["cpu", "uptime"], 342)).toEqual(["cpu", "uptime"]);
    expect(fitColumns(["network", "cpu"], 300)).toEqual([]);
  });
});
