import { describe, expect, it } from "vitest";
import type { AiEntryView } from "@/ipc/types";
import { replies } from "./replies";

const user = (id: string, text: string): AiEntryView => ({ role: "user", entry_id: id, created_at: 0, text });
const assistant = (id: string, text: string, calls = 0): AiEntryView => ({
  role: "assistant",
  entry_id: id,
  created_at: 0,
  provider_id: "p",
  model_id: "m",
  text,
  reasoning: "Thinking it over.",
  tool_calls: Array.from({ length: calls }, (_, i) => ({ id: `${id}c${i}`, name: "run_command", arguments: "{}" })),
  finish: calls > 0 ? "tool_calls" : "stop",
  usage: null,
});
const tool = (id: string, call: string): AiEntryView => ({ role: "tool", entry_id: id, created_at: 0, tool_call_id: call, status: "ok", content: "output" });

describe("replies", () => {
  it("joins a reply's text across its tool calls, under its last entry", () => {
    const { texts, open } = replies([
      user("u1", "How full is the disk?"),
      assistant("a1", "Let me check.\n", 1),
      tool("t1", "a1c0"),
      assistant("a2", "", 1),
      tool("t2", "a2c0"),
      assistant("a3", "It is **92%** full."),
      user("u2", "Thanks"),
      assistant("a4", "You're welcome."),
    ]);
    expect([...texts]).toEqual([
      ["a3", "Let me check.\n\nIt is **92%** full."],
      ["a4", "You're welcome."],
    ]);
    expect(open).toBe("a4");
  });

  it("ends a reply at its last entry even when only an earlier one has text", () => {
    const { texts, open } = replies([user("u1", "Check"), assistant("a1", "Checking.", 1), tool("t1", "a1c0"), assistant("a2", "", 1)]);
    expect([...texts]).toEqual([["a2", "Checking."]]);
    expect(open).toBe("a2");
  });

  it("keeps a reply without text, and has no open reply after the user's last message", () => {
    const { texts, open } = replies([user("u1", "Run it"), assistant("a1", " ", 1), tool("t1", "a1c0"), user("u2", "Again")]);
    expect([...texts]).toEqual([["a1", ""]]);
    expect(open).toBeNull();
  });
});
