import type { HostFilter } from "@/app/store";
import type { HostView } from "@/ipc/types";
import type { HostSort } from "./ui";

/** What the host list filters and sorts by, which SSH and Telnet hosts both have. */
export type HostLike = Pick<HostView, "name" | "address" | "port" | "username" | "tags" | "favorite" | "group_id" | "last_connected_at">;

/** The hosts a sidebar filter shows: all, favorites, recent, a group or a tag. */
export function applyFilter<T extends HostLike>(hosts: T[], filter: HostFilter): T[] {
  switch (filter.kind) {
    case "all":
      return hosts;
    case "favorites":
      return hosts.filter((h) => h.favorite);
    case "recent":
      return hosts.filter((h) => h.last_connected_at !== null);
    case "group":
      return hosts.filter((h) => h.group_id === filter.id);
    case "tag":
      return hosts.filter((h) => h.tags.includes(filter.name));
  }
}

/** The host list's order (HOST-05): the latest connection first, the name, or the address. */
export function sortHosts<T extends HostLike>(hosts: T[], sort: HostSort, locale: string): T[] {
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
