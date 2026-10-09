import { useSyncExternalStore } from "react";
import { create } from "zustand";
import { formatBytes, translate, type Locale } from "@/i18n";
import { api } from "@/ipc/api";
import type { AppError, ServerStatsView } from "@/ipc/types";

/** How often a reading arrives (TERM-12). */
const STATS_INTERVAL_MS = 2000;
/** The time the popover's charts span, and so how long readings are kept. */
export const STATS_WINDOW_MS = 180_000;
/** Readings further apart than this (sampling paused while the tab was in the background) are not joined in the charts. */
export const STATS_GAP_MS = 3 * STATS_INTERVAL_MS;

export interface StatsReading extends ServerStatsView {
  /** When the reading arrived, Unix ms. */
  at: number;
}

export type StatsState =
  | { kind: "starting" }
  | { kind: "live" }
  | { kind: "unsupported"; system: string }
  | { kind: "failed"; error: AppError };

export interface SessionStats {
  state: StatsState;
  /** Oldest first, from the last {@link STATS_WINDOW_MS}. */
  history: StatsReading[];
}

const useStatsStore = create<{ bySession: Record<string, SessionStats> }>(() => ({ bySession: {} }));

/** Bumped by every start and stop, so events from a sampling that was replaced are dropped. */
const generations = new Map<string, number>();
/** The id of each session's running sampling, once its start has answered (`null` if it failed). */
const startIds = new Map<string, Promise<number | null>>();

function bump(sessionId: string): number {
  const gen = (generations.get(sessionId) ?? 0) + 1;
  generations.set(sessionId, gen);
  return gen;
}

function update(sessionId: string, fn: (s: SessionStats) => SessionStats) {
  useStatsStore.setState((st) => {
    const current = st.bySession[sessionId] ?? { state: { kind: "starting" }, history: [] };
    return { bySession: { ...st.bySession, [sessionId]: fn(current) } };
  });
}

/** The readings of the {@link STATS_WINDOW_MS} up to `now` (none from after it, should the clock have gone back). */
function recent(history: StatsReading[], now: number): StatsReading[] {
  return history.filter((r) => r.at <= now && r.at >= now - STATS_WINDOW_MS);
}

/** Adds a reading, dropping those it leaves out of the window. */
export function appendReading(history: StatsReading[], reading: StatsReading): StatsReading[] {
  return [...recent(history, reading.at), reading];
}

/** Starts sampling a session's resource usage. A server already found not to run Linux is not asked again. */
export function startStats(sessionId: string, now = Date.now()) {
  const current = useStatsStore.getState().bySession[sessionId];
  if (current?.state.kind === "unsupported") return;
  const gen = bump(sessionId);
  // Readings from before a pause stay while they are in the window; the charts show the pause as a gap.
  update(sessionId, () => ({ state: { kind: "starting" }, history: recent(current?.history ?? [], now) }));
  const live = () => generations.get(sessionId) === gen;
  const started = api
    .ssh_stats_start(sessionId, (event) => {
      if (!live()) return;
      switch (event.kind) {
        case "stats":
          update(sessionId, (s) => ({ state: { kind: "live" }, history: appendReading(s.history, { ...event.stats, at: Date.now() }) }));
          break;
        case "unsupported":
          update(sessionId, (s) => ({ ...s, state: { kind: "unsupported", system: event.system } }));
          break;
        case "ended":
          update(sessionId, (s) => ({ ...s, state: { kind: "failed", error: event.error } }));
          break;
      }
    })
    .catch((error: AppError) => {
      if (live()) update(sessionId, (s) => ({ ...s, state: { kind: "failed", error } }));
      return null;
    });
  startIds.set(sessionId, started);
}

/** Stops a session's sampling. A stop sent before its start has answered waits for it, so it stops that sampling and no later one. */
export function stopStats(sessionId: string) {
  bump(sessionId);
  const started = startIds.get(sessionId);
  startIds.delete(sessionId);
  void started?.then((id) => (id == null ? undefined : api.ssh_stats_stop(sessionId, id))).catch(() => {});
}

/** Forgets a session's readings once the session is gone. */
export function clearStats(sessionId: string) {
  generations.delete(sessionId);
  startIds.delete(sessionId);
  useStatsStore.setState((st) => {
    if (!(sessionId in st.bySession)) return st;
    const { [sessionId]: _gone, ...rest } = st.bySession;
    return { bySession: rest };
  });
}

export function getSessionStats(sessionId: string): SessionStats | undefined {
  return useStatsStore.getState().bySession[sessionId];
}

export function useSessionStats(sessionId: string | null): SessionStats | undefined {
  return useStatsStore((st) => (sessionId ? st.bySession[sessionId] : undefined));
}

function subscribeVisibility(cb: () => void) {
  document.addEventListener("visibilitychange", cb);
  return () => document.removeEventListener("visibilitychange", cb);
}

/** `false` while the window is minimized or hidden, when there is no point in sampling. */
export function useDocumentVisible(): boolean {
  return useSyncExternalStore(subscribeVisibility, () => document.visibilityState !== "hidden");
}

// ───────────────────────── Formatting ─────────────────────────

/** Share of `total` taken by `used`, 0–100; `null` without both. */
export function percentOf(used: number | null, total: number | null): number | null {
  if (used == null || total == null || total <= 0) return null;
  return Math.min(100, Math.max(0, (used / total) * 100));
}

/** `12%`; `<1%` for a share that rounds to zero but is not. */
export function formatPercent(p: number): string {
  if (p > 0 && p < 0.5) return "<1%";
  return `${Math.round(p)}%`;
}

export function formatRate(bytesPerSecond: number): string {
  return `${formatBytes(Math.round(bytesPerSecond))}/s`;
}

/** How full a meter reads: its fill turns orange at 80% and red at 95%. */
export function meterLevel(p: number): "normal" | "warning" | "critical" {
  if (p >= 95) return "critical";
  if (p >= 80) return "warning";
  return "normal";
}

/** `12 days 3 h`, `5 h 12 min` or `42 min`. */
export function formatUptime(locale: Locale, secs: number): string {
  const days = Math.floor(secs / 86_400);
  const hours = Math.floor((secs % 86_400) / 3600);
  const minutes = Math.floor((secs % 3600) / 60);
  if (days > 0) return translate(locale, "terminal.stats.uptimeDays", { n: days, h: hours });
  if (hours > 0) return translate(locale, "terminal.stats.uptimeHours", { n: hours, m: minutes });
  return translate(locale, "terminal.stats.uptimeMinutes", { n: minutes });
}

/** `0.52 0.40 0.31`, as `uptime` prints the load averages. */
export function formatLoad(load: ServerStatsView["load"]): string | null {
  if (!load || load.some((l) => l == null)) return null;
  return load.map((l) => (l as number).toFixed(2)).join(" ");
}
