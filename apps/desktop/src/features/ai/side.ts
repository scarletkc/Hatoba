import type { AiFinish, AiSideEvent } from "@/ipc/types";
import type { Slot } from "./store";
import { approvalOf } from "./turn";

/**
 * A side question (AI-42) and its answer as the panel shows them, at the end of the conversation.
 * Nothing stores either: closing the bubble, a newer side question, a message or another conversation
 * in the slot discards them.
 */
export interface SideQuestion {
  /** Tells a question's events from those of the one that replaced it. */
  id: number;
  conversationId: string;
  /** What the user typed, without the attachments. */
  question: string;
  /** What went to Rust: the attachment blocks the input area added, then the typed text (spec §13.3). */
  sent: string;
  /** `waiting`: nothing has streamed yet; `thinking`: only reasoning has. */
  phase: "waiting" | "thinking" | "streaming" | "done" | "stopped" | "error";
  answer: string;
  /** How the answer ended, once it has. */
  finish: AiFinish | null;
  /** Why the question failed. */
  error: string | null;
}

export function newSideQuestion(id: number, conversationId: string, question: string, sent: string): SideQuestion {
  return { id, conversationId, question, sent, phase: "waiting", answer: "", finish: null, error: null };
}

/** Applies one streamed event. `restarted`: the answer called tools, so Rust asks again without them and what streamed goes. */
export function reduceSideEvent(q: SideQuestion, ev: AiSideEvent): SideQuestion {
  if (!sideRunning(q)) return q;
  switch (ev.kind) {
    case "reasoning":
      return q.phase === "waiting" ? { ...q, phase: "thinking" } : q;
    case "text":
      return { ...q, phase: "streaming", answer: q.answer + ev.delta };
    case "restarted":
      return { ...q, phase: "waiting", answer: "" };
  }
}

/** The answer is on its way, so Stop and Esc reach it. */
export function sideRunning(q: SideQuestion | null | undefined): boolean {
  return !!q && (q.phase === "waiting" || q.phase === "thinking" || q.phase === "streaming");
}

/** Whether the slot can ask a side question: a stored conversation that is not compacting, and something to ask. */
export function canAskSide(slot: Slot, text: string): boolean {
  return !!slot.conversationId && !slot.compacting && !slot.loading && !!text.trim();
}

/** The call whose approval card waits, which the request tells the model about (AI-17). */
export function approvalCall(slot: Slot): string | null {
  return approvalOf(slot.turn)?.id ?? null;
}
