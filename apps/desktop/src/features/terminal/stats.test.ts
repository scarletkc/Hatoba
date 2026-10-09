import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppError, ServerStatsView, StatsEvent } from "@/ipc/types";

const ipc = vi.hoisted(() => ({
  starts: [] as { sessionId: string; emit: (event: StatsEvent) => void; answer: () => void }[],
  stops: [] as [string, number][],
  reject: null as AppError | null,
  /** Starts answer only when `answer` is called. */
  hold: false,
  lastId: 0,
}));

vi.mock("@/ipc/api", () => ({
  api: {
    ssh_stats_start: async (sessionId: string, onEvent: (event: StatsEvent) => void) => {
      const id = ++ipc.lastId;
      let answer = () => {};
      const answered = new Promise<void>((resolve) => (answer = resolve));
      ipc.starts.push({ sessionId, emit: onEvent, answer });
      if (ipc.hold) await answered;
      if (ipc.reject) throw ipc.reject;
      return id;
    },
    ssh_stats_stop: async (sessionId: string, statsId: number) => {
      ipc.stops.push([sessionId, statsId]);
    },
  },
}));

import {
  STATS_HISTORY,
  clearStats,
  formatLoad,
  formatPercent,
  formatRate,
  formatUptime,
  getSessionStats,
  meterLevel,
  percentOf,
  startStats,
  stopStats,
} from "./stats";

const reading = (cpu: number | null): ServerStatsView => ({
  cpu_percent: cpu,
  cpus: 4,
  load: [0.5, 0.4, 0.3],
  mem_total: 8,
  mem_used: 4,
  swap_total: 0,
  swap_used: 0,
  net_rx_rate: null,
  net_tx_rate: null,
  net_interfaces: ["eth0"],
  disk_total: 10,
  disk_used: 4,
  disk_available: 5,
  uptime_secs: 100,
});

const lastStart = () => ipc.starts[ipc.starts.length - 1];
const flush = () => new Promise((r) => setTimeout(r, 0));

let n = 0;
let sid: string;
beforeEach(() => {
  sid = `s-stats-${++n}`;
  ipc.starts = [];
  ipc.stops = [];
  ipc.reject = null;
  ipc.hold = false;
});

describe("resource usage sampling (TERM-12)", () => {
  it("collects readings and keeps the last few minutes", () => {
    startStats(sid);
    expect(getSessionStats(sid)?.state.kind).toBe("starting");
    const { emit } = lastStart();
    for (let i = 0; i < STATS_HISTORY + 5; i++) emit({ kind: "stats", stats: reading(i) });
    const stats = getSessionStats(sid)!;
    expect(stats.state.kind).toBe("live");
    expect(stats.history).toHaveLength(STATS_HISTORY);
    expect(stats.history[0].cpu_percent).toBe(5);
    expect(stats.history.at(-1)?.cpu_percent).toBe(STATS_HISTORY + 4);
  });

  it("drops what a stopped or replaced sampling still sends", async () => {
    startStats(sid);
    const first = lastStart().emit;
    first({ kind: "stats", stats: reading(10) });
    stopStats(sid);
    await flush();
    expect(ipc.stops).toEqual([[sid, ipc.lastId]]);
    first({ kind: "stats", stats: reading(99) });
    expect(getSessionStats(sid)?.history.map((r) => r.cpu_percent)).toEqual([10]);

    startStats(sid);
    const second = lastStart().emit;
    first({ kind: "ended", error: { code: "ssh", detail: "old" } });
    second({ kind: "stats", stats: reading(20) });
    expect(getSessionStats(sid)?.state.kind).toBe("live");
  });

  it("stops a sampling whose start has not answered yet once it does, by its id", async () => {
    ipc.hold = true;
    startStats(sid);
    const first = ipc.lastId;
    stopStats(sid);
    startStats(sid);
    await flush();
    expect(ipc.stops).toEqual([]);
    ipc.starts.forEach((s) => s.answer());
    await flush();
    // Only the first start is stopped; the one sent after it keeps running.
    expect(ipc.stops).toEqual([[sid, first]]);
  });

  it("sends no stop for a start that failed", async () => {
    ipc.reject = { code: "not_found", detail: "session" };
    startStats(sid);
    stopStats(sid);
    await flush();
    expect(ipc.stops).toEqual([]);
  });

  it("keeps the charts across a short pause and starts them over after a long one", () => {
    startStats(sid);
    lastStart().emit({ kind: "stats", stats: reading(10) });
    stopStats(sid);
    startStats(sid, Date.now() + 2_000);
    expect(getSessionStats(sid)?.history).toHaveLength(1);
    stopStats(sid);
    startStats(sid, Date.now() + 60_000);
    expect(getSessionStats(sid)?.history).toHaveLength(0);
  });

  it("asks a server that is not Linux only once", () => {
    startStats(sid);
    lastStart().emit({ kind: "unsupported", system: "FreeBSD" });
    expect(getSessionStats(sid)?.state).toEqual({ kind: "unsupported", system: "FreeBSD" });
    stopStats(sid);
    startStats(sid);
    expect(ipc.starts).toHaveLength(1);
  });

  it("reports a failure, and a retry starts again", async () => {
    startStats(sid);
    lastStart().emit({ kind: "stats", stats: reading(10) });
    lastStart().emit({ kind: "ended", error: { code: "ssh", detail: "exit status 2", ssh_kind: "other" } });
    const stats = getSessionStats(sid)!;
    expect(stats.state).toMatchObject({ kind: "failed", error: { detail: "exit status 2" } });
    // The readings so far stay on screen.
    expect(stats.history).toHaveLength(1);

    ipc.reject = { code: "not_found", detail: "session" };
    startStats(sid);
    await flush();
    expect(getSessionStats(sid)?.state).toMatchObject({ kind: "failed", error: { code: "not_found" } });
  });

  it("forgets a session that is gone", () => {
    startStats(sid);
    lastStart().emit({ kind: "stats", stats: reading(10) });
    clearStats(sid);
    expect(getSessionStats(sid)).toBeUndefined();
  });
});

describe("formatting", () => {
  it("shows shares and meters", () => {
    expect(percentOf(1, 4)).toBe(25);
    expect(percentOf(5, 4)).toBe(100);
    expect(percentOf(1, 0)).toBeNull();
    expect(percentOf(null, 4)).toBeNull();
    expect(formatPercent(0)).toBe("0%");
    expect(formatPercent(0.2)).toBe("<1%");
    expect(formatPercent(41.6)).toBe("42%");
    expect(meterLevel(79.9)).toBe("normal");
    expect(meterLevel(80)).toBe("warning");
    expect(meterLevel(95)).toBe("critical");
  });

  it("shows rates, load and uptime", () => {
    expect(formatRate(512)).toBe("512 B/s");
    expect(formatRate(1.5 * 1024 * 1024)).toBe("1.5 MB/s");
    expect(formatLoad([0.5, 0.123, 2])).toBe("0.50 0.12 2.00");
    expect(formatLoad([0.5, null, 2])).toBeNull();
    expect(formatLoad(null)).toBeNull();
    expect(formatUptime("en", 42 * 60 + 5)).toBe("42 min");
    expect(formatUptime("en", 5 * 3600 + 12 * 60)).toBe("5 h 12 min");
    expect(formatUptime("en", 86_400 + 3 * 3600)).toBe("1 day 3 h");
    expect(formatUptime("en", 12 * 86_400)).toBe("12 days 0 h");
    expect(formatUptime("zh-CN", 12 * 86_400 + 3 * 3600)).toBe("12 天 3 小时");
  });
});
