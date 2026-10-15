import { useEffect } from "react";
import { create } from "zustand";
import { api, toAppError } from "@/ipc/api";
import type { AppError, UpdateCheck, UpdateProgress } from "@/ipc/types";
import { useApp } from "./store";

/** A running Hatoba checks again this long after the last check that succeeded. */
const RECHECK_MS = 24 * 60 * 60 * 1000;
/** How often the automatic check looks at the clock, so a failed check is tried again this soon. */
const TICK_MS = 60 * 60 * 1000;

interface UpdateState {
  checking: boolean;
  /** The last check that succeeded. A later failure keeps it, so the sidebar dot stays. */
  result: UpdateCheck | null;
  /** When `result` arrived, in milliseconds since the epoch. */
  checkedAt: number | null;
  /** Why the latest check failed; cleared when the next one starts. */
  error: AppError | null;
  /** The install in progress: `starting` until the first progress arrives, null when none runs. */
  install: UpdateProgress | { kind: "starting" } | null;
  /** Why the latest install failed; cleared when the next check or install starts. */
  installError: AppError | null;
  /** Checks for a newer release (spec §11). A call while a check is running joins that check. */
  check(): Promise<void>;
  /** Downloads and installs the update the last check found. Hatoba closes when it succeeds. */
  installUpdate(): Promise<void>;
}

let running: Promise<void> | null = null;

export const useUpdate = create<UpdateState>((set, get) => ({
  checking: false,
  result: null,
  checkedAt: null,
  error: null,
  install: null,
  installError: null,
  check: () => {
    running ??= (async () => {
      set({ checking: true, error: null, installError: null });
      try {
        set({ result: await api.update_check(), checkedAt: Date.now() });
      } catch (e) {
        set({ error: toAppError(e) });
      } finally {
        set({ checking: false });
        running = null;
      }
    })();
    return running;
  },
  installUpdate: async () => {
    if (get().install || get().checking) return;
    set({ install: { kind: "starting" }, installError: null });
    try {
      await api.update_install((install) => set({ install }));
    } catch (e) {
      set({ installError: toAppError(e) });
    } finally {
      set({ install: null });
    }
  },
}));

export const selectUpdateAvailable = (s: UpdateState) => s.result?.update != null;

let started = false;

/**
 * With the automatic check on, checks after the first unlock of each launch, then again 24 hours
 * after the last check that succeeded for as long as Hatoba runs, hidden in the tray too (WIN-10).
 */
export function startAutoCheck() {
  if (started) return;
  started = true;
  autoCheck();
  // Hourly, against the wall clock: a timer does not count the time the computer sleeps.
  setInterval(autoCheck, TICK_MS);
}

function autoCheck() {
  if (!useApp.getState().prefs.auto_update_check) return;
  const { checkedAt, install } = useUpdate.getState();
  if (install) return;
  if (checkedAt === null || Date.now() - checkedAt >= RECHECK_MS) void useUpdate.getState().check();
}

export function useAutoUpdateCheck(unlocked: boolean) {
  useEffect(() => {
    if (unlocked) startAutoCheck();
  }, [unlocked]);
}
