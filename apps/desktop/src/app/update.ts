import { useEffect } from "react";
import { create } from "zustand";
import { api, toAppError } from "@/ipc/api";
import type { AppError, UpdateCheck } from "@/ipc/types";
import { useApp } from "./store";

interface UpdateState {
  checking: boolean;
  /** The last check that succeeded. A later failure keeps it, so the sidebar dot stays. */
  result: UpdateCheck | null;
  /** Why the latest check failed; cleared when the next one starts. */
  error: AppError | null;
  /** Checks GitHub Releases (spec §11). A call while a check is running joins that check. */
  check(): Promise<void>;
}

let running: Promise<void> | null = null;

export const useUpdate = create<UpdateState>((set) => ({
  checking: false,
  result: null,
  error: null,
  check: () => {
    running ??= (async () => {
      set({ checking: true, error: null });
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
}));

export const selectUpdateAvailable = (s: UpdateState) => s.result?.update_available === true;

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
