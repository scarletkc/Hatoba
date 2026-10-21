import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AiConversationDetail, AiConversationView, AiEntryView, AiProviderView, AiSendInput, AiSendStarted, AiTurnEndReason, AiTurnEvent } from "@/ipc/types";
import type { PasteAttachment } from "./attachments";

/** The calls the actions make, answered by each test. */
const ipc = vi.hoisted(() => ({
  send: vi.fn<(input: AiSendInput, onEvent: (event: AiTurnEvent) => void) => Promise<AiSendStarted>>(),
  stop: vi.fn<(conversationId: string) => Promise<void>>(),
  get: vi.fn<(id: string) => Promise<AiConversationDetail>>(),
}));
vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: { ai_send: ipc.send, ai_stop: ipc.stop, ai_conversation_get: ipc.get, notification_show: () => Promise.resolve() },
}));
// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const {
  askAiAboutConnection,
  discardQueued,
  editQueued,
  newConversation,
  noteSelection,
  queueMessage,
  reloadSlot,
  removeQueuedAttachment,
  resetForLock,
  selectionAttachment,
  sendMessage,
  stopTurn,
  tabSelection,
} = await import("./actions");
const { Composer } = await import("./Composer");
const { joinMessages, messageBack, sendsQueued, takenBeside } = await import("./queue");
const { blankSlot, getSlot, HOME_SLOT, patchSlot, useAi } = await import("./store");
const { newTurn } = await import("./turn");
type Slot = import("./store").Slot;
type QueuedMessage = import("./store").QueuedMessage;

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const view: AiConversationView = {
  id: "c1",
  title: "nginx 502",
  host_id: null,
  quick_target: null,
  origin: null,
      local: null,
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
const paste = (text: string): PasteAttachment => ({ kind: "paste", text, lines: 1 });
const TAB = "tab-1";

function slot(patch: Partial<Slot> = {}): Slot {
  return { ...blankSlot("manual"), conversationId: view.id, conversation: view, entries, ...patch };
}

/** Each `ai_send` in order: the text it sent, and its channel, on which the test runs the turn. */
function turns() {
  const sent: { text: string; emit: (event: AiTurnEvent) => void }[] = [];
  ipc.send.mockImplementation(async (input, onEvent) => {
    sent.push({ text: input.text, emit: onEvent });
    return { conversation: view, user_entry: { role: "user", entry_id: `u${sent.length}`, created_at: 0, text: input.text } };
  });
  return sent;
}

/** Sends `text` and lets its turn get going, so that it can be stopped. */
async function running(sent: ReturnType<typeof turns>, slotId = HOME_SLOT, text = "first") {
  await sendMessage(slotId, text);
  sent[sent.length - 1].emit({ kind: "request_started" });
}

const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  ipc.send.mockReset();
  ipc.stop.mockReset().mockResolvedValue(undefined);
  ipc.get.mockReset().mockResolvedValue({ conversation: view, entries, running: false, file_changes: [] });
  useAi.setState({
    slots: { [HOME_SLOT]: slot(), [TAB]: slot({ conversationId: "c2", conversation: { ...view, id: "c2" } }) },
    catalog: { providers: [provider], settings: null, loaded: true },
    history: null,
    selections: {},
    diagnostics: {},
  });
});

describe("joining queued messages (§13.1)", () => {
  const message = (typed: string, extras: string[] = []): QueuedMessage => ({
    typed,
    chips: [],
    extras: extras.map((id) => ({ id, attachment: paste(id) })),
    restore: vi.fn(() => []),
  });

  it("makes one message in the order it was written", () => {
    const a = message("check the logs", ["x1"]);
    const b = message("and restart it", ["x2"]);
    const joined = joinMessages(a, b);
    expect(joined).toMatchObject({ typed: "check the logs\n\nand restart it", extras: [{ id: "x1" }, { id: "x2" }] });
    joined.restore([]);
    expect(a.restore).toHaveBeenCalledWith([]);
    expect(b.restore).toHaveBeenCalledWith([]);
  });

  it("goes back into the input area before what it holds", () => {
    const m = message("queued", ["x1"]);
    expect(messageBack({ draft: "", extras: [] }, m)).toMatchObject({ draft: "queued", extras: [{ id: "x1" }] });
    expect(messageBack({ draft: "typed since", extras: m.extras.map((x) => ({ ...x, id: "x2" })) }, m)).toMatchObject({
      draft: "queued\n\ntyped since",
      extras: [{ id: "x1" }, { id: "x2" }],
    });
  });

  it("leaves the input area's chip of a kind the queued message has", () => {
    const selection = { kind: "selection" as const, host: "", lines: 1, text: "502", truncated: false };
    const diagnostics = { kind: "diagnostics" as const, host: "prod", text: "timeout" };
    const queued = { ...message("why?"), chips: [selection] };
    const input = [diagnostics, { ...selection, text: "504" }, paste("log")];
    expect(takenBeside(input, null)).toEqual(input);
    expect(takenBeside(input, queued)).toEqual([diagnostics, paste("log")]);
  });

  it("is sent after a turn that completed or that the user stopped", () => {
    expect(sendsQueued("completed", false)).toBe(true);
    expect(sendsQueued("stopped", true)).toBe(true);
    expect(sendsQueued("stopped", false)).toBe(false);
    for (const reason of ["error", "length", "refused", "empty"] as const) expect(sendsQueued(reason, true)).toBe(false);
  });
});

describe("the queued message (§13.1)", () => {
  it("is sent once, in order, when the turn ends", async () => {
    const sent = turns();
    await running(sent);
    patchSlot(HOME_SLOT, { extras: [{ id: "x1", attachment: paste("error.log") }] });
    expect(queueMessage(HOME_SLOT, " check the logs ")).toBe(true);
    expect(queueMessage(HOME_SLOT, "then restart it")).toBe(true);
    expect(getSlot(HOME_SLOT)).toMatchObject({ draft: "", extras: [], queued: { typed: "check the logs\n\nthen restart it" } });
    expect(ipc.send).toHaveBeenCalledTimes(1);

    patchSlot(HOME_SLOT, { draft: "still typing" });
    sent[0].emit({ kind: "turn_ended", reason: "completed" });
    await flush();
    expect(sent.map((x) => x.text)).toEqual(["first", '<pasted_text lines="1">\nerror.log\n</pasted_text>\n\ncheck the logs\n\nthen restart it']);
    expect(getSlot(HOME_SLOT)).toMatchObject({ queued: null, draft: "still typing" });

    sent[1].emit({ kind: "request_started" });
    sent[1].emit({ kind: "turn_ended", reason: "completed" });
    await flush();
    expect(ipc.send).toHaveBeenCalledTimes(2);
  });

  it("is sent once Stop or Esc has stopped the turn", async () => {
    const sent = turns();
    await running(sent);
    queueMessage(HOME_SLOT, "do this instead");
    await stopTurn(HOME_SLOT);
    expect(ipc.stop).toHaveBeenCalledWith("c1");
    expect(ipc.send).toHaveBeenCalledTimes(1);

    sent[0].emit({ kind: "turn_ended", reason: "stopped" });
    await flush();
    await flush();
    expect(ipc.get).toHaveBeenCalledWith("c1");
    expect(sent[1]?.text).toBe("do this instead");
    expect(getSlot(HOME_SLOT).queued).toBeNull();
  });

  it("goes back into the input area, unsent, when the turn ends any other way", async () => {
    for (const reason of ["error", "length", "refused", "empty"] satisfies AiTurnEndReason[]) {
      useAi.setState({ slots: { [HOME_SLOT]: slot() } });
      ipc.send.mockReset();
      const sent = turns();
      await running(sent);
      patchSlot(HOME_SLOT, { extras: [{ id: "x1", attachment: paste("a") }] });
      queueMessage(HOME_SLOT, "then this");
      // Typed and added while it waited.
      patchSlot(HOME_SLOT, { draft: "and this", extras: [{ id: "x2", attachment: paste("b") }] });

      sent[0].emit({ kind: "turn_ended", reason });
      await flush();
      expect(ipc.send).toHaveBeenCalledTimes(1);
      expect(getSlot(HOME_SLOT)).toMatchObject({ queued: null, draft: "then this\n\nand this", extras: [{ id: "x1" }, { id: "x2" }] });
    }
  });

  it("goes back into the input area when the vault locks, with its chips", async () => {
    useAi.setState({ slots: { [TAB]: slot({ conversationId: "c2", conversation: { ...view, id: "c2" } }) }, selections: {}, diagnostics: {} });
    const sent = turns();
    await running(sent, TAB);
    noteSelection(TAB, "502 Bad Gateway");
    queueMessage(TAB, "why?");
    expect(getSlot(TAB).queued?.chips).toMatchObject([{ kind: "selection", text: "502 Bad Gateway" }]);
    expect(tabSelection(TAB).hidden).not.toBeNull();

    resetForLock();
    await flush();
    expect(getSlot(TAB)).toMatchObject({ queued: null, draft: "why?" });
    expect(tabSelection(TAB).hidden).toBeNull();
    expect(ipc.send).toHaveBeenCalledTimes(1);
  });

  it("stays with its conversation when another one is shown, and goes with its chips when the turn ends (AI-07)", async () => {
    useAi.setState({ slots: { [TAB]: slot({ conversationId: "c2", conversation: { ...view, id: "c2" } }) }, background: {}, selections: {}, diagnostics: {} });
    const sent = turns();
    await running(sent, TAB);
    noteSelection(TAB, "502 Bad Gateway");
    queueMessage(TAB, "why?");

    newConversation(TAB);
    expect(getSlot(TAB)).toMatchObject({ conversationId: null, queued: null, draft: "" });
    expect(tabSelection(TAB).hidden).not.toBeNull();
    sent[0].emit({ kind: "turn_ended", reason: "completed" });
    await flush();
    expect(sent[1].text).toMatch(/^<terminal_selection [^]*502 Bad Gateway[^]*why\?$/);
    expect(getSlot(TAB).draft).toBe("");
  });

  it("leaves out, when it goes back, a chip removed from it", async () => {
    const sent = turns();
    await running(sent, TAB);
    noteSelection(TAB, "502 Bad Gateway");
    askAiAboutConnection(TAB, "prod-api", "kind: timeout");
    queueMessage(TAB, "why?");
    for (const chip of getSlot(TAB).queued?.chips ?? []) removeQueuedAttachment(TAB, chip);
    editQueued(TAB);
    expect(getSlot(TAB)).toMatchObject({ queued: null, draft: "why?" });
    expect(selectionAttachment(TAB)).toBeNull();
    expect(useAi.getState().diagnostics[TAB]).toBeUndefined();
  });

  it("keeps its chips when it goes back after the tab's selection or diagnostics changed", async () => {
    for (const change of [() => noteSelection(TAB, "second error"), () => noteSelection(TAB, "")]) {
      useAi.setState({ slots: { [TAB]: slot({ conversationId: "c2", conversation: { ...view, id: "c2" } }) }, selections: {}, diagnostics: {} });
      ipc.send.mockReset();
      const sent = turns();
      await running(sent, TAB);
      noteSelection(TAB, "first error");
      askAiAboutConnection(TAB, "prod-api", "kind: timeout");
      queueMessage(TAB, "why?");
      change();
      askAiAboutConnection(TAB, "prod-api", "kind: refused");
      editQueued(TAB);

      // Their places are taken, so they come back beside the pastes and files, and go with the next message.
      expect(getSlot(TAB).extras.map((x) => x.attachment)).toMatchObject([
        { kind: "diagnostics", text: expect.stringContaining("timeout") },
        { kind: "selection", text: "first error" },
      ]);
      expect(useAi.getState().diagnostics[TAB]).toMatchObject({ text: expect.stringContaining("refused") });
      sent[0].emit({ kind: "turn_ended", reason: "completed" });
      await flush();
      await sendMessage(TAB, getSlot(TAB).draft);
      expect(sent[1].text).toMatch(/^<connection_diagnostics [^]*timeout[^]*<terminal_selection [^]*first error[^]*why\?$/);
      // The tab's own chips stay for a later message.
      expect(useAi.getState().diagnostics[TAB]).toMatchObject({ text: expect.stringContaining("refused") });
      expect(getSlot(TAB).extras).toEqual([]);
    }
    expect(selectionAttachment(TAB)).toBeNull();
    noteSelection(TAB, "third error");
    expect(selectionAttachment(TAB)).toMatchObject({ text: "third error" });
  });

  it("is sent once Stop has stopped a turn an earlier page started, and goes back when that turn ends on its own", async () => {
    const sent = turns();
    patchSlot(HOME_SLOT, { remoteRunning: true });
    queueMessage(HOME_SLOT, "after the remote turn");
    // The poll sees the turn end while the stop is on its way.
    ipc.stop.mockImplementationOnce(() => reloadSlot(HOME_SLOT));
    await stopTurn(HOME_SLOT);
    expect(sent.map((x) => x.text)).toEqual(["after the remote turn"]);

    useAi.setState({ slots: { [HOME_SLOT]: slot({ remoteRunning: true }) } });
    queueMessage(HOME_SLOT, "after the remote turn");
    await reloadSlot(HOME_SLOT);
    expect(getSlot(HOME_SLOT)).toMatchObject({ remoteRunning: false, queued: null, draft: "after the remote turn" });
    expect(ipc.send).toHaveBeenCalledTimes(1);
  });

  it("takes at most one selection: a second one stays in the input area", async () => {
    const sent = turns();
    await running(sent, TAB);
    noteSelection(TAB, "first error");
    queueMessage(TAB, "what is this?");
    noteSelection(TAB, "second error");
    queueMessage(TAB, "and this?");
    expect(getSlot(TAB).queued?.chips).toMatchObject([{ kind: "selection", text: "first error" }]);
    expect(selectionAttachment(TAB)).toMatchObject({ text: "second error" });
  });

  it("goes back into the box with Edit, and × discards it", async () => {
    const sent = turns();
    await running(sent);
    queueMessage(HOME_SLOT, "check the logs");
    patchSlot(HOME_SLOT, { draft: "more" });
    editQueued(HOME_SLOT);
    expect(getSlot(HOME_SLOT)).toMatchObject({ queued: null, draft: "check the logs\n\nmore" });

    queueMessage(HOME_SLOT, getSlot(HOME_SLOT).draft);
    discardQueued(HOME_SLOT);
    sent[0].emit({ kind: "turn_ended", reason: "completed" });
    await flush();
    expect(getSlot(HOME_SLOT)).toMatchObject({ queued: null, draft: "" });
    expect(ipc.send).toHaveBeenCalledTimes(1);
  });

  it("waits for the turn while a message is still on its way", async () => {
    let started!: (s: AiSendStarted) => void;
    ipc.send.mockImplementationOnce(() => new Promise((resolve) => (started = resolve)));
    void sendMessage(HOME_SLOT, "first");
    // Ctrl+Enter cannot stop a turn that has not started: the message is queued.
    expect(await sendMessage(HOME_SLOT, "second")).toBe(true);
    expect(ipc.send).toHaveBeenCalledTimes(1);
    expect(getSlot(HOME_SLOT).queued?.typed).toBe("second");
    started({ conversation: view, user_entry: { role: "user", entry_id: "u1", created_at: 0, text: "first" } });
  });

  it("goes back after the message it waited for, when that one was stopped before it was stored", async () => {
    let refuse!: (e: unknown) => void;
    ipc.send.mockImplementationOnce(() => new Promise((_, reject) => (refuse = reject)));
    const first = sendMessage(HOME_SLOT, "first");
    queueMessage(HOME_SLOT, "second");
    patchSlot(HOME_SLOT, { draft: "third" });
    refuse({ code: "cancelled", detail: "the message was stopped" });
    expect(await first).toBe(false);
    expect(getSlot(HOME_SLOT)).toMatchObject({ turn: null, queued: null, draft: "first\n\nsecond\n\nthird" });
  });
});

describe("Ctrl+Enter during a turn (§13.1)", () => {
  it("stops the turn and sends the queued message with the box at once", async () => {
    const sent = turns();
    await running(sent, HOME_SLOT, "run the build");
    sent[0].emit({
      kind: "entry",
      entry: {
        role: "assistant",
        entry_id: "a1",
        created_at: 0,
        provider_id: "p1",
        model_id: "m1",
        text: "",
        reasoning: null,
        tool_calls: [{ id: "call_1", name: "run_command", arguments: "{}" }],
        finish: "tool_calls",
        usage: null,
      },
    });
    queueMessage(HOME_SLOT, "check the logs");
    patchSlot(HOME_SLOT, { draft: "now" });

    expect(await sendMessage(HOME_SLOT, "now")).toBe(true);
    expect(sent.map((x) => x.text)).toEqual(["run the build", "check the logs\n\nnow"]);
    expect(getSlot(HOME_SLOT)).toMatchObject({ queued: null, draft: "" });

    // What the stopped turn sends after that no longer reaches the slot. Rust's cancelled result for its
    // call arrives with the new turn, before the new message.
    sent[0].emit({ kind: "turn_ended", reason: "stopped" });
    await flush();
    expect(getSlot(HOME_SLOT).turn).not.toBeNull();
    const cancelled: AiEntryView = { role: "tool", entry_id: "c9", created_at: 0, tool_call_id: "call_1", status: "cancelled", content: "cancelled", auto: null };
    sent[1].emit({ kind: "entry", entry: cancelled });
    const ids = getSlot(HOME_SLOT).entries.map((e) => e.entry_id);
    expect(ids.indexOf("c9")).toBeLessThan(ids.indexOf("u2"));
    expect(ipc.send).toHaveBeenCalledTimes(2);
  });
});

describe("the input area (§13.1)", () => {
  let unmount: (() => void) | null = null;
  afterEach(() => {
    act(() => unmount?.());
    unmount = null;
  });

  function Harness() {
    const s = useAi((st) => st.slots[HOME_SLOT]);
    return <Composer slotId={HOME_SLOT} slot={s} model={{ provider_id: "p1", model_id: "m1" }} tools={false} />;
  }

  function render(): HTMLTextAreaElement {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    act(() => root.render(<Harness />));
    unmount = () => {
      root.unmount();
      host.remove();
    };
    return host.querySelector("textarea")!;
  }

  const key = (box: HTMLTextAreaElement, init: KeyboardEventInit & { keyCode?: number }) =>
    act(() => {
      box.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
    });

  it("never queues or sends while an IME composes", () => {
    turns();
    patchSlot(HOME_SLOT, { turn: { ...newTurn(), phase: "streaming" }, draft: "日本語" });
    const box = render();
    for (const ime of [{ isComposing: true, keyCode: 13 }, { keyCode: 229 }]) {
      key(box, { key: "Enter", ...ime });
      key(box, { key: "Enter", ctrlKey: true, ...ime });
    }
    expect(getSlot(HOME_SLOT)).toMatchObject({ queued: null, draft: "日本語" });
    expect(ipc.send).not.toHaveBeenCalled();

    key(box, { key: "Enter", keyCode: 13 });
    expect(getSlot(HOME_SLOT)).toMatchObject({ queued: { typed: "日本語" }, draft: "" });
    // ↑ in the empty box takes it back.
    key(box, { key: "ArrowUp", keyCode: 38 });
    expect(getSlot(HOME_SLOT)).toMatchObject({ queued: null, draft: "日本語" });
    expect(ipc.send).not.toHaveBeenCalled();
  });

  it("sends with Enter and with Ctrl+Enter while no turn runs", async () => {
    const sent = turns();
    patchSlot(HOME_SLOT, { draft: "hello" });
    const box = render();
    key(box, { key: "Enter", keyCode: 13 });
    await act(flush);
    sent[0].emit({ kind: "turn_ended", reason: "completed" });
    await act(flush);
    act(() => patchSlot(HOME_SLOT, { draft: "again" }));
    key(box, { key: "Enter", ctrlKey: true, keyCode: 13 });
    await act(flush);
    expect(sent.map((x) => x.text)).toEqual(["hello", "again"]);
  });

  it("sends the queued message alone with the card's Send Now, leaving the box", async () => {
    const sent = turns();
    await running(sent);
    queueMessage(HOME_SLOT, "do this instead");
    patchSlot(HOME_SLOT, { draft: "still typing" });
    render();
    const now = document.querySelector<HTMLButtonElement>('button[aria-label="Send Now"]')!;
    expect(now.title).toBe("Stop and Send the Queued Message (Esc)");
    await act(async () => now.click());
    expect(ipc.stop).toHaveBeenCalledWith("c1");

    sent[0].emit({ kind: "turn_ended", reason: "stopped" });
    await act(flush);
    await act(flush);
    expect(sent.map((x) => x.text)).toEqual(["first", "do this instead"]);
    expect(getSlot(HOME_SLOT)).toMatchObject({ queued: null, draft: "still typing" });
  });
});
