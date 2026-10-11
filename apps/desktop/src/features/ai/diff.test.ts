import { describe, expect, it } from "vitest";
import { diffLines, diffStats, withContext, type DiffRow } from "./diff";

/** A diff as `diff -u` writes its lines: " " unchanged, "-" removed, "+" added, "…n" a gap. */
function unified(rows: DiffRow[]): string[] {
  return rows.map((r) => (r.kind === "gap" ? `…${r.count}` : `${r.kind === "same" ? " " : r.kind === "del" ? "-" : "+"}${r.text}`));
}

describe("the diff on the approval card (AI-39, AI-40)", () => {
  it("numbers lines on both sides and lists removals before additions", () => {
    const lines = diffLines("a\nb\nc\n", "a\nB\nc\nd\n");
    expect(lines).toEqual([
      { kind: "same", text: "a", old: 1, new: 1 },
      { kind: "del", text: "b", old: 2, new: null },
      { kind: "add", text: "B", old: null, new: 2 },
      { kind: "same", text: "c", old: 3, new: 3 },
      { kind: "add", text: "d", old: null, new: 4 },
    ]);
    expect(diffStats(lines)).toEqual({ added: 2, removed: 1 });
    // Several changed lines in a row: every removal first.
    expect(unified(withContext(diffLines("x\n1\n2\ny\n", "x\none\ntwo\ny\n")))).toEqual([" x", "-1", "-2", "+one", "+two", " y"]);
  });

  it("keeps three lines of context and folds the rest", () => {
    const before = Array.from({ length: 20 }, (_, i) => `line ${i + 1}`).join("\n") + "\n";
    const after = before.replace("line 10\n", "line ten\n");
    expect(unified(withContext(diffLines(before, after)))).toEqual(["…6", " line 7", " line 8", " line 9", "-line 10", "+line ten", " line 11", " line 12", " line 13", "…7"]);
    // Two changes close together share their context.
    const two = after.replace("line 14\n", "line fourteen\n");
    const rows = withContext(diffLines(before, two));
    expect(rows.filter((r) => r.kind === "gap")).toEqual([
      { kind: "gap", count: 6 },
      { kind: "gap", count: 3 },
    ]);
    expect(withContext(diffLines(before, before))).toEqual([]);
  });

  it("shows a new file as added lines and an emptied one as removed lines", () => {
    expect(unified(withContext(diffLines("", "hello\nworld\n")))).toEqual(["+hello", "+world"]);
    expect(unified(withContext(diffLines("bye\n", "")))).toEqual(["-bye"]);
  });

  it("marks a line break added or removed at the end", () => {
    const lines = diffLines("a\nb", "a\nb\n");
    expect(lines).toEqual([
      { kind: "same", text: "a", old: 1, new: 1 },
      { kind: "del", text: "b", old: 2, new: null, noEol: true },
      { kind: "add", text: "b", old: null, new: 2 },
    ]);
    // Without a final line break on either side, nothing is marked.
    expect(diffLines("a\nb", "a\nc").some((l) => l.noEol)).toBe(false);
  });

  it("stays cheap on a large file with a small change", () => {
    const before = Array.from({ length: 50_000 }, (_, i) => `row ${i}`).join("\n");
    const after = before.replace("row 25000\n", "row 25000 changed\n");
    const rows = withContext(diffLines(before, after));
    expect(diffStats(diffLines(before, after))).toEqual({ added: 1, removed: 1 });
    expect(rows).toHaveLength(10);
  });
});
