import { describe, expect, it } from "vitest";
import type { AiEntryView, AiFileChange, AiTurnEvent } from "@/ipc/types";
import {
  callRevealKey,
  editedFiles,
  entriesBefore,
  laterContextStart,
  laterMessages,
  LOCAL_ENTRY,
  newTurn,
  placeUserEntry,
  reduceTurnEvent,
  revealKey,
  unansweredCalls,
  upsertEntry,
  type TurnSnapshot,
} from "./turn";

const start: TurnSnapshot = { entries: [], turn: newTurn(), outcome: null, contextStart: null };
const run = (events: AiTurnEvent[], s: TurnSnapshot = start) => events.reduce(reduceTurnEvent, s);

const assistant: AiEntryView = {
  role: "assistant",
  entry_id: "e2",
  created_at: 0,
  provider_id: "p",
  model_id: "m",
  text: "Checking the disk.",
  reasoning: "The user wants disk usage.",
  tool_calls: [
    { id: "c1", name: "run_command", arguments: '{"command":"df -h"}' },
    { id: "c2", name: "read_terminal", arguments: "{}" },
  ],
  finish: "tool_calls",
  usage: { input_tokens: 10, output_tokens: 5, estimated: false },
};

describe("reduceTurnEvent (§13.1)", () => {
  it("accumulates a streamed response until its entry is stored", () => {
    const s = run([
      { kind: "request_started" },
      { kind: "reasoning", delta: "The user " },
      { kind: "reasoning", delta: "wants disk usage." },
      { kind: "text", delta: "Checking " },
      { kind: "text", delta: "the disk." },
      { kind: "tool_call", id: "c1", name: "run_command", arguments: '{"command":"df -h"}' },
      { kind: "usage", input_tokens: 10, output_tokens: 5, estimated: false },
    ]);
    expect(s.turn?.phase).toBe("streaming");
    expect(s.turn?.live).toEqual({
      text: "Checking the disk.",
      reasoning: "The user wants disk usage.",
      toolCalls: [{ id: "c1", name: "run_command", arguments: '{"command":"df -h"}' }],
      usage: { input_tokens: 10, output_tokens: 5, estimated: false },
    });

    const stored = run([{ kind: "entry", entry: assistant }, { kind: "done", finish: "tool_calls" }], s);
    expect(stored.turn?.live).toBeNull();
    expect(stored.turn?.phase).toBe("tools");
    expect(stored.entries).toEqual([assistant]);
  });

  it("ends with the reason and keeps the provider error for Retry", () => {
    const s = run([
      { kind: "request_started" },
      { kind: "error", status: 529, message: "Overloaded" },
      { kind: "turn_ended", reason: "error" },
    ]);
    expect(s.turn).toBeNull();
    expect(s.outcome).toEqual({ reason: "error", status: 529, message: "Overloaded" });

    expect(run([{ kind: "turn_ended", reason: "length" }]).outcome).toEqual({ reason: "length" });
    expect(run([{ kind: "turn_ended", reason: "completed" }]).outcome).toBeNull();
  });

  it("keeps the part of a stopped reply that Rust stored, and says it was stopped", () => {
    const user: AiEntryView = { role: "user", entry_id: "e1", created_at: 0, text: "df?" };
    const stopped: AiEntryView = { ...assistant, text: "The disk is 4", reasoning: null, tool_calls: [], finish: "stop", usage: null };
    const s = run([
      { kind: "request_started" },
      { kind: "reasoning", delta: "Thinking" },
      { kind: "text", delta: "The disk is 4" },
      // Rust stores what streamed and sends it before the end of the turn (§13.1).
      { kind: "entry", entry: stopped },
      { kind: "turn_ended", reason: "stopped" },
    ], { ...start, entries: [user] });
    expect(s.entries).toEqual([user, stopped]);
    expect(s.turn).toBeNull();
    expect(s.outcome).toEqual({ reason: "stopped" });
  });

  it("says when a reply ended without an answer, keeping its reasoning (§13.1)", () => {
    const user: AiEntryView = { role: "user", entry_id: "e1", created_at: 0, text: "df?" };
    const empty: AiEntryView = { ...assistant, text: "", reasoning: "Thinking it over.", tool_calls: [], finish: "stop" };
    const s = run([
      { kind: "request_started" },
      { kind: "reasoning", delta: "Thinking it over." },
      { kind: "entry", entry: empty },
      { kind: "done", finish: "stop" },
      { kind: "turn_ended", reason: "empty" },
    ], { ...start, entries: [user] });
    expect(s.entries).toEqual([user, empty]);
    expect(s.turn).toBeNull();
    expect(s.outcome).toEqual({ reason: "empty" });
    // Retry starts a new request, which clears it.
    expect(run([{ kind: "request_started" }], { ...s, turn: newTurn() }).outcome).toBeNull();
  });

  it("clears the last outcome when a new request starts", () => {
    const s = run([{ kind: "request_started" }], { entries: [], turn: newTurn(), outcome: { reason: "refused" }, contextStart: null });
    expect(s.outcome).toBeNull();
    expect(s.turn?.phase).toBe("waiting");
  });

  it("notes a thinking level the provider refused, for the rest of the turn (AI-05)", () => {
    const s = run([{ kind: "request_started" }, { kind: "effort_ignored" }, { kind: "text", delta: "Hi" }, { kind: "turn_ended", reason: "completed" }]);
    expect(s.effortIgnored).toBe(true);
    expect(s.turn).toBeNull();
    expect(run([{ kind: "request_started" }]).effortIgnored).toBeUndefined();
  });

  it("ignores deltas when no turn is tracked but keeps entries", () => {
    const idle: TurnSnapshot = { entries: [], turn: null, outcome: null, contextStart: null };
    expect(run([{ kind: "text", delta: "x" }], idle)).toEqual(idle);
    expect(run([{ kind: "entry", entry: assistant }], idle).entries).toHaveLength(1);
  });
});

describe("automatic compaction (AI-22)", () => {
  const summary: AiEntryView = { role: "summary", entry_id: "e9", created_at: 0, text: "Summary" };

  it("moves the context start to a summary that arrives during a turn", () => {
    const s = run([{ kind: "request_started" }, { kind: "entry", entry: summary }], { ...start, contextStart: "e0" });
    expect(s.contextStart).toBe("e9");
    expect(s.entries).toEqual([summary]);
    expect(s.turn?.phase).toBe("waiting");
  });

  it("keeps the context start for other entries", () => {
    expect(run([{ kind: "entry", entry: assistant }], { ...start, contextStart: "e0" }).contextStart).toBe("e0");
  });

  it("never moves the start back to an older one a command returned", () => {
    const entries: AiEntryView[] = [
      { role: "summary", entry_id: "s1", created_at: 0, text: "old" },
      { role: "user", entry_id: "u1", created_at: 0, text: "hi" },
      { role: "summary", entry_id: "s2", created_at: 0, text: "new" },
    ];
    expect(laterContextStart(entries, "s2", "s1")).toBe("s2");
    expect(laterContextStart(entries, "s1", "s2")).toBe("s2");
    expect(laterContextStart(entries, null, "s1")).toBe("s1");
    expect(laterContextStart(entries, "s2", null)).toBe("s2");
    expect(laterContextStart(entries, "gone", "s1")).toBe("s1");
  });
});

describe("edit and resend (AI-26)", () => {
  const entries: AiEntryView[] = [
    { role: "user", entry_id: "u1", created_at: 0, text: "first" },
    assistant,
    { role: "tool", entry_id: "t1", created_at: 0, tool_call_id: "c1", status: "ok", content: "ok" },
    { role: "user", entry_id: "u2", created_at: 0, text: "second" },
  ];

  it("keeps the entries before the edited message", () => {
    expect(entriesBefore(entries, "u2").map((e) => e.entry_id)).toEqual(["u1", "e2", "t1"]);
    expect(entriesBefore(entries, "u1")).toEqual([]);
  });

  it("counts the messages that would be deleted, not their tool results", () => {
    expect(laterMessages(entries, "u1")).toBe(2);
    expect(laterMessages(entries, "u2")).toBe(0);
    expect(laterMessages(entries, "missing")).toBe(0);
  });
});

describe("the file changes an edit deletes (AI-26, AI-41)", () => {
  const response = (id: string, calls: [string, string][]): AiEntryView => ({
    ...assistant,
    entry_id: id,
    tool_calls: calls.map(([callId, name]) => ({ id: callId, name, arguments: "{}" })),
  });
  const result = (id: string, callId: string, status: "ok" | "error" = "ok"): AiEntryView => ({ role: "tool", entry_id: id, created_at: 0, tool_call_id: callId, status, content: "" });
  const change = (responseId: string, callId: string, state: AiFileChange["state"] = "written"): AiFileChange => ({
    response_id: responseId,
    tool_call_id: callId,
    server: "root@10.0.0.5:22",
    path: "/etc/app.conf",
    created: false,
    state,
  });
  const entries: AiEntryView[] = [
    { role: "user", entry_id: "u1", created_at: 0, text: "first" },
    response("a1", [["c1", "edit_file"]]),
    result("t1", "c1"),
    { role: "user", entry_id: "u2", created_at: 0, text: "second" },
    response("a2", [
      ["c2", "write_file"],
      ["c3", "edit_file"],
      ["c4", "run_command"],
      ["c5", "write_file"],
      ["c6", "edit_file"],
    ]),
    result("t2", "c2"),
    result("t3", "c3"),
    result("t4", "c4"),
    result("t5", "c5", "error"),
    result("t6", "c6"),
  ];

  it("offers to revert the kept changes of the deleted calls, oldest first, unfinished ones too", () => {
    const changes = [change("a1", "c1"), change("a2", "c2"), change("a2", "c3", "reverted"), change("a2", "c5", "unfinished"), change("a2", "c6", "dropped")];
    expect(editedFiles(entries, changes, "u2").revert).toEqual([change("a2", "c2"), change("a2", "c5", "unfinished")]);
    expect(editedFiles(entries, changes, "u1").revert).toEqual([change("a1", "c1"), change("a2", "c2"), change("a2", "c5", "unfinished")]);
  });

  it("counts the calls that wrote and whose change this device does not keep, or dropped", () => {
    // c6 wrote and has no change here; c5 failed, c4 is no file tool, and c2 and c3 are kept.
    expect(editedFiles(entries, [change("a2", "c2"), change("a2", "c3", "reverted")], "u2").untracked).toBe(1);
    expect(editedFiles(entries, [change("a2", "c2"), change("a2", "c3", "reverted"), change("a2", "c6", "dropped")], "u2").untracked).toBe(1);
    expect(editedFiles(entries, [], "u2").untracked).toBe(3);
    expect(editedFiles(entries, [], "u1").untracked).toBe(4);
    // c5 failed partway: its dropped change stays on the host as well, and counts once with c6.
    expect(editedFiles(entries, [change("a2", "c2"), change("a2", "c3", "reverted"), change("a2", "c5", "dropped")], "u2").untracked).toBe(2);
  });

  it("finds nothing after the newest message or for one it does not have", () => {
    expect(editedFiles(entries, [change("a2", "c2")], "missing")).toEqual({ revert: [], untracked: 0 });
    const last: AiEntryView[] = [...entries, { role: "user", entry_id: "u3", created_at: 0, text: "third" }];
    expect(editedFiles(last, [change("a2", "c2")], "u3")).toEqual({ revert: [], untracked: 0 });
  });

  it("reveals a result in the block of its own response's call when responses reuse call ids", () => {
    const reused: AiEntryView[] = [...entries, { role: "user", entry_id: "u3", created_at: 0, text: "third" }, response("a3", [["c1", "edit_file"]]), result("t7", "c1")];
    const at = (id: string) => reused.find((e) => e.entry_id === id)!;
    expect(revealKey(reused, at("t1"))).toBe(callRevealKey("a1", "c1"));
    expect(revealKey(reused, at("t7"))).toBe(callRevealKey("a3", "c1"));
    expect(revealKey(reused, at("u2"))).toBe("entry:u2");
    expect(revealKey(reused, at("a2"))).toBe("entry:a2");
    expect(revealKey([result("t9", "c9")], result("t9", "c9"))).toBeNull();
  });
});

describe("entries", () => {
  it("upserts by entry id", () => {
    const a = upsertEntry([], assistant);
    const b = upsertEntry(a, { ...assistant, text: "changed" });
    expect(b).toHaveLength(1);
    expect(b[0].role === "assistant" && b[0].text).toBe("changed");
  });

  it("lists the newest response's calls without a result, in order", () => {
    const result: AiEntryView = { role: "tool", entry_id: "e3", created_at: 0, tool_call_id: "c1", status: "ok", content: "ok" };
    expect(unansweredCalls([assistant]).map((c) => c.id)).toEqual(["c1", "c2"]);
    expect(unansweredCalls([assistant, result]).map((c) => c.id)).toEqual(["c2"]);
    expect(unansweredCalls([])).toEqual([]);
  });

  it("pairs results with calls that share an id in order, as Rust does", () => {
    const shared: AiEntryView = {
      ...assistant,
      tool_calls: [
        { id: "dup", name: "read_skill", arguments: '{"name":"a"}' },
        { id: "dup", name: "read_skill", arguments: '{"name":"b"}' },
      ],
    };
    const result = (n: number): AiEntryView => ({ role: "tool", entry_id: `e3${n}`, created_at: 0, tool_call_id: "dup", status: "ok", content: "ok" });
    expect(unansweredCalls([shared]).map((c) => c.arguments)).toEqual(['{"name":"a"}', '{"name":"b"}']);
    expect(unansweredCalls([shared, result(1)]).map((c) => c.arguments)).toEqual(['{"name":"b"}']);
    expect(unansweredCalls([shared, result(1), result(2)])).toEqual([]);
  });

  it("keeps stored entries in entry id order, after them the pending message", () => {
    const user = (id: string): AiEntryView => ({ role: "user", entry_id: id, created_at: 0, text: id });
    const pending = user(LOCAL_ENTRY);
    expect(upsertEntry([user("0001"), pending], user("0002")).map((e) => e.entry_id)).toEqual(["0001", "0002", LOCAL_ENTRY]);
    expect(upsertEntry([user("0001"), user("0003")], user("0002")).map((e) => e.entry_id)).toEqual(["0001", "0002", "0003"]);
    expect(upsertEntry([user("0001")], user("0009")).map((e) => e.entry_id)).toEqual(["0001", "0009"]);
  });
});

describe("a message sent after an automatic compaction (AI-22)", () => {
  // UUIDv7 ids, as Rust makes them: later entries sort after earlier ones.
  const id = (n: number) => `0192f0aa-0000-7000-8000-${n.toString().padStart(12, "0")}`;
  const before: AiEntryView[] = [
    { role: "user", entry_id: id(1), created_at: 0, text: "first" },
    { ...assistant, entry_id: id(2) },
    { role: "tool", entry_id: id(3), created_at: 0, tool_call_id: "c1", status: "ok", content: "ok" },
    { role: "tool", entry_id: id(4), created_at: 0, tool_call_id: "c2", status: "ok", content: "ok" },
  ];
  const summary: AiEntryView = { role: "summary", entry_id: id(5), created_at: 0, text: "Summary" };
  const stored: AiEntryView = { role: "user", entry_id: id(6), created_at: 0, text: "second" };
  const local: AiEntryView = { role: "user", entry_id: LOCAL_ENTRY, created_at: 0, text: "second" };

  it("shows the summary before the message, as stored, when the summary arrives first", () => {
    // `ai_send` sends the summary on the turn's channel, then returns the stored message.
    const s = run([{ kind: "entry", entry: summary }], { ...start, entries: [...before, local] });
    expect(s.entries.map((e) => e.entry_id)).toEqual([id(1), id(2), id(3), id(4), id(5), LOCAL_ENTRY]);
    expect(s.contextStart).toBe(id(5));
    const placed = placeUserEntry(s.entries, stored);
    expect(placed.map((e) => e.entry_id)).toEqual([id(1), id(2), id(3), id(4), id(5), id(6)]);
    // Everything from the context start on is the message's context: the message is inside it.
    expect(placed.findIndex((e) => e.entry_id === s.contextStart)).toBeLessThan(placed.findIndex((e) => e.entry_id === id(6)));
  });

  it("does the same when the stored message is placed before the summary arrives", () => {
    const placed = placeUserEntry([...before, local], stored);
    const s = run([{ kind: "entry", entry: summary }], { ...start, entries: placed });
    expect(s.entries.map((e) => e.entry_id)).toEqual([id(1), id(2), id(3), id(4), id(5), id(6)]);
  });

  it("places an edited message after the summary of what came before it (AI-26)", () => {
    const edited = entriesBefore([...before, { role: "user", entry_id: id(5), created_at: 0, text: "old" }], id(5));
    const s = run([{ kind: "entry", entry: { ...summary, entry_id: id(7) } }], { ...start, entries: [...edited, local] });
    const placed = placeUserEntry(s.entries, { ...stored, entry_id: id(8) });
    expect(placed.map((e) => e.entry_id)).toEqual([id(1), id(2), id(3), id(4), id(7), id(8)]);
  });
});
