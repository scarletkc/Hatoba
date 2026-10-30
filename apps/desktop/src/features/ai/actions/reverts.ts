/** AI-41: reverting the change of a file tool call, from its block, or before an edit deletes the call (AI-26). */

import { confirm, toast } from "@hatoba/ui";
import { t } from "@/i18n";
import { api } from "@/ipc/api";
import type { AiFileChange, AiFileRevert } from "@/ipc/types";
import { confirmRevert } from "../calls/RevertConfirm";
import { getSlot, patchSlot } from "../store";
import { aiErrorMessage, attachedTab } from "./context";
import { fileChangeDiff, refreshFileChanges } from "./fileChanges";
import { slotBusy } from "./slots";

/**
 * AI-41: reverts the change of a file tool call on the slot's tab, once the conversation is idle. The user
 * confirms first, with the diff the revert writes back; a file the call created is deleted. With `restore`
 * (Restore anyway), it writes over what the diff of the last revert showed, which the user confirms too;
 * `recreate`: that diff showed the file missing, so it is created again. `onStart` runs once the revert is under way. Null when nothing was asked of Rust or it refused; otherwise
 * what the revert did, for the call's block to show.
 */
export async function revertFileChange(
  slotId: string,
  change: AiFileChange,
  options: { restore?: boolean; recreate?: boolean; onStart?: () => void } = {},
): Promise<AiFileRevert | null> {
  const convId = getSlot(slotId).conversationId;
  if (!convId || slotBusy(slotId)) return null;
  const path = change.path;
  const ok = options.restore
    ? await confirm({
        ...(change.created
          ? {
              title: t("ai.revert.deleteAnywayTitle", { path }),
              body: t("ai.revert.deleteAnywayBody"),
              confirmLabel: t("ai.revert.deleteAnyway"),
            }
          : {
              title: t("ai.revert.restoreTitle", { path }),
              body: t(options.recreate ? "ai.revert.recreateBody" : "ai.revert.restoreBody"),
              confirmLabel: t("ai.revert.restore"),
            }),
        danger: true,
      })
    : await fileChangeDiff(slotId, change).then(
        (preview) => confirmRevert(change, preview),
        // The confirmation promises the diff it writes back: without it, nothing is asked or written.
        (e: unknown) => {
          toast(aiErrorMessage(e), "error");
          return false;
        },
      );
  if (!ok || getSlot(slotId).conversationId !== convId || slotBusy(slotId)) return null;
  options.onStart?.();
  patchSlot(slotId, { reverting: true });
  try {
    const outcome = await api.ai_file_revert(
      convId,
      change.response_id,
      change.tool_call_id,
      revertSession(slotId),
      !!options.restore,
    );
    if (outcome.status === "reverted") {
      markReverted(slotId, convId, change);
      // Nothing to write: a revert whose result was lost did it already, or the write never changed the file.
      const done = outcome.wrote ? "ai.revert.done" : change.created ? "ai.revert.alreadyGone" : "ai.revert.already";
      toast(t(done, { path }), "success");
    }
    return outcome;
  } catch (e) {
    toast(aiErrorMessage(e), "error");
    // The size limit may have dropped the change meanwhile, or another device's edit deleted its call.
    void refreshFileChanges(convId);
    return null;
  } finally {
    patchSlot(slotId, { reverting: false });
  }
}

/** AI-41: a revert of the slot's conversation runs, so nothing is sent until it ends (Rust refuses it too). */
export function reverting(slotId: string): boolean {
  if (!getSlot(slotId).reverting) return false;
  toast(t("ai.revert.wait"), "error");
  return true;
}

/** The session a revert runs on: the slot's tab, when it is connected. */
export function revertSession(slotId: string): string | null {
  const target = attachedTab(slotId);
  return target?.tab.status === "connected" ? (target.tab.sessionId ?? null) : null;
}

function markReverted(slotId: string, convId: string, change: AiFileChange) {
  const same = (c: AiFileChange) => c.response_id === change.response_id && c.tool_call_id === change.tool_call_id;
  patchSlot(slotId, (s) =>
    s.conversationId === convId
      ? { fileChanges: s.fileChanges.map((c) => (same(c) ? { ...c, state: "reverted" as const } : c)) }
      : {},
  );
}

/** AI-26, AI-41: reverts a change an edit is about to delete the call of; an error is a failure the edit's dialog lists. */
export async function revertChange(slotId: string, convId: string, change: AiFileChange): Promise<AiFileRevert> {
  try {
    const outcome = await api.ai_file_revert(
      convId,
      change.response_id,
      change.tool_call_id,
      revertSession(slotId),
      false,
    );
    if (outcome.status === "reverted") markReverted(slotId, convId, change);
    return outcome;
  } catch (e) {
    void refreshFileChanges(convId);
    return { status: "failed", reason: "failed", detail: aiErrorMessage(e) };
  }
}
