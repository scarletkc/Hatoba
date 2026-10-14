import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionTab } from "@/app/tabs";
import type {
  AiAllowance,
  AiAutoVerdict,
  AiConversationView,
  AiEntryView,
  AiPermissionMode,
  AiProviderView,
  AiSendInput,
  AiSendStarted,
  AiSettingsView,
  AiToolCall,
  AiToolResultInput,
  AiToolRun,
  AiTurnEvent,
} from "@/ipc/types";

/** The calls a turn makes, answered by each test. */
const ipc = vi.hoisted(() => ({
  send: vi.fn<(input: AiSendInput, onEvent: (event: AiTurnEvent) => void) => Promise<AiSendStarted>>(),
  stop: vi.fn<(conversationId: string) => Promise<void>>(),
  judge: vi.fn<(conversationId: string, toolCallId: string, screenTail: string | null) => Promise<AiAutoVerdict>>(),
  run: vi.fn<(conversationId: string, toolCallId: string, sessionId: string | null, edited: string | null, judged: boolean) => Promise<AiToolRun>>(),
  result: vi.fn<(conversationId: string, toolCallId: string, result: AiToolResultInput) => Promise<AiEntryView>>(),
  allowances: vi.fn<() => Promise<AiAllowance[]>>(),
}));
vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: {
    ai_send: ipc.send,
    ai_stop: ipc.stop,
    ai_judge: ipc.judge,
    ai_tool_run: ipc.run,
    ai_tool_result: ipc.result,
    ai_allowances_list: ipc.allowances,
    ai_settings_get: () => Promise.resolve(settings),
    ai_providers_list: () => Promise.resolve([provider]),
    notification_show: () => Promise.resolve(),
  },
}));
// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
// The tab's terminal, which `read_terminal` reads.
vi.mock("@/features/terminal/session", async (importActual) => {
  const line = (text: string) => ({ isWrapped: false, translateToString: () => text });
  const rows = ["$ uptime", " 10:00:00 up 3 days", "$ "];
  const active = { type: "normal", baseY: 0, cursorY: 2, length: rows.length, getLine: (y: number) => (rows[y] === undefined ? undefined : line(rows[y])) };
  const term = { rows: rows.length, buffer: { active } };
  return { ...(await importActual<typeof import("@/features/terminal/session")>()), getSession: (tabId: string) => (tabId === "tab-1" ? { status: "connected", term } : undefined) };
});

const { decideCall, resetForLock, sendMessage, stopTurn } = await import("./actions");
const { blankSlot, getSlot, patchSlot, useAi } = await import("./store");
const { useTabs } = await import("@/app/tabs");
const { MAX_CONCURRENT_CHANNELS } = await import("./batches");

const TAB = "tab-1";
const tab: SessionTab = { id: TAB, hostId: null, target: { address: "10.0.0.8", port: 22, username: "root" }, title: "root@10.0.0.8", status: "connected", sessionId: "s1" };
const view: AiConversationView = {
  id: "conv-1",
  title: "nginx",
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
const provider: AiProviderView = {
  id: "p1",
  name: "Mock",
  protocol: "chat_completions",
  base_url: "http://127.0.0.1/v1",
  has_api_key: false,
  auth_header: "x-api-key",
  models: [
    { id: "m1", name: "Model 1", context_window: null, max_output_tokens: null, efforts: null, adaptive_thinking: null },
    { id: "judge", name: "Judge", context_window: null, max_output_tokens: null, efforts: null, adaptive_thinking: null },
  ],
  updated_at: 0,
};
const settings: AiSettingsView = {
  default_model: null,
  default_effort: null,
  search_provider_id: null,
  native_search: true,
  builtin_skill_enabled: true,
  custom_instructions: "",
  send_terminal_screen: true,
  judge_model: { provider_id: "p1", model_id: "judge" },
};

const call = (id: string, name: string, args: Record<string, unknown>): AiToolCall => ({ id, name, arguments: JSON.stringify(args) });
const read = (id: string, path = `/etc/${id}`) => call(id, "read_file", { path });
const toolEntry = (callId: string, status: "ok" | "cancelled" = "ok"): AiEntryView => ({ role: "tool", entry_id: `t-${callId}`, created_at: 0, tool_call_id: callId, status, content: "", auto: null });

const flush = () => new Promise((r) => setTimeout(r, 0));
async function until(check: () => boolean) {
  for (let i = 0; i < 100; i++) {
    if (check()) return;
    await flush();
  }
  throw new Error("not reached in time");
}

type Emit = (event: AiTurnEvent) => void;

/** Sends a message in `mode`, and the model answers with `calls`; returns what emits the turn's later events. */
async function respond(mode: AiPermissionMode, calls: AiToolCall[]): Promise<Emit> {
  patchSlot(TAB, { mode });
  let emit!: Emit;
  ipc.send.mockImplementation(async (input, onEvent) => {
    emit = onEvent;
    return { conversation: view, user_entry: { role: "user", entry_id: "u1", created_at: 0, text: input.text } };
  });
  await sendMessage(TAB, "check nginx");
  emit({ kind: "request_started" });
  emit({
    kind: "entry",
    entry: { role: "assistant", entry_id: "a1", created_at: 0, provider_id: "p1", model_id: "m1", text: "", reasoning: null, tool_calls: calls, finish: "tool_calls", usage: null },
  });
  emit({ kind: "done", finish: "tool_calls" });
  return emit;
}

/** `ai_tool_run` calls that end when the test says; each stores its result as it ends. */
const pending = new Map<string, { finish: () => void; fail: (e: unknown) => void }>();
function finish(...ids: string[]) {
  for (const id of ids) {
    const p = pending.get(id);
    if (!p) throw new Error(`${id} is not running`);
    pending.delete(id);
    p.finish();
  }
}

const running = () => [...pending.keys()];
const shown = () => getSlot(TAB).turn?.calls.map((c) => `${c.id}:${c.state}`) ?? [];
const ran = () => ipc.run.mock.calls.map((c) => c[1]);

beforeEach(() => {
  pending.clear();
  ipc.send.mockReset();
  ipc.stop.mockReset().mockResolvedValue(undefined);
  ipc.judge.mockReset();
  ipc.run.mockReset().mockImplementation(
    (_conv, callId) =>
      new Promise<AiToolRun>((resolve, reject) => {
        pending.set(callId, {
          finish: () => {
            useAi.setState((st) => ({ slots: { ...st.slots, [TAB]: { ...st.slots[TAB], entries: [...st.slots[TAB].entries, toolEntry(callId)] } } }));
            resolve({ entry: toolEntry(callId), dropped_file_changes: false });
          },
          fail: reject,
        });
      }),
  );
  ipc.result.mockReset().mockImplementation(async (_conv, callId) => toolEntry(callId));
  ipc.allowances.mockReset().mockResolvedValue([]);
  useTabs.setState({ tabs: [tab] });
  useAi.setState({
    slots: { [TAB]: { ...blankSlot("manual"), conversationId: view.id, conversation: view } },
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

describe("calls that only read run together (§13.1)", () => {
  it("runs a run of read-only calls at once, and a call that asks alone, after them and before the rest", async () => {
    await respond("manual", [
      read("call_1"),
      call("call_2", "grep", { pattern: "listen", path: "/etc/nginx" }),
      call("call_3", "run_command", { command: "systemctl status nginx && ls /etc/nginx" }),
      call("call_4", "read_terminal", {}),
      call("call_5", "write_file", { path: "/etc/motd", content: "hi" }),
      read("call_6"),
      call("call_7", "glob", { pattern: "**/*.conf", path: "/etc/nginx" }),
    ]);
    await until(() => running().length === 3);
    expect(ran()).toEqual(["call_1", "call_2", "call_3"]);
    // read_terminal runs in the panel, and its result goes to Rust, which stores it after the earlier calls'.
    await until(() => ipc.result.mock.calls.length === 1);
    expect(ipc.result.mock.calls[0][1]).toBe("call_4");
    expect(shown()).toEqual(["call_1:running", "call_2:running", "call_3:running"]);

    // The write waits for every call before it.
    finish("call_2", "call_3");
    await flush();
    expect(shown()).toEqual(["call_1:running"]);
    expect(ran()).toHaveLength(3);
    finish("call_1");
    await until(() => shown().join() === "call_5:approval");
    // ...and the calls after it wait for the write.
    expect(ran()).toHaveLength(3);

    decideCall(TAB, { kind: "run", edited: null });
    await until(() => running().join() === "call_5");
    expect(shown()).toEqual(["call_5:running"]);
    await flush();
    expect(ran()).toHaveLength(4);
    finish("call_5");
    await until(() => running().length === 2);
    expect(ran().slice(4)).toEqual(["call_6", "call_7"]);
    finish("call_6", "call_7");
    await until(() => shown().length === 0);
  });

  it("runs a command that changes something alone, between reads", async () => {
    await respond("bypass", [read("call_1"), read("call_2"), call("call_3", "run_command", { command: "systemctl reload nginx" }), read("call_4")]);
    await until(() => running().length === 2);
    finish("call_1");
    await flush();
    expect(ran()).toEqual(["call_1", "call_2"]);
    finish("call_2");
    await until(() => running().join() === "call_3");
    await flush();
    expect(ran()).toHaveLength(3);
    finish("call_3");
    await until(() => running().join() === "call_4");
    finish("call_4");
  });

  it("runs MCP calls, typing into the shell and edits one at a time even in bypass", async () => {
    await respond("bypass", [
      call("call_1", "edit_file", { path: "/etc/a", old_string: "x", new_string: "y" }),
      call("call_2", "edit_file", { path: "/etc/b", old_string: "x", new_string: "y" }),
    ]);
    await until(() => running().length === 1);
    await flush();
    expect(ran()).toEqual(["call_1"]);
    finish("call_1");
    await until(() => running().join() === "call_2");
    finish("call_2");
  });

  it("holds at most four exec channels of the connection at once", async () => {
    await respond("manual", [1, 2, 3, 4, 5, 6].map((i) => read(`call_${i}`)));
    await until(() => running().length === MAX_CONCURRENT_CHANNELS);
    await flush();
    expect(ran()).toEqual(["call_1", "call_2", "call_3", "call_4"]);
    finish("call_3");
    await until(() => ran().length === 5);
    finish("call_1", "call_2");
    await until(() => ran().length === 6);
    finish("call_4", "call_5", "call_6");
    await until(() => shown().length === 0);
  });

  it("frees the channel of a call whose tool ended while its result waits for a slower earlier call's", async () => {
    const emit = await respond("manual", [1, 2, 3, 4, 5, 6].map((i) => read(`call_${i}`)));
    await until(() => running().length === MAX_CONCURRENT_CHANNELS);
    expect(ran()).toEqual(["call_1", "call_2", "call_3", "call_4"]);
    // call_1 is slow; the reads after it are done on the host, and their results wait for call_1's.
    emit({ kind: "tool_ran", tool_call_id: "call_2" });
    emit({ kind: "tool_ran", tool_call_id: "call_3" });
    await until(() => ran().length === 6);
    expect(running()).toEqual(["call_1", "call_2", "call_3", "call_4", "call_5", "call_6"]);
    finish(...running());
    await until(() => shown().length === 0);
  });

  it("runs one web search at a time, beside the other calls", async () => {
    const search = (id: string) => call(id, "web_search", { query: id });
    await respond("manual", [read("call_1"), search("call_2"), search("call_3"), read("call_4")]);
    await until(() => running().length === 2);
    await flush();
    // call_3 waits for call_2's search, and call_4 waits for call_3 to start.
    expect(ran()).toEqual(["call_1", "call_2"]);
    finish("call_2");
    await until(() => ran().length === 4);
    finish("call_1", "call_3", "call_4");
    await until(() => shown().length === 0);
  });

  it("asks about a fetch the user has not allowed, alone, and runs one on an allowed domain with the reads", async () => {
    useAi.setState({ allowed: { [view.id]: ["fetch_url:nginx.org"] } });
    await respond("manual", [
      call("call_1", "fetch_url", { url: "https://nginx.org/en/docs/" }),
      read("call_2"),
      call("call_3", "fetch_url", { url: "https://example.com/" }),
      read("call_4"),
    ]);
    await until(() => running().length === 2);
    expect(ran()).toEqual(["call_1", "call_2"]);
    finish("call_1", "call_2");
    await until(() => shown().join() === "call_3:approval");
    decideCall(TAB, { kind: "reject", reason: "" });
    await until(() => running().join() === "call_4");
    expect(ipc.result).toHaveBeenCalledWith("conv-1", "call_3", { status: "rejected", content: "", edited_arguments: null, judged: false });
    finish("call_4");
  });

  it("asks Auto's judge ahead about a call between reads, which runs alone once the reads are done", async () => {
    ipc.judge.mockResolvedValue({ provider_id: "p1", model_id: "judge", decision: "run", answers: [], concerns: [], failure: null });
    await respond("auto", [read("call_1"), read("call_2"), call("call_3", "run_command", { command: "systemctl reload nginx" }), read("call_4"), read("call_5")]);
    await until(() => running().length === 2);
    // Asked as the response arrived, while the reads run.
    expect(ipc.judge.mock.calls.map((c) => c[1])).toEqual(["call_3"]);
    finish("call_1", "call_2");
    await until(() => running().join() === "call_3");
    expect(ipc.run).toHaveBeenLastCalledWith("conv-1", "call_3", "s1", null, true);
    finish("call_3");
    await until(() => running().length === 2);
    expect(ran().slice(3)).toEqual(["call_4", "call_5"]);
    finish("call_4", "call_5");
    expect(ipc.judge).toHaveBeenCalledTimes(1);
  });

  it("shows the card for calls still waiting for room after a switch to a stricter mode", async () => {
    const fetches = Array.from({ length: 12 }, (_, i) => call(`call_${i + 1}`, "fetch_url", { url: `https://nginx.org/${i + 1}` }));
    await respond("bypass", fetches);
    await until(() => running().length === 10);
    // §13.5: a switch to a stricter mode applies to the turn's next call.
    patchSlot(TAB, { mode: "manual" });
    finish(...running());
    await until(() => shown().join() === "call_11:approval");
    expect(ran()).toHaveLength(10);
    decideCall(TAB, { kind: "run", edited: null });
    await until(() => running().join() === "call_11");
    finish("call_11");
    await until(() => shown().join() === "call_12:approval");
    decideCall(TAB, { kind: "reject", reason: "" });
    await until(() => shown().length === 0);
    expect(ran()).toHaveLength(11);
  });

  it("stops every running call of a batch on Stop and starts none of the rest", async () => {
    await respond("manual", [1, 2, 3, 4, 5, 6].map((i) => read(`call_${i}`)));
    await until(() => running().length === MAX_CONCURRENT_CHANNELS);
    await stopTurn(TAB);
    expect(ipc.stop).toHaveBeenCalledWith("conv-1");
    // Rust cancels the running tools and gives every call without a result a cancelled result.
    for (const id of running()) {
      pending.get(id)!.fail({ code: "cancelled", detail: "the tool call was stopped" });
      pending.delete(id);
    }
    await until(() => shown().length === 0);
    await flush();
    expect(ran()).toEqual(["call_1", "call_2", "call_3", "call_4"]);
  });
});
