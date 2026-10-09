import { describe, expect, it } from "vitest";
import { cleanOutput, OutputCapture, readBuffer, stripAnsi, type BufferLike } from "./terminalText";

/** A fake xterm buffer: `[text, isWrapped]` rows, each padded to `cols` like xterm's cells. */
function buffer(rows: [string, boolean?][], baseY: number, type: "normal" | "alternate" = "normal", cols = 10): BufferLike {
  return {
    type,
    length: rows.length,
    baseY,
    getLine: (y) => {
      const row = rows[y];
      if (!row) return undefined;
      const cells = row[0].padEnd(cols, " ");
      return { isWrapped: !!row[1], translateToString: (trim?: boolean) => (trim ? cells.trimEnd() : cells) };
    },
  };
}

describe("readBuffer (AI-11)", () => {
  const rows: [string, boolean?][] = [
    ["old 1"],
    ["old 2"],
    ["$ echo abc"],
    ["defghij kl", false],
    ["mnop", true],
    ["$ "],
    [""],
    [""],
  ];

  it("returns the screen and the requested scrollback, joining soft wraps", () => {
    const r = readBuffer(buffer(rows, 2), 6, 1);
    expect(r).toEqual({ text: "old 2\n$ echo abc\ndefghij klmnop\n$", alternate: false, scrollback: 1 });
  });

  it("keeps spaces at the wrap point and trims trailing blank lines", () => {
    const r = readBuffer(buffer([["abc def   "], ["ghi", true], [""]], 0, "normal", 10), 3, 0);
    expect(r.text).toBe("abc def   ghi");
  });

  it("caps scrollback at what exists and widens to a wrapped line's start", () => {
    expect(readBuffer(buffer(rows, 2), 6, 1000).scrollback).toBe(2);
    const wrapped = buffer([["aaaaaaaaaa"], ["bbbbbbbbbb", true], ["cc", true], ["$ "]], 3);
    const r = readBuffer(wrapped, 1, 1);
    expect(r.text).toBe("aaaaaaaaaabbbbbbbbbbcc\n$");
    expect(r.scrollback).toBe(3);
  });

  it("reports the alternate screen", () => {
    const r = readBuffer(buffer([["top - 09:41"], ["Tasks: 1"]], 0, "alternate"), 2, 100);
    expect(r.alternate).toBe(true);
    expect(r.scrollback).toBe(0);
  });
});

describe("ANSI handling (AI-13)", () => {
  it("strips CSI, OSC and other escapes", () => {
    expect(stripAnsi("\x1b[1;32mok\x1b[0m \x1b]0;title\x07done\x1b(B\x1b=")).toBe("ok done");
    expect(stripAnsi("\x1b]8;;https://x\x1b\\link\x1b]8;;\x1b\\")).toBe("link");
  });

  it("applies carriage returns, backspaces and erase-line like a terminal", () => {
    expect(cleanOutput("$ ls\r\nfile1  file2\r\n$ ")).toBe("$ ls\nfile1  file2\n$");
    expect(cleanOutput("10%\r50%\r100%\r\n")).toBe("100%");
    expect(cleanOutput("downloading 100%\r\x1b[Kdone\n")).toBe("done");
    expect(cleanOutput("abc\b\b\bxy\n")).toBe("xyc");
    expect(cleanOutput("ab\x1b[1Dc\x07\n")).toBe("ac");
    expect(cleanOutput("\x1b[32mgreen\x1b[0m\n\n\n")).toBe("green");
    expect(cleanOutput("a\x1b[5Gb")).toBe("a   b");
  });
});

describe("OutputCapture", () => {
  it("keeps everything while small", () => {
    const c = new OutputCapture(10, 10);
    expect(c.empty).toBe(true);
    c.push("hello ");
    c.push("world\r\n");
    expect(c.text()).toBe("hello world");
  });

  it("keeps the head and the tail of long output", () => {
    const c = new OutputCapture(4, 4);
    c.push("abcd");
    for (let i = 0; i < 10; i++) c.push("xxxx");
    c.push("WXYZ");
    const text = c.text();
    expect(text.startsWith("abcd\n[… ")).toBe(true);
    expect(text.endsWith("WXYZ")).toBe(true);
    expect(text).toContain("characters of output left out");
  });
});
