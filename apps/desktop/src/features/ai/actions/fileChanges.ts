/**
 * What file tool calls change: the diff an approval card shows (AI-39, AI-40), and the changes this device keeps
 * (AI-41).
 */

import { api } from "@/ipc/api";
import type { AiFileChange, AiFilePreview } from "@/ipc/types";
import { getSlot, patchSlot, slotOf, useAi } from "../store";
import { attachedTab, unlocked } from "./context";

/** AI-41: reads again the file changes of every open conversation, as the size limit spans them all. */
export function refreshAllFileChanges() {
  for (const s of Object.values(useAi.getState().slots))
    if (s.conversationId) void refreshFileChanges(s.conversationId);
}

/** AI-41: reads again which changes of the conversation's file tool calls this device keeps, for the slot that holds it then. */
export async function refreshFileChanges(convId: string) {
  try {
    const fileChanges = await api.ai_file_changes(convId);
    const target = slotOf(convId);
    // The lock cleared them meanwhile: they name servers and paths.
    if (unlocked() && target) patchSlot(target, { fileChanges });
  } catch {
    // Locked or deleted meanwhile; the next load reads them again.
  }
}

/** AI-41: what a call changed, from the snapshot this device keeps; null once it was reverted or dropped. Rejects when it cannot be read. */
export async function fileChangeDiff(
  slotId: string,
  change: Pick<AiFileChange, "response_id" | "tool_call_id">,
): Promise<AiFilePreview | null> {
  const convId = getSlot(slotId).conversationId;
  if (!convId) return null;
  return api.ai_file_change_diff(convId, change.response_id, change.tool_call_id);
}

/**
 * AI-39, AI-40: the change an `edit_file` or `write_file` call waiting on the slot's approval card would
 * make, read from the host of the slot's tab, with the arguments as the user edited them on the card.
 */
export function previewFileCall(slotId: string, callId: string, edited: string | null): Promise<AiFilePreview> {
  const convId = getSlot(slotId).conversationId;
  if (!convId) return Promise.reject(new Error("the conversation is not stored yet"));
  const target = attachedTab(slotId);
  const sessionId = target?.tab.status === "connected" ? target.tab.sessionId : null;
  return api.ai_file_preview(convId, callId, sessionId, edited);
}
