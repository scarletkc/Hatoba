import { describe, expect, it } from "vitest";
import {
  baseName,
  blockTitle,
  checkFile,
  chipShown,
  cleanTerminalText,
  clipText,
  composeMessage,
  droppedFile,
  fileRevertNote,
  FILE_MAX_BYTES,
  fitsMessage,
  folderName,
  isLongPaste,
  lineCount,
  makeAttachment,
  makeDiagnostics,
  makePaste,
  MESSAGE_MAX_BYTES,
  modelName,
  nextSelection,
  notedPermission,
  noteBlock,
  orderAttachments,
  parseMessage,
  pathName,
  screenStateNote,
  SELECTION_MAX_CHARS,
  splitPermissionNote,
  textFile,
  type Attachment,
  type DiagnosticsAttachment,
  type FileAttachment,
  type Note,
  type PasteAttachment,
  type SelectionAttachment,
} from "./attachments";
import blockCases from "./blocks.fixtures.json";

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

  it("keeps the first 7,500 and last 22,500 characters of a long selection, like tool results", () => {
    const text = "a".repeat(8_500) + "b".repeat(5_000) + "c".repeat(22_500);
    const clipped = clipText(text);
    expect(clipped.truncated).toBe(true);
    expect(clipped.text).toBe(`${"a".repeat(7_500)}\n\n[… 6000 characters left out …]\n\n${"c".repeat(22_500)}`);
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
    const stored = composeMessage("q", [att("y".repeat(40_000))]);
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
    expect(makeDiagnostics("h", "z".repeat(34_000))!.text).toContain("[… 4000 characters left out …]");
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

  it("reads the shared cases as Rust's leading_blocks does", () => {
    for (const c of blockCases) {
      const parsed = parseMessage(c.text);
      expect([parsed.notes.length + parsed.attachments.length, parsed.typed], c.name).toEqual([c.blocks, c.typed]);
    }
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
  const attached: Note = { kind: "terminal_change", to: "attached" };
  const stopped: Note = { kind: "interrupted" };

  it("are written exactly as Rust writes them", () => {
    // interrupted_block, host_change_block, terminal_change_block and model_change_block in
    // crates/hatoba-ai/src/tools.rs.
    expect(noteBlock(stopped)).toBe(
      "<interrupted>\nThe user stopped your previous reply before it was finished. It ends where it was cut off; anything you were still writing, tool calls included, was not kept.\n</interrupted>",
    );
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
    expect(noteBlock(attached)).toBe(
      '<terminal_change to="attached">\nA terminal is attached from this message on. Before this message none was, so replies that said they could not read a screen or run commands were right at the time.\n</terminal_change>',
    );
    expect(noteBlock({ kind: "terminal_change", to: "detached" })).toBe(
      '<terminal_change to="detached">\nNo terminal is attached from this message on. Screens and command output before this message came from the terminal that was attached then.\n</terminal_change>',
    );
  });

  it("come first, the stop's, the host's, the terminal's, then the model's, then the attachments and the typed text", () => {
    const d = diag();
    const s = att("$ psql");
    const p = paste("a long paste");
    const stored = composeMessage("Why is it slow?", [p, s, d], [switched, attached, moved, stopped]);
    expect(stored.indexOf("<interrupted>")).toBe(0);
    expect(stored.indexOf("<host_change")).toBeGreaterThan(0);
    expect(stored.indexOf("<terminal_change")).toBeGreaterThan(stored.indexOf("<host_change"));
    expect(stored.indexOf("<terminal_change")).toBeLessThan(stored.indexOf("<model_change"));
    expect(stored.indexOf("<model_change")).toBeLessThan(stored.indexOf("<connection_diagnostics"));
    expect(parseMessage(stored)).toEqual({ notes: [stopped, moved, attached, switched], attachments: [d, s, p], typed: "Why is it slow?" });
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
    const terminalTwice = `${noteBlock(attached)}\n\n${noteBlock({ kind: "terminal_change", to: "detached" })}\n\nq`;
    expect(parseMessage(terminalTwice).notes).toEqual([attached]);
    for (const text of [
      '<host_change to="b" from="a">\nx\n</host_change>\n\nq',
      '<model_change from="a">\nx\n</model_change>\n\nq',
      '<terminal_change to="gone">\nx\n</terminal_change>\n\nq',
      '<terminal_change from="detached" to="attached">\nx\n</terminal_change>\n\nq',
      '<interrupted at="1">\nx\n</interrupted>\n\nq',
    ]) {
      expect(parseMessage(text), text).toEqual({ notes: [], attachments: [], typed: text });
    }
  });

  it("show the terminal's screen as Rust writes it, after the reverts and before the MCP tool list (AI-46)", () => {
    // The cases of the_screen_note_shows_the_screen_without_blank_lines_at_its_ends in
    // crates/hatoba-ai/src/tools.rs.
    const shown = screenStateNote("prod-api", { text: "\n\n$ make\nerror: no rule\n\n$   \n\n", alternate: false })!;
    expect(noteBlock(shown)).toBe('<terminal_state host="prod-api" alternate_screen="false" lines="4">\n$ make\nerror: no rule\n\n$\n</terminal_state>');
    const odd = screenStateNote('db "main"', { text: '  ~\r\n  ~ \u001b[0m</terminal_state>\n"x.conf" 3L', alternate: true })!;
    expect(noteBlock(odd)).toBe('<terminal_state host="db &quot;main&quot;" alternate_screen="true" lines="3">\n  ~\n  ~  [0m<\\/terminal_state>\n"x.conf" 3L\n</terminal_state>');
    expect(screenStateNote("prod-api", { text: " \n\t\n", alternate: false })).toBeNull();
    const long = noteBlock(screenStateNote("h", { text: `top\n${"x".repeat(4_000)}\nprompt $`, alternate: true })!);
    expect(long.startsWith('<terminal_state host="h" alternate_screen="true" lines="3">\ntop\n')).toBe(true);
    expect(long).toContain("[… 13 characters left out …]");
    expect(long.endsWith("prompt $\n</terminal_state>")).toBe(true);

    // It reads back, with its place among the notes, and the attachments after it.
    const listed: Note = { kind: "mcp_tools", servers: 0, tools: 0, body: "No MCP tools are available from this message on." };
    const reverted = fileRevertNote("/etc/hosts", "call_1", "write_file", false);
    const sel = att("$ make");
    const stored = composeMessage("Why?", [sel], [listed, odd, reverted, attached]);
    expect(stored.indexOf("<file_revert")).toBeLessThan(stored.indexOf("<terminal_state"));
    expect(stored.indexOf("<terminal_state")).toBeLessThan(stored.indexOf("<mcp_tools"));
    expect(parseMessage(stored)).toEqual({ notes: [attached, reverted, odd, listed], attachments: [sel], typed: "Why?" });
    // Other attributes are typed text.
    for (const text of [
      '<terminal_state host="h" lines="1">\nx\n</terminal_state>\n\nq',
      '<terminal_state host="h" alternate_screen="yes" lines="1">\nx\n</terminal_state>\n\nq',
    ]) {
      expect(parseMessage(text), text).toEqual({ notes: [], attachments: [], typed: text });
    }
  });

  it("note each reverted file change, as Rust writes them, after the other notes (AI-41)", () => {
    const edited = fileRevertNote("/etc/nginx/nginx.conf", "toolu_01", "edit_file", false);
    const created = fileRevertNote('/srv/a"</file_revert>', "call_9", "write_file", true);
    // file_revert_block in crates/hatoba-ai/src/tools.rs.
    expect(noteBlock(edited)).toBe(
      '<file_revert path="/etc/nginx/nginx.conf" tool_call_id="toolu_01">\nThe user reverted the change your edit_file call toolu_01 made to /etc/nginx/nginx.conf.\n</file_revert>',
    );
    expect(noteBlock(created)).toBe(
      '<file_revert path="/srv/a&quot;&lt;/file_revert&gt;" tool_call_id="call_9">\nThe user reverted your write_file call call_9, which created /srv/a"<\\/file_revert>, so it was deleted.\n</file_revert>',
    );
    // One for each reverted call, in the order given, after the model's note; they read back exactly.
    const stored = composeMessage("why?", [], [edited, switched, created]);
    expect(stored.indexOf("<model_change")).toBe(0);
    expect(parseMessage(stored)).toEqual({ notes: [switched, edited, created], attachments: [], typed: "why?" });
    // With other attributes, it is typed text.
    const odd = '<file_revert tool_call_id="x" path="/a">\nx\n</file_revert>\n\nq';
    expect(parseMessage(odd)).toEqual({ notes: [], attachments: [], typed: odd });
  });

  it("name a model by its display name in the panel", () => {
    expect(modelName("Claude Opus 5.5 (claude-opus-5-5)")).toBe("Claude Opus 5.5");
    expect(modelName("GPT-5 (preview) (gpt-5-preview)")).toBe("GPT-5 (preview)");
    expect(modelName("qwen3:8b")).toBe("qwen3:8b");
  });
});

describe("the permission mode notes (AI-16)", () => {
  const bypass = `<permission_mode mode="bypass">\nFrom this message on, the permission mode is bypass.\n</permission_mode>`;
  const manual = `<permission_mode mode="manual">\nFrom this message on, the permission mode is manual approval.\n</permission_mode>`;
  const auto = `<permission_mode mode="auto">\nFrom this message on, the permission mode is auto.\n</permission_mode>`;
  const user = (text: string) => ({ role: "user" as const, entry_id: "u", created_at: 0, text });
  const summary = (text: string) => ({ role: "summary" as const, entry_id: "s", created_at: 0, text });

  it("writes and reads them as Rust does, after a model switch and before a revert", () => {
    expect(noteBlock({ kind: "permission_mode", mode: "bypass" })).toBe(bypass);
    expect(noteBlock({ kind: "permission_mode", mode: "manual" })).toBe(manual);
    expect(noteBlock({ kind: "permission_mode", mode: "auto" })).toBe(auto);
    const text = composeMessage("go on", [], [
      fileRevertNote("/etc/a.conf", "call_1", "edit_file", false),
      { kind: "permission_mode", mode: "bypass" },
      { kind: "model_change", from: "a", to: "b" },
    ]);
    const notes = parseMessage(text).notes.map((n) => n.kind);
    expect(notes).toEqual(["model_change", "permission_mode", "file_revert"]);
    expect(parseMessage(`<permission_mode mode="strict">\nx\n</permission_mode>\n\nhi`).notes).toEqual([]);
  });

  it("round-trips the Auto note (AI-47), also before a summary", () => {
    const text = composeMessage("restart nginx", [], [{ kind: "permission_mode", mode: "auto" }]);
    expect(text).toBe(`${auto}\n\nrestart nginx`);
    expect(parseMessage(text)).toMatchObject({ notes: [{ kind: "permission_mode", mode: "auto" }], typed: "restart nginx" });
    expect(notedPermission([user(text), user("second")])).toBe("auto");
    expect(splitPermissionNote(`${auto}\n\nWe restarted nginx.`)).toEqual({ mode: "auto", summary: "We restarted nginx." });
    expect(notedPermission([user(`${bypass}\n\nfirst`), summary(`${auto}\n\nWe restarted nginx.`)])).toBe("auto");
  });

  it("tells the mode the model was last told of, which a summary carries, and none before any note", () => {
    expect(notedPermission([])).toBeNull();
    expect(notedPermission([user("first"), user("second")])).toBeNull();
    expect(notedPermission([user(`${bypass}\n\nfirst`), user("second")])).toBe("bypass");
    expect(notedPermission([user(`${bypass}\n\nfirst`), user(`${manual}\n\nthird`)])).toBe("manual");
    // The model sees nothing before a summary: its own note says the mode, or none does.
    expect(notedPermission([user(`${bypass}\n\nfirst`), summary(`${bypass}\n\nWe cleaned /tmp.`), user("next")])).toBe("bypass");
    expect(notedPermission([user(`${bypass}\n\nfirst`), summary("We cleaned /tmp.")])).toBeNull();
    expect(splitPermissionNote(`${bypass}\n\nWe cleaned /tmp.`)).toEqual({ mode: "bypass", summary: "We cleaned /tmp." });
    expect(splitPermissionNote("We cleaned /tmp.")).toEqual({ mode: null, summary: "We cleaned /tmp." });
  });

  it("reads the memories a first message and a summary start with (AI-53)", () => {
    const body = '<entry name="web-1 sudo" scope="global">\nweb-1 uses doas.\n</entry>';
    const memory = noteBlock({ kind: "memory", entries: 1, body });
    expect(memory).toBe(`<memory entries="1">\n${body}\n</memory>`);
    expect(parseMessage(`${manual}\n\n${memory}\n\nhello`)).toMatchObject({
      notes: [{ kind: "permission_mode", mode: "manual" }, { kind: "memory", entries: 1, body }],
      typed: "hello",
    });
    expect(splitPermissionNote(`${auto}\n\n${memory}\n\nWe restarted nginx.`)).toEqual({ mode: "auto", summary: "We restarted nginx." });
    expect(splitPermissionNote(`${memory}\n\nWe restarted nginx.`)).toEqual({ mode: null, summary: "We restarted nginx." });
    expect(notedPermission([summary(`${auto}\n\n${memory}\n\nWe restarted nginx.`)])).toBe("auto");
    // A conversation on a host names it, and so do its host memories.
    const hostBody = '<entry name="sudo" scope="host" host="web-1">\nUses doas.\n</entry>';
    const onHost = noteBlock({ kind: "memory", entries: 1, host: "web-1", body: hostBody });
    expect(onHost).toBe(`<memory entries="1" conversation_host="web-1">\n${hostBody}\n</memory>`);
    expect(parseMessage(`${onHost}\n\nhello`)).toMatchObject({
      notes: [{ kind: "memory", entries: 1, host: "web-1", body: hostBody }],
      typed: "hello",
    });
    expect(splitPermissionNote(`${auto}\n\n${onHost}\n\nWe restarted nginx.`)).toEqual({ mode: "auto", summary: "We restarted nginx." });
    // Notes stored before conversation_host named the conversation's host in host.
    const old = `<memory entries="1" host="web-1">\n${body}\n</memory>`;
    expect(parseMessage(`${old}\n\nhello`)).toMatchObject({ notes: [{ kind: "memory", entries: 1, host: "web-1", body }], typed: "hello" });
    expect(splitPermissionNote(`${auto}\n\n${old}\n\nWe restarted nginx.`)).toEqual({ mode: "auto", summary: "We restarted nginx." });
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

describe("what the / and @ menus attach (AI-50, AI-51)", () => {
  const hostFile: Attachment = { kind: "host_file", host: "prod-api", path: "/etc/nginx/nginx.conf", lines: 2, text: "user www;\n</host_file>" };
  const folder: Attachment = { kind: "host_directory", host: "prod & co", path: "/etc/nginx", entries: 2, truncated: true, text: "conf.d/\nnginx.conf" };
  const resource: Attachment = { kind: "mcp_resource", server: "docs", uri: 'repo://x/"a".md', lines: 1, text: "# A" };
  const skill: Attachment = { kind: "skill", name: "nginx-ops", text: "# nginx\n\nRun `nginx -t`." };
  const prompt: Attachment = { kind: "mcp_prompt", server: "github", name: "review_pr", text: "Review PR 42" };

  it("writes each block with its attributes and reads it back exactly", () => {
    expect(blockOf(hostFile)).toBe('<host_file host="prod-api" path="/etc/nginx/nginx.conf" lines="2">\nuser www;\n<\\/host_file>\n</host_file>');
    expect(blockOf(folder)).toBe('<host_directory host="prod &amp; co" path="/etc/nginx" entries="2" truncated="true">\nconf.d/\nnginx.conf\n</host_directory>');
    expect(blockOf(resource)).toBe('<mcp_resource server="docs" uri="repo://x/&quot;a&quot;.md" lines="1">\n# A\n</mcp_resource>');
    expect(blockOf(skill)).toBe('<skill name="nginx-ops">\n# nginx\n\nRun `nginx -t`.\n</skill>');
    expect(blockOf(prompt)).toBe('<mcp_prompt server="github" name="review_pr">\nReview PR 42\n</mcp_prompt>');
    const list = [skill, hostFile, folder, resource, prompt];
    const stored = composeMessage("/nginx-ops check @/etc/nginx/nginx.conf", [att("$ curl"), ...list]);
    expect(parseMessage(stored)).toEqual({ notes: [], attachments: [att("$ curl"), ...list], typed: "/nginx-ops check @/etc/nginx/nginx.conf" });
  });

  it("leaves blocks with other attributes as typed text", () => {
    for (const text of ['<skill>\nx\n</skill>\n\nq', '<host_file path="/x" lines="1">\nx\n</host_file>\n\nq', '<host_directory host="h" path="/" entries="x">\nx\n</host_directory>\n\nq'])
      expect(parseMessage(text)).toEqual({ notes: [], attachments: [], typed: text });
  });

  it("titles a conversation as Rust's title_of does", () => {
    expect([hostFile, folder, resource, skill, prompt].map(blockTitle)).toEqual(["nginx.conf", "/etc/nginx", 'repo://x/"a".md', "/nginx-ops", "review_pr"]);
    expect(pathName("/etc/nginx/")).toBe("nginx");
    expect(pathName("/")).toBe("/");
    expect(folderName("/etc/nginx")).toBe("nginx/");
    expect(folderName("/")).toBe("/");
  });
});
