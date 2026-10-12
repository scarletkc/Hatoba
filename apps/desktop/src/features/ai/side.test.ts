import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AiConversationView, AiEntryView, AiProviderView, AiSendStarted, AiSideAnswer, AiSideEvent, AiSideQuestionInput } from "@/ipc/types";
import type { PasteAttachment } from "./attachments";
import { newTurn } from "./turn";

/** The calls the actions make, answered by each test. */
const side = vi.hoisted(() => ({
  question: vi.fn<(input: AiSideQuestionInput, onEvent: (event: AiSideEvent) => void) => Promise<AiSideAnswer>>(),
  stop: vi.fn<(conversationId: string, questionId: number) => Promise<void>>(),
  send: vi.fn<() => Promise<AiSendStarted>>(),
}));
vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: { ai_side_question: side.question, ai_side_stop: side.stop, ai_send: side.send },
}));
// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { askAiAboutConnection, askSideQuestion, dismissSideQuestion, noteSelection, retrySideQuestion, sendMessage, stopSideQuestion, tabSelection } = await import("./actions");
const { approvalCall, canAskSide, newSideQuestion, reduceSideEvent, sideRunning } = await import("./side");
const { blankSlot, getSlot, HOME_SLOT, patchSlot, slotActivity, useAi } = await import("./store");
type Slot = import("./store").Slot;

const view: AiConversationView = {
  id: "c1",
  title: "nginx 502",
  host_id: null,
  quick_target: null,
  origin: null,
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
  side.send.mockReset().mockReturnValue(new Promise(() => {}));
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

  it("counts a call Auto's judge is checking as running, not waiting for the user (AI-47)", () => {
    const judging = slot({ turn: { ...newTurn(), call: { id: "c9", state: "judging" } } });
    expect(approvalCall(judging)).toBeNull();
    expect(slotActivity(judging)).toBe("running");
    expect(slotActivity(slot({ turn: { ...newTurn(), call: { id: "c9", state: "approval" } } }))).toBe("waiting");
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

  it("stops only the side question, by its number, and × discards it", async () => {
    const turn = newTurn();
    patchSlot(HOME_SLOT, { turn });
    const p = pending();
    askSideQuestion(HOME_SLOT, "why?");
    const id = p.input()?.question_id;
    expect(getSlot(HOME_SLOT).side?.id).toBe(id);
    p.emit({ kind: "text", delta: "Because" });
    stopSideQuestion(HOME_SLOT);
    expect(side.stop).toHaveBeenCalledWith("c1", id);
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

  it("stays stopped when a failure arrives after the stop", async () => {
    const p = pending();
    askSideQuestion(HOME_SLOT, "why?");
    stopSideQuestion(HOME_SLOT);
    p.fail("ai", "the provider is overloaded");
    await flush();
    expect(getSlot(HOME_SLOT).side).toMatchObject({ phase: "stopped", error: null });
  });

  it("replaces a side question still on its way, and keeps the input area as the newer one left it", async () => {
    const first = pending();
    askSideQuestion(HOME_SLOT, "first");
    const second = pending();
    askSideQuestion(HOME_SLOT, "second");
    expect(second.input()?.question_id).not.toBe(first.input()?.question_id);
    first.emit({ kind: "text", delta: "old" });
    first.fail("cancelled", "the side question was stopped");
    second.emit({ kind: "text", delta: "new" });
    await flush();
    expect(getSlot(HOME_SLOT).side).toMatchObject({ question: "second", phase: "streaming", answer: "new" });
    expect(getSlot(HOME_SLOT).draft).toBe("");
  });

  it("gives what it took back to the input area when it is refused, fails or is stopped", async () => {
    const TAB = "tab-1";
    const ends: ((p: ReturnType<typeof pending>) => void)[] = [
      (p) => p.fail("invalid_input", "the conversation is being compacted"),
      (p) => p.fail("ai", "the provider is overloaded"),
      () => stopSideQuestion(TAB),
      () => dismissSideQuestion(TAB),
    ];
    for (const end of ends) {
      useAi.setState({ slots: { [TAB]: slot() }, selections: {}, diagnostics: {} });
      noteSelection(TAB, "502 Bad Gateway");
      askAiAboutConnection(TAB, "prod-api", "kind: timeout");
      patchSlot(TAB, { draft: " why? ", extras: [{ id: "x1", attachment: paste }] });
      const p = pending();
      askSideQuestion(TAB, getSlot(TAB).draft);
      expect(p.input()?.text).toMatch(/^<connection_diagnostics [^]*<terminal_selection [^]*<pasted_text [^]*why\?$/);
      expect(getSlot(TAB)).toMatchObject({ draft: "", extras: [] });
      expect(useAi.getState().diagnostics[TAB]).toBeUndefined();
      expect(tabSelection(TAB).hidden).not.toBeNull();
      // Added while the answer was on its way.
      const later = { id: "x2", attachment: paste };
      patchSlot(TAB, { extras: [later] });

      end(p);
      await flush();
      expect(getSlot(TAB)).toMatchObject({ draft: " why? ", extras: [{ id: "x1" }, { id: "x2" }] });
      expect(useAi.getState().diagnostics[TAB]).toBeTruthy();
      expect(tabSelection(TAB).hidden).toBeNull();
    }
    // Text typed meanwhile stays; the question's attachments still come back.
    useAi.setState({ slots: { [TAB]: slot() }, selections: {}, diagnostics: {} });
    patchSlot(TAB, { extras: [{ id: "x1", attachment: paste }] });
    const p = pending();
    askSideQuestion(TAB, "why?");
    patchSlot(TAB, { draft: "next message" });
    p.fail("ai", "the provider is overloaded");
    await flush();
    expect(getSlot(TAB)).toMatchObject({ draft: "next message", extras: [{ id: "x1" }] });
    expect(getSlot(TAB).side).toMatchObject({ phase: "error" });
  });

  it("shows a failure without what streamed of it, and Retry asks the same question again", async () => {
    patchSlot(HOME_SLOT, { extras: [{ id: "x1", attachment: paste }] });
    const p = pending();
    askSideQuestion(HOME_SLOT, "why?");
    const sent = p.input()?.text;
    p.emit({ kind: "text", delta: "Let me check the logs." });
    p.fail("ai", "the model answered with tool calls instead of text");
    await flush();
    expect(getSlot(HOME_SLOT).side).toMatchObject({ phase: "error", answer: "" });
    expect(getSlot(HOME_SLOT).side?.error).toBeTruthy();

    const again = pending();
    retrySideQuestion(HOME_SLOT);
    expect(again.input()?.text).toBe(sent);
    expect(getSlot(HOME_SLOT).side).toMatchObject({ question: "why?", phase: "waiting" });
  });

  it("goes when a message is sent, stopping first while its answer is on its way", async () => {
    const p = pending();
    askSideQuestion(HOME_SLOT, "why?");
    const id = p.input()?.question_id;
    void sendMessage(HOME_SLOT, "restart nginx");
    expect(side.stop).toHaveBeenCalledWith("c1", id);
    expect(side.send).toHaveBeenCalled();
    expect(getSlot(HOME_SLOT).side).toBeNull();
    // The question was asked: its text does not come back into the input area.
    p.fail("cancelled", "the side question was stopped");
    await flush();
    expect(getSlot(HOME_SLOT)).toMatchObject({ draft: "", side: null });

    // A finished one just goes.
    const done = pending();
    patchSlot(HOME_SLOT, { turn: null });
    askSideQuestion(HOME_SLOT, "and now?");
    done.answer("Fine.");
    await flush();
    side.stop.mockClear();
    void sendMessage(HOME_SLOT, "thanks");
    expect(side.stop).not.toHaveBeenCalled();
    expect(getSlot(HOME_SLOT).side).toBeNull();
  });
});
