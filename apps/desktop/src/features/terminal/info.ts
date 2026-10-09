import { create } from "zustand";
import type { AppError } from "@/ipc/types";

/** UI-facing state of one terminal tab that doesn't belong in `useTabs` (status/sessionId live there). */
export interface SessionInfo {
  latencyMs: number | null;
  /** Why the last connection attempt failed. */
  error: AppError | null;
  errorAt: number | null;
  /** Why a connected session ended (already localised), if known. */
  reason: string | null;
  /** Connection attempts made in this tab (retries show in the error card). */
  attempts: number;
  sftpOpen: boolean;
  /** A quick connection's tab shows the server's resource usage (TERM-12); a saved host keeps this itself. */
  statsOn: boolean;
  /** Bumped to ask the view to open its find bar (Ctrl+Shift+F while the terminal has focus). */
  findTick: number;
}

export const EMPTY_INFO: SessionInfo = {
  latencyMs: null,
  error: null,
  errorAt: null,
  reason: null,
  attempts: 0,
  sftpOpen: false,
  statsOn: false,
  findTick: 0,
};

const useInfoStore = create<{ byTab: Record<string, SessionInfo> }>(() => ({ byTab: {} }));

export function patchInfo(tabId: string, patch: Partial<SessionInfo>) {
  useInfoStore.setState((s) => ({ byTab: { ...s.byTab, [tabId]: { ...(s.byTab[tabId] ?? EMPTY_INFO), ...patch } } }));
}

export function getInfo(tabId: string): SessionInfo {
  return useInfoStore.getState().byTab[tabId] ?? EMPTY_INFO;
}

export function clearInfo(tabId: string) {
  useInfoStore.setState((s) => {
    const { [tabId]: _gone, ...rest } = s.byTab;
    return { byTab: rest };
  });
}

export function useSessionInfo(tabId: string): SessionInfo {
  return useInfoStore((s) => s.byTab[tabId] ?? EMPTY_INFO);
}
