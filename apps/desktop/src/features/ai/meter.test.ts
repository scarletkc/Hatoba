import { describe, expect, it } from "vitest";
import type { AiEntryView, AiUsage } from "@/ipc/types";
import { contextUsage, estimateTokens, formatTokens } from "./meter";

const user = (id: string, text: string): AiEntryView => ({ role: "user", entry_id: id, created_at: 0, text });
const tool = (id: string, content: string): AiEntryView => ({ role: "tool", entry_id: id, created_at: 0, tool_call_id: "c", status: "ok", content });
const summary = (id: string, text: string): AiEntryView => ({ role: "summary", entry_id: id, created_at: 0, text });
const assistant = (id: string, usage: AiUsage | null, model = "m1"): AiEntryView => ({
  role: "assistant",
  entry_id: id,
  created_at: 0,
  provider_id: "p",
  model_id: model,
  text: "ok",
  reasoning: null,
  tool_calls: [],
  finish: "stop",
  usage,
});
const M1 = { provider_id: "p", model_id: "m1" };

describe("context meter (AI-20)", () => {
  it("adds an estimate for text after the last reported usage", () => {
    const entries = [user("1", "hi"), assistant("2", { input_tokens: 900, output_tokens: 100, estimated: false }), tool("3", "x".repeat(400))];
    const m = contextUsage({ entries, contextStart: null, model: M1, contextWindow: 2000 });
    expect(m.tokens).toBe(1100);
    expect(m.ratio).toBeCloseTo(0.55);
    expect(m.estimated).toBe(false);
    expect(m.warn).toBe(false);
  });

  it("counts streamed text and warns at 80%", () => {
    const entries = [user("1", "hi"), assistant("2", { input_tokens: 1500, output_tokens: 100, estimated: false })];
    const m = contextUsage({ entries, contextStart: null, model: M1, contextWindow: 2000, pending: "y".repeat(40) });
    expect(m.tokens).toBe(1610);
    expect(m.warn).toBe(true);
  });

  it("marks estimates: server-estimated usage, no usage yet, and after a model switch", () => {
    const est = [user("1", "hi"), assistant("2", { input_tokens: 10, output_tokens: 5, estimated: true })];
    expect(contextUsage({ entries: est, contextStart: null, model: M1, contextWindow: null }).estimated).toBe(true);

    const fresh = [user("1", "x".repeat(8))];
    const m = contextUsage({ entries: fresh, contextStart: null, model: M1, contextWindow: null });
    expect(m).toMatchObject({ tokens: 2, estimated: true, ratio: null, window: null, warn: false });

    const switched = [user("1", "hi"), assistant("2", { input_tokens: 10, output_tokens: 5, estimated: false })];
    expect(contextUsage({ entries: switched, contextStart: null, model: { provider_id: "p", model_id: "m2" }, contextWindow: 100 }).estimated).toBe(true);
  });

  it("counts only entries from context_start (AI-21)", () => {
    const entries = [
      user("1", "a".repeat(4000)),
      assistant("2", { input_tokens: 5000, output_tokens: 500, estimated: false }),
      summary("3", "s".repeat(40)),
      user("4", "q".repeat(8)),
    ];
    const m = contextUsage({ entries, contextStart: "3", model: M1, contextWindow: 1000 });
    expect(m.tokens).toBe(12);
    expect(m.estimated).toBe(true);
  });

  it("is empty for an empty conversation", () => {
    expect(contextUsage({ entries: [], contextStart: null, model: null, contextWindow: 1000 })).toMatchObject({ tokens: 0, estimated: false, ratio: 0 });
  });
});

describe("token helpers", () => {
  it("estimates about four characters per token", () => {
    expect(estimateTokens("")).toBe(0);
    expect(estimateTokens("abcde")).toBe(2);
  });

  it("formats counts compactly", () => {
    expect(formatTokens(950)).toBe("950");
    expect(formatTokens(1200)).toBe("1.2k");
    expect(formatTokens(4000)).toBe("4k");
    expect(formatTokens(128_400)).toBe("128k");
    expect(formatTokens(1_000_000)).toBe("1M");
    expect(formatTokens(1_250_000)).toBe("1.3M");
  });
});
