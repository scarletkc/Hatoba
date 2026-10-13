import { useEffect, useRef } from "react";
import { create } from "zustand";
import { appendReading, type StatsReading } from "@/features/terminal/stats";
import { api } from "@/ipc/api";
import type { AppError, DashboardEvent, DashboardHold } from "@/ipc/types";

/** Where the dashboard stands with a host (HOST-14). */
export type HostPhase = "waiting" | "connecting" | "reading" | "held" | "failed" | "unsupported" | "offline";

export interface HostReadout {
  phase: HostPhase;
  /** Whether the host's first hop answered the last probe; null until probed. */
  online: boolean | null;
  /** The server's OS as the connection reported it (HOST-11). */
  os: string | null;
  /** Read over an open terminal tab's connection. */
  tab: boolean;
  hold: DashboardHold | null;
  error: AppError | null;
  /** The system of a server that does not run Linux; empty when unknown. */
  system: string | null;
  /** The readings of the last few minutes, oldest first. They stay when the host stops answering. */
  history: StatsReading[];
}

export const NO_READOUT: HostReadout = {
  phase: "waiting",
  online: null,
  os: null,
  tab: false,
  hold: null,
  error: null,
  system: null,
  history: [],
};

/** A host's readout after `event`, which arrived at `now`. */
export function applyEvent(prev: HostReadout | undefined, event: DashboardEvent, now: number): HostReadout {
  const r = prev ?? NO_READOUT;
  switch (event.kind) {
    case "probe":
      if (!event.online) return { ...r, online: false, phase: "offline" };
      return { ...r, online: true, phase: r.phase === "offline" ? "waiting" : r.phase };
    case "connecting":
      return { ...r, online: true, phase: "connecting", hold: null, error: null };
    case "reset":
      return { ...r, os: null, history: [] };
    case "reading":
      return { ...r, online: true, phase: "reading", os: event.os ?? r.os, tab: event.tab, hold: null, error: null, system: null };
    case "stats":
      return { ...r, online: true, phase: "reading", history: appendReading(r.history, { ...event.stats, at: now }) };
    case "unsupported":
      return { ...r, phase: "unsupported", system: event.system, hold: null, error: null };
    case "held":
      return { ...r, phase: "held", hold: event.hold, error: event.error };
    case "failed":
      return { ...r, phase: "failed", error: event.error };
  }
}

/** The readings stay in memory on this device; they are never stored or synced. */
const useReadouts = create<{ hosts: Record<string, HostReadout> }>(() => ({ hosts: {} }));

/** Events waiting for the next batch, so many hosts reading at once re-render the page once. */
let queued: [DashboardEvent, number][] = [];
let flushTimer: ReturnType<typeof setTimeout> | null = null;
const BATCH_MS = 50;

function flush() {
  flushTimer = null;
  const events = queued;
  queued = [];
  useReadouts.setState((st) => {
    const hosts = { ...st.hosts };
    for (const [event, at] of events) hosts[event.host_id] = applyEvent(hosts[event.host_id], event, at);
    return { hosts };
  });
}

export function dispatchDashboardEvent(event: DashboardEvent, now = Date.now()) {
  queued.push([event, now]);
  flushTimer ??= setTimeout(flush, BATCH_MS);
}

/** Forgets every reading, when the vault locks. */
export function clearReadouts() {
  generation++;
  queued = [];
  if (flushTimer !== null) clearTimeout(flushTimer);
  flushTimer = null;
  useReadouts.setState({ hosts: {} });
}

export function useReadoutMap(): Record<string, HostReadout> {
  return useReadouts((st) => st.hosts);
}

/** Bumped by every stop, so events of a watch that was stopped are dropped. */
let generation = 0;

/**
 * Reads `hostIds` every `intervalSecs` while `active`: the page is shown, the window is visible and the vault
 * unlocked (HOST-14). A change of hosts or interval replaces the watch, so the hosts both watches read keep
 * their connections; going inactive stops it, and the backend closes its connections a minute later.
 */
export function useDashboardWatch(hostIds: string[], intervalSecs: number, active: boolean) {
  const latest = useRef<Promise<number | null> | null>(null);
  // In a fixed order, so a reload that reorders the same hosts keeps the watch.
  const key = [...hostIds].sort().join("\n");

  useEffect(() => {
    if (!active) return;
    const gen = generation;
    const ids = key ? key.split("\n") : [];
    const started = api
      .dashboard_watch(ids, intervalSecs, (event) => {
        if (gen === generation) dispatchDashboardEvent(event);
      })
      .catch(() => null);
    latest.current = started;
  }, [key, intervalSecs, active]);

  useEffect(() => {
    if (!active) return;
    return () => {
      generation++;
      const started = latest.current;
      latest.current = null;
      void started?.then((id) => (id == null ? undefined : api.dashboard_unwatch(id))).catch(() => {});
    };
  }, [active]);
}
