import { create } from "zustand";
import { api } from "@/ipc/api";
import type { ForwardState, ForwardStatusEvent } from "@/ipc/types";

/** What one session knows about one saved forward (FWD-01..04). Forwards without an entry are stopped. */
export interface ForwardRun {
  state: ForwardState;
  /** Where it listens while it runs: on the server for a remote forward. */
  port: number | null;
  error: string | null;
  /** Ordering stamp, so a late `forwards_active` snapshot never overwrites a newer event. */
  seq: number;
}

export type ForwardRuns = Readonly<Record<string, ForwardRun>>;

const NONE: ForwardRuns = {};

/**
 * Forward state per backend session id. It is keyed by session (not by tab) because the backend
 * starts auto-start forwards right after connecting, so `ssh://forward` events can arrive before
 * the UI has been told the session id; they simply wait here until a tab picks the session up.
 */
const useStore = create<{ bySession: Record<string, ForwardRuns> }>(() => ({ bySession: {} }));

let seq = 0;
/** Sessions that ended: late events for them are dropped instead of resurrecting state. */
const ended = new Set<string>();

function put(sessionId: string, forwardId: string, run: Omit<ForwardRun, "seq">) {
  if (ended.has(sessionId)) return;
  useStore.setState((s) => ({
    bySession: {
      ...s.bySession,
      [sessionId]: { ...(s.bySession[sessionId] ?? NONE), [forwardId]: { ...run, seq: ++seq } },
    },
  }));
}

/** `ssh://forward` */
export function applyForwardEvent(ev: ForwardStatusEvent) {
  put(ev.session_id, ev.forward_id, {
    state: ev.state,
    port: ev.state === "running" ? ev.port : null,
    error: ev.state === "failed" ? ev.error : null,
  });
}

export const markForwardRunning = (sessionId: string, forwardId: string, port: number) =>
  put(sessionId, forwardId, { state: "running", port, error: null });

export const markForwardStopped = (sessionId: string, forwardId: string) =>
  put(sessionId, forwardId, { state: "stopped", port: null, error: null });

export const markForwardFailed = (sessionId: string, forwardId: string, error: string) =>
  put(sessionId, forwardId, { state: "failed", port: null, error });

/**
 * Initial state of a session that just connected: auto-start forwards may already be running and
 * their events may have fired before we knew the session id (or before the listener was attached).
 */
export async function syncActiveForwards(sessionId: string): Promise<void> {
  const before = seq;
  let active: [string, number][];
  try {
    active = await api.forwards_active(sessionId);
  } catch {
    return;
  }
  if (ended.has(sessionId)) return;
  useStore.setState((s) => {
    const runs: Record<string, ForwardRun> = { ...(s.bySession[sessionId] ?? NONE) };
    for (const [forwardId, port] of active) {
      if ((runs[forwardId]?.seq ?? 0) > before) continue; // an event newer than this request wins
      runs[forwardId] = { state: "running", port, error: null, seq: ++seq };
    }
    return { bySession: { ...s.bySession, [sessionId]: runs } };
  });
}

/** The session disconnected or its tab closed: the backend tears its forwards down with it. */
export function dropSessionForwards(sessionId: string) {
  ended.add(sessionId);
  if (ended.size > 64) ended.delete(ended.values().next().value!);
  useStore.setState((s) => {
    const { [sessionId]: _gone, ...rest } = s.bySession;
    return { bySession: rest };
  });
}

export const getForwardRuns = (sessionId: string): ForwardRuns => useStore.getState().bySession[sessionId] ?? NONE;

export function useForwardRuns(sessionId: string | null): ForwardRuns {
  return useStore((s) => (sessionId ? s.bySession[sessionId] : undefined) ?? NONE);
}

/** Every session's forwards, for a view that looks across a host's tabs. */
export function useAllForwardRuns(): Readonly<Record<string, ForwardRuns>> {
  return useStore((s) => s.bySession);
}

export const countRunning = (runs: ForwardRuns): number => Object.values(runs).filter((r) => r.state === "running").length;
