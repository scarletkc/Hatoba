import { describe, expect, it } from "vitest";
import { keySequence, parseSendKey, readTerminalInput, SEND_KEYS, showTerminalInput } from "./sendKeys";
import { HIDDEN } from "./commandRules";

describe("terminal keyboard encoding (AI-13)", () => {
  it.each([
    ["ctrl+x", "\x18"],
    ["ctrl+o", "\x0f"],
    ["ctrl+w", "\x17"],
    ["ctrl+z", "\x1a"],
    ["ctrl_c", "\x03"],
    ["CTRL_D", "\x04"],
    ["ctrl+space", "\0"],
    ["ctrl+[", "\x1b"],
    ["alt+f", "\x1bf"],
    ["alt+F", "\x1bF"],
    ["alt+ctrl+x", "\x1b\x18"],
    ["shift+tab", "\x1b[Z"],
    ["backspace", "\x7f"],
    ["ctrl+backspace", "\b"],
    ["delete", "\x1b[3~"],
    ["insert", "\x1b[2~"],
    ["page_up", "\x1b[5~"],
    ["page_down", "\x1b[6~"],
    ["home", "\x1b[H"],
    ["end", "\x1b[F"],
    ["f1", "\x1bOP"],
    ["f2", "\x1bOQ"],
    ["f3", "\x1bOR"],
    ["f4", "\x1bOS"],
    ["f5", "\x1b[15~"],
    ["f6", "\x1b[17~"],
    ["f7", "\x1b[18~"],
    ["f8", "\x1b[19~"],
    ["f9", "\x1b[20~"],
    ["f10", "\x1b[21~"],
    ["f11", "\x1b[23~"],
    ["f12", "\x1b[24~"],
    ["n", "n"],
    ["N", "N"],
    ["+", "+"],
    ["alt++", "\x1b+"],
    ["alt+1", "\x1b1"],
    ["space", " "],
    ["alt+enter", "\x1b\r"],
    ["ctrl+left", "\x1b[1;5D"],
    ["shift+f10", "\x1b[21;2~"],
  ])("encodes %s like xterm's keyboard", (key, bytes) => expect(keySequence(key, false)).toBe(bytes));

  it("follows DECCKM for arrows, Home and End, without changing function keys", () => {
    for (const [key, code] of Object.entries({ up: "A", down: "B", left: "D", right: "C", home: "H", end: "F" })) {
      expect(keySequence(key, false)).toBe(`\x1b[${code}`);
      expect(keySequence(key, true)).toBe(`\x1bO${code}`);
    }
    expect(keySequence("ctrl+up", true)).toBe("\x1b[1;5A");
    expect(keySequence("f10", true)).toBe("\x1b[21~");
  });

  it("validates the complete sequence before anything is sent", () => {
    for (const key of SEND_KEYS) expect(parseSendKey(key), key).not.toBeNull();
    for (const key of [
      null,
      7,
      "",
      "f0",
      "f13",
      "ctrl+banana",
      "ctrl+ctrl+x",
      "meta+x",
      "constructor",
      "__proto__",
      "\x18",
      "\n",
      "\u202e",
      "\u200b",
      "a\nb",
    ])
      expect(parseSendKey(key)).toBeNull();
  });

  it("rejects every hidden character in the command rules, including around a key name", () => {
    for (let code = 0; code <= 0xffff; code++) {
      const char = String.fromCharCode(code);
      if (!HIDDEN.test(char)) continue;
      for (const key of [char, `alt+${char}`, `${char}enter`, `enter${char}`])
        expect(parseSendKey(key), JSON.stringify(key)).toBeNull();
    }
  });

  it("shows controls reversibly without treating a literal backslash escape as input", () => {
    const text = "\x18\x1b\t\r\0\x7f\n\\x18\\e\\n^X";
    expect(showTerminalInput(text)).toBe("⟦^X⟧⟦Esc⟧⟦Tab⟧⟦CR⟧⟦^@⟧⟦DEL⟧\n\\x18\\e\\n^X");
    expect(readTerminalInput(showTerminalInput(text), text)).toBe(text);
    const controls = Array.from({ length: 160 }, (_, i) => String.fromCharCode(i)).join("");
    expect(readTerminalInput(showTerminalInput(controls), controls)).toBe(controls);
    expect(showTerminalInput("printf '\\n'\x18")).toBe("printf '\\n'⟦^X⟧");
  });

  it("keeps typed paths, regular expressions and literal control markers unchanged", () => {
    for (const text of ["cd C:\\temp", "grep '^Error' log", "\\t\\r\\e\\x18^@^_", "⟦Esc⟧⟦⟦Tab⟧"]) {
      expect(showTerminalInput(text)).toBe(text);
      if (!text.startsWith("⟦")) expect(readTerminalInput(text, "")).toBe(text);
      expect(readTerminalInput(showTerminalInput(text), text)).toBe(text);
    }
    const text = "literal ⟦Esc⟧ with control \x1b";
    expect(showTerminalInput(text)).toBe("literal ⟦Esc⟧ with control ⟦⟦Esc⟧");
    expect(readTerminalInput(showTerminalInput(text), text)).toBe(text);
  });
});
