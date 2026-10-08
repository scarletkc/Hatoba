import { describe, expect, it } from "vitest";
import type { AiEntryView, AiTurnEvent } from "@/ipc/types";
import { newTurn, reduceTurnEvent, unansweredCalls, upsertEntry, type TurnSnapshot } from "./turn";

const start: TurnSnapshot = { entries: [], turn: newTurn(), outcome: null };
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
    const s = run([{ kind: "request_started" }], { entries: [], turn: newTurn(), outcome: { reason: "refused" } });
    expect(s.outcome).toBeNull();
    expect(s.turn?.phase).toBe("waiting");
  });

  it("ignores deltas when no turn is tracked but keeps entries", () => {
    const idle: TurnSnapshot = { entries: [], turn: null, outcome: null };
    expect(run([{ kind: "text", delta: "x" }], idle)).toEqual(idle);
    expect(run([{ kind: "entry", entry: assistant }], idle).entries).toHaveLength(1);
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
