import { create } from "zustand";
import type { HostsSort } from "@/ipc/types";

/** The orders that need no readings (HOST-05), and the custom order (HOST-16). */
export type ListSort = Extract<HostsSort, "recent" | "name" | "address" | "custom">;
/** Every order of the hosts page; the resource orders apply while resource usage shows (HOST-14). */
export type HostSort = HostsSort;
export const LIST_SORTS: ListSort[] = ["recent", "name", "address", "custom"];
export const STATS_SORTS: Exclude<HostSort, ListSort>[] = ["status", "cpu", "memory", "disk", "network", "uptime"];

export const isListSort = (sort: HostSort): sort is ListSort => (LIST_SORTS as HostSort[]).includes(sort);

/**
 * Hosts page state that survives navigating to the editor and back. The view, the order, and whether resource usage
 * shows are device-local prefs (`hosts_view`, `hosts_sort`, `hosts_stats`).
 */
interface HostsUi {
  /** The selected rows in the order they were picked; the arrow keys move on from the last. */
  selection: string[];
  favorites: boolean;
  tags: string[];
  select(selection: string[]): void;
  setFavorites(favorites: boolean): void;
  setTags(tags: string[]): void;
  /** Shows every host again, for a search that comes from outside the page. */
  clearFilters(): void;
}

export const useHostsUi = create<HostsUi>((set) => ({
  selection: [],
  favorites: false,
  tags: [],
  select: (selection) => set({ selection }),
  setFavorites: (favorites) => set({ favorites }),
  setTags: (tags) => set({ tags }),
  clearFilters: () => set({ favorites: false, tags: [] }),
}));
