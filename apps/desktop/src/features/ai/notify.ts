import { useApp } from "@/app/store";
import { useTabs } from "@/app/tabs";
import { t, type MessageKey, type Params } from "@/i18n";
import { api } from "@/ipc/api";
import type { NotificationInput } from "@/ipc/types";
import { windowFocused } from "@/lib/native";
import { getSlot, HOME_SLOT, tabOf } from "./store";

/** AI-07: what a notification tells: the turn ended, or ended on an error, or a tool call waits for approval. */
export type AiNotice = "done" | "failed" | "approval";

const BODIES: Record<AiNotice, MessageKey> = {
  done: "ai.notify.done",
  failed: "ai.notify.failed",
  approval: "ai.notify.approval",
};

export interface NoticeContext {
  /** Settings → General's switch (`ai_notifications`). */
  enabled: boolean;
  /** The window has the focus, so the tab bar shows what happened (AI-07). */
  focused: boolean;
  /** The tab the conversation belongs to, also from its background (AI-07). */
  tab: string;
  /** The tab's title; null for the home tab. */
  tabTitle: string | null;
  conversationId: string | null;
  conversationTitle: string;
}

/**
 * AI-07: the system notification about a conversation, or null when none goes: switched off, or the window has the
 * focus. It names the tab, and the conversation with what happened; a click on it shows the conversation.
 */
export function aiNotification(notice: AiNotice, ctx: NoticeContext, tr: (key: MessageKey, params?: Params) => string = t): NotificationInput | null {
  if (!ctx.enabled || ctx.focused) return null;
  return {
    title: ctx.tabTitle ?? tr("ai.title"),
    body: tr(BODIES[notice], { title: ctx.conversationTitle || tr("ai.untitled") }),
    target: { tab: ctx.tab, conversation_id: ctx.conversationId, host_id: null },
  };
}

/** AI-07: notifies about the conversation in a slot, a tab's or a background one, unless the window has the focus. */
export async function notifyAi(slotId: string, notice: AiNotice) {
  // Read before asking for the focus, which the slot's conversation may leave meanwhile.
  const { conversationId, conversation } = getSlot(slotId);
  const tab = tabOf(slotId);
  const tabTitle = tab === HOME_SLOT ? null : (useTabs.getState().tabs.find((x) => x.id === tab)?.title ?? null);
  const enabled = useApp.getState().prefs.ai_notifications;
  const focused = enabled ? await windowFocused() : true;
  const notification = aiNotification(notice, { enabled, focused, tab, tabTitle, conversationId, conversationTitle: conversation?.title ?? "" });
  if (notification) await api.notification_show(notification).catch(() => {});
}
