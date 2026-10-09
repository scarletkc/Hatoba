import { describe, expect, it } from "vitest";
import {
  baseName,
  checkFile,
  chipShown,
  cleanTerminalText,
  clipText,
  composeMessage,
  droppedFile,
  FILE_MAX_BYTES,
  fitsMessage,
  isLongPaste,
  lineCount,
  makeAttachment,
  makeDiagnostics,
  makePaste,
  MESSAGE_MAX_BYTES,
  modelName,
  nextSelection,
  noteBlock,
  orderAttachments,
  parseMessage,
  SELECTION_MAX_CHARS,
  textFile,
  type Attachment,
  type DiagnosticsAttachment,
  type FileAttachment,
  type Note,
  type PasteAttachment,
  type SelectionAttachment,
} from "./attachments";

const att = (text: string, host = "prod-api"): SelectionAttachment => makeAttachment(host, text)!;
const DIAG = "Hatoba connection diagnostics\nHost: staging-web-02 (172.31.40.8:22)\nUser: ubuntu\nError: ETIMEDOUT (ssh/timeout)";
const diag = (text = DIAG, host = "staging-web-02"): DiagnosticsAttachment => makeDiagnostics(host, text)!;
const paste = (text: string): PasteAttachment => makePaste(text)!;
const file = (name: string, text: string): FileAttachment => {
  const r = textFile(name, new TextEncoder().encode(text));
  if (!r.ok) throw new Error(r.reason);
  return r.file;
};
/** A block on its own, as composeMessage writes it before the typed text. */
const blockOf = (a: Attachment) => composeMessage("", [a]).replace(/\n\n$/, "");

describe("the terminal selection (AI-10)", () => {
  it("cleans terminal text without touching indentation", () => {
    expect(cleanTerminalText("\r\n\n  indented   \r\nnext  \n\n")).toBe("  indented\nnext");
  });

  it("attaches nothing for an empty or blank selection", () => {
    expect(makeAttachment("h", "")).toBeNull();
    expect(makeAttachment("h", "  \n \t\n")).toBeNull();
  });

  it("counts the selection's lines", () => {
    expect(att("a\nb\nc  \n")).toEqual({ kind: "selection", host: "prod-api", lines: 3, text: "a\nb\nc", truncated: false });
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

describe("the stored blocks", () => {
  it("puts the selection before the typed text", () => {
    expect(composeMessage("Why?", [att("error: boom")])).toBe('<terminal_selection host="prod-api" lines="1">\nerror: boom\n</terminal_selection>\n\nWhy?');
    expect(composeMessage("Why?", [])).toBe("Why?");
  });

  it("reads back what it wrote", () => {
    const a = att("$ make\ncc -o app main.c\nmain.c:3: error: expected ';'");
    expect(parseMessage(composeMessage("What is wrong?\nAnd how do I fix it?", [a]))).toEqual({ notes: [], attachments: [a], typed: "What is wrong?\nAnd how do I fix it?" });
  });

  it("keeps a selection that contains the closing tag inside the block, and restores it exactly", () => {
    const tricky = 'echo "</terminal_selection>"\n</terminal_selection>\n<\\/terminal_selection> and </TERMINAL_SELECTION>\n<\\\\/terminal_selection>';
    const stored = composeMessage("explain", [att(tricky)]);
    expect(stored.match(/<\/terminal_selection>/g)).toHaveLength(1);
    expect(stored.toLowerCase().match(/<\/terminal_selection>/g)).toHaveLength(1);
    const parsed = parseMessage(stored);
    expect(parsed.attachments[0].text).toBe(tricky);
    expect(parsed.typed).toBe("explain");
  });

  it("escapes the host name in its attribute", () => {
    const a = att("x", 'web "a" <b> & c');
    expect(composeMessage("q", [a]).split("\n")[0]).toBe('<terminal_selection host="web &quot;a&quot; &lt;b&gt; &amp; c" lines="1">');
    expect(parseMessage(composeMessage("q", [a])).attachments[0]).toEqual(a);
  });

  it("marks a truncated selection", () => {
    const stored = composeMessage("q", [att("y".repeat(20_000))]);
    expect(stored.startsWith('<terminal_selection host="prod-api" lines="1" truncated="true">\n')).toBe(true);
    expect(parseMessage(stored).attachments[0]).toMatchObject({ kind: "selection", truncated: true });
  });

  it("leaves a message without a block as typed text", () => {
    expect(parseMessage("hello")).toEqual({ notes: [], attachments: [], typed: "hello" });
    for (const text of [
      'see <terminal_selection host="x" lines="1">\nx\n</terminal_selection>',
      '<terminal_selection host="x" lines="1">\nno end',
      // Attributes must be the kind's own, in order, as Rust's title_of requires.
      '<terminal_selection lines="1" host="x">\nx\n</terminal_selection>\n\nq',
      '<terminal_selection host="x" lines="one">\nx\n</terminal_selection>\n\nq',
      '<connection_diagnostics host="x" lines="1">\nx\n</connection_diagnostics>\n\nq',
      '<pasted_text>\nx\n</pasted_text>\n\nq',
      '<pasted_text lines="1" name="x">\nx\n</pasted_text>\n\nq',
      '<file lines="1" name="a.txt">\nx\n</file>\n\nq',
      '<file name="a.txt">\nx\n</file>\n\nq',
      '<image name="a.png" lines="1">\nx\n</image>\n\nq',
    ]) {
      expect(parseMessage(text), text).toEqual({ notes: [], attachments: [], typed: text });
    }
  });

  it("ends a block only at a closing tag that a line break or the end follows, as Rust's title_of does", () => {
    const stored = '<terminal_selection host="x" lines="3">\na\n</terminal_selection> tail\nb\n</terminal_selection>\nwhat now';
    expect(parseMessage(stored)).toEqual({
      notes: [],
      attachments: [{ kind: "selection", host: "x", lines: 3, truncated: false, text: "a\n</terminal_selection> tail\nb" }],
      typed: "what now",
    });
  });
});

describe("connection diagnostics (AI-10)", () => {
  it("writes a block with the host's display name only", () => {
    expect(composeMessage("Why?", [diag("Error: ETIMEDOUT")])).toBe('<connection_diagnostics host="staging-web-02">\nError: ETIMEDOUT\n</connection_diagnostics>\n\nWhy?');
  });

  it("attaches nothing without text, and clips long text like a selection", () => {
    expect(makeDiagnostics("h", " \r\n ")).toBeNull();
    expect(makeDiagnostics("h", "a\r\nb\r\n")).toEqual({ kind: "diagnostics", host: "h", text: "a\nb" });
    expect(makeDiagnostics("h", "z".repeat(20_000))!.text).toContain("[… 4000 characters left out …]");
  });

  it("escapes its own closing tag, not the selection's", () => {
    const text = "Detail: </connection_diagnostics> and </terminal_selection>";
    const stored = composeMessage("q", [diag(text)]);
    expect(stored).toContain("Detail: <\\/connection_diagnostics> and </terminal_selection>");
    expect(parseMessage(stored).attachments[0].text).toBe(text);
  });

  it("reads a message that is only a block", () => {
    const d = diag();
    expect(parseMessage(blockOf(d))).toEqual({ notes: [], attachments: [d], typed: "" });
  });
});

describe("long pastes (AI-35)", () => {
  it("turns a paste of 2,000 characters or 30 lines into an attachment", () => {
    expect(isLongPaste("x".repeat(1_999))).toBe(false);
    expect(isLongPaste("x".repeat(2_000))).toBe(true);
    expect(isLongPaste("a\n".repeat(29))).toBe(false);
    expect(isLongPaste("a\n".repeat(30))).toBe(true);
    expect(isLongPaste("a\r\n".repeat(30))).toBe(true);
  });

  it("keeps the whole text, with no cut, and counts lines without a final break", () => {
    const long = "line\n".repeat(10_000);
    const p = paste(long);
    expect(p).toEqual({ kind: "paste", lines: 10_000, text: long });
    expect(paste("a\r\nb")).toEqual({ kind: "paste", lines: 2, text: "a\nb" });
    expect(makePaste(" \n\t ")).toBeNull();
    expect(lineCount("")).toBe(0);
    expect(lineCount("a")).toBe(1);
    expect(lineCount("a\n")).toBe(1);
    expect(lineCount("a\n\n")).toBe(2);
  });

  it("writes a block with the line count and escapes its own closing tag", () => {
    const text = "x </pasted_text> y\n</pasted_text>";
    const stored = composeMessage("q", [paste(text)]);
    expect(stored).toBe('<pasted_text lines="2">\nx <\\/pasted_text> y\n<\\/pasted_text>\n</pasted_text>\n\nq');
    expect(parseMessage(stored)).toEqual({ notes: [], attachments: [paste(text)], typed: "q" });
  });
});

describe("text files (AI-35)", () => {
  it("keeps only a file's base name, escaped in its attribute", () => {
    expect(baseName("C:\\Users\\kc\\notes.txt")).toBe("notes.txt");
    expect(baseName("/home/kc/notes.txt")).toBe("notes.txt");
    const f = file('/home/kc/a "b" <c>.log', "ok\n");
    expect(f).toEqual({ kind: "file", name: 'a "b" <c>.log', lines: 1, text: "ok\n" });
    const stored = composeMessage("q", [f]);
    expect(stored.split("\n")[0]).toBe('<file name="a &quot;b&quot; &lt;c&gt;.log" lines="1">');
    expect(parseMessage(stored)).toEqual({ notes: [], attachments: [f], typed: "q" });
  });

  it("accepts UTF-8 text and refuses NUL bytes and invalid UTF-8", () => {
    expect(textFile("a.txt", new TextEncoder().encode("日本語 ✓\r\n"))).toEqual({ ok: true, file: { kind: "file", name: "a.txt", lines: 1, text: "日本語 ✓\n" } });
    expect(textFile("a.bin", new Uint8Array([0x61, 0x00, 0x62]))).toEqual({ ok: false, reason: "binary" });
    expect(textFile("a.bin", new Uint8Array([0xc3, 0x28]))).toEqual({ ok: false, reason: "binary" });
    expect(textFile("big.txt", new Uint8Array(FILE_MAX_BYTES + 1).fill(0x61))).toEqual({ ok: false, reason: "too_large" });
    expect(textFile("max.txt", new Uint8Array(FILE_MAX_BYTES).fill(0x61)).ok).toBe(true);
  });

  it("refuses images by type or name, and large files before reading them", () => {
    expect(checkFile("shot.png", "", 10)).toEqual({ ok: false, reason: "image" });
    expect(checkFile("photo", "image/jpeg", 10)).toEqual({ ok: false, reason: "image" });
    expect(checkFile("logo.SVG", "", 10)).toEqual({ ok: false, reason: "image" });
    expect(checkFile("big.log", "text/plain", FILE_MAX_BYTES + 1)).toEqual({ ok: false, reason: "too_large" });
    expect(checkFile("app.log", "text/plain", 100)).toBeNull();
  });

  it("makes files Rust read from a drop on the desktop app into the same attachments", () => {
    // What `textFile` makes of the same bytes.
    const bytes = new TextEncoder().encode("server {\r\n  listen 80;\r\n}\n");
    const read = textFile("nginx.conf", bytes);
    expect(droppedFile({ status: "ok", name: "nginx.conf", text: "server {\r\n  listen 80;\r\n}\n" })).toEqual(read);
    expect(droppedFile({ status: "ok", name: "C:\\Users\\kc\\x.md", text: "x" })).toEqual({ ok: true, file: { kind: "file", name: "x.md", lines: 1, text: "x" } });
    // Each refusal keeps its reason and names the file, never a path.
    for (const reason of ["image", "too_large", "binary", "unreadable"] as const)
      expect(droppedFile({ status: "refused", name: "/home/kc/data.bin", reason })).toEqual({ ok: false, name: "data.bin", reason });
    // The size cap holds here too, in UTF-8 bytes.
    expect(droppedFile({ status: "ok", name: "big.txt", text: "é".repeat(FILE_MAX_BYTES / 2 + 1) })).toEqual({ ok: false, name: "big.txt", reason: "too_large" });
    expect(droppedFile({ status: "ok", name: "max.txt", text: "a".repeat(FILE_MAX_BYTES) }).ok).toBe(true);
  });
});

describe("several attachments in one message (AI-35)", () => {
  it("stores the diagnostics, then the selection, then pastes and files in the order they were added", () => {
    const d = diag();
    const s = att("$ ssh db-1");
    const p1 = paste("first paste");
    const f1 = file("a.conf", "server {}\n");
    const p2 = paste("second paste");
    const f2 = file("b.log", "boom");
    const list: Attachment[] = [p1, f1, s, p2, d, f2];
    expect(orderAttachments(list)).toEqual([d, s, p1, f1, p2, f2]);
    const stored = composeMessage("What now?", list);
    expect(stored.indexOf("<connection_diagnostics")).toBe(0);
    expect(parseMessage(stored)).toEqual({ notes: [], attachments: [d, s, p1, f1, p2, f2], typed: "What now?" });
  });

  it("keeps at most one diagnostics and one selection", () => {
    const list = [att("one"), att("two"), diag("a"), diag("b")];
    expect(orderAttachments(list)).toEqual([diag("a"), att("one")]);
  });

  it("reads blocks in any order, and a second diagnostics or selection block is typed text", () => {
    const d = diag();
    const s = att("ls");
    const f = file("x.txt", "x");
    expect(parseMessage(`${blockOf(f)}\n\n${blockOf(s)}\n\n${blockOf(d)}\n\nq`)).toEqual({ notes: [], attachments: [f, s, d], typed: "q" });
    expect(parseMessage(`${blockOf(d)}\n\n${blockOf(d)}\n\nq`)).toEqual({ notes: [], attachments: [d], typed: `${blockOf(d)}\n\nq` });
    expect(parseMessage(`${blockOf(f)}\n\n${blockOf(f)}\n\nq`)).toEqual({ notes: [], attachments: [f, f], typed: "q" });
  });

  it("caps all attachments of a message at 512 KB together", () => {
    const quarter = paste("x".repeat(128 * 1024));
    const three = [quarter, quarter, quarter];
    expect(fitsMessage(three, quarter)).toBe(true);
    expect(fitsMessage([...three, quarter], paste("y"))).toBe(false);
    // UTF-8 bytes, not characters: 日 is three bytes.
    expect(fitsMessage([], paste("日".repeat(Math.floor(MESSAGE_MAX_BYTES / 3))))).toBe(true);
    expect(fitsMessage([], paste("日".repeat(Math.floor(MESSAGE_MAX_BYTES / 3) + 1)))).toBe(false);
  });

  it("stores a message near the cap whole", () => {
    const big = file("big.log", "0123456789abcdef\n".repeat(15_000));
    const p = paste("p".repeat(200_000));
    const stored = composeMessage("look", [big, p]);
    expect(stored.length).toBeGreaterThan(450_000);
    expect(parseMessage(stored)).toEqual({ notes: [], attachments: [big, p], typed: "look" });
  });
});

describe("Hatoba's notes (AI-05, AI-09)", () => {
  const moved: Note = { kind: "host_change", from: "staging-web", to: "prod-db" };
  const switched: Note = { kind: "model_change", from: "Claude Sonnet 5.5 (claude-sonnet-5-5)", to: "Claude Opus 5.5 (claude-opus-5-5)" };

  it("are written exactly as Rust writes them", () => {
    // host_change_block and model_change_block in crates/hatoba-ai/src/tools.rs.
    expect(noteBlock(moved)).toBe(
      '<host_change from="staging-web" to="prod-db">\nThe conversation moved to another host. Screens and command output before this message came from "staging-web".\n</host_change>',
    );
    expect(noteBlock(switched)).toBe(
      '<model_change from="Claude Sonnet 5.5 (claude-sonnet-5-5)" to="Claude Opus 5.5 (claude-opus-5-5)">\nEarlier replies in this conversation came from another model.\n</model_change>',
    );
    expect(noteBlock({ kind: "host_change", from: "", to: "prod-db" })).toBe(
      '<host_change from="" to="prod-db">\nThe conversation moved to another host. Screens and command output before this message came from another host.\n</host_change>',
    );
    expect(noteBlock({ kind: "host_change", from: 'a"b</host_change>', to: "c&d\n" })).toBe(
      '<host_change from="a&quot;b&lt;/host_change&gt;" to="c&amp;d">\nThe conversation moved to another host. Screens and command output before this message came from "a"b<\\/host_change>".\n</host_change>',
    );
  });

  it("come first, the host's before the model's, then the attachments and the typed text", () => {
    const d = diag();
    const s = att("$ psql");
    const p = paste("a long paste");
    const stored = composeMessage("Why is it slow?", [p, s, d], [switched, moved]);
    expect(stored.indexOf("<host_change")).toBe(0);
    expect(stored.indexOf("<model_change")).toBeLessThan(stored.indexOf("<connection_diagnostics"));
    expect(parseMessage(stored)).toEqual({ notes: [moved, switched], attachments: [d, s, p], typed: "Why is it slow?" });
    // Without attachments, and a message that is only notes.
    expect(parseMessage(composeMessage("hi", [], [moved]))).toEqual({ notes: [moved], attachments: [], typed: "hi" });
    expect(parseMessage(composeMessage("", [], [moved, switched]))).toEqual({ notes: [moved, switched], attachments: [], typed: "" });
  });

  it("read back names with quotes, ampersands and closing tags", () => {
    const odd: Note = { kind: "host_change", from: 'db "main" <1> & </host_change>', to: "x" };
    expect(parseMessage(composeMessage("q", [], [odd]))).toEqual({ notes: [odd], attachments: [], typed: "q" });
  });

  it("are typed text after an attachment, as a second note of a kind, or with other attributes", () => {
    const f = file("x.txt", "x");
    const afterAttachment = `${blockOf(f)}\n\n${noteBlock(moved)}\n\nq`;
    expect(parseMessage(afterAttachment)).toEqual({ notes: [], attachments: [f], typed: `${noteBlock(moved)}\n\nq` });
    const twice = `${noteBlock(moved)}\n\n${noteBlock(moved)}\n\nq`;
    expect(parseMessage(twice)).toEqual({ notes: [moved], attachments: [], typed: `${noteBlock(moved)}\n\nq` });
    for (const text of ['<host_change to="b" from="a">\nx\n</host_change>\n\nq', '<model_change from="a">\nx\n</model_change>\n\nq']) {
      expect(parseMessage(text), text).toEqual({ notes: [], attachments: [], typed: text });
    }
  });

  it("name a model by its display name in the panel", () => {
    expect(modelName("Claude Opus 5.5 (claude-opus-5-5)")).toBe("Claude Opus 5.5");
    expect(modelName("GPT-5 (preview) (gpt-5-preview)")).toBe("GPT-5 (preview)");
    expect(modelName("qwen3:8b")).toBe("qwen3:8b");
  });
});

describe("the selection chip", () => {
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
