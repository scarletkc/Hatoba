/**
 * What the AI panel's actions share: the text of a failed command, the terminal tab a slot belongs to, and what a
 * request may act on (`turnContext`).
 */

import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { useTabs, type SessionTab } from "@/app/tabs";
import { getSession, type LiveSession } from "@/features/terminal/session";
import { t, type MessageKey, type Params } from "@/i18n";
import type { AiEffort, AiModelRef, AiTurnContext } from "@/ipc/types";
import { chosenEffort } from "../effort";
import { conversationModel } from "../models";
import { getSlot, HOME_SLOT, sendsScreen, tabOf, useAi } from "../store";
import { screenNote } from "../terminal/terminalText";

/** User-facing text for a failed AI command (`ai` errors carry the provider's status and message). */
export function aiErrorMessage(e: unknown, tr: (key: MessageKey, params?: Params) => string = t): string {
  return errorMessage(tr, e);
}

/** The terminal tab a slot belongs to (`tabOf`, AI-07); null for the home tab or a tab that is gone. */
export function attachedTab(slotId: string): { tab: SessionTab; session: LiveSession | undefined } | null {
  const tabId = tabOf(slotId);
  if (tabId === HOME_SLOT) return null;
  const tab = useTabs.getState().tabs.find((x) => x.id === tabId);
  return tab ? { tab, session: getSession(tab.id) } : null;
}

/** The model the next request of a slot uses (AI-05). */
export function slotModel(slotId: string): AiModelRef | null {
  const slot = getSlot(slotId);
  const { providers, settings } = useAi.getState().catalog;
  return conversationModel(slot.entries, providers, settings, slot.model);
}

/** The thinking level the next request of a slot is made with (AI-05); null is Default. Rust fits it to the model. */
export function slotEffort(slotId: string): AiEffort | null {
  const slot = getSlot(slotId);
  return chosenEffort(slot.effort, slot.conversationId, slot.conversation, useAi.getState().catalog.settings);
}

/**
 * What a request may act on: the tab's host or quick-connect target and, when connected, its terminal
 * tools and session, whose server's identification string the system prompt states (AI-08, AI-09,
 * §13.1), the permission mode a `permission_mode` note tells the model of (§13.5), and the screen
 * a message shows while **Send Terminal Screen** is on (AI-46). The home tab names the
 * conversation's own host or target, which leaves it where it is.
 */
export function turnContext(slotId: string): AiTurnContext | null {
  const model = slotModel(slotId);
  if (!model) return null;
  const attached = attachedTab(slotId);
  const connected = !!attached && attached.tab.status === "connected";
  const slot = getSlot(slotId);
  const term = connected && sendsScreen(slot, useAi.getState().catalog.settings) ? attached.session?.term : undefined;
  const conversation = slot.conversation;
  return {
    provider_id: model.provider_id,
    model_id: model.model_id,
    effort: slotEffort(slotId),
    host_id: attached ? attached.tab.hostId : (conversation?.host_id ?? null),
    target: attached ? attached.tab.target : (conversation?.quick_target ?? null),
    local: attached
      ? localOf(attached.tab)
      : conversation?.local
        ? { device: conversation.local, shell: null, kind: null }
        : null,
    tab: connected,
    session_id: connected ? (attached.tab.sessionId ?? null) : null,
    disabled_mcp_servers: slot.mcpOff,
    permission_mode: slot.mode,
    screen: term ? screenNote(term.buffer.active, term.rows) : null,
  };
}

/** A local tab's computer and shell, as a turn's context names them (§7.5); null for an SSH tab. */
function localOf(tab: SessionTab): AiTurnContext["local"] {
  if (!tab.local) return null;
  return { device: useApp.getState().info.device_name, shell: tab.local.name, kind: tab.local.kind };
}

/** The vault is unlocked: what a command returned may go into the store. */
export function unlocked(): boolean {
  return useApp.getState().phase === "unlocked";
}
