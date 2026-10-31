/**
 * The providers and models of Settings → AI, and History's conversations: listed, renamed, pinned, forked, exported
 * and deleted.
 */

import { toast } from "@hatoba/ui";
import { useVaultData } from "@/app/data";
import { getLocale, t } from "@/i18n";
import { api } from "@/ipc/api";
import { pickSavePath } from "@/lib/native";
import { hostInfo } from "../calls/hostInfo";
import { conversationMarkdown, exportFileName } from "../history/exportMarkdown";
import { forkModel, forkTitle } from "../fork";
import { getSlot, slotOf, updateConversation, useAi } from "../store";
import { aiErrorMessage } from "./context";
import { focusInput } from "./panel";
import { lastChange, runners } from "./runner";
import {
  clearSlot,
  conversationBusy,
  openConversation,
  reloadSlot,
  setSlotModel,
  slotWorking,
  stopSlot,
} from "./slots";

/** Providers and the default model (Settings → AI). */
export async function loadCatalog() {
  try {
    const [providers, settings] = await Promise.all([api.ai_providers_list(), api.ai_settings_get()]);
    useAi.setState({ catalog: { providers, settings, loaded: true } });
  } catch {
    useAi.setState((st) => ({ catalog: { ...st.catalog, loaded: true } }));
  }
}

export async function loadHistory() {
  try {
    const history = await api.ai_conversations_list();
    useAi.setState({ history, historyFailed: false });
  } catch {
    useAi.setState((st) => ({ history: st.history ?? [], historyFailed: true }));
  }
}

/** A sync pull may have brought entries from another device (§13.7). Running turns keep their state. */
export function refreshAfterSync() {
  void loadCatalog();
  if (useAi.getState().history) void loadHistory();
  for (const [id, s] of Object.entries(useAi.getState().slots)) {
    if (s.conversationId && !s.turn && !s.loading && !s.compacting && !runners.has(id)) void reloadSlot(id);
  }
}

export async function renameConversation(id: string, title: string) {
  try {
    updateConversation(await api.ai_conversation_rename(id, title));
  } catch (e) {
    toast(aiErrorMessage(e), "error");
  }
}

export async function pinConversation(id: string, pinned: boolean) {
  try {
    updateConversation(await api.ai_conversation_pin(id, pinned));
  } catch (e) {
    toast(aiErrorMessage(e), "error");
  }
}

/**
 * AI-43: copies a conversation up to the entry `entryId` (all of it with null) into a new conversation, and
 * opens that in the slot as History's Open does (AI-09), so a busy conversation the slot showed goes on in the
 * background (AI-07). Not while the conversation is busy, or while a fork of it is being made. The fork stays in
 * History when the slot moved on while Rust copied (another conversation, or a message, a turn or a compaction
 * started, even one that has ended since), or a file change of its conversation is being reverted (AI-41).
 */
export async function forkConversation(slotId: string, id: string, entryId: string | null) {
  if (conversationBusy(id) || useAi.getState().forking.includes(id)) return;
  useAi.setState((st) => ({ forking: [...st.forking, id] }));
  const shown = slotOf(id);
  const source = shown ? getSlot(shown) : null;
  const original = source?.conversation ?? useAi.getState().history?.find((c) => c.id === id) ?? null;
  // A message the original's next reply answered with another model goes on with that model (AI-05).
  const model = entryId && source ? forkModel(source.entries, entryId) : null;
  // The stored slot itself: one never used is not stored, and getSlot makes a new blank one each time.
  const before = useAi.getState().slots[slotId];
  const changedBefore = lastChange.get(slotId);
  const working = slotWorking(slotId);
  try {
    // The number in the title skips those History has (AI-43).
    if (!useAi.getState().history) await loadHistory();
    const taken = (useAi.getState().history ?? []).map((c) => c.title);
    const fork = await api.ai_conversation_fork(id, entryId, forkTitle(t, original?.title ?? "", taken));
    updateConversation(fork);
    // Another conversation shown, or a turn or compaction started since, even one that has ended or a conversation
    // shown again, is the slot moving on; what one under way when Fork was chosen does meanwhile is not. Every
    // message, stored entry and reload gives the slot a new entries array, so in an idle slot the same array also
    // means nothing else happened in it.
    const now = useAi.getState().slots[slotId];
    const movedOn =
      now?.conversationId !== before?.conversationId ||
      lastChange.get(slotId) !== changedBefore ||
      (!working && (now?.entries !== before?.entries || !!now?.turn));
    if (movedOn || before?.reverting || now?.reverting) {
      toast(t("ai.fork.inHistory"), "success");
      return;
    }
    await openConversation(slotId, fork.id);
    if (model && getSlot(slotId).conversationId === fork.id) setSlotModel(slotId, model);
    toast(t("ai.fork.done"), "success");
    focusInput();
  } catch (e) {
    toast(aiErrorMessage(e), "error");
  } finally {
    useAi.setState((st) => ({ forking: st.forking.filter((x) => x !== id) }));
  }
}

/** AI-25: saves the conversation as a Markdown file the user picks. */
export async function exportConversation(id: string) {
  try {
    const detail = await api.ai_conversation_get(id);
    const conversation = detail.conversation;
    const { hosts, telnetHosts } = useVaultData.getState();
    const host = [...hosts, ...telnetHosts].find((h) => h.id === conversation.host_id);
    const when = new Intl.DateTimeFormat(getLocale(), { dateStyle: "medium", timeStyle: "short" });
    const text = conversationMarkdown(
      {
        conversation,
        entries: detail.entries,
        host: hostInfo(host),
      },
      { t, time: (ms) => when.format(ms) },
    );
    const path = await pickSavePath(exportFileName(conversation.title, t("ai.export.fileName")), {
      name: "Markdown",
      extension: "md",
    });
    if (!path) return;
    await api.save_text_file(path, text);
    toast(t("ai.export.done"), "success");
  } catch (e) {
    toast(aiErrorMessage(e), "error");
  }
}

export async function deleteConversation(id: string) {
  const slotId = slotOf(id);
  if (slotId) {
    stopSlot(slotId);
    clearSlot(slotId);
  }
  try {
    await api.ai_conversation_delete(id);
    useAi.setState((st) => ({ history: st.history?.filter((c) => c.id !== id) ?? null }));
  } catch (e) {
    toast(aiErrorMessage(e), "error");
  }
}
