import { describe, expect, it } from "vitest";
import {
  HISTORY_LIMIT,
  InputTracker,
  commandNameFor,
  eraseKeys,
  isPrompt,
  listMatches,
  nextWord,
  suggestionFor,
  textAfterCursor,
  textBeforeCursor,
  withCommand,
  type SuggestBuffer,
  type SuggestCell,
  type SuggestLine,
} from "./suggest";

const COLS = 40;

/** A row of `COLS` cells; cells from `suggestFrom` on are drawn dim, as a shell's own suggestion. */
function row(text: string, { wrapped = false, suggestFrom = Infinity } = {}): SuggestLine {
  const chars = Array.from(text.padEnd(COLS, " ").slice(0, COLS));
  const cell = (x: number): SuggestCell => ({
    getChars: () => (chars[x] === " " && x >= text.length ? "" : chars[x]!),
    getWidth: () => 1,
    isDim: () => (x >= suggestFrom ? 1 : 0),
    isFgPalette: () => false,
    getFgColor: () => 0,
  });
  return {
    isWrapped: wrapped,
    length: COLS,
    translateToString: (trimRight = false, start = 0, end = COLS) => {
      const s = chars.slice(start, end).join("");
      return trimRight ? s.trimEnd() : s;
    },
    getCell: (x) => (x < COLS ? cell(x) : undefined),
  };
}

function screen(rows: SuggestLine[], x: number, y: number, type: "normal" | "alternate" = "normal"): SuggestBuffer {
  return { type, baseY: 0, viewportY: 0, cursorX: x, cursorY: y, getLine: (i) => rows[i] };
}

/** A prompt line with `typed` after it and the cursor at its end. */
function at(prompt: string, typed = ""): SuggestBuffer {
  return screen([row(prompt + typed)], prompt.length + typed.length, 0);
}

describe("isPrompt (TERM-14)", () => {
  it("knows the usual prompt endings, with or without a space", () => {
    expect(isPrompt("kc@box:~/src$ ", null)).toBe(true);
    expect(isPrompt("root@box:/# ", null)).toBe(true);
    expect(isPrompt("PS C:\\Users\\kc> ", null)).toBe(true);
    expect(isPrompt("~/src ❯ ", null)).toBe(true);
    expect(isPrompt("host% ", null)).toBe(true);
    expect(isPrompt("Router#", null)).toBe(true);
  });

  it("refuses password and question prompts, and empty lines", () => {
    expect(isPrompt("Password: ", null)).toBe(false);
    expect(isPrompt("Token> ", null)).toBe(false);
    expect(isPrompt("Enter API key>", null)).toBe(false);
    expect(isPrompt("请输入密码> ", null)).toBe(false);
    expect(isPrompt("PS C:\\keys> ", null)).toBe(true);
    expect(isPrompt("PS C:\\token> ", null)).toBe(true);
    expect(isPrompt("C:\\Users\\kc\\key>", null)).toBe(true);
    expect(isPrompt("~/password> ", null)).toBe(true);
    expect(isPrompt("Continue? [y/N] ", null)).toBe(false);
    expect(isPrompt("", null)).toBe(false);
    expect(isPrompt("   ", "➜")).toBe(false);
  });

  it("takes a prompt that starts like the line of the last Enter", () => {
    expect(isPrompt("➜  src git:(main) ✗ ", null)).toBe(false);
    expect(isPrompt("➜  src git:(main) ✗ ", "➜")).toBe(true);
  });
});

describe("the history list (TERM-15)", () => {
  it("puts a command at the front once", () => {
    expect(withCommand(["ls", "pwd"], "pwd")).toEqual(["pwd", "ls"]);
  });

  it("leaves out empty commands and those that start with a space", () => {
    expect(withCommand(["ls"], "")).toEqual(["ls"]);
    expect(withCommand(["ls"], "   ")).toEqual(["ls"]);
    expect(withCommand(["ls"], " export TOKEN=x")).toEqual(["ls"]);
  });

  it("keeps the newest commands up to the limit", () => {
    const full = Array.from({ length: HISTORY_LIMIT }, (_, i) => `cmd ${i}`);
    const next = withCommand(full, "new");
    expect(next).toHaveLength(HISTORY_LIMIT);
    expect(next[0]).toBe("new");
    expect(next).not.toContain(`cmd ${HISTORY_LIMIT - 1}`);
  });
});

describe("suggestions", () => {
  const history = ["git status", "git commit -m 'x'", "ls -la"];

  it("suggests the rest of the newest command that starts with what is typed", () => {
    expect(suggestionFor("git", history)).toBe(" status");
    expect(suggestionFor("git c", history)).toBe("ommit -m 'x'");
    expect(suggestionFor("git status", history)).toBeNull();
    expect(suggestionFor("", history)).toBeNull();
    expect(suggestionFor("Git", history)).toBeNull();
  });

  it("completes the first word from the server's command names (TERM-16)", () => {
    const names = ["docker", "docker-compose", "git", "ls", "lsblk", "lsof"];
    expect(commandNameFor("do", names)).toBe("cker");
    expect(commandNameFor("lsb", names)).toBe("lk");
    // A word that is a command already suggests no longer name.
    expect(commandNameFor("ls", names)).toBeNull();
    expect(commandNameFor("docker", names)).toBeNull();
    expect(commandNameFor("git s", names)).toBeNull();
    expect(commandNameFor("", names)).toBeNull();
    expect(commandNameFor("zz", names)).toBeNull();
  });

  it("lists commands that start with the filter first, ignoring case", () => {
    expect(listMatches("LS", ["echo ls", "ls -la", "pwd"])).toEqual(["ls -la", "echo ls"]);
    expect(listMatches("", ["b", "a"])).toEqual(["b", "a"]);
  });

  it("takes the next word with the spaces before it", () => {
    expect(nextWord(" commit -m 'x'")).toBe(" commit");
    expect(nextWord("tatus")).toBe("tatus");
    expect(nextWord("   ")).toBe("   ");
    expect(nextWord("\\ notes.txt -l")).toBe("\\ notes.txt");
    expect(nextWord("end\\")).toBe("end\\");
  });

  it("erases one character per Backspace", () => {
    expect(eraseKeys("ls 中文")).toBe("\x7f".repeat(5));
  });
});

describe("reading the line", () => {
  it("joins the soft-wrapped rows before the cursor", () => {
    const first = "$ " + "a".repeat(COLS - 2);
    const buf = screen([row(first), row("bc", { wrapped: true })], 2, 1);
    expect(textBeforeCursor(buf)).toBe(first + "bc");
  });

  it("tells a shell's own suggestion from typed text after the cursor", () => {
    const own = screen([row("$ git status", { suggestFrom: 5 })], 5, 0);
    expect(textAfterCursor(own)).toEqual({ text: " status", suggested: true });
    const typed = screen([row("$ git status")], 5, 0);
    expect(textAfterCursor(typed)).toEqual({ text: " status", suggested: false });
  });

  it("reads a right prompt as nothing", () => {
    const line = "% ls".padEnd(COLS - 8, " ") + "[10:42]";
    expect(textAfterCursor(screen([row(line)], 4, 0)).text).toBe("");
  });
});

describe("InputTracker (TERM-15)", () => {
  it("records the command typed at a prompt", () => {
    const tr = new InputTracker();
    tr.onInput("l", at("kc@box:~$ "));
    expect(tr.typedAtEnd(at("kc@box:~$ ", "ls -la"))).toBe("ls -la");
    expect(tr.onInput("\r", at("kc@box:~$ ", "ls -la")).command).toBe("ls -la");
  });

  it("finds the input again after the shell draws the line elsewhere", () => {
    const tr = new InputTracker();
    tr.onInput("g", at("$ "));
    // Ctrl+L: the prompt and the input are drawn again on the screen's first row, after a Tab list.
    const moved = screen([row("$ git"), row("a  b  c"), row("$ git st")], 8, 2);
    expect(tr.typedAtEnd(moved)).toBe("git st");
  });

  it("finds the prompt when the first key was typed before the shell drew it", () => {
    const tr = new InputTracker();
    // The previous command's output ended on an empty row, where the cursor waited for the prompt.
    tr.onInput("u", screen([row(" 09:41:12 up 41 days"), row("")], 0, 1));
    const drawn = screen([row(" 09:41:12 up 41 days"), row("deploy@api:/srv$ up")], 19, 1);
    expect(tr.typedAtEnd(drawn)).toBe("up");
    expect(tr.onInput("\r", drawn).command).toBe("up");
  });

  it("keeps what the shell does not echo out of the history", () => {
    const tr = new InputTracker();
    tr.onInput("s", at("Password: "));
    expect(tr.onInput("\r", at("Password: ")).command).toBeNull();
  });

  it("records only what was typed when the shell shows its own suggestion", () => {
    const tr = new InputTracker();
    tr.onInput("g", at("$ "));
    const buf = screen([row("$ git status", { suggestFrom: 5 })], 5, 0);
    expect(tr.typedAtEnd(buf)).toBeNull();
    expect(tr.onInput("\r", buf).command).toBe("git");
  });

  it("records the whole line when Enter is pressed with the cursor inside it", () => {
    const tr = new InputTracker();
    tr.onInput("g", at("$ "));
    expect(tr.onInput("\r", screen([row("$ git status")], 5, 0)).command).toBe("git status");
  });

  it("leaves a right prompt out of the command", () => {
    const tr = new InputTracker();
    tr.onInput("l", at("% "));
    const line = "% ls".padEnd(COLS - 8, " ") + "[10:42]";
    expect(tr.onInput("\r", screen([row(line)], 4, 0)).command).toBe("ls");
  });

  it("learns a prompt with no prompt character at its second Enter", () => {
    const tr = new InputTracker();
    const prompt = "➜  src git:(main) ✗ ";
    tr.onInput("l", at(prompt));
    expect(tr.onInput("\r", at(prompt, "ls")).command).toBeNull();
    tr.onInput("p", at(prompt));
    expect(tr.typedAtEnd(at(prompt, "pw"))).toBe("pw");
    expect(tr.onInput("\r", at(prompt, "pwd")).command).toBe("pwd");
  });

  it("forgets the prompt on Ctrl+C and records nothing in a full-screen program", () => {
    const tr = new InputTracker();
    tr.onInput("l", at("$ "));
    tr.onInput("\x03", at("$ ", "ls"));
    expect(tr.typedAtEnd(at("$ ", "ls"))).toBeNull();
    const vim = screen([row("$ ls")], 4, 0, "alternate");
    tr.onInput("l", vim);
    expect(tr.onInput("\r", vim).command).toBeNull();
  });

  it("clears the whole input for the list, also with the cursor inside it", () => {
    const tr = new InputTracker();
    tr.onInput("g", at("$ "));
    expect(tr.clearKeys(at("$ ", "git"), "→")).toBe("\x7f".repeat(3));
    expect(tr.clearKeys(screen([row("$ git status")], 5, 0), "→")).toBe("→".repeat(7) + "\x7f".repeat(10));
    // The shell's own suggestion after the cursor is no input.
    expect(tr.clearKeys(screen([row("$ git status", { suggestFrom: 5 })], 5, 0), "→")).toBe("\x7f".repeat(3));
    expect(new InputTracker().clearKeys(at("$ ", "git"), "→")).toBe("");
  });

  it("leaves out a command that starts with a space", () => {
    const tr = new InputTracker();
    tr.onInput(" ", at("$ "));
    expect(tr.onInput("\r", at("$ ", " export TOKEN=x")).command).toBeNull();
  });
});
