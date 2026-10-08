import { describe, expect, it } from "vitest";
import type { AiEntryView, AiTurnEvent } from "@/ipc/types";
import { entriesBefore, laterContextStart, laterMessages, newTurn, reduceTurnEvent, unansweredCalls, upsertEntry, type TurnSnapshot } from "./turn";

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

  it("clears the last outcome when a new request starts", () => {
    const s = run([{ kind: "request_started" }], { entries: [], turn: newTurn(), outcome: { reason: "refused" }, contextStart: null });
    expect(s.outcome).toBeNull();
    expect(s.turn?.phase).toBe("waiting");
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
});
