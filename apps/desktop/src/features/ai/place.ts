import type { TabSource } from "@/app/tabs";
import { formatTarget, sameTarget, savedHostFor } from "@/features/hosts/quickConnect";
import type { MessageKey, Params } from "@/i18n";
import type { AiConversationView, HostView, QuickTarget } from "@/ipc/types";

type Tr = (key: MessageKey, params?: Params) => string;
type Recorded = Pick<AiConversationView, "host_id" | "quick_target">;

/** Like Rust's `AiQuickTarget::new` (AI-09): a target as a conversation records it, the address in lowercase and without brackets. */
export function recordedTarget(t: QuickTarget): QuickTarget {
  return { address: t.address.replace(/^\[(.*)\]$/, "$1").toLowerCase(), port: t.port, username: t.username };
}

/**
 * Where a conversation last worked (AI-09): a saved host, a host deleted since, a quick-connect target
 * (HOST-12), or nowhere, as a home tab chat.
 */
export type ConversationPlace = { kind: "host"; host: HostView } | { kind: "deleted" } | { kind: "target"; target: QuickTarget } | { kind: "none" };

export function conversationPlace(c: Recorded, hosts: HostView[]): ConversationPlace {
  if (c.host_id) {
    const host = hosts.find((h) => h.id === c.host_id);
    return host ? { kind: "host", host } : { kind: "deleted" };
  }
  return c.quick_target ? { kind: "target", target: c.quick_target } : { kind: "none" };
}

/** How History and the export name a conversation's place (AI-23, AI-25). */
export function placeName(t: Tr, place: ConversationPlace): string {
  switch (place.kind) {
    case "host":
      return place.host.name;
    case "deleted":
      return t("ai.history.hostGone");
    case "target":
      return formatTarget(place.target);
    case "none":
      return t("ai.history.noHost");
  }
}

/**
 * What **Connect to** opens for a conversation on the home tab (AI-09), and the name it shows: a tab on its
 * host, or on its quick-connect target (HOST-12), as the saved host with the target's address, port, and user
 * when there is one, so its key and jump host are used as in quick connect. Null when it has neither.
 */
export function connectionFor(c: Recorded, hosts: HostView[]): { name: string; source: TabSource } | null {
  const place = conversationPlace(c, hosts);
  if (place.kind === "host") return { name: place.host.name, source: { hostId: place.host.id, target: null } };
  if (place.kind !== "target") return null;
  const saved = savedHostFor(hosts, place.target);
  return saved ? { name: saved.name, source: { hostId: saved.id, target: null } } : { name: formatTarget(place.target), source: { hostId: null, target: place.target } };
}

/**
 * Whether the next message in `tab` moves the conversation to the tab's host or quick-connect target
 * (AI-09), as Rust's `open_turn` decides.
 */
export function moves(c: Recorded, tab: TabSource): boolean {
  return tab.hostId !== null ? c.host_id !== tab.hostId : c.host_id !== null || !c.quick_target || !sameTarget(c.quick_target, tab.target);
}

/**
 * The `host_change` note the next message in `tab` gets (AI-09), as Rust's `moved_from` and `notes_before`
 * name it: `from`, the conversation's host (empty when it was deleted since) or target, and `to`, the tab's.
 * Null when Rust writes none: the message does not move the conversation, it was nowhere (a home tab chat), the
 * move is between a target and the saved host with its address, port, and user (as after Save as Host…), the
 * tab's saved host was deleted, or both have one name.
 */
export function movedFrom(c: Recorded, tab: TabSource, hosts: HostView[]): { from: string; to: string } | null {
  if (!moves(c, tab)) return null;
  const old = hosts.find((h) => h.id === c.host_id);
  const next = hosts.find((h) => h.id === tab.hostId);
  let from: string | null;
  if (c.host_id) from = tab.target && old && sameTarget(old, tab.target) ? null : (old?.name ?? "");
  else from = c.quick_target && !(next && sameTarget(next, c.quick_target)) ? formatTarget(c.quick_target) : null;
  const to = tab.hostId !== null ? next?.name : formatTarget(recordedTarget(tab.target));
  return from !== null && to !== undefined && from !== to ? { from, to } : null;
}
