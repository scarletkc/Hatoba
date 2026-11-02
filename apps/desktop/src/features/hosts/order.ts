import type { HostView } from "@/ipc/types";
import type { ListSort } from "./ui";

/** What the host list filters and sorts by, which SSH and Telnet hosts both have. */
export type HostLike = Pick<
  HostView,
  "name" | "address" | "port" | "username" | "tags" | "favorite" | "group_id" | "last_connected_at" | "sort"
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

/**
 * The host list's order (HOST-05): the latest connection first, the name, the address, or the custom order the user
 * made (HOST-16), in which hosts without a place come last.
 */
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
    case "custom": {
      const place = (h: T) => (h.sort > 0 ? h.sort : Infinity);
      return out.sort((a, b) => place(a) - place(b) || byName(a, b));
    }
  }
}

/**
 * {@link sortHosts}, where `pending`, a custom order just made, stands in for the hosts' places until they reload with
 * the new ones.
 */
export function orderHosts<T extends HostLike & Pick<HostView, "id">>(
  hosts: T[],
  sort: ListSort,
  locale: string,
  pending: string[] | null,
): T[] {
  const sorted = sortHosts(hosts, sort, locale);
  if (sort !== "custom" || !pending) return sorted;
  const at = new Map(pending.map((id, i) => [id, i]));
  const index = (h: T) => at.get(h.id) ?? pending.length;
  return sorted.sort((a, b) => index(a) - index(b));
}

/**
 * The custom order after moving `moving` before the host `before`, or after the last of the `shown` hosts that stays
 * where it is (HOST-16). `order` is every host's id in the order shown, and `shown` the ones the page lists. The moved
 * hosts keep their order among themselves.
 */
export function moveHosts(order: string[], shown: string[], moving: string[], before: string | null): string[] {
  const moved = new Set(moving);
  const rest = order.filter((id) => !moved.has(id));
  let at: number;
  if (before !== null && !moved.has(before)) {
    at = rest.indexOf(before);
  } else {
    const staying = shown.filter((id) => !moved.has(id));
    if (staying.length === 0) return order;
    at = rest.indexOf(staying[staying.length - 1]) + 1;
  }
  if (at < 0) return order;
  return [...rest.slice(0, at), ...order.filter((id) => moved.has(id)), ...rest.slice(at)];
}
