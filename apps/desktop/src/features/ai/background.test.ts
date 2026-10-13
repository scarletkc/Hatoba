import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionTab } from "@/app/tabs";
import type {
  AiConversationDetail,
  AiConversationView,
  AiEntryView,
  AiProviderView,
  AiSendInput,
  AiSendStarted,
  AiToolCall,
  AiToolResultInput,
  AiToolRun,
  AiTurnEvent,
} from "@/ipc/types";
import type { ToolOutcome } from "./terminalTools";
import { approvalOf } from "./turn";

/** The calls the actions make, answered by each test. */
const ipc = vi.hoisted(() => ({
  send: vi.fn<(input: AiSendInput, onEvent: (event: AiTurnEvent) => void) => Promise<AiSendStarted>>(),
  stop: vi.fn<(conversationId: string) => Promise<void>>(),
  get: vi.fn<(id: string) => Promise<AiConversationDetail>>(),
  run: vi.fn<(conversationId: string, toolCallId: string, sessionId: string | null, edited: string | null, judged: boolean) => Promise<AiToolRun>>(),
  result: vi.fn<(conversationId: string, toolCallId: string, result: AiToolResultInput) => Promise<AiEntryView>>(),
  sideStop: vi.fn<(conversationId: string, questionId: number) => Promise<void>>(),
  remove: vi.fn<(conversationId: string) => Promise<void>>(),
}));
vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: {
    ai_send: ipc.send,
    ai_stop: ipc.stop,
    ai_conversation_get: ipc.get,
    ai_tool_run: ipc.run,
    ai_tool_result: ipc.result,
    ai_side_stop: ipc.sideStop,
    ai_conversation_delete: ipc.remove,
    ai_allowances_list: () => Promise.resolve([]),
  },
}));
// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
// The tabs' terminals: connected, with an empty screen.
vi.mock("@/features/terminal/session", async (importActual) => {
  const active = { type: "normal", baseY: 0, length: 1, getLine: () => ({ isWrapped: false, translateToString: () => "$ " }) };
  const session = { status: "connected", term: { rows: 1, buffer: { active } } };
  return { ...(await importActual<typeof import("@/features/terminal/session")>()), getSession: (tabId: string) => (tabId.startsWith("tab-") ? session : undefined) };
});
/** `send_input` typing into a tab's terminal: each call that types waits until the test answers it; one without text, or whose write fails, types nothing. */
const typing = vi.hoisted(() => ({ calls: [] as { args: string; done: (outcome: ToolOutcome) => void }[] }));
vi.mock("./terminalTools", async (importActual) => ({
  ...(await importActual<typeof import("./terminalTools")>()),
  sendInput: (_session: unknown, args: string, _signal: AbortSignal, onType: () => () => void) => {
    const { text } = JSON.parse(args) as { text?: unknown };
    if (typeof text !== "string") return Promise.resolve<ToolOutcome>({ status: "error", content: "Invalid arguments: text is required" });
    const untype = onType();
    // "lost" stands for a write the connection dropped.
    if (text === "lost") {
      untype();
      return Promise.resolve<ToolOutcome>({ status: "error", content: "The input could not be sent: connection lost" });
    }
    return new Promise<ToolOutcome>((done) => typing.calls.push({ args, done }));
  },
}));

const { decideCall, deleteConversation, detachSlot, newConversation, noteSelection, openConversation, queueMessage, resetForLock, selectionAttachment, sendMessage, stopTurn, tabSelection } =
  await import("./actions");
const { blankSlot, conversationActivity, getSlot, patchSlot, slotOf, tabActivity, useAi } = await import("./store");
const { newSideQuestion } = await import("./side");
const { useTabs } = await import("@/app/tabs");

const TAB = "tab-1";
const OTHER = "tab-2";
const tab = (id: string): SessionTab => ({ id, hostId: null, target: { address: "10.0.0.8", port: 22, username: "root" }, title: "root@10.0.0.8", status: "connected", sessionId: `s-${id}` });
const conv = (id: string, title: string): AiConversationView => ({
  id,
  title,
  host_id: null,
  quick_target: null,
  origin: null,
  pinned: false,
  context_start: null,
  effort: null,
  created_at: 0,
  updated_at: 0,
  last_activity: 0,
});
const nginx = conv("conv-1", "nginx 502");
const disk = conv("conv-2", "disk usage");
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

const call = (id: string, name: string, args: Record<string, unknown>): AiToolCall => ({ id, name, arguments: JSON.stringify(args) });
const toolEntry = (callId: string): AiEntryView => ({ role: "tool", entry_id: `t-${callId}`, created_at: 0, tool_call_id: callId, status: "ok", content: "ok", auto: null });

const flush = () => new Promise((r) => setTimeout(r, 0));
async function until(check: () => boolean) {
  for (let i = 0; i < 100; i++) {
    if (check()) return;
    await flush();
  }
  throw new Error("not reached in time");
}

type Emit = (event: AiTurnEvent) => void;

/** Each `ai_send` in order: the text it sent and its channel, on which the test runs the turn. A new conversation is stored as `disk`. */
function turns() {
  const sent: { text: string; conversationId: string; emit: Emit }[] = [];
  ipc.send.mockImplementation(async (input, onEvent) => {
    const view = input.conversation_id === nginx.id ? nginx : disk;
    sent.push({ text: input.text, conversationId: view.id, emit: onEvent });
    return { conversation: view, user_entry: { role: "user", entry_id: `u${sent.length}`, created_at: 0, text: input.text } };
  });
  return sent;
}

/** The model answers with `calls`, in the response `entryId`. */
function answer(emit: Emit, entryId: string, calls: AiToolCall[]) {
  emit({ kind: "request_started" });
  emit({
    kind: "entry",
    entry: { role: "assistant", entry_id: entryId, created_at: 0, provider_id: "p1", model_id: "m1", text: "", reasoning: null, tool_calls: calls, finish: "tool_calls", usage: null },
  });
  emit({ kind: "done", finish: "tool_calls" });
}

/** Where `conv-1` is held now, and its slot. */
const held = () => slotOf(nginx.id);
const nginxSlot = () => getSlot(held()!);

beforeEach(() => {
  ipc.send.mockReset();
  ipc.stop.mockReset().mockResolvedValue(undefined);
  ipc.get.mockReset().mockImplementation(async (id) => ({ conversation: id === nginx.id ? nginx : disk, entries: [], running: false, file_changes: [] }));
  ipc.run.mockReset().mockImplementation(async (_conv, callId) => ({ entry: toolEntry(callId), dropped_file_changes: false }));
  ipc.result.mockReset().mockImplementation(async (_conv, callId) => toolEntry(callId));
  ipc.sideStop.mockReset().mockResolvedValue(undefined);
  ipc.remove.mockReset().mockResolvedValue(undefined);
  typing.calls = [];
  useTabs.setState({ tabs: [tab(TAB), tab(OTHER)], active: TAB });
  useAi.setState({
    slots: { [TAB]: { ...blankSlot("manual"), conversationId: nginx.id, conversation: nginx } },
    background: {},
    catalog: { providers: [provider], settings: null, loaded: true },
    history: null,
    allowed: {},
    selections: {},
    diagnostics: {},
  });
});

afterEach(() => {
  resetForLock();
});

describe("leaving a conversation while its turn runs (AI-07)", () => {
  it("keeps the turn and its waiting approval card, and shows them again when the conversation opens", async () => {
    const sent = turns();
    await sendMessage(TAB, "restart nginx");
    answer(sent[0].emit, "a1", [call("call_1", "run_command", { command: "sudo systemctl restart nginx" })]);
    await until(() => !!approvalOf(getSlot(TAB).turn));

    newConversation(TAB);
    expect(ipc.stop).not.toHaveBeenCalled();
    expect(getSlot(TAB)).toMatchObject({ conversationId: null, turn: null });
    expect(held()).not.toBe(TAB);
    expect(useAi.getState().background).toEqual({ [held()!]: TAB });
    // The tab bar and History mark it while it waits.
    expect(tabActivity(useAi.getState(), TAB)).toBe("waiting");
    expect(conversationActivity(useAi.getState(), nginx.id)).toBe("waiting");
    expect(tabActivity(useAi.getState(), OTHER)).toBe("idle");

    await openConversation(TAB, nginx.id);
    expect(held()).toBe(TAB);
    expect(useAi.getState().background).toEqual({});
    expect(approvalOf(getSlot(TAB).turn)).toMatchObject({ id: "call_1", state: "approval" });
    // Shown from memory, as it is: nothing is read again.
    expect(ipc.get).not.toHaveBeenCalled();

    decideCall(TAB, { kind: "run", edited: null });
    await until(() => ipc.run.mock.calls.length === 1);
    expect(ipc.run).toHaveBeenCalledWith(nginx.id, "call_1", `s-${TAB}`, null, false);
  });

  it("applies the turn's events in the background, and keeps how it ended", async () => {
    const sent = turns();
    await sendMessage(TAB, "why 502?");
    sent[0].emit({ kind: "request_started" });
    newConversation(TAB);

    sent[0].emit({ kind: "text", delta: "Checking the upstream" });
    expect(nginxSlot().turn?.live?.text).toBe("Checking the upstream");
    expect(tabActivity(useAi.getState(), TAB)).toBe("running");
    sent[0].emit({ kind: "error", status: 529, message: "overloaded" });
    sent[0].emit({ kind: "turn_ended", reason: "error" });
    await flush();
    expect(tabActivity(useAi.getState(), TAB)).toBe("idle");
    expect(conversationActivity(useAi.getState(), nginx.id)).toBe("idle");

    // An idle conversation moves to the tab that opens it, with the error and its Retry.
    useTabs.setState({ active: OTHER });
    await openConversation(OTHER, nginx.id);
    expect(held()).toBe(OTHER);
    expect(useTabs.getState().active).toBe(OTHER);
    expect(getSlot(OTHER).outcome).toEqual({ reason: "error", status: 529, message: "overloaded" });
    expect(useAi.getState().background).toEqual({});
  });

  it("sends the queued message when the turn ends, while the tab keeps its input area", async () => {
    const sent = turns();
    await sendMessage(TAB, "check the logs");
    sent[0].emit({ kind: "request_started" });
    queueMessage(TAB, "then restart it");
    patchSlot(TAB, { draft: "something else" });
    newConversation(TAB);
    expect(getSlot(TAB)).toMatchObject({ queued: null, draft: "something else" });
    expect(nginxSlot().queued?.typed).toBe("then restart it");

    sent[0].emit({ kind: "turn_ended", reason: "completed" });
    await until(() => sent.length === 2);
    expect(sent[1]).toMatchObject({ text: "then restart it", conversationId: nginx.id });
    expect(nginxSlot().queued).toBeNull();
    expect(getSlot(TAB).draft).toBe("something else");
    expect(tabActivity(useAi.getState(), TAB)).toBe("running");
  });

  it("puts a queued message that cannot go into the conversation's own input area, which comes back with it", async () => {
    const sent = turns();
    await sendMessage(TAB, "check the logs");
    sent[0].emit({ kind: "request_started" });
    queueMessage(TAB, "then restart it");
    newConversation(TAB);
    patchSlot(TAB, { draft: "typed since" });
    sent[0].emit({ kind: "turn_ended", reason: "refused" });
    await flush();
    expect(ipc.send).toHaveBeenCalledTimes(1);
    expect(nginxSlot()).toMatchObject({ queued: null, draft: "then restart it" });

    await openConversation(TAB, nginx.id);
    expect(getSlot(TAB)).toMatchObject({ conversationId: nginx.id, draft: "then restart it\n\ntyped since" });
  });

  it("gives a new conversation's first message back to the tab when it fails after the conversation was left", async () => {
    useAi.setState({ slots: { [TAB]: blankSlot("manual") } });
    let fail!: (e: unknown) => void;
    ipc.send.mockImplementationOnce(() => new Promise((_, reject) => (fail = reject)));
    const sending = sendMessage(TAB, "first question");
    queueMessage(TAB, "and a follow-up");
    newConversation(TAB);
    patchSlot(TAB, { draft: "typed since" });
    expect(tabActivity(useAi.getState(), TAB)).toBe("running");

    fail({ code: "cancelled", detail: "stopped" });
    expect(await sending).toBe(false);
    expect(useAi.getState().background).toEqual({});
    expect(getSlot(TAB)).toMatchObject({ conversationId: null, turn: null, queued: null, draft: "first question\n\nand a follow-up\n\ntyped since" });
  });

  it("keeps the chips of a queued message that goes back in the background with its conversation", async () => {
    const sent = turns();
    await sendMessage(TAB, "check the logs");
    sent[0].emit({ kind: "request_started" });
    noteSelection(TAB, "502 Bad Gateway");
    queueMessage(TAB, "why?");
    newConversation(TAB);
    sent[0].emit({ kind: "turn_ended", reason: "refused" });
    await flush();
    // The tab's other conversation does not get the selection the message took.
    expect(selectionAttachment(TAB)).toBeNull();
    expect(tabSelection(TAB).hidden).not.toBeNull();
    expect(nginxSlot()).toMatchObject({ draft: "why?", extras: [{ attachment: { kind: "selection", text: "502 Bad Gateway" } }] });

    await openConversation(TAB, nginx.id);
    await sendMessage(TAB, getSlot(TAB).draft);
    expect(sent[1].text).toMatch(/^<terminal_selection [^]*502 Bad Gateway[^]*why\?$/);
  });

  it("gives the queued message of a background conversation that is deleted back to its tab", async () => {
    const sent = turns();
    await sendMessage(TAB, "check the logs");
    sent[0].emit({ kind: "request_started" });
    queueMessage(TAB, "then restart it");
    newConversation(TAB);
    patchSlot(TAB, { draft: "typed since" });

    await deleteConversation(nginx.id);
    expect(ipc.stop).toHaveBeenCalledWith(nginx.id);
    expect(useAi.getState().background).toEqual({});
    expect(getSlot(TAB)).toMatchObject({ conversationId: null, draft: "then restart it\n\ntyped since" });
  });

  it("gives a message still on its way in a background conversation that is deleted back to its tab", async () => {
    let fail!: (e: unknown) => void;
    ipc.send.mockImplementationOnce(() => new Promise((_, reject) => (fail = reject)));
    const sending = sendMessage(TAB, "check the logs");
    queueMessage(TAB, "then restart it");
    newConversation(TAB);
    patchSlot(TAB, { draft: "typed since" });

    await deleteConversation(nginx.id);
    fail({ code: "cancelled", detail: "stopped" });
    expect(await sending).toBe(false);
    expect(useAi.getState().background).toEqual({});
    expect(getSlot(TAB)).toMatchObject({ conversationId: null, turn: null, draft: "check the logs\n\nthen restart it\n\ntyped since" });
  });

  it("discards a message still on its way in a background conversation when its tab closes", async () => {
    let fail!: (e: unknown) => void;
    ipc.send.mockImplementationOnce(() => new Promise((_, reject) => (fail = reject)));
    const sending = sendMessage(TAB, "check the logs");
    newConversation(TAB);

    detachSlot(TAB);
    fail({ code: "cancelled", detail: "stopped" });
    expect(await sending).toBe(false);
    expect(useAi.getState().slots).not.toHaveProperty(TAB);
    expect(useAi.getState().background).toEqual({});
  });

  it("gives a new conversation's messages back to the tab when the vault locks before the first one is stored", async () => {
    for (const settle of ["reject", "resolve"] as const) {
      useAi.setState({ slots: { [TAB]: blankSlot("manual") }, background: {} });
      ipc.send.mockReset();
      let done!: { resolve: (v: AiSendStarted) => void; reject: (e: unknown) => void };
      ipc.send.mockImplementationOnce(() => new Promise((resolve, reject) => (done = { resolve, reject })));
      const sending = sendMessage(TAB, "first question");
      queueMessage(TAB, "and a follow-up");
      newConversation(TAB);

      resetForLock();
      if (settle === "reject") done.reject({ code: "cancelled", detail: "locked" });
      else done.resolve({ conversation: disk, user_entry: { role: "user", entry_id: "u1", created_at: 0, text: "first question" } });
      await sending;
      expect(useAi.getState().background).toEqual({});
      expect(getSlot(TAB).draft).toBe(settle === "reject" ? "first question\n\nand a follow-up" : "and a follow-up");
    }
  });

  it("sends the queued message once Stop has stopped a turn an earlier page started, also when the conversation was left meanwhile", async () => {
    const sent = turns();
    patchSlot(TAB, { remoteRunning: true });
    queueMessage(TAB, "after the remote turn");
    let stopped!: () => void;
    ipc.stop.mockImplementationOnce(() => new Promise<void>((resolve) => (stopped = resolve)));
    const stopping = stopTurn(TAB);
    newConversation(TAB);
    expect(nginxSlot()).toMatchObject({ remoteRunning: true });

    stopped();
    await stopping;
    expect(sent.map((x) => x.text)).toEqual(["after the remote turn"]);
    expect(nginxSlot().queued).toBeNull();
    expect(getSlot(TAB)).toMatchObject({ conversationId: null, turn: null });
  });

  it("puts an entry stored after the turn ended in the slot that holds the conversation by then", async () => {
    const sent = turns();
    await sendMessage(TAB, "check the logs");
    sent[0].emit({ kind: "request_started" });
    sent[0].emit({ kind: "turn_ended", reason: "completed" });
    await flush();
    useTabs.setState({ active: OTHER });
    await openConversation(OTHER, nginx.id);

    const late: AiEntryView = { role: "tool", entry_id: "t-late", created_at: 0, tool_call_id: "call_9", status: "cancelled", content: "cancelled", auto: null };
    sent[0].emit({ kind: "entry", entry: late });
    expect(getSlot(OTHER).entries).toContainEqual(late);
    expect(getSlot(TAB).entries).toEqual([]);
  });

  it("stops the side question, but not the turn", async () => {
    const sent = turns();
    await sendMessage(TAB, "restart nginx");
    sent[0].emit({ kind: "request_started" });
    patchSlot(TAB, { side: newSideQuestion(7, nginx.id, "is it safe?", "is it safe?") });
    newConversation(TAB);
    expect(ipc.sideStop).toHaveBeenCalledWith(nginx.id, 7);
    expect(ipc.stop).not.toHaveBeenCalled();
    expect(nginxSlot().side).toBeNull();
  });

  it("brings the conversation's tab to the front when another tab opens it while busy, and shows it there", async () => {
    const sent = turns();
    await sendMessage(TAB, "restart nginx");
    answer(sent[0].emit, "a1", [call("call_1", "run_command", { command: "sudo systemctl restart nginx" })]);
    await until(() => !!approvalOf(getSlot(TAB).turn));
    newConversation(TAB);
    useTabs.setState({ active: OTHER });

    await openConversation(OTHER, nginx.id);
    expect(useTabs.getState().active).toBe(TAB);
    expect(held()).toBe(TAB);
    expect(approvalOf(getSlot(TAB).turn)?.id).toBe("call_1");
    expect(getSlot(OTHER).conversationId).toBeNull();
  });

  it("stops the turns a tab left in the background when it closes", async () => {
    const sent = turns();
    await sendMessage(TAB, "restart nginx");
    sent[0].emit({ kind: "request_started" });
    queueMessage(TAB, "then this");
    newConversation(TAB);

    detachSlot(TAB);
    expect(ipc.stop).toHaveBeenCalledWith(nginx.id);
    expect(held()).toBeUndefined();
    expect(useAi.getState().background).toEqual({});
    sent[0].emit({ kind: "turn_ended", reason: "stopped" });
    await flush();
    expect(ipc.send).toHaveBeenCalledTimes(1);
  });

  it("stops a turn an earlier page started when its tab closes, also from the background", async () => {
    patchSlot(TAB, { remoteRunning: true });
    newConversation(TAB);
    expect(nginxSlot()).toMatchObject({ remoteRunning: true });
    expect(ipc.stop).not.toHaveBeenCalled();

    detachSlot(TAB);
    expect(ipc.stop).toHaveBeenCalledWith(nginx.id);
    expect(held()).toBeUndefined();
  });

  it("drops the turns in the background when the vault locks, and gives their queued messages back", async () => {
    const sent = turns();
    await sendMessage(TAB, "restart nginx");
    sent[0].emit({ kind: "request_started" });
    queueMessage(TAB, "then this");
    newConversation(TAB);

    resetForLock();
    expect(tabActivity(useAi.getState(), TAB)).toBe("idle");
    expect(nginxSlot()).toMatchObject({ turn: null, queued: null, draft: "then this", entries: [] });
  });
});

describe("the marks of a tab whose conversations run several calls at once (AI-07)", () => {
  it("count a background turn's concurrent calls as running, and an approval card of another conversation as waiting", async () => {
    const reads: (() => void)[] = [];
    ipc.run.mockImplementation((_conv, callId) => new Promise((done) => reads.push(() => done({ entry: toolEntry(callId), dropped_file_changes: false }))));
    const sent = turns();
    await sendMessage(TAB, "read the configs");
    answer(sent[0].emit, "a1", [call("call_1", "read_file", { path: "/etc/nginx/nginx.conf" }), call("call_2", "read_file", { path: "/etc/hosts" })]);
    await until(() => reads.length === 2);

    newConversation(TAB);
    expect(nginxSlot().turn?.calls.map((c) => [c.id, c.state])).toEqual([
      ["call_1", "running"],
      ["call_2", "running"],
    ]);
    expect(tabActivity(useAi.getState(), TAB)).toBe("running");
    expect(conversationActivity(useAi.getState(), nginx.id)).toBe("running");

    // The tab's other conversation asks for approval: the tab shows that it waits, History each one's own state.
    await sendMessage(TAB, "restart nginx");
    answer(sent[1].emit, "b1", [call("call_3", "run_command", { command: "sudo systemctl restart nginx" })]);
    await until(() => !!approvalOf(getSlot(TAB).turn));
    expect(tabActivity(useAi.getState(), TAB)).toBe("waiting");
    expect(conversationActivity(useAi.getState(), nginx.id)).toBe("running");
    expect(conversationActivity(useAi.getState(), disk.id)).toBe("waiting");

    for (const done of reads) done();
    await until(() => nginxSlot().turn?.calls.length === 0);
    // Its turn goes on until the next response ends it.
    expect(conversationActivity(useAi.getState(), nginx.id)).toBe("running");
    expect(tabActivity(useAi.getState(), TAB)).toBe("waiting");
  });
});

describe("the exec channels of a tab's connection, shared by its conversations (§13.1, AI-07)", () => {
  /** The `ai_tool_run` calls of `channels` that have not ended. */
  const open = new Map<string, () => void>();
  // Rust ends every running call when the turns stop, as on the lock after each test.
  afterEach(() => [...open.values()].forEach((end) => end()));

  /** `ai_tool_run` calls that end when the test says, counting how many run at once. */
  function channels() {
    open.clear();
    let most = 0;
    ipc.run.mockImplementation(
      (_conv, callId) =>
        new Promise((done) => {
          open.set(callId, () => {
            open.delete(callId);
            done({ entry: toolEntry(callId), dropped_file_changes: false });
          });
          most = Math.max(most, open.size);
        }),
    );
    return { open, most: () => most, end: (...ids: string[]) => ids.forEach((id) => open.get(id)!()) };
  }
  const reads = (prefix: string, n: number) => Array.from({ length: n }, (_, i) => call(`${prefix}${i}`, "read_file", { path: `/etc/file${i}` }));
  const ran = () => ipc.run.mock.calls.map(([, callId]) => callId);

  it("run at most four at once across the tab's conversations, a batch's calls and a call that runs alone", async () => {
    const ch = channels();
    const sent = turns();
    patchSlot(TAB, { mode: "bypass" });
    await sendMessage(TAB, "check the configs");
    answer(sent[0].emit, "a1", reads("a", 4));
    await until(() => ch.open.size === 4);

    // The tab's other conversation runs a command alone: it waits, queued, while the batch holds every channel.
    newConversation(TAB);
    patchSlot(TAB, { mode: "bypass" });
    await sendMessage(TAB, "free some space");
    answer(sent[1].emit, "b1", [call("b_rm", "run_command", { command: "rm -rf /var/cache/apt/archives" })]);
    await flush();
    expect(ran()).toEqual(["a0", "a1", "a2", "a3"]);
    expect(getSlot(TAB).turn?.calls).toEqual([]);

    ch.end("a0");
    await until(() => ch.open.has("b_rm"));
    expect(getSlot(TAB).turn?.calls).toEqual([{ id: "b_rm", state: "running" }]);

    // The background conversation's next batch gets the three channels the command leaves.
    ch.end("a1", "a2", "a3");
    await until(() => nginxSlot().turn?.calls.length === 0);
    await flush();
    answer(sent[0].emit, "a2", reads("c", 4));
    await until(() => ch.open.size === 4);
    await flush();
    expect(ran().slice(5)).toEqual(["c0", "c1", "c2"]);
    ch.end("b_rm");
    await until(() => ch.open.has("c3"));
    expect(ch.most()).toBe(4);
  });

  it("let a call that waits for a channel give up when its turn stops", async () => {
    const ch = channels();
    const sent = turns();
    patchSlot(TAB, { mode: "bypass" });
    await sendMessage(TAB, "check the configs");
    answer(sent[0].emit, "a1", reads("a", 4));
    await until(() => ch.open.size === 4);

    newConversation(TAB);
    patchSlot(TAB, { mode: "bypass" });
    await sendMessage(TAB, "free some space");
    answer(sent[1].emit, "b1", [call("b_rm", "run_command", { command: "rm -rf /var/cache/apt/archives" })]);
    await flush();
    await stopTurn(TAB);
    expect(ipc.stop).toHaveBeenCalledWith(disk.id);
    sent[1].emit({ kind: "turn_ended", reason: "stopped" });

    ch.end("a0", "a1", "a2", "a3");
    await until(() => nginxSlot().turn?.calls.length === 0);
    await flush();
    expect(ran()).toEqual(["a0", "a1", "a2", "a3"]);
    // Nothing kept the channel it waited for.
    answer(sent[0].emit, "a2", reads("c", 4));
    await until(() => ch.open.size === 4);
  });
});

describe("a tab's terminal shared by its conversations (AI-07)", () => {
  it("is not held by a send_input call that typed nothing", async () => {
    const sent = turns();
    patchSlot(TAB, { mode: "bypass" });
    await sendMessage(TAB, "tail the log");
    answer(sent[0].emit, "a1", [call("call_1", "send_input", { key: "enter" }), call("call_1b", "send_input", { text: "lost", key: "enter" })]);
    await until(() => ipc.result.mock.calls.length === 2);
    expect(ipc.result).toHaveBeenCalledWith(nginx.id, "call_1", expect.objectContaining({ status: "error" }));
    expect(ipc.result).toHaveBeenCalledWith(nginx.id, "call_1b", expect.objectContaining({ status: "error", content: expect.stringContaining("could not be sent") }));

    newConversation(TAB);
    patchSlot(TAB, { mode: "bypass" });
    await sendMessage(TAB, "how full is the disk?");
    answer(sent[1].emit, "b1", [call("call_2", "send_input", { text: "df -h", key: "enter" })]);
    await until(() => typing.calls.length === 1);
    expect(typing.calls[0].args).toContain("df -h");
  });

  it("serves one turn's send_input at a time, while other tools run alongside", async () => {
    const sent = turns();
    patchSlot(TAB, { mode: "bypass" });
    await sendMessage(TAB, "tail the log");
    answer(sent[0].emit, "a1", [call("call_1", "send_input", { text: "tail -f /var/log/nginx/error.log", key: "enter" })]);
    await until(() => typing.calls.length === 1);

    newConversation(TAB);
    patchSlot(TAB, { mode: "bypass" });
    await sendMessage(TAB, "how full is the disk?");
    answer(sent[1].emit, "b1", [call("call_2", "send_input", { text: "df -h", key: "enter" }), call("call_3", "run_command", { command: "df -h" })]);
    await until(() => ipc.run.mock.calls.length === 1);
    expect(typing.calls).toHaveLength(1);
    expect(ipc.result).toHaveBeenCalledWith(disk.id, "call_2", {
      status: "error",
      content: expect.stringContaining('("nginx 502")'),
      edited_arguments: null,
      judged: false,
    });
    expect(ipc.run).toHaveBeenCalledWith(disk.id, "call_3", `s-${TAB}`, null, false);

    // Once the first turn ends, the terminal is free for the other one.
    typing.calls[0].done({ status: "ok", content: "Output after the input:\n\n(no output)" });
    await until(() => ipc.result.mock.calls.some(([id]) => id === nginx.id));
    sent[0].emit({ kind: "turn_ended", reason: "completed" });
    answer(sent[1].emit, "b2", [call("call_4", "send_input", { text: "df -h", key: "enter" })]);
    await until(() => typing.calls.length === 2);
    expect(typing.calls[1].args).toContain("df -h");
  });
});
