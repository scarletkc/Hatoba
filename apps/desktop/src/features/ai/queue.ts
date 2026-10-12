import type { AiTurnEndReason } from "@/ipc/types";
import { attachmentBytes, composeMessage, MESSAGE_MAX_BYTES, type Attachment } from "./attachments";
import type { QueuedMessage, Slot } from "./store";

/** What a message attaches, in the order it is stored (AI-10, AI-35). */
export function messageAttachments(m: QueuedMessage): Attachment[] {
  return [...m.chips, ...m.extras.map((x) => x.attachment)];
}

/** The user entry's text the message is sent as. */
export function messageText(m: QueuedMessage): string {
  return composeMessage(m.typed, messageAttachments(m));
}

/** Whether the message's attachments stay within the per-message cap (AI-35). */
export function withinCap(m: QueuedMessage): boolean {
  return messageAttachments(m).reduce((n, a) => n + attachmentBytes(a), 0) <= MESSAGE_MAX_BYTES;
}

/**
 * §13.1: a second message queued, or the input area sent with the queued message, makes one message
 * in the order it was written: the texts a blank line apart, and the attachments in turn.
 */
export function joinMessages(first: QueuedMessage, then: QueuedMessage): QueuedMessage {
  return {
    typed: [first.typed, then.typed].filter(Boolean).join("\n\n"),
    chips: [...first.chips, ...then.chips],
    extras: [...first.extras, ...then.extras],
    restore: (kept) => {
      first.restore(kept);
      then.restore(kept);
    },
  };
}

/**
 * Of the input area's attachments, those a message joining `queued` takes: a message takes at most one
 * selection and one set of diagnostics (AI-10), so a chip of a kind the queued message has stays.
 */
export function takenBeside(attachments: Attachment[], queued: QueuedMessage | null): Attachment[] {
  if (!queued) return attachments;
  return attachments.filter((a) => (a.kind !== "selection" && a.kind !== "diagnostics") || !queued.chips.some((c) => c.kind === a.kind));
}

/** A message going back into the input area comes before what it holds: its text first, its pastes and files first. */
export function messageBack(s: Pick<Slot, "draft" | "extras">, m: QueuedMessage): Pick<Slot, "draft" | "extras"> {
  return { draft: s.draft.trim() ? `${m.typed}\n\n${s.draft}` : m.typed, extras: [...m.extras, ...s.extras] };
}

/**
 * §13.1: the queued message goes once the turn completes, or once the user stopped it (Stop or Esc).
 * Any other end goes back into the input area, unsent.
 */
export function sendsQueued(reason: AiTurnEndReason, stoppedByUser: boolean): boolean {
  return reason === "completed" || (reason === "stopped" && stoppedByUser);
}
