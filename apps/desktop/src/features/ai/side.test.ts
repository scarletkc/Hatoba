import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AiConversationView, AiEntryView, AiProviderView, AiSideAnswer, AiSideEvent, AiSideQuestionInput } from "@/ipc/types";
import type { PasteAttachment } from "./attachments";
import { newTurn } from "./turn";

/** The calls the actions make, answered by each test. */
const side = vi.hoisted(() => ({
  question: vi.fn<(input: AiSideQuestionInput, onEvent: (event: AiSideEvent) => void) => Promise<AiSideAnswer>>(),
  stop: vi.fn<(conversationId: string) => Promise<void>>(),
}));
vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: { ai_side_question: side.question, ai_side_stop: side.stop },
}));
// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { askSideQuestion, dismissSideQuestion, retrySideQuestion, stopSideQuestion } = await import("./actions");
const { approvalCall, canAskSide, newSideQuestion, reduceSideEvent, sideRunning } = await import("./side");
const { blankSlot, getSlot, HOME_SLOT, patchSlot, useAi } = await import("./store");
type Slot = import("./store").Slot;

const view: AiConversationView = {
  id: "c1",
  title: "nginx 502",
  host_id: null,
  quick_target: null,
  pinned: false,
  context_start: null,
  effort: null,
  created_at: 0,
  updated_at: 0,
  last_activity: 0,
};
const entries: AiEntryView[] = [{ role: "user", entry_id: "e1", created_at: 0, text: "Why does nginx return 502?" }];
const provider: AiProviderView = {
  id: "p1",
  name: "Mock",
  protocol: "chat_completions",
  base_url: "http://127.0.0.1/v1",
  has_api_key: false,
  auth_header: "x-api-key",
  models: [{ id: "m1", name: "Model 1", context_window: null, max_output_tokens: null, efforts: null, adaptive_thinking: null }],
  updated_at: 0,
};
const paste: PasteAttachment = { kind: "paste", text: "error.log line", lines: 1 };

function slot(patch: Partial<Slot> = {}): Slot {
  return { ...blankSlot("manual"), conversationId: view.id, conversation: view, entries, ...patch };
}

/** A side question whose answer the test hands out through `answer` and `fail`. */
function pending() {
  let events: (event: AiSideEvent) => void = () => {};
  let input: AiSideQuestionInput | null = null;
  let settle!: { resolve: (a: AiSideAnswer) => void; reject: (e: unknown) => void };
  side.question.mockImplementationOnce((i, onEvent) => {
    input = i;
    events = onEvent;
    return new Promise((resolve, reject) => (settle = { resolve, reject }));
  });
  return {
    input: () => input,
    emit: (event: AiSideEvent) => events(event),
    answer: (text: string) => settle.resolve({ text, finish: "stop" }),
    fail: (code: string, detail: string) => settle.reject({ code, detail }),
  };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  side.question.mockReset();
  side.stop.mockReset().mockResolvedValue(undefined);
  useAi.setState({
    slots: { [HOME_SLOT]: slot() },
    catalog: { providers: [provider], settings: null, loaded: true },
    selections: {},
    diagnostics: {},
  });
});

describe("the side question's state (AI-42)", () => {
  const q = newSideQuestion(1, "c1", "Is it safe?", "Is it safe?");

  it("streams reasoning, then the answer", () => {
    const thinking = reduceSideEvent(q, { kind: "reasoning", delta: "Hmm" });
    expect(thinking.phase).toBe("thinking");
    const streaming = reduceSideEvent(reduceSideEvent(thinking, { kind: "text", delta: "It " }), { kind: "text", delta: "is." });
    expect(streaming).toMatchObject({ phase: "streaming", answer: "It is." });
    expect(reduceSideEvent(streaming, { kind: "reasoning", delta: "more" }).phase).toBe("streaming");
  });

  it("drops what streamed when the question goes again without tools", () => {
    const streamed = reduceSideEvent(q, { kind: "text", delta: "Checking." });
    expect(reduceSideEvent(streamed, { kind: "restarted" })).toMatchObject({ phase: "waiting", answer: "" });
  });

  it("ignores events once the question has stopped", () => {
    const stopped = { ...q, phase: "stopped" as const };
    expect(reduceSideEvent(stopped, { kind: "text", delta: "late" })).toBe(stopped);
    expect(sideRunning(stopped)).toBe(false);
    expect(sideRunning(q)).toBe(true);
  });

  it("needs a stored conversation that is not compacting, and a question", () => {
    expect(canAskSide(slot(), "why?")).toBe(true);
    expect(canAskSide(slot(), "  ")).toBe(false);
    expect(canAskSide(slot({ conversationId: null }), "why?")).toBe(false);
    expect(canAskSide(slot({ compacting: true }), "why?")).toBe(false);
  });

  it("names the call whose approval card waits", () => {
    expect(approvalCall(slot({ turn: { ...newTurn(), call: { id: "c9", state: "approval" } } }))).toBe("c9");
    expect(approvalCall(slot({ turn: { ...newTurn(), call: { id: "c9", state: "running" } } }))).toBeNull();
    expect(approvalCall(slot())).toBeNull();
  });
});

describe("askSideQuestion (AI-42)", () => {
  it("takes the input area and its attachments, names the waiting call, and leaves the turn alone", async () => {
    const turn = { ...newTurn(), phase: "tools" as const, call: { id: "call_1", state: "approval" as const } };
    patchSlot(HOME_SLOT, { turn, draft: "  Is it safe?  ", extras: [{ id: "x1", attachment: paste }] });
    const p = pending();
    expect(askSideQuestion(HOME_SLOT, getSlot(HOME_SLOT).draft)).toBe(true);

    expect(p.input()).toMatchObject({ conversation_id: "c1", approval_call_id: "call_1", context: { provider_id: "p1", model_id: "m1" } });
    expect(p.input()?.text).toMatch(/^<pasted_text lines="1">\nerror\.log line\n<\/pasted_text>\n\nIs it safe\?$/);
    expect(getSlot(HOME_SLOT)).toMatchObject({ draft: "", extras: [], turn, entries });
    expect(getSlot(HOME_SLOT).side).toMatchObject({ question: "Is it safe?", phase: "waiting" });

    p.emit({ kind: "text", delta: "It deletes the build." });
    expect(getSlot(HOME_SLOT).side).toMatchObject({ phase: "streaming", answer: "It deletes the build." });
    p.answer("It deletes the build.");
    await flush();
    expect(getSlot(HOME_SLOT).side).toMatchObject({ phase: "done", answer: "It deletes the build.", finish: "stop" });
    expect(getSlot(HOME_SLOT)).toMatchObject({ turn, entries });
  });

  it("asks only the card's text from an approval card, and keeps the input area", () => {
    patchSlot(HOME_SLOT, { draft: "half a message", extras: [{ id: "x1", attachment: paste }] });
    const p = pending();
    expect(askSideQuestion(HOME_SLOT, "What will this do?", false)).toBe(true);
    expect(p.input()?.text).toBe("What will this do?");
    expect(getSlot(HOME_SLOT)).toMatchObject({ draft: "half a message", extras: [{ id: "x1" }] });
  });

  it("is refused without a stored conversation, while compacting, or with nothing to ask", () => {
    patchSlot(HOME_SLOT, { compacting: true });
    expect(askSideQuestion(HOME_SLOT, "why?")).toBe(false);
    patchSlot(HOME_SLOT, { compacting: false, conversationId: null });
    expect(askSideQuestion(HOME_SLOT, "why?")).toBe(false);
    expect(side.question).not.toHaveBeenCalled();
  });

  it("stops only the side question, and × discards it", async () => {
    const turn = newTurn();
    patchSlot(HOME_SLOT, { turn });
    const p = pending();
    askSideQuestion(HOME_SLOT, "why?");
    p.emit({ kind: "text", delta: "Because" });
    stopSideQuestion(HOME_SLOT);
    expect(side.stop).toHaveBeenCalledWith("c1");
    expect(getSlot(HOME_SLOT).side).toMatchObject({ phase: "stopped", answer: "Because" });
    expect(getSlot(HOME_SLOT).turn).toBe(turn);
    // What Rust sends after the stop changes nothing.
    p.emit({ kind: "text", delta: " late" });
    p.fail("cancelled", "the side question was stopped");
    await flush();
    expect(getSlot(HOME_SLOT).side).toMatchObject({ phase: "stopped", answer: "Because" });

    dismissSideQuestion(HOME_SLOT);
    expect(getSlot(HOME_SLOT).side).toBeNull();
  });

  it("replaces a side question still on its way", async () => {
    const first = pending();
    askSideQuestion(HOME_SLOT, "first");
    const second = pending();
    askSideQuestion(HOME_SLOT, "second");
    first.emit({ kind: "text", delta: "old" });
    first.fail("cancelled", "the side question was stopped");
    second.emit({ kind: "text", delta: "new" });
    await flush();
    expect(getSlot(HOME_SLOT).side).toMatchObject({ question: "second", phase: "streaming", answer: "new" });
  });

  it("shows a failure, and Retry asks the same question again", async () => {
    patchSlot(HOME_SLOT, { extras: [{ id: "x1", attachment: paste }] });
    const p = pending();
    askSideQuestion(HOME_SLOT, "why?");
    const sent = p.input()?.text;
    p.fail("ai", "Overloaded");
    await flush();
    expect(getSlot(HOME_SLOT).side).toMatchObject({ phase: "error" });
    expect(getSlot(HOME_SLOT).side?.error).toBeTruthy();

    const again = pending();
    retrySideQuestion(HOME_SLOT);
    expect(again.input()?.text).toBe(sent);
    expect(getSlot(HOME_SLOT).side).toMatchObject({ question: "why?", phase: "waiting" });
  });
});
