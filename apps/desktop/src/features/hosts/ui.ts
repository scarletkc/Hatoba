import { create } from "zustand";
import type { DashboardSort } from "@/features/dashboard/logic";

/** The orders that need no readings (HOST-05). */
export type ListSort = "recent" | "name" | "address";
/** Every order of the hosts page; the resource orders apply while resource usage shows (HOST-14). */
export type HostSort = ListSort | Exclude<DashboardSort, "list" | "name">;
export const LIST_SORTS: ListSort[] = ["recent", "name", "address"];
export const STATS_SORTS: Exclude<DashboardSort, "list" | "name">[] = ["status", "cpu", "memory", "disk", "network", "uptime"];

export const isListSort = (sort: HostSort): sort is ListSort => (LIST_SORTS as HostSort[]).includes(sort);

export type HostsView = "list" | "cards";

/** Hosts page state that survives navigating to the editor and back. */
interface HostsUi {
  selectedId: string | null;
  sort: HostSort;
  favorites: boolean;
  tags: string[];
  view: HostsView;
  /** Resource usage shows, which signs in to the listed hosts in the background (HOST-14). */
  stats: boolean;
  select(id: string | null): void;
  setSort(sort: HostSort): void;
  setFavorites(favorites: boolean): void;
  setTags(tags: string[]): void;
  setView(view: HostsView): void;
  setStats(stats: boolean): void;
}

export const useHostsUi = create<HostsUi>((set) => ({
  selectedId: null,
  sort: "recent",
  favorites: false,
  tags: [],
  view: "list",
  stats: false,
  select: (selectedId) => set({ selectedId }),
  setSort: (sort) => set({ sort }),
  setFavorites: (favorites) => set({ favorites }),
  setTags: (tags) => set({ tags }),
  setView: (view) => set({ view }),
  // Without readings, a resource order falls back to the latest connection first.
  setStats: (stats) => set((st) => ({ stats, sort: stats || isListSort(st.sort) ? st.sort : "recent" })),
}));
