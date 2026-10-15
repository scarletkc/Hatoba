import type { TabSource } from "@/app/tabs";
import { formatTarget, sameTarget, savedHostFor } from "@/features/hosts/quickConnect";
import type { MessageKey, Params } from "@/i18n";
import type { AiConversationView, HostView, QuickTarget } from "@/ipc/types";

type Tr = (key: MessageKey, params?: Params) => string;
type Recorded = Pick<AiConversationView, "host_id" | "quick_target"> & Partial<Pick<AiConversationView, "origin" | "local">>;

/** What follows a computer's name where a host's display name would be, as Rust's `local_name` writes it (§7.5). */
const LOCAL_SUFFIX = " (local terminal)";

/** How notes and the blocks the panel writes name a local tab's computer (§7.5), as Rust's `local_name`. */
export function localName(device: string): string {
  return `${device}${LOCAL_SUFFIX}`;
}

/**
 * How the panel shows a place a note or a block names: the local terminal of `device`, this computer,
 * as **This Computer**, another computer's with its name, and anything else as it is.
 */
export function placeLabel(t: Tr, name: string, device = ""): string {
  if (!name.endsWith(LOCAL_SUFFIX)) return name;
  const of = name.slice(0, -LOCAL_SUFFIX.length);
  return of === device ? t("terminal.local.thisComputer") : t("ai.place.localOn", { device: of });
}

/** Like Rust's `AiQuickTarget::new` (AI-09): a target as a conversation records it, the address in lowercase and without brackets. */
export function recordedTarget(t: QuickTarget): QuickTarget {
  return { address: t.address.replace(/^\[(.*)\]$/, "$1").toLowerCase(), port: t.port, username: t.username };
}

/**
 * Where a conversation last worked (AI-09): a saved host, a host deleted since, a quick-connect target
 * (HOST-12), a computer's local terminal (§7.5), or nowhere, as a home tab chat.
 */
export type ConversationPlace =
  | { kind: "host"; host: HostView }
  | { kind: "deleted" }
  | { kind: "target"; target: QuickTarget }
  | { kind: "local"; device: string }
  | { kind: "none" };

export function conversationPlace(c: Recorded, hosts: HostView[]): ConversationPlace {
  if (c.host_id) {
    const host = hosts.find((h) => h.id === c.host_id);
    return host ? { kind: "host", host } : { kind: "deleted" };
  }
  if (c.quick_target) return { kind: "target", target: c.quick_target };
  return c.local ? { kind: "local", device: c.local } : { kind: "none" };
}

/** How History and the export name a conversation's place (AI-23, AI-25). */
export function placeName(t: Tr, place: ConversationPlace, device = ""): string {
  switch (place.kind) {
    case "host":
      return place.host.name;
    case "deleted":
      return t("ai.history.hostGone");
    case "target":
      return formatTarget(place.target);
    case "local":
      return placeLabel(t, localName(place.device), device);
    case "none":
      return t("ai.history.noHost");
  }
}

/**
 * What **Connect to** opens for a conversation on the home tab (AI-09), and the name it shows: a tab on its
 * host, or on its quick-connect target (HOST-12), as the saved host with the target's address, port, and user
 * when there is one, so its key and jump host are used as in quick connect, or a local terminal for one that was
 * on this computer's (§7.5). Null when it has none of them.
 */
export function connectionFor(
  c: Recorded,
  hosts: HostView[],
  device = "",
): { name: string; source: TabSource | "local" } | null {
  const place = conversationPlace(c, hosts);
  if (place.kind === "host") return { name: place.host.name, source: { hostId: place.host.id, target: null } };
  if (place.kind === "local") return place.device === device ? { name: localName(device), source: "local" } : null;
  if (place.kind !== "target") return null;
  const saved = savedHostFor(hosts, place.target);
  return saved ? { name: saved.name, source: { hostId: saved.id, target: null } } : { name: formatTarget(place.target), source: { hostId: null, target: place.target } };
}

/**
 * Whether the next message in `tab` moves the conversation to the tab's host or quick-connect target
 * (AI-09), or to the computer of a local tab (§7.5), as Rust's `open_turn` decides.
 */
export function moves(c: Recorded, tab: TabSource, device = ""): boolean {
  if (tab.local) return c.host_id !== null || c.quick_target !== null || c.local !== device;
  return tab.hostId !== null ? c.host_id !== tab.hostId : c.host_id !== null || !c.quick_target || !sameTarget(c.quick_target, tab.target);
}

/**
 * The `host_change` note the next message in `tab` gets (AI-09), as Rust's `moved_from` and `notes_before`
 * name it: `from`, the conversation's host (empty when it was deleted since) or target, or with neither, a fork's
 * `origin` (AI-43), and `to`, the tab's.
 * Null when Rust writes none: the message does not move the conversation, it was nowhere (a home tab chat), the
 * move is between a target and the saved host with its address, port, and user (as after Save as Host…), the
 * tab's saved host was deleted, or both have one name.
 */
export function movedFrom(
  c: Recorded,
  tab: TabSource,
  hosts: HostView[],
  device = "",
): { from: string; to: string } | null {
  if (!moves(c, tab, device)) return null;
  const old = hosts.find((h) => h.id === c.host_id);
  const next = hosts.find((h) => h.id === tab.hostId);
  let from: string | null;
  if (c.host_id) from = tab.target && old && sameTarget(old, tab.target) ? null : (old?.name ?? "");
  else if (c.quick_target) from = next && sameTarget(next, c.quick_target) ? null : formatTarget(c.quick_target);
  else from = c.local ? localName(c.local) : (c.origin ?? null);
  const to = tab.local ? localName(device) : tab.hostId !== null ? next?.name : formatTarget(recordedTarget(tab.target));
  return from !== null && to !== undefined && from !== to ? { from, to } : null;
}
