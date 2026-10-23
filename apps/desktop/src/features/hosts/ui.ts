import { create } from "zustand";
import type { DashboardSort } from "@/features/dashboard/logic";

/** The orders that need no readings (HOST-05). */
export type ListSort = "recent" | "name" | "address";
/** Every order of the hosts page; the resource orders apply while resource usage shows (HOST-14). */
export type HostSort = ListSort | Exclude<DashboardSort, "list" | "name">;
export const LIST_SORTS: ListSort[] = ["recent", "name", "address"];
export const STATS_SORTS: Exclude<DashboardSort, "list" | "name">[] = ["status", "cpu", "memory", "disk", "network", "uptime"];

export const isListSort = (sort: HostSort): sort is ListSort => (LIST_SORTS as HostSort[]).includes(sort);

/**
 * Hosts page state that survives navigating to the editor and back. The view and whether resource usage shows are
 * device-local prefs (`hosts_view`, `hosts_stats`).
 */
interface HostsUi {
  selectedId: string | null;
  /** A resource order applies while resource usage shows; otherwise the latest connection comes first. */
  sort: HostSort;
  favorites: boolean;
  tags: string[];
  select(id: string | null): void;
  setSort(sort: HostSort): void;
  setFavorites(favorites: boolean): void;
  setTags(tags: string[]): void;
}

export const useHostsUi = create<HostsUi>((set) => ({
  selectedId: null,
  sort: "recent",
  favorites: false,
  tags: [],
  select: (selectedId) => set({ selectedId }),
  setSort: (sort) => set({ sort }),
  setFavorites: (favorites) => set({ favorites }),
  setTags: (tags) => set({ tags }),
}));
