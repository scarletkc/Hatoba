import { describe, expect, it } from "vitest";
import { cleanOutput, OutputCapture, readBuffer, screenNote, SCREEN_NOTE_LINES, stripAnsi, type BufferLike } from "./terminalText";

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

  it("returns the last lines up to the last one with text, joining soft wraps, and counts all lines", () => {
    expect(readBuffer(buffer(rows, 2), 6, 2)).toEqual({ text: "defghij klmnop\n$", alternate: false, shown: 2, total: 5 });
    // The count reaches into the scrollback when the screen has fewer lines.
    expect(readBuffer(buffer(rows, 2), 6, 4)).toEqual({ text: "old 2\n$ echo abc\ndefghij klmnop\n$", alternate: false, shown: 4, total: 5 });
  });

  it("keeps spaces at the wrap point and trims trailing blank lines", () => {
    const r = readBuffer(buffer([["abc def   "], ["ghi", true], [""]], 0, "normal", 10), 3, 100);
    expect(r.text).toBe("abc def   ghi");
  });

  it("stops at the first row and reads a wrapped line whole", () => {
    expect(readBuffer(buffer(rows, 2), 6, 1000)).toEqual({ text: "old 1\nold 2\n$ echo abc\ndefghij klmnop\n$", alternate: false, shown: 5, total: 5 });
    const wrapped = buffer([["aaaaaaaaaa"], ["bbbbbbbbbb", true], ["cc", true], ["$ "]], 3);
    expect(readBuffer(wrapped, 1, 2)).toEqual({ text: "aaaaaaaaaabbbbbbbbbbcc\n$", alternate: false, shown: 2, total: 2 });
    // A wrapped line above the ones read counts once.
    expect(readBuffer(wrapped, 1, 1)).toEqual({ text: "$", alternate: false, shown: 1, total: 2 });
  });

  it("reads at least one line and reports an empty terminal", () => {
    expect(readBuffer(buffer(rows, 2), 6, 0).text).toBe("$");
    expect(readBuffer(buffer([[""], [""]], 0), 2, 100)).toEqual({ text: "", alternate: false, shown: 0, total: 0 });
  });

  it("reads as many lines as a terminal_state note by default", () => {
    const long = Array.from({ length: 30 }, (_, i): [string] => [`row ${i}`]);
    expect(readBuffer(buffer(long, 0), 30, null)).toMatchObject({ shown: SCREEN_NOTE_LINES, total: 30 });
    expect(readBuffer(buffer(long, 0, "alternate"), 30, null)).toMatchObject({ shown: 30, total: 30 });
  });

  it("reports the alternate screen", () => {
    const alt = buffer([["top - 09:41"], ["Tasks: 1"]], 0, "alternate");
    expect(readBuffer(alt, 2, 100)).toEqual({ text: "top - 09:41\nTasks: 1", alternate: true, shown: 2, total: 2 });
    expect(readBuffer(alt, 2, 1)).toEqual({ text: "Tasks: 1", alternate: true, shown: 1, total: 2 });
  });
});

describe("screenNote (AI-46)", () => {
  it("takes the screen's last lines up to the last one with text, without scrollback", () => {
    const rows: [string, boolean?][] = [["old"], ...Array.from({ length: 30 }, (_, i): [string] => [`line ${i}`]), ["$ "], [""], [""]];
    const s = screenNote(buffer(rows, 1, "normal"), 33);
    expect(s.alternate).toBe(false);
    const lines = s.text.split("\n");
    expect(lines).toHaveLength(SCREEN_NOTE_LINES);
    expect(lines[0]).toBe("line 11");
    expect(lines.at(-1)).toBe("$");
    expect(s.text).not.toContain("old");
    // A short screen goes whole.
    expect(screenNote(buffer([["$ make"], ["error"], ["$ "]], 0), 3)).toEqual({ text: "$ make\nerror\n$", alternate: false });
  });

  it("takes the whole screen of a full-screen program", () => {
    const rows = Array.from({ length: 30 }, (_, i): [string] => [`row ${i}`]);
    const s = screenNote(buffer(rows, 0, "alternate"), 30);
    expect(s).toEqual({ text: rows.map((r) => r[0]).join("\n"), alternate: true });
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
