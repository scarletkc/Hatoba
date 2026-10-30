import type { HostView } from "@/ipc/types";
import type { ListSort } from "./ui";

/** What the host list filters and sorts by, which SSH and Telnet hosts both have. */
export type HostLike = Pick<
  HostView,
  "name" | "address" | "port" | "username" | "tags" | "favorite" | "group_id" | "last_connected_at"
>;

/** What the hosts page lists: a group or every host, narrowed to favorites and to hosts with every one of `tags`. */
export interface HostFilter {
  group: string | null;
  favorites: boolean;
  tags: string[];
}

export function applyFilter<T extends HostLike>(hosts: T[], filter: HostFilter): T[] {
  return hosts.filter(
    (h) =>
      (filter.group === null || h.group_id === filter.group) &&
      (!filter.favorites || h.favorite) &&
      filter.tags.every((tag) => h.tags.includes(tag)),
  );
}

/** The host list's order (HOST-05): the latest connection first, the name, or the address. */
export function sortHosts<T extends HostLike>(hosts: T[], sort: ListSort, locale: string): T[] {
  const collator = new Intl.Collator(locale, { numeric: true, sensitivity: "base" });
  const byName = (a: T, b: T) => collator.compare(a.name, b.name);
  const out = [...hosts];
  switch (sort) {
    case "recent":
      return out.sort((a, b) => (b.last_connected_at ?? -1) - (a.last_connected_at ?? -1) || byName(a, b));
    case "name":
      return out.sort(byName);
    case "address":
      return out.sort((a, b) => collator.compare(a.address, b.address) || byName(a, b));
  }
}
