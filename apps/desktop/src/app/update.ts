import { useEffect } from "react";
import { create } from "zustand";
import { api, toAppError } from "@/ipc/api";
import type { AppError, UpdateCheck, UpdateProgress } from "@/ipc/types";
import { useApp } from "./store";

interface UpdateState {
  checking: boolean;
  /** The last check that succeeded. A later failure keeps it, so the sidebar dot stays. */
  result: UpdateCheck | null;
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
  error: null,
  install: null,
  installError: null,
  check: () => {
    running ??= (async () => {
      set({ checking: true, error: null, installError: null });
      try {
        set({ result: await api.update_check() });
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

let startupChecked = false;

/** With the automatic check on, checks once after the first unlock of each launch. */
export function checkOnFirstUnlock() {
  if (startupChecked) return;
  startupChecked = true;
  if (useApp.getState().prefs.auto_update_check) void useUpdate.getState().check();
}

export function useStartupUpdateCheck(unlocked: boolean) {
  useEffect(() => {
    if (unlocked) checkOnFirstUnlock();
  }, [unlocked]);
}
