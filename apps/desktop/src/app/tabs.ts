import { create } from "zustand";
import type { QuickTarget } from "@/ipc/types";

export type TabStatus = "connecting" | "connected" | "failed" | "disconnected";

/** One terminal tab (TERM-01). The same host may be open in several tabs. */
export interface SessionTab {
  id: string;
  /** The saved host; null for a quick connection. */
  hostId: string | null;
  /** The target of a quick connection (HOST-12), which is not a saved host; null otherwise. */
  target: QuickTarget | null;
  title: string;
  status: TabStatus;
  /** Backend session id once `ssh_connect` resolved; null while connecting or after closing. */
  sessionId: string | null;
}

interface TabsState {
  tabs: SessionTab[];
  /** "home" or a SessionTab id. */
  active: string;
  /** A session has connected since launch. */
  connectedOnce: boolean;
  openSession(hostId: string | null, title: string, target?: QuickTarget | null): string;
  closeTab(id: string): void;
  activate(id: string): void;
  update(id: string, patch: Partial<SessionTab>): void;
  cycle(direction: 1 | -1): void;
}

let seq = 0;

export const useTabs = create<TabsState>((set, get) => ({
  tabs: [],
  active: "home",
  connectedOnce: false,
  openSession: (hostId, title, target = null) => {
    const id = `tab-${++seq}`;
    set((s) => ({ tabs: [...s.tabs, { id, hostId, target, title, status: "connecting", sessionId: null }], active: id }));
    return id;
  },
  closeTab: (id) =>
    set((s) => {
      const idx = s.tabs.findIndex((t) => t.id === id);
      const tabs = s.tabs.filter((t) => t.id !== id);
      let active = s.active;
      if (active === id) active = tabs[Math.min(idx, tabs.length - 1)]?.id ?? "home";
      return { tabs, active };
    }),
  activate: (active) => set({ active }),
  update: (id, patch) =>
    set((s) => ({
      tabs: s.tabs.map((t) => (t.id === id ? { ...t, ...patch } : t)),
      connectedOnce: s.connectedOnce || patch.status === "connected",
    })),
  cycle: (direction) => {
    const { tabs, active } = get();
    const order = ["home", ...tabs.map((t) => t.id)];
    const i = order.indexOf(active);
    set({ active: order[(i + direction + order.length) % order.length] });
  },
}));
