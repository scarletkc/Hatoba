import type { AiTurnEndReason } from "@/ipc/types";
import { attachmentBytes, composeMessage, MESSAGE_MAX_BYTES, type Attachment } from "./attachments";
import type { InputChip, PendingAttachment, QueuedMessage, Slot } from "./store";

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
 * in the order it was written: the texts a blank line apart, and the pastes and files in turn. The
 * diagnostics and the selection go before them, as in any message (`orderAttachments`), and the skill or
 * prompt of a `/` command that both ran goes once (AI-50).
 */
export function joinMessages(first: QueuedMessage, then: QueuedMessage): QueuedMessage {
  // A `/` command run in both keeps the later run's skill or prompt (AI-50).
  const rerun = new Set(then.extras.flatMap((x) => (x.command ? [x.command] : [])));
  return {
    typed: [first.typed, then.typed].filter(Boolean).join("\n\n"),
    chips: [...first.chips, ...then.chips],
    extras: [...first.extras.filter((x) => !x.command || !rerun.has(x.command)), ...then.extras],
    restore: (kept) => [...first.restore(kept), ...then.restore(kept)],
  };
}

/**
 * What a message takes of the input area (AI-10, AI-35): at most one set of diagnostics and one selection, a
 * chip a message gave back (in `extras`, §13.1) before the tab's own (`live`), and none of a kind the message
 * it joins has already (`beside`); then the pastes and files. `left` stays in the input area.
 */
export function pickInput(
  live: readonly InputChip[],
  extras: readonly PendingAttachment[],
  beside: readonly Attachment[] = [],
): { chips: InputChip[]; extras: PendingAttachment[]; left: PendingAttachment[] } {
  const kinds = new Set(beside.map((a) => a.kind));
  const taken: PendingAttachment[] = [];
  const left: PendingAttachment[] = [];
  for (const x of extras) {
    const kind = x.attachment.kind;
    if (kind === "selection" || kind === "diagnostics") {
      if (kinds.has(kind)) {
        left.push(x);
        continue;
      }
      kinds.add(kind);
    }
    taken.push(x);
  }
  return { chips: live.filter((c) => !kinds.has(c.kind)), extras: taken, left };
}

/** Of the attachments the input area shows, those a message joining `queued` takes, as `pickInput` picks them. */
export function takenBeside(attachments: readonly Attachment[], queued: QueuedMessage | null): Attachment[] {
  const kinds = new Set(queued ? messageAttachments(queued).map((a) => a.kind) : []);
  return attachments.filter((a) => {
    if (a.kind !== "selection" && a.kind !== "diagnostics") return true;
    if (kinds.has(a.kind)) return false;
    kinds.add(a.kind);
    return true;
  });
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
