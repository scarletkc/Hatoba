/** The message a slot queues while its turn runs: it goes when the turn ends, or back into the input area (§13.1). */

import { toast } from "@hatoba/ui";
import type { Attachment } from "../attachments";
import { joinMessages, messageAttachments, messageBack, pickInput, withinCap } from "../queue";
import {
  getSlot,
  inBackground,
  patchSlot,
  useAi,
  type InputChip,
  type PendingAttachment,
  type QueuedMessage,
} from "../store";
import {
  extraId,
  hideSelection,
  liveChips,
  removeDiagnostics,
  setTabSelection,
  tabSelection,
  tooMuchMessage,
} from "./attach";
import { focusInput } from "./panel";

/**
 * A message of the typed text with what the input area sends with it, as `pickInput` picks it (AI-10, AI-35):
 * a chip of a kind the queued message it joins (`beside`) has already stays (`left`) for a later one. For a
 * message that was not sent, `restore` puts its chips still in `kept` back in their places: the selection
 * while the tab's selection is the same, the diagnostics while the tab has none. It returns the others.
 */
function inputMessage(
  slotId: string,
  typed: string,
  beside: QueuedMessage | null,
): { message: QueuedMessage; left: PendingAttachment[] } {
  const picked = pickInput(liveChips(slotId), getSlot(slotId).extras, beside ? messageAttachments(beside) : []);
  const { seq, hidden: hiddenBefore } = tabSelection(slotId);
  const restore = (kept: readonly Attachment[]) => {
    const stray: InputChip[] = [];
    for (const chip of picked.chips) {
      if (!kept.includes(chip)) continue;
      if (chip.kind === "selection" && tabSelection(slotId).seq === seq)
        setTabSelection(slotId, (sel) => ({ ...sel, hidden: hiddenBefore }));
      else if (chip.kind === "diagnostics" && !useAi.getState().diagnostics[slotId])
        useAi.setState((st) => ({ diagnostics: { ...st.diagnostics, [slotId]: chip } }));
      else stray.push(chip);
    }
    return stray;
  };
  return { message: { typed, chips: picked.chips, extras: picked.extras, restore }, left: picked.left };
}

/**
 * `inputMessage`, which then leaves the input area: the box empties, and the chips it took hide. Its pastes and
 * files go back to whichever slot shows the conversation by then.
 */
export function takeInput(slotId: string, typed: string, beside: QueuedMessage | null = null): QueuedMessage {
  const { message, left } = inputMessage(slotId, typed, beside);
  if (message.chips.some((c) => c.kind === "selection")) hideSelection(slotId);
  if (message.chips.some((c) => c.kind === "diagnostics")) removeDiagnostics(slotId);
  patchSlot(slotId, { draft: "", extras: left });
  return message;
}

/**
 * Puts a message that was not sent back into the slot's input area, before what it holds (§13.1): its chips in
 * their places, or, where another chip of their kind took that place meanwhile, with its pastes and files.
 */
export function putBack(slotId: string, message: QueuedMessage) {
  // A conversation in the background (AI-07) keeps its chips in its own input area: their places are its tab's.
  const chips = inBackground(slotId) ? message.chips : message.restore(message.chips);
  const stray = chips.map((attachment) => ({ id: extraId(), attachment }));
  patchSlot(slotId, (s) => messageBack(s, { ...message, extras: [...stray, ...message.extras] }));
}

/** Whether the input area's message, joined to the queued one, keeps its attachments within the cap (AI-35). */
export function joinFits(slotId: string, typed: string, queued: QueuedMessage | null): boolean {
  if (!queued || !typed || withinCap(joinMessages(queued, inputMessage(slotId, typed, queued).message))) return true;
  toast(tooMuchMessage(), "error");
  return false;
}

/**
 * Enter while a turn runs (§13.1): the message waits for the turn to end, with what it takes from the input
 * area, and shows above the input box. A second one joins it, so one message goes.
 */
export function queueMessage(slotId: string, raw: string): boolean {
  const typed = raw.trim();
  const queued = getSlot(slotId).queued;
  if (!typed || !joinFits(slotId, typed, queued)) return false;
  const taken = takeInput(slotId, typed, queued);
  patchSlot(slotId, { queued: queued ? joinMessages(queued, taken) : taken });
  return true;
}

/** The slot's queued message goes back into its input area, unsent, before what that holds (§13.1). */
export function giveBackQueued(slotId: string) {
  const queued = getSlot(slotId).queued;
  if (!queued) return;
  patchSlot(slotId, { queued: null });
  putBack(slotId, queued);
}

/** **Edit** on the queued message, or ↑ in the empty input box: it goes back into the box to be changed (§13.1). */
export function editQueued(slotId: string) {
  giveBackQueued(slotId);
  focusInput();
}

/** × on the queued message: it goes, with what it took from the input area. */
export function discardQueued(slotId: string) {
  patchSlot(slotId, { queued: null });
}

/** × on an attachment of the queued message. */
export function removeQueuedAttachment(slotId: string, attachment: Attachment) {
  patchSlot(slotId, (s) =>
    s.queued
      ? {
          queued: {
            ...s.queued,
            chips: s.queued.chips.filter((c) => c !== attachment),
            extras: s.queued.extras.filter((x) => x.attachment !== attachment),
          },
        }
      : {},
  );
}
