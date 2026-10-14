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
  AiSettingsView,
  AiSendStarted,
  AiToolCall,
  AiToolResultInput,
  AiToolRun,
  AiTurnEvent,
  McpToolInfo,
} from "@/ipc/types";

/** The calls a turn in Auto mode makes (AI-47), answered by each test. */
const ipc = vi.hoisted(() => ({
  send: vi.fn<(input: AiSendInput, onEvent: (event: AiTurnEvent) => void) => Promise<AiSendStarted>>(),
  judge: vi.fn<(conversationId: string, toolCallId: string, screenTail: string | null) => Promise<AiAutoVerdict>>(),
  run: vi.fn<(conversationId: string, toolCallId: string, sessionId: string | null, edited: string | null, judged: boolean) => Promise<AiToolRun>>(),
  result: vi.fn<(conversationId: string, toolCallId: string, result: AiToolResultInput) => Promise<AiEntryView>>(),
  allowances: vi.fn<() => Promise<AiAllowance[]>>(),
  settings: vi.fn<() => Promise<AiSettingsView>>(),
}));
vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: {
    ai_send: ipc.send,
    ai_stop: () => Promise.resolve(),
    ai_judge: ipc.judge,
    ai_tool_run: ipc.run,
    ai_tool_result: ipc.result,
    ai_allowances_list: ipc.allowances,
    ai_settings_get: ipc.settings,
    ai_providers_list: () => Promise.resolve([provider]),
    mcp_tool_info: () => Promise.resolve(shell),
    mcp_proxy_info: () => Promise.resolve({ tool: shell, runs: true }),
  },
}));
// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
// The tab's terminal, whose last lines a `send_input` call's judge and hard rules read.
const screen = vi.hoisted(() => ({ rows: ["$ "] }));
vi.mock("@/features/terminal/session", async (importActual) => {
  const line = (text: string) => ({ isWrapped: false, translateToString: () => text });
  const active = {
    type: "normal",
    baseY: 0,
    get length() {
      return screen.rows.length;
    },
    getLine: (y: number) => (screen.rows[y] === undefined ? undefined : line(screen.rows[y])),
  };
  const term = {
    get rows() {
      return screen.rows.length;
    },
    buffer: { active },
  };
  return { ...(await importActual<typeof import("@/features/terminal/session")>()), getSession: (tabId: string) => (tabId === "tab-1" ? { status: "connected", term } : undefined) };
});

const { decideCall, resetForLock, sendMessage, setSlotMode } = await import("./actions");
const { blankSlot, getSlot, patchSlot, useAi } = await import("./store");
const { useTabs } = await import("@/app/tabs");
type CallState = import("./turn").CallState;

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
/** Settings → AI with the judge of {@link verdict}, or with `judge`. */
const settings = (judge: AiSettingsView["judge_model"] = { provider_id: "p1", model_id: "judge" }): AiSettingsView => ({
  default_model: null,
  default_effort: null,
  search_provider_id: null,
  native_search: true,
  builtin_skill_enabled: true,
  custom_instructions: "",
  send_terminal_screen: true,
  judge_model: judge,
});

/** The conversation's MCP tool: a server's shell. */
const shell: McpToolInfo = {
  server_id: "s-shell",
  server_name: "shell",
  always_ask: false,
  tool: {
    name: "mcp__shell__exec",
    tool: "exec",
    description: "",
    annotations: { title: null, read_only_hint: null, destructive_hint: null, idempotent_hint: null, open_world_hint: null },
    always_allow: false,
  },
};

const call = (id: string, name: string, args: Record<string, unknown>): AiToolCall => ({ id, name, arguments: JSON.stringify(args) });
const verdict = (decision: AiAutoVerdict["decision"], concerns: string[] = []): AiAutoVerdict => ({
  provider_id: "p1",
  model_id: "judge",
  decision,
  answers: [
    { question: "requested", probability: 0.96 },
    { question: "destroys_data", probability: concerns.includes("destroys_data") ? 0.64 : 0.02 },
  ],
  concerns,
  failure: null,
});
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

/** The model answers with `calls`, in the response `entryId`. */
function answer(emit: Emit, entryId: string, calls: AiToolCall[]) {
  emit({ kind: "request_started" });
  emit({
    kind: "entry",
    entry: { role: "assistant", entry_id: entryId, created_at: 0, provider_id: "p1", model_id: "m1", text: "", reasoning: null, tool_calls: calls, finish: "tool_calls", usage: null },
  });
  emit({ kind: "done", finish: "tool_calls" });
}

/** Sends a message in `mode`, and the model answers with `calls`; returns what emits the turn's later events. */
async function respond(mode: AiPermissionMode, calls: AiToolCall[]): Promise<Emit> {
  patchSlot(TAB, { mode });
  let emit!: Emit;
  ipc.send.mockImplementation(async (input, onEvent) => {
    emit = onEvent;
    return { conversation: view, user_entry: { role: "user", entry_id: "u1", created_at: 0, text: input.text } };
  });
  await sendMessage(TAB, "restart nginx");
  answer(emit, "a1", calls);
  return emit;
}

/** The call being handled: a turn in Auto mode handles the calls that ask one at a time. */
const current = () => getSlot(TAB).turn?.calls.at(-1) ?? null;

/** Every state the slot's current call goes through, in order. */
const unsubscribe: (() => void)[] = [];
function watchStates(): CallState[] {
  const seen: CallState[] = [];
  unsubscribe.push(
    useAi.subscribe((st) => {
      const state = st.slots[TAB]?.turn?.calls.at(-1)?.state;
      if (state && seen.at(-1) !== state) seen.push(state);
    }),
  );
  return seen;
}

beforeEach(() => {
  ipc.send.mockReset();
  ipc.judge.mockReset();
  ipc.run.mockReset().mockImplementation(async (_conv, callId) => ({ entry: toolEntry(callId), dropped_file_changes: false }));
  ipc.result.mockReset().mockImplementation(async (_conv, callId) => toolEntry(callId));
  ipc.allowances.mockReset().mockResolvedValue([]);
  ipc.settings.mockReset().mockResolvedValue(settings());
  screen.rows = ["$ "];
  useTabs.setState({ tabs: [tab] });
  useAi.setState({
    slots: { [TAB]: { ...blankSlot("auto"), conversationId: view.id, conversation: view } },
    catalog: { providers: [provider], settings: null, loaded: true },
    history: null,
    allowed: {},
    selections: {},
    diagnostics: {},
  });
});

afterEach(() => {
  for (const off of unsubscribe.splice(0)) off();
  resetForLock();
});

describe("a turn in Auto mode (AI-47)", () => {
  it("runs a call the judge lets run without the card, and stores its result with the verdict", async () => {
    ipc.judge.mockResolvedValue(verdict("run"));
    const seen = watchStates();
    await respond("auto", [call("call_1", "run_command", { command: "sudo systemctl restart nginx" })]);
    await until(() => ipc.run.mock.calls.length === 1);
    expect(ipc.judge).toHaveBeenCalledWith("conv-1", "call_1", null);
    expect(ipc.run).toHaveBeenCalledWith("conv-1", "call_1", "s1", null, true);
    expect(seen).toContain("judging");
    expect(seen).not.toContain("approval");
  });

  it("shows the card with the verdict when the judge asks, and stores what the user decides with it", async () => {
    const asks = verdict("ask", ["destroys_data"]);
    ipc.judge.mockResolvedValue(asks);
    await respond("auto", [call("call_1", "run_command", { command: "rm /etc/motd" }), call("call_2", "run_command", { command: "rm /etc/issue" })]);
    await until(() => current()?.state === "approval");
    expect(current()).toMatchObject({ id: "call_1", auto: { verdict: asks } });

    decideCall(TAB, { kind: "run", edited: null });
    await until(() => current()?.id === "call_2" && current()?.state === "approval");
    expect(ipc.run).toHaveBeenCalledWith("conv-1", "call_1", "s1", null, true);
    expect(current()).toMatchObject({ id: "call_2", auto: { verdict: asks } });

    decideCall(TAB, { kind: "reject", reason: "keep it" });
    await until(() => ipc.result.mock.calls.length === 1);
    expect(ipc.result).toHaveBeenCalledWith("conv-1", "call_2", { status: "rejected", content: "keep it", edited_arguments: null, judged: true });
    // Both verdicts were asked for as the response arrived.
    expect(ipc.judge).toHaveBeenCalledTimes(2);
  });

  it("asks about the rest of a response without the judge once the user rejects one of its calls, and judges the next response", async () => {
    ipc.judge.mockImplementation(async (_conv, callId) => (callId === "call_1" ? verdict("ask", ["destroys_data"]) : verdict("run")));
    const emit = await respond("auto", [
      call("call_1", "run_command", { command: "rm /etc/motd" }),
      // The judge lets these two run.
      call("call_2", "write_file", { path: "/srv/app/maintenance.html", content: "<h1>Back soon</h1>" }),
      // Only reads, so it runs without asking, as in manual approval.
      call("call_3", "run_command", { command: "ls -la /srv" }),
      call("call_4", "fetch_url", { url: "https://nginx.org/en/docs/" }),
    ]);
    await until(() => current()?.id === "call_1" && current()?.state === "approval");
    decideCall(TAB, { kind: "reject", reason: "keep it" });

    await until(() => current()?.id === "call_2" && current()?.state === "approval");
    expect(current()?.auto).toEqual({ rule: { kind: "after_reject" } });
    decideCall(TAB, { kind: "run", edited: null });
    await until(() => current()?.id === "call_4" && current()?.state === "approval");
    expect(current()?.auto).toEqual({ rule: { kind: "after_reject" } });
    decideCall(TAB, { kind: "reject", reason: "" });
    await until(() => current() === null);
    // The verdicts asked for ahead go with no result.
    expect(ipc.run.mock.calls.map((c) => [c[1], c[4]])).toEqual([
      ["call_2", false],
      ["call_3", false],
    ]);
    expect(ipc.result.mock.calls.map((c) => [c[1], c[2].judged])).toEqual([
      ["call_1", true],
      ["call_4", false],
    ]);

    answer(emit, "a2", [call("call_5", "write_file", { path: "/srv/app/maintenance.html", content: "<h1>Back</h1>" })]);
    await until(() => ipc.run.mock.calls.length === 3);
    expect(ipc.run).toHaveBeenLastCalledWith("conv-1", "call_5", "s1", null, true);
    expect(ipc.judge.mock.calls.map((c) => c[1])).toEqual(["call_1", "call_2", "call_4", "call_5"]);
  });

  it("checks a call_mcp_tool call's hard rules against the arguments of the tool it names", async () => {
    // The tool's arguments as a string that holds them, which Rust reads as the object.
    await respond("auto", [call("call_1", "call_mcp_tool", { tool: "mcp__shell__exec", arguments: JSON.stringify({ command: "rm -rf /srv/old" }) })]);
    await until(() => current()?.state === "approval");
    expect(current()?.auto).toEqual({ rule: { kind: "command", command: "rm -r" } });
    expect(ipc.judge).not.toHaveBeenCalled();
  });

  it("sends a call a hard rule covers to the card without asking the judge", async () => {
    await respond("auto", [call("call_1", "run_command", { command: "sh -c 'rm -rf /var/lib/mysql'" })]);
    await until(() => current()?.state === "approval");
    expect(current()?.auto).toEqual({ rule: { kind: "command", command: "rm -r" } });
    expect(ipc.judge).not.toHaveBeenCalled();

    decideCall(TAB, { kind: "reject", reason: "" });
    await until(() => ipc.result.mock.calls.length === 1);
    expect(ipc.result.mock.calls[0][2]).toMatchObject({ status: "rejected", judged: false });
  });

  it("asks without the judge while the screen asks for a password, and gives the judge the screen otherwise", async () => {
    screen.rows = ["$ sudo apt upgrade", "[sudo] password for root:"];
    await respond("auto", [call("call_1", "send_input", { text: "hunter2", key: "enter" })]);
    await until(() => current()?.state === "approval");
    expect(current()?.auto).toEqual({ rule: { kind: "password" } });
    expect(ipc.judge).not.toHaveBeenCalled();
    resetForLock();

    screen.rows = ["$ sudo apt upgrade", "Do you want to continue? [Y/n]", ""];
    ipc.judge.mockResolvedValue(verdict("ask", ["destroys_data"]));
    await respond("auto", [call("call_2", "send_input", { text: "y", key: "enter" })]);
    await until(() => current()?.state === "approval");
    expect(ipc.judge).toHaveBeenCalledWith("conv-1", "call_2", "$ sudo apt upgrade\nDo you want to continue? [Y/n]");
  });

  it("never asks the judge in manual approval, and stores the result without a verdict", async () => {
    await respond("manual", [call("call_1", "run_command", { command: "sudo systemctl restart nginx" })]);
    await until(() => current()?.state === "approval");
    expect(current()?.auto ?? null).toBeNull();
    decideCall(TAB, { kind: "run", edited: null });
    await until(() => ipc.run.mock.calls.length === 1);
    expect(ipc.run).toHaveBeenCalledWith("conv-1", "call_1", "s1", null, false);
    expect(ipc.judge).not.toHaveBeenCalled();
  });

  it("asks the judge about a response's calls at once, but not about calls it will not decide", async () => {
    const pending: ((v: AiAutoVerdict) => void)[] = [];
    ipc.judge.mockImplementation(() => new Promise((resolve) => pending.push(resolve)));
    await respond("auto", [
      call("call_1", "run_command", { command: "sudo systemctl restart nginx" }),
      call("call_2", "write_file", { path: "/srv/app/maintenance.html", content: "<h1>Back soon</h1>" }),
      call("call_3", "fetch_url", { url: "https://nginx.org/en/docs/" }),
      // Only reads, so it runs without asking.
      call("call_4", "run_command", { command: "ls -la /srv" }),
      // A hard rule sends it to the card.
      call("call_5", "run_command", { command: "sudo reboot" }),
    ]);
    await until(() => current()?.state === "judging");
    expect(ipc.judge.mock.calls.map((c) => c[1])).toEqual(["call_1", "call_2", "call_3"]);

    for (const resolve of pending) resolve(verdict("run"));
    await until(() => current()?.id === "call_5" && current()?.state === "approval");
    expect(ipc.run.mock.calls.map((c) => [c[1], c[4]])).toEqual([
      ["call_1", true],
      ["call_2", true],
      ["call_3", true],
      ["call_4", false],
    ]);
    expect(ipc.judge).toHaveBeenCalledTimes(3);
  });

  it("reads the saved approvals before asking ahead, so a call they cover never reaches the judge", async () => {
    // Right after an unlock, the saved approvals are still being read.
    let read!: (list: AiAllowance[]) => void;
    ipc.allowances.mockReturnValue(new Promise((resolve) => (read = resolve)));
    ipc.judge.mockResolvedValue(verdict("run"));
    await respond("auto", [
      call("call_1", "run_command", { command: "sudo systemctl restart nginx" }),
      call("call_2", "fetch_url", { url: "https://nginx.org/en/docs/" }),
    ]);
    await flush();
    expect(ipc.judge).not.toHaveBeenCalled();

    read([{ kind: "domain", host_id: null, target: null, value: "nginx.org" }]);
    await until(() => ipc.run.mock.calls.length === 2);
    expect(ipc.judge.mock.calls.map((c) => c[1])).toEqual(["call_1"]);
    expect(ipc.run.mock.calls.map((c) => [c[1], c[4]])).toEqual([
      ["call_1", true],
      ["call_2", false],
    ]);
  });

  it("judges a call again when its judge was replaced after it was judged ahead", async () => {
    const unavailable: AiAutoVerdict = { ...verdict("ask"), answers: [], failure: "unavailable" };
    let replaced = false;
    ipc.judge.mockImplementation(async (_conv, callId) =>
      replaced ? unavailable : callId === "call_1" ? verdict("ask", ["destroys_data"]) : verdict("run"),
    );
    await respond("auto", [
      call("call_1", "run_command", { command: "sudo systemctl restart nginx" }),
      call("call_2", "write_file", { path: "/srv/app/maintenance.html", content: "<h1>Back soon</h1>" }),
    ]);
    // call_2's verdict came ahead while call_1's card waits; then the user replaces the judge.
    await until(() => current()?.id === "call_1" && current()?.state === "approval");
    expect(ipc.judge.mock.calls.map((c) => c[1])).toEqual(["call_1", "call_2"]);
    replaced = true;
    ipc.settings.mockResolvedValue(settings({ provider_id: "p1", model_id: "m1" }));
    decideCall(TAB, { kind: "run", edited: null });

    // The old judge's "run" does not count: the judge chosen now decides, and asks.
    await until(() => current()?.id === "call_2" && current()?.state === "approval");
    expect(ipc.judge.mock.calls.map((c) => c[1])).toEqual(["call_1", "call_2", "call_2"]);
    expect(current()?.auto).toMatchObject({ verdict: { failure: "unavailable" } });
    expect(ipc.run.mock.calls.map((c) => c[1])).toEqual(["call_1"]);
  });

  it("judges a call again when its judge was replaced after it asked ahead, and the new judge decides", async () => {
    let replaced = false;
    ipc.judge.mockImplementation(async () => (replaced ? verdict("run") : verdict("ask", ["destroys_data"])));
    await respond("auto", [
      call("call_1", "run_command", { command: "sudo systemctl restart nginx" }),
      call("call_2", "write_file", { path: "/srv/app/maintenance.html", content: "<h1>Back soon</h1>" }),
    ]);
    await until(() => current()?.id === "call_1" && current()?.state === "approval");
    replaced = true;
    ipc.settings.mockResolvedValue(settings({ provider_id: "p1", model_id: "m1" }));
    decideCall(TAB, { kind: "run", edited: null });

    // The old judge's "ask" does not count either: the judge chosen now lets the call run.
    await until(() => ipc.run.mock.calls.length === 2);
    expect(ipc.judge.mock.calls.map((c) => c[1])).toEqual(["call_1", "call_2", "call_2"]);
    expect(ipc.run.mock.calls.map((c) => [c[1], c[4]])).toEqual([
      ["call_1", true],
      ["call_2", true],
    ]);
  });

  it("runs a call judged ahead when its judge is still the one chosen", async () => {
    ipc.judge.mockResolvedValue(verdict("run"));
    await respond("auto", [call("call_1", "write_file", { path: "/srv/app/maintenance.html", content: "<h1>Back soon</h1>" })]);
    await until(() => ipc.run.mock.calls.length === 1);
    expect(ipc.judge).toHaveBeenCalledTimes(1);
    expect(ipc.run).toHaveBeenCalledWith("conv-1", "call_1", "s1", null, true);
  });

  it("shows the card as manual approval does after a switch to it while the judge answers", async () => {
    let answer!: (v: AiAutoVerdict) => void;
    ipc.judge.mockImplementation(() => new Promise((resolve) => (answer = resolve)));
    await respond("auto", [call("call_1", "run_command", { command: "sudo systemctl restart nginx" })]);
    await until(() => current()?.state === "judging");
    setSlotMode(TAB, "manual");
    answer(verdict("run"));
    await until(() => current()?.state === "approval");
    expect(current()?.auto ?? null).toBeNull();
    decideCall(TAB, { kind: "run", edited: null });
    await until(() => ipc.run.mock.calls.length === 1);
    expect(ipc.run).toHaveBeenCalledWith("conv-1", "call_1", "s1", null, false);
  });

  it("asks, saying so, when the judge cannot be asked", async () => {
    ipc.judge.mockRejectedValue({ code: "internal", detail: "boom" });
    await respond("auto", [call("call_1", "run_command", { command: "sudo systemctl restart nginx" })]);
    await until(() => current()?.state === "approval");
    expect(current()?.auto).toMatchObject({ verdict: { decision: "ask", failure: "error" } });
  });
});
