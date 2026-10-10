import type { TabSource } from "@/app/tabs";
import { formatTarget, parseTarget, sameTarget, savedHostFor } from "@/features/hosts/quickConnect";
import type { MessageKey, Params } from "@/i18n";
import type { AiConversationView, HostView } from "@/ipc/types";

type Tr = (key: MessageKey, params?: Params) => string;
type Recorded = Pick<AiConversationView, "host_id" | "quick_target">;

/**
 * Where a conversation last worked (AI-09): a saved host, a host deleted since, a quick-connect target
 * (HOST-12), or nowhere, as a home tab chat.
 */
export type ConversationPlace = { kind: "host"; host: HostView } | { kind: "deleted" } | { kind: "target"; target: string } | { kind: "none" };

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
      return place.target;
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
  const target = place.kind === "target" ? parseTarget(place.target) : null;
  if (!target) return null;
  const saved = savedHostFor(hosts, target);
  return saved ? { name: saved.name, source: { hostId: saved.id, target: null } } : { name: formatTarget(target), source: { hostId: null, target } };
}

/**
 * Where the next message in `tab` moves the conversation from (AI-09), named as Rust's `host_change` note
 * names it: its host's display name, empty when the host was deleted since, or its target. Null when the
 * message does not move it, and, as in Rust's `moved_from`, when it was nowhere (a home tab chat) or the move
 * is between a target and the saved host with its address, port, and user (as after Save as Host…). Targets
 * compare as quick connect compares them (HOST-12), as Rust's `same_target` does.
 */
export function movedFrom(c: Recorded, tab: TabSource, hosts: HostView[]): string | null {
  const recorded = c.quick_target ? parseTarget(c.quick_target) : null;
  const moves = tab.hostId !== null ? c.host_id !== tab.hostId : c.host_id !== null || !recorded || !sameTarget(recorded, tab.target);
  if (!moves) return null;
  const host = (id: string | null) => hosts.find((h) => h.id === id);
  if (c.host_id) {
    const old = host(c.host_id);
    return old && tab.target && sameTarget(old, tab.target) ? null : (old?.name ?? "");
  }
  const now = host(tab.hostId);
  return c.quick_target && !(recorded && now && sameTarget(now, recorded)) ? c.quick_target : null;
}
