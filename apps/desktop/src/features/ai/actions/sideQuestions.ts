/** Side questions (AI-42): asked about a conversation without stopping its turn, and never stored. */

import { toast } from "@hatoba/ui";
import { t } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { AiTurnContext } from "@/ipc/types";
import { messageText } from "../queue";
import { approvalCall, canAskSide, newSideQuestion, reduceSideEvent, sideRunning, type SideQuestion } from "../side";
import { getSlot, patchSlot, slotOf, type QueuedMessage } from "../store";
import { extraId } from "./attach";
import { aiErrorMessage, turnContext } from "./context";
import { takeInput } from "./queued";

let sideSeq = 0;

/** What each side question asked from the input area took from it (AI-42), until it is answered or replaced. */
const sideInputs = new Map<number, { raw: string; message: QueuedMessage }>();

/** Updates the side question `id` in the slot that shows its conversation now: it may have moved meanwhile. */
function patchSide(convId: string, id: number, update: (q: SideQuestion) => SideQuestion) {
  const target = slotOf(convId);
  if (target) patchSlot(target, (s) => (s.side?.id === id ? { side: update(s.side) } : {}));
}

/**
 * A side question from the input area was refused, failed or was stopped: what it took goes back, as for a
 * message that was not sent. The typed text fills the box only when it is empty, the pastes and files go
 * before those there, and the selection and diagnostics chips come back. Its bubble keeps Retry.
 */
function giveBackSideInput(convId: string, id: number) {
  const taken = sideInputs.get(id);
  if (!taken) return;
  sideInputs.delete(id);
  const { message } = taken;
  const stray = message.restore(message.chips).map((attachment) => ({ id: extraId(), attachment }));
  const target = slotOf(convId);
  if (target)
    patchSlot(target, (s) => ({ draft: s.draft || taken.raw, extras: [...stray, ...message.extras, ...s.extras] }));
}

/**
 * AI-42: asks a side question about the slot's conversation without stopping its turn. From the input area
 * (`fromInput`) it is what the box holds, with the attachments a message would take, which leave the input
 * area; from an approval card only the card's text. The answer shows at the end of the conversation and is
 * not stored. A side question already on its way in the conversation is replaced, and what it took from the
 * input area stays out.
 */
export function askSideQuestion(slotId: string, raw: string, fromInput = true, shown: string = raw): boolean {
  const slot = getSlot(slotId);
  const typed = raw.trim();
  const convId = slot.conversationId;
  if (!convId || !canAskSide(slot, typed)) return false;
  const context = turnContext(slotId);
  if (!context) {
    toast(t("ai.err.noModel"), "error");
    return false;
  }
  if (slot.side) sideInputs.delete(slot.side.id);
  const id = ++sideSeq;
  let text = typed;
  if (fromInput) {
    const taken = takeInput(slotId, typed);
    text = messageText(taken);
    // What goes back into the box when the question fails: what was typed, `/btw` included (AI-50).
    sideInputs.set(id, { raw: shown, message: taken });
  }
  void runSide(slotId, newSideQuestion(id, convId, typed, text), context, approvalCall(slot));
  return true;
}

/** Retry on a side question that failed: the same question, with what it took from the input area. */
export function retrySideQuestion(slotId: string) {
  const slot = getSlot(slotId);
  const q = slot.side;
  if (!q || sideRunning(q) || slot.conversationId !== q.conversationId || !canAskSide(slot, q.sent)) return;
  const context = turnContext(slotId);
  if (!context) {
    toast(t("ai.err.noModel"), "error");
    return;
  }
  void runSide(slotId, newSideQuestion(++sideSeq, q.conversationId, q.question, q.sent), context, approvalCall(slot));
}

async function runSide(slotId: string, q: SideQuestion, context: AiTurnContext, approval: string | null) {
  patchSlot(slotId, { side: q });
  const update = (f: (cur: SideQuestion) => SideQuestion) => patchSide(q.conversationId, q.id, f);
  try {
    const answer = await api.ai_side_question(
      { conversation_id: q.conversationId, question_id: q.id, text: q.sent, context, approval_call_id: approval },
      (ev) => update((cur) => reduceSideEvent(cur, ev)),
    );
    sideInputs.delete(q.id);
    update((cur) => (sideRunning(cur) ? { ...cur, phase: "done", answer: answer.text, finish: answer.finish } : cur));
  } catch (e) {
    // Refused, failed, or the vault locked (a stop gave the input back already, and a newer side question
    // or a message that replaced it keeps the input area as it is).
    giveBackSideInput(q.conversationId, q.id);
    // Stopped, replaced by a newer side question, or the vault locked.
    if (toAppError(e).code === "cancelled") update((cur) => (sideRunning(cur) ? { ...cur, phase: "stopped" } : cur));
    // What streamed before the failure goes: it may be the start of an answer that was dropped. A failure
    // that comes after the user stopped the question changes nothing.
    else update((cur) => (sideRunning(cur) ? { ...cur, phase: "error", answer: "", error: aiErrorMessage(e) } : cur));
  }
}

/**
 * Stop on the side question, or Esc while it runs (AI-42): only the side question stops, never the turn,
 * and what it took from the input area goes back there.
 */
export function stopSideQuestion(slotId: string) {
  const q = getSlot(slotId).side;
  if (!q || !sideRunning(q)) return;
  patchSide(q.conversationId, q.id, (cur) => ({ ...cur, phase: "stopped" }));
  giveBackSideInput(q.conversationId, q.id);
  void api.ai_side_stop(q.conversationId, q.id).catch(() => {});
}

/** × on the side question: it goes, and stops first when its answer is still on its way. */
export function dismissSideQuestion(slotId: string) {
  stopSideQuestion(slotId);
  patchSlot(slotId, { side: null });
}

/**
 * A message discards the slot's side question (AI-42), whose bubble would otherwise stay below the new turn.
 * One still on its way stops first; what it took from the input area stays out, since it was asked.
 */
export function discardSideQuestion(slotId: string) {
  const q = getSlot(slotId).side;
  if (!q) return;
  sideInputs.delete(q.id);
  if (sideRunning(q)) void api.ai_side_stop(q.conversationId, q.id).catch(() => {});
  patchSlot(slotId, { side: null });
}
