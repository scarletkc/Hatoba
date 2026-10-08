import { describe, expect, it } from "vitest";
import {
  chipShown,
  cleanTerminalText,
  clipText,
  composeMessage,
  makeAttachment,
  makeDiagnostics,
  nextSelection,
  parseMessage,
  SELECTION_MAX_CHARS,
  type DiagnosticsAttachment,
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

const DIAG = "Hatoba connection diagnostics\nHost: staging-web-02 (172.31.40.8:22)\nUser: ubuntu\nError: ETIMEDOUT (ssh/timeout)";
const diag = (text = DIAG, host = "staging-web-02"): DiagnosticsAttachment => makeDiagnostics(host, text)!;

describe("the stored blocks", () => {
  it("puts the selection before the typed text", () => {
    expect(composeMessage("Why?", { selection: att("error: boom") })).toBe('<terminal_selection host="prod-api" lines="1">\nerror: boom\n</terminal_selection>\n\nWhy?');
    expect(composeMessage("Why?", {})).toBe("Why?");
  });

  it("reads back what it wrote", () => {
    const a = att("$ make\ncc -o app main.c\nmain.c:3: error: expected ';'");
    expect(parseMessage(composeMessage("What is wrong?\nAnd how do I fix it?", { selection: a }))).toEqual({
      diagnostics: null,
      selection: a,
      typed: "What is wrong?\nAnd how do I fix it?",
    });
  });

  it("keeps a selection that contains the closing tag inside the block, and restores it exactly", () => {
    const tricky = 'echo "</terminal_selection>"\n</terminal_selection>\n<\\/terminal_selection> and </TERMINAL_SELECTION>\n<\\\\/terminal_selection>';
    const stored = composeMessage("explain", { selection: att(tricky) });
    expect(stored.match(/<\/terminal_selection>/g)).toHaveLength(1);
    expect(stored.toLowerCase().match(/<\/terminal_selection>/g)).toHaveLength(1);
    const parsed = parseMessage(stored);
    expect(parsed.selection?.text).toBe(tricky);
    expect(parsed.typed).toBe("explain");
  });

  it("escapes the host name in its attribute", () => {
    const a = att("x", 'web "a" <b> & c');
    expect(composeMessage("q", { selection: a }).split("\n")[0]).toBe('<terminal_selection host="web &quot;a&quot; &lt;b&gt; &amp; c" lines="1">');
    expect(parseMessage(composeMessage("q", { selection: a })).selection?.host).toBe('web "a" <b> & c');
  });

  it("marks a truncated selection", () => {
    const stored = composeMessage("q", { selection: att("y".repeat(20_000)) });
    expect(stored.startsWith('<terminal_selection host="prod-api" lines="1" truncated="true">\n')).toBe(true);
    expect(parseMessage(stored).selection?.truncated).toBe(true);
  });

  it("leaves a message without a block as typed text", () => {
    expect(parseMessage("hello")).toEqual({ diagnostics: null, selection: null, typed: "hello" });
    expect(parseMessage('see <terminal_selection host="x" lines="1">\nx\n</terminal_selection>').selection).toBeNull();
    expect(parseMessage('<terminal_selection host="x" lines="1">\nno end').selection).toBeNull();
    // Attributes must be the kind's own, in order, as Rust's title_of requires.
    expect(parseMessage('<terminal_selection lines="1" host="x">\nx\n</terminal_selection>\n\nq').selection).toBeNull();
    expect(parseMessage('<terminal_selection host="x" lines="one">\nx\n</terminal_selection>\n\nq').selection).toBeNull();
    expect(parseMessage('<connection_diagnostics host="x" lines="1">\nx\n</connection_diagnostics>\n\nq').diagnostics).toBeNull();
  });

  it("ends a block only at a closing tag that a line break or the end follows, as Rust's title_of does", () => {
    const stored = '<terminal_selection host="x" lines="3">\na\n</terminal_selection> tail\nb\n</terminal_selection>\nwhat now';
    expect(parseMessage(stored)).toEqual({
      diagnostics: null,
      selection: { host: "x", lines: 3, truncated: false, text: "a\n</terminal_selection> tail\nb" },
      typed: "what now",
    });
  });
});

describe("connection diagnostics (AI-10)", () => {
  it("writes a block with the host's display name only", () => {
    expect(composeMessage("Why?", { diagnostics: diag("Error: ETIMEDOUT") })).toBe(
      '<connection_diagnostics host="staging-web-02">\nError: ETIMEDOUT\n</connection_diagnostics>\n\nWhy?',
    );
  });

  it("attaches nothing without text, and clips long text like a selection", () => {
    expect(makeDiagnostics("h", " \r\n ")).toBeNull();
    expect(makeDiagnostics("h", "a\r\nb\r\n")).toEqual({ host: "h", text: "a\nb" });
    expect(makeDiagnostics("h", "z".repeat(20_000))!.text).toContain("[… 4000 characters left out …]");
  });

  it("escapes its own closing tag, not the selection's", () => {
    const text = "Detail: </connection_diagnostics> and </terminal_selection>";
    const stored = composeMessage("q", { diagnostics: diag(text) });
    expect(stored).toContain("Detail: <\\/connection_diagnostics> and </terminal_selection>");
    expect(parseMessage(stored).diagnostics?.text).toBe(text);
  });

  it("puts the diagnostics before the selection, and reads both back", () => {
    const d = diag();
    const a = att("Connecting to 172.31.40.8:22 (attempt 3)…");
    const stored = composeMessage("What is wrong?", { diagnostics: d, selection: a });
    expect(stored.indexOf("<connection_diagnostics")).toBe(0);
    expect(stored.indexOf("</connection_diagnostics>\n\n<terminal_selection")).toBeGreaterThan(0);
    expect(parseMessage(stored)).toEqual({ diagnostics: d, selection: a, typed: "What is wrong?" });
  });

  it("reads the blocks in either order, at most one of each kind", () => {
    const d = diag();
    const a = att("ls");
    const sel = composeMessage("", { selection: a }).replace(/\n\n$/, "");
    const dia = composeMessage("", { diagnostics: d }).replace(/\n\n$/, "");
    expect(parseMessage(`${sel}\n\n${dia}\n\nq`)).toEqual({ diagnostics: d, selection: a, typed: "q" });
    expect(parseMessage(`${dia}\n\n${dia}\n\nq`)).toEqual({ diagnostics: d, selection: null, typed: `${dia}\n\nq` });
  });

  it("reads a message that is only a block", () => {
    const d = diag();
    expect(parseMessage(composeMessage("", { diagnostics: d }).replace(/\n\n$/, ""))).toEqual({ diagnostics: d, selection: null, typed: "" });
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
