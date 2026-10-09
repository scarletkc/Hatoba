import { create } from "zustand";
import type { QuickTarget } from "@/ipc/types";

export type TabStatus = "connecting" | "connected" | "failed" | "disconnected";

/** What a tab connects to: a saved host, or a quick-connect target that is not one (HOST-12). */
export type TabSource = { hostId: string; target: null } | { hostId: null; target: QuickTarget };

/** One terminal tab (TERM-01). The same host may be open in several tabs. */
export type SessionTab = TabSource & {
  id: string;
  title: string;
  status: TabStatus;
  /** Backend session id once `ssh_connect` resolved; null while connecting or after closing. */
  sessionId: string | null;
};

type TabPatch = Partial<Pick<SessionTab, "title" | "status" | "sessionId">>;

interface TabsState {
  tabs: SessionTab[];
  /** "home" or a SessionTab id. */
  active: string;
  /** A session has connected since launch. */
  connectedOnce: boolean;
  openSession(source: TabSource, title: string): string;
  closeTab(id: string): void;
  activate(id: string): void;
  update(id: string, patch: TabPatch): void;
  /** A quick-connection tab connects to the saved host from now on (HOST-12). */
  adoptHost(id: string, hostId: string, title: string): void;
  cycle(direction: 1 | -1): void;
}

let seq = 0;

export const useTabs = create<TabsState>((set, get) => ({
  tabs: [],
  active: "home",
  connectedOnce: false,
  openSession: (source, title) => {
    const id = `tab-${++seq}`;
    set((s) => ({ tabs: [...s.tabs, { ...source, id, title, status: "connecting", sessionId: null }], active: id }));
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
  adoptHost: (id, hostId, title) =>
    set((s) => ({ tabs: s.tabs.map((t) => (t.id === id ? { ...t, hostId, target: null, title } : t)) })),
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
