import { describe, expect, it } from "vitest";
import { translate, type Locale, type MessageKey, type Params } from "@/i18n";
import type { AiEntryView } from "@/ipc/types";
import { forkCopies, forkModel, forkPoints, forkTitle } from "./fork";

const user = (id: string, text = "question"): AiEntryView => ({ role: "user", entry_id: id, created_at: 0, text });
/** `calls` are the IDs of its tool calls; `model` the model that wrote it. */
const assistant = (id: string, calls: string[] = [], model = "m1"): AiEntryView => ({
  role: "assistant",
  entry_id: id,
  created_at: 0,
  provider_id: "p",
  model_id: model,
  text: "answer",
  reasoning: null,
  tool_calls: calls.map((call) => ({ id: call, name: "run_command", arguments: "{}" })),
  finish: calls.length > 0 ? "tool_calls" : "stop",
  usage: null,
});
const tool = (id: string, call: string): AiEntryView => ({ role: "tool", entry_id: id, created_at: 0, tool_call_id: call, status: "ok", content: "output", auto: null });
const summary = (id: string): AiEntryView => ({ role: "summary", entry_id: id, created_at: 0, text: "Summary." });

describe("forkCopies", () => {
  const entries = [user("u1"), assistant("a1", ["c1", "c2"]), tool("t1", "c1"), tool("t2", "c2"), assistant("a2"), summary("s1"), user("u2"), assistant("a3")];

  it("copies everything up to the point, and the results of a reply's calls with it", () => {
    expect(forkCopies(entries, 1)).toEqual([0, 1, 2, 3]);
    expect(forkCopies(entries, 4)).toEqual([0, 1, 2, 3, 4]);
    expect(forkCopies(entries, 6)).toEqual([0, 1, 2, 3, 4, 5, 6]);
    expect(forkCopies(entries, 7)).toEqual([0, 1, 2, 3, 4, 5, 6, 7]);
  });

  it("refuses results, summaries, pending messages and replies still waiting for a result", () => {
    expect(forkCopies(entries, 2)).toBeNull();
    expect(forkCopies(entries, 5)).toBeNull();
    expect(forkCopies(entries, 8)).toBeNull();
    expect(forkCopies([user("~local")], 0)).toBeNull();
    expect(forkCopies([user("u1"), assistant("a1", ["c1"])], 1)).toBeNull();
  });

  it("takes a result stored after the point, as when it arrived from another device later", () => {
    const merged = [user("u1"), assistant("a1", ["c1"]), user("u2"), tool("t1", "c1"), assistant("a2", ["c2"]), tool("t2", "c2")];
    expect(forkCopies(merged, 1)).toEqual([0, 1, 3]);
    expect(forkCopies(merged, 2)).toEqual([0, 1, 2, 3]);
    // A reused call ID: each call keeps the result that answers it.
    const reused = [user("u1"), assistant("a1", ["c"]), tool("t1", "c"), assistant("a2", ["c"]), tool("t2", "c")];
    expect(forkCopies(reused, 1)).toEqual([0, 1, 2]);
    expect(forkCopies(reused, 3)).toEqual([0, 1, 2, 3, 4]);
  });
});

describe("forkPoints", () => {
  it("lists the user's stored messages and the replies whose calls all have results", () => {
    const entries = [user("u1"), assistant("a1", ["c1"]), tool("t1", "c1"), assistant("a2"), summary("s1"), user("u2"), assistant("a3", ["c2"]), user("~local")];
    expect([...forkPoints(entries)]).toEqual(["u1", "a1", "a2", "u2"]);
    for (const id of forkPoints(entries)) expect(forkCopies(entries, entries.findIndex((e) => e.entry_id === id))).not.toBeNull();
  });
});

describe("forkModel", () => {
  it("goes on with the model that answered the message, when it was another one", () => {
    const entries = [user("u1"), assistant("a1", [], "m1"), user("u2"), assistant("a2", [], "m2"), user("u3"), user("u4"), assistant("a3", [], "m2")];
    expect(forkModel(entries, "u2")).toEqual({ provider_id: "p", model_id: "m2" });
    expect(forkModel(entries, "u1")).toEqual({ provider_id: "p", model_id: "m1" });
    // The same model as the newest copied reply, a message without its own reply, or a reply: no choice.
    expect(forkModel([user("u1"), assistant("a1"), user("u2"), assistant("a2")], "u2")).toBeNull();
    expect(forkModel(entries, "u3")).toBeNull();
    expect(forkModel(entries, "a2")).toBeNull();
    expect(forkModel(entries, "missing")).toBeNull();
  });
});

describe("forkTitle", () => {
  const tr = (locale: Locale) => (key: MessageKey, params?: Params) => translate(locale, key, params);

  it("marks the original's title in the panel's language", () => {
    expect(forkTitle(tr("en"), "Disk usage", [])).toBe("Disk usage (fork)");
    expect(forkTitle(tr("zh-CN"), "磁盘空间", [])).toBe("磁盘空间（分支）");
    expect(forkTitle(tr("ja"), "ディスク", [])).toBe("ディスク（分岐）");
    expect(forkTitle(tr("en"), "  ", [])).toBe("Untitled conversation (fork)");
  });

  it("numbers a fork whose title History has already", () => {
    const taken = ["Disk usage", "Disk usage (fork)", "Disk usage (fork 2)", "Other (fork 3)"];
    expect(forkTitle(tr("en"), "Disk usage", taken)).toBe("Disk usage (fork 3)");
    expect(forkTitle(tr("zh-CN"), "磁盘空间", ["磁盘空间（分支）"])).toBe("磁盘空间（分支 2）");
    expect(forkTitle(tr("ja"), "ディスク", ["ディスク（分岐）"])).toBe("ディスク（分岐 2）");
  });

  it("takes off the marks a fork's title ends with, in any language, numbered or not", () => {
    const taken = ["Disk usage (fork)"];
    expect(forkTitle(tr("en"), "Disk usage (fork)", taken)).toBe("Disk usage (fork 2)");
    expect(forkTitle(tr("en"), "Disk usage (fork 7)", taken)).toBe("Disk usage (fork 2)");
    expect(forkTitle(tr("en"), "Disk usage（分支 2）", taken)).toBe("Disk usage (fork 2)");
    expect(forkTitle(tr("zh-CN"), "Disk usage (fork) (fork)", [])).toBe("Disk usage（分支）");
    expect(forkTitle(tr("ja"), "磁盘空间（分支）", [])).toBe("磁盘空间（分岐）");
    // Only a mark at the end, and only a whole one.
    expect(forkTitle(tr("en"), "(fork) of nginx", [])).toBe("(fork) of nginx (fork)");
    expect(forkTitle(tr("en"), "Disk usage (forks)", [])).toBe("Disk usage (forks) (fork)");
    expect(forkTitle(tr("en"), "(fork)", [])).toBe("Untitled conversation (fork)");
  });

  it("cuts a long title so the mark still fits in 200 characters", () => {
    for (const locale of ["en", "zh-CN", "ja"] as const) {
      const title = forkTitle(tr(locale), "日".repeat(200), []);
      expect([...title].length).toBe(200);
      expect(title.endsWith(translate(locale, "ai.fork.title", { title: "" }).trim())).toBe(true);
      const numbered = forkTitle(tr(locale), "日".repeat(200), [title]);
      expect([...numbered].length).toBe(200);
      expect(numbered.endsWith(translate(locale, "ai.fork.titleNumbered", { title: "", n: 2 }).trim())).toBe(true);
    }
    // A cut that ends at a space leaves none before the mark.
    expect(forkTitle(tr("en"), `${"a".repeat(192)} b`, [])).toBe(`${"a".repeat(192)} (fork)`);
  });
});
