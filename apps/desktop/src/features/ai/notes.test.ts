import { describe, expect, it } from "vitest";
import { translate, type Locale, type MessageKey, type Params } from "@/i18n";
import { composeMessage, parseMessage } from "./attachments";
import { noteLabel, noteTitle } from "./notes";

const tr = (locale: Locale) => (key: MessageKey, params?: Params) => translate(locale, key, params);

describe("notes as dividers (AI-05, AI-09)", () => {
  it("say where the conversation moved and where it was", () => {
    const note = { kind: "host_change", from: "staging-web", to: "prod-db" } as const;
    expect(noteLabel(tr("zh-CN"), note)).toBe("已转到 prod-db（之前在 staging-web）");
    expect(noteLabel(tr("en"), note)).toBe("Moved to prod-db (was on staging-web)");
    expect(noteTitle(tr("en"), note)).toBe("Moved to prod-db (was on staging-web)");
    // The earlier host was deleted.
    expect(noteLabel(tr("en"), { ...note, from: "" })).toBe("Moved to prod-db (was on Deleted host)");
  });

  it("name models by their names, with the IDs in the tooltip", () => {
    const note = { kind: "model_change", from: "Claude Sonnet 5.5 (claude-sonnet-5-5)", to: "Claude Opus 5.5 (claude-opus-5-5)" } as const;
    expect(noteLabel(tr("zh-CN"), note)).toBe("已切换到 Claude Opus 5.5（之前是 Claude Sonnet 5.5）");
    expect(noteLabel(tr("en"), note)).toBe("Switched to Claude Opus 5.5 (was Claude Sonnet 5.5)");
    expect(noteTitle(tr("en"), note)).toBe("Switched to Claude Opus 5.5 (claude-opus-5-5) (was Claude Sonnet 5.5 (claude-sonnet-5-5))");
    expect(noteLabel(tr("en"), { ...note, from: "qwen3:8b" })).toBe("Switched to Claude Opus 5.5 (was qwen3:8b)");
  });

  it("come out of a stored message in order, apart from its bubble", () => {
    const stored = composeMessage("Why?", [{ kind: "paste", lines: 1, text: "x" }], [
      { kind: "model_change", from: "A (a)", to: "B (b)" },
      { kind: "host_change", from: "one", to: "two" },
    ]);
    const parts = parseMessage(stored);
    expect(parts.notes.map((n) => noteLabel(tr("en"), n))).toEqual(["Moved to two (was on one)", "Switched to B (was A)"]);
    expect(parts.typed).toBe("Why?");
    expect(parts.attachments).toHaveLength(1);
  });
});
