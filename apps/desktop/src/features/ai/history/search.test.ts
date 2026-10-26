import { describe, expect, it } from "vitest";
import { highlightParts, oneLine } from "./search";

describe("search highlights (AI-24)", () => {
  it("marks every case-insensitive match", () => {
    expect(highlightParts("Nginx 502 on nginx", "NGINX")).toEqual([
      { text: "Nginx", match: true },
      { text: " 502 on ", match: false },
      { text: "nginx", match: true },
    ]);
  });

  it("treats the query as text, not a pattern", () => {
    expect(highlightParts("a.b axb", "a.b")).toEqual([
      { text: "a.b", match: true },
      { text: " axb", match: false },
    ]);
  });

  it("returns the text unmarked without a query or a match", () => {
    expect(highlightParts("磁盘用量", "  ")).toEqual([{ text: "磁盘用量", match: false }]);
    expect(highlightParts("磁盘用量", "内存")).toEqual([{ text: "磁盘用量", match: false }]);
    expect(highlightParts("", "x")).toEqual([]);
    expect(highlightParts("查看磁盘用量", "磁盘")).toEqual([
      { text: "查看", match: false },
      { text: "磁盘", match: true },
      { text: "用量", match: false },
    ]);
  });

  it("puts a snippet on one line", () => {
    expect(oneLine("  a\n\n b\tc ")).toBe("a b c");
  });
});
