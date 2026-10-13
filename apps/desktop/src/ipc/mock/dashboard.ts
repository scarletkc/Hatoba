import type { HatobaApi } from "../api";
import type { DashboardEvent, HostView } from "../types";
import { mockStats, PROBE, type StatsProfile } from "./data";

type DashboardApi = Pick<HatobaApi, "dashboard_watch" | "dashboard_unwatch" | "dashboard_retry">;

/** What the mock dashboard does with a host (HOST-14). Hosts not listed read as a small Linux server. */
type Scenario =
  | { kind: "read"; profile?: Partial<StatsProfile>; lostAfter?: number }
  | { kind: "sign_in_failed" }
  | { kind: "host_key_new" };

const SCENARIOS: Record<string, Scenario> = {
  "h-api-tokyo": { kind: "read", profile: { cpus: 8, memGib: 16, cpu: 34, mem: 0.58, uptimeDays: 41 } },
  "h-bastion": { kind: "read", profile: { cpus: 2, memGib: 2, cpu: 4, mem: 0.31, disks: [["/", 6, 20]], uptimeDays: 97 } },
  "h-db-osaka-01": {
    kind: "read",
    profile: {
      cpus: 16,
      memGib: 64,
      cpu: 22,
      mem: 0.81,
      disks: [
        ["/", 18, 50],
        ["/srv/pgdata", 930, 1000],
        ["/var/backups", 1210, 2000],
      ],
      uptimeDays: 203,
    },
  },
  "h-api-tokyo-02": { kind: "read", profile: { cpus: 8, memGib: 16, cpu: 93, mem: 0.71, uptimeDays: 41 } },
  // A connection that drops after a few readings: the card keeps them, dimmed, with their time.
  "h-db-osaka-02": { kind: "read", profile: { cpus: 16, memGib: 64, cpu: 12, mem: 0.44, uptimeDays: 2 }, lostAfter: 3 },
  "h-staging-worker": { kind: "sign_in_failed" },
  "h-pihole": { kind: "host_key_new" },
};

/** Systems the mock pretends some servers run, which have no `/proc` to read. */
const NO_PROC: Record<string, string> = { windows: "Windows", freebsd: "FreeBSD", netbsd: "NetBSD" };

/**
 * The dashboard's background readings for the browser mock (HOST-14): each watched host connects after a
 * short, staggered delay and reads every interval, as `SCENARIOS` says. **Retry** signs in to a host whose
 * sign-in failed, which then reads.
 */
export function createDashboardMock(ctx: { host(id: string): HostView | undefined; unlocked(): boolean }): DashboardApi & { stopAll(): void } {
  let watchId = 0;
  let current: { id: number; interval: number; emit: (e: DashboardEvent) => void } | null = null;
  const timers = new Map<string, ReturnType<typeof setTimeout>[]>();
  const retried = new Set<string>();
  const readings = new Map<string, number>();

  const stop = (hostId: string) => {
    for (const t of timers.get(hostId) ?? []) {
      clearTimeout(t);
      clearInterval(t);
    }
    timers.delete(hostId);
  };
  const stopAll = () => {
    for (const id of [...timers.keys()]) stop(id);
    current = null;
  };
  const later = (hostId: string, ms: number, fn: () => void) => {
    const list = timers.get(hostId) ?? [];
    list.push(setTimeout(fn, ms));
    timers.set(hostId, list);
  };

  const start = (hostId: string, order: number) => {
    const host = ctx.host(hostId);
    const watch = current;
    if (!host || !watch) return;
    // Like the backend, a later watch of the same host takes over its events.
    const emit = (e: DashboardEvent) => current?.emit(e);
    const online = PROBE[hostId] ?? true;
    const scenario: Scenario = retried.has(hostId) ? { kind: "read" } : (SCENARIOS[hostId] ?? { kind: "read" });
    // Staggered, as eight slots at a time would spread them.
    const at = 200 + order * 120;
    later(hostId, at, () => emit({ kind: "probe", host_id: hostId, online }));
    if (!online) return;
    const system = host.os ? NO_PROC[host.os] : undefined;
    if (system) {
      later(hostId, at, () => emit({ kind: "unsupported", host_id: hostId, system }));
      return;
    }
    if (host.auth_kind === "ask") {
      later(hostId, at, () => emit({ kind: "held", host_id: hostId, hold: "password", error: null }));
      return;
    }
    later(hostId, at + 10, () => emit({ kind: "connecting", host_id: hostId }));
    if (scenario.kind === "sign_in_failed") {
      later(hostId, at + 900, () =>
        emit({
          kind: "held",
          host_id: hostId,
          hold: "sign_in_failed",
          error: { code: "ssh", detail: "authentication failed: no method succeeded (tried publickey)", ssh_kind: "auth_failed" },
        }),
      );
      return;
    }
    if (scenario.kind === "host_key_new") {
      later(hostId, at + 600, () => emit({ kind: "held", host_id: hostId, hold: "host_key_new", error: null }));
      return;
    }
    const sample = mockStats({ ...scenario.profile, interval: watch.interval });
    later(hostId, at + 700, () => {
      emit({ kind: "reading", host_id: hostId, os: host.os, tab: false });
      let n = readings.get(hostId) ?? 0;
      const read = () => {
        if (scenario.lostAfter !== undefined && n >= scenario.lostAfter) {
          stop(hostId);
          emit({ kind: "failed", host_id: hostId, error: { code: "ssh", detail: "connection reset by peer", ssh_kind: "disconnected" } });
          return;
        }
        emit({ kind: "stats", host_id: hostId, stats: sample(n === 0) });
        readings.set(hostId, ++n);
      };
      read();
      const list = timers.get(hostId) ?? [];
      list.push(setInterval(read, watch.interval * 1000));
      timers.set(hostId, list);
    });
  };

  return {
    dashboard_watch: async (hostIds, intervalSecs, onEvent) => {
      if (!ctx.unlocked()) throw { code: "locked", detail: "vault is locked" };
      const interval = Math.min(60, Math.max(1, intervalSecs));
      const restart = current !== null && current.interval !== interval;
      const wanted = new Set(hostIds);
      for (const id of [...timers.keys()]) if (restart || !wanted.has(id)) stop(id);
      current = { id: ++watchId, interval, emit: onEvent };
      let order = 0;
      for (const id of wanted) if (!timers.has(id)) start(id, order++);
      return current.id;
    },
    dashboard_unwatch: async (id) => {
      if (current?.id === id) stopAll();
    },
    dashboard_retry: async (hostId) => {
      if (!current) return;
      retried.add(hostId);
      readings.delete(hostId);
      stop(hostId);
      start(hostId, 0);
    },
    stopAll,
  };
}
