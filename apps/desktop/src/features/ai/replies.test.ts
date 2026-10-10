import { describe, expect, it } from "vitest";
import type { AiEntryView } from "@/ipc/types";
import { replies } from "./replies";

const user = (id: string, text: string, at = 0): AiEntryView => ({ role: "user", entry_id: id, created_at: at, text });
const assistant = (id: string, text: string, calls = 0, at = 0): AiEntryView => ({
  role: "assistant",
  entry_id: id,
  created_at: at,
  provider_id: "p",
  model_id: "m",
  text,
  reasoning: "Thinking it over.",
  tool_calls: Array.from({ length: calls }, (_, i) => ({ id: `${id}c${i}`, name: "run_command", arguments: "{}" })),
  finish: calls > 0 ? "tool_calls" : "stop",
  usage: null,
});
const tool = (id: string, call: string, at = 0): AiEntryView => ({ role: "tool", entry_id: id, created_at: at, tool_call_id: call, status: "ok", content: "output" });
const summary = (id: string, at: number): AiEntryView => ({ role: "summary", entry_id: id, created_at: at, text: "Summary." });

describe("replies", () => {
  it("joins a reply's text across its tool calls, under its last entry", () => {
    const { ends, open } = replies([
      user("u1", "How full is the disk?", 1),
      assistant("a1", "Let me check.\n", 1, 2),
      tool("t1", "a1c0", 3),
      assistant("a2", "", 1, 4),
      tool("t2", "a2c0", 5),
      assistant("a3", "It is **92%** full.", 0, 6),
      user("u2", "Thanks", 7),
      assistant("a4", "You're welcome.", 0, 8),
    ]);
    expect([...ends]).toEqual([
      ["a3", { text: "Let me check.\n\nIt is **92%** full.", at: 6 }],
      ["a4", { text: "You're welcome.", at: 8 }],
    ]);
    expect(open).toBe("a4");
  });

  it("keeps the indentation that makes a code block", () => {
    const { ends } = replies([user("u1", "Show it"), assistant("a1", "\n\n    make deploy\n    make test\n\n")]);
    expect(ends.get("a1")?.text).toBe("    make deploy\n    make test");
  });

  it("ends a reply that stopped after its tool calls when their results came", () => {
    const { ends, open } = replies([user("u1", "Check", 1), assistant("a1", "Checking.", 1, 2), tool("t1", "a1c0", 90), assistant("a2", "", 1, 95), tool("t2", "a2c0", 300), summary("s1", 400)]);
    expect([...ends]).toEqual([["a2", { text: "Checking.", at: 300 }]]);
    expect(open).toBe("a2");
  });

  it("counts a result stored after the user's next message for the reply that asked for it", () => {
    const { ends } = replies([
      user("u1", "Run it", 1),
      assistant("a1", "Running.", 1, 2),
      user("u2", "typed while it ran", 3),
      tool("t1", "a1c0", 50),
      assistant("a2", "Done.", 0, 60),
    ]);
    expect([...ends]).toEqual([
      ["a1", { text: "Running.", at: 50 }],
      ["a2", { text: "Done.", at: 60 }],
    ]);
  });

  it("keeps a reply without text, and has no open reply after the user's last message", () => {
    const { ends, open } = replies([user("u1", "Run it"), assistant("a1", " ", 1, 2), tool("t1", "a1c0", 3), user("u2", "Again", 4)]);
    expect([...ends]).toEqual([["a1", { text: "", at: 3 }]]);
    expect(open).toBeNull();
  });
});
