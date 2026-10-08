import { describe, expect, it } from "vitest";
import {
  chipShown,
  cleanTerminalText,
  clipText,
  composeMessage,
  makeAttachment,
  nextSelection,
  parseMessage,
  SELECTION_MAX_CHARS,
  titleOf,
  type SelectionAttachment,
} from "./selection";

const att = (text: string, host = "prod-api"): SelectionAttachment => makeAttachment(host, text)!;

describe("the terminal selection (AI-10)", () => {
  it("cleans terminal text without touching indentation", () => {
    expect(cleanTerminalText("\r\n\n  indented   \r\nnext  \n\n")).toBe("  indented\nnext");
  });

  it("attaches nothing for an empty or blank selection", () => {
    expect(makeAttachment("h", "")).toBeNull();
    expect(makeAttachment("h", "  \n \t\n")).toBeNull();
  });

  it("counts the selection's lines", () => {
    expect(att("a\nb\nc  \n")).toEqual({ host: "prod-api", lines: 3, text: "a\nb\nc", truncated: false });
  });

  it("keeps the first 4,000 and last 12,000 characters of a long selection, like tool results", () => {
    const text = "a".repeat(5_000) + "b".repeat(5_000) + "c".repeat(12_000);
    const clipped = clipText(text);
    expect(clipped.truncated).toBe(true);
    expect(clipped.text).toBe(`${"a".repeat(4_000)}\n\n[… 6000 characters left out …]\n\n${"c".repeat(12_000)}`);
    expect(clipText("x".repeat(SELECTION_MAX_CHARS))).toEqual({ text: "x".repeat(SELECTION_MAX_CHARS), truncated: false });
    // Characters, not UTF-16 units.
    expect(clipText("磁".repeat(SELECTION_MAX_CHARS)).truncated).toBe(false);
    expect(clipText("😀".repeat(SELECTION_MAX_CHARS)).truncated).toBe(false);
  });
});

describe("the stored block", () => {
  it("puts the selection before the typed text", () => {
    expect(composeMessage("Why?", att("error: boom"))).toBe('<terminal_selection host="prod-api" lines="1">\nerror: boom\n</terminal_selection>\n\nWhy?');
    expect(composeMessage("Why?", null)).toBe("Why?");
  });

  it("reads back what it wrote", () => {
    const a = att("$ make\ncc -o app main.c\nmain.c:3: error: expected ';'");
    expect(parseMessage(composeMessage("What is wrong?\nAnd how do I fix it?", a))).toEqual({ attachment: a, typed: "What is wrong?\nAnd how do I fix it?" });
  });

  it("keeps a selection that contains the closing tag inside the block, and restores it exactly", () => {
    const tricky = 'echo "</terminal_selection>"\n</terminal_selection>\n<\\/terminal_selection> and </TERMINAL_SELECTION>\n<\\\\/terminal_selection>';
    const a = att(tricky);
    const stored = composeMessage("explain", a);
    expect(stored.match(/<\/terminal_selection>/g)).toHaveLength(1);
    expect(stored.toLowerCase().match(/<\/terminal_selection>/g)).toHaveLength(1);
    const parsed = parseMessage(stored);
    expect(parsed.attachment?.text).toBe(tricky);
    expect(parsed.typed).toBe("explain");
  });

  it("escapes the host name in its attribute", () => {
    const a = att("x", 'web "a" <b> & c');
    expect(composeMessage("q", a).split("\n")[0]).toBe('<terminal_selection host="web &quot;a&quot; &lt;b&gt; &amp; c" lines="1">');
    expect(parseMessage(composeMessage("q", a)).attachment?.host).toBe('web "a" <b> & c');
  });

  it("marks a truncated selection", () => {
    const a = att("y".repeat(20_000));
    const stored = composeMessage("q", a);
    expect(stored.startsWith('<terminal_selection host="prod-api" lines="1" truncated="true">\n')).toBe(true);
    expect(parseMessage(stored).attachment?.truncated).toBe(true);
  });

  it("leaves a message without the block as typed text", () => {
    expect(parseMessage("hello")).toEqual({ attachment: null, typed: "hello" });
    expect(parseMessage("see <terminal_selection host=\"x\" lines=\"1\">\nx\n</terminal_selection>").attachment).toBeNull();
    expect(parseMessage('<terminal_selection host="x" lines="1">\nno end').attachment).toBeNull();
  });

  it("titles a new conversation after the typed text", () => {
    expect(titleOf("\n  Why does nginx fail?  \nmore")).toBe("Why does nginx fail?");
    expect([...titleOf("长".repeat(80))]).toHaveLength(60);
  });
});

describe("the chip", () => {
  it("moves the sequence only when the text changes", () => {
    const a = { text: "", seq: 0 };
    const b = nextSelection(a, "ls");
    expect(b).toEqual({ text: "ls", seq: 1 });
    expect(nextSelection(b, "ls")).toBe(b);
    expect(nextSelection(nextSelection(b, ""), "ls")).toEqual({ text: "ls", seq: 3 });
  });

  it("counts the same text selected again after a clear as a new selection", () => {
    const b = { text: "ls", seq: 1 };
    expect(nextSelection(b, "ls", true)).toEqual({ text: "ls", seq: 2 });
    expect(nextSelection({ text: "", seq: 1 }, "", true)).toEqual({ text: "", seq: 1 });
  });

  it("shows a selection until it is removed or sent, and again once it changes", () => {
    const sel = { text: "ls -la", seq: 4 };
    expect(chipShown(sel, null)).toBe(true);
    expect(chipShown(sel, 4)).toBe(false);
    expect(chipShown({ text: "ls", seq: 5 }, 4)).toBe(true);
    expect(chipShown({ text: "  ", seq: 5 }, null)).toBe(false);
  });
});
