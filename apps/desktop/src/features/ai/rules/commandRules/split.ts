import type { LocalShellKind } from "@/ipc/types";

/**
 * The syntax a command line is read in: a POSIX shell's, or on a local Windows tab PowerShell's or Command
 * Prompt's (§7.5).
 */
export type ShellDialect = "posix" | "powershell" | "cmd";

/**
 * The dialect of a tab's shell, by its local shell's kind: PowerShell and Command Prompt have their own,
 * and SSH tabs (no kind) and every other local shell, Git Bash and WSL too, read POSIX.
 */
export function dialectOf(kind: LocalShellKind | null | undefined): ShellDialect {
  return kind === "powershell" || kind === "cmd" ? kind : "posix";
}

/** One simple command of a command line: its words after quote removal. */
export interface Simple {
  words: string[];
  /** Whether a word has an unquoted `*`, `?` or `[`, which the shell may expand into other words. */
  globs: boolean[];
  /** The `cd` a call's `workdir` stands for (`callCommands`), whose allowance names its folder. */
  workdir?: true;
}

/** Controls but tab and line break, and the bidirectional and zero-width format characters. */
export const HIDDEN = /[\u0000-\u0008\u000b-\u001f\u007f-\u009f\u061c\u200b-\u200f\u202a-\u202e\u2060-\u2069\ufeff]/;

/** The words and commands a splitter has read so far. */
class Reader {
  readonly out: Simple[] = [];
  word = "";
  inWord = false;
  glob = false;
  /** `|`, `&&` or `||` was last, so another command must follow. */
  open = false;
  private words: string[] = [];
  private globs: boolean[] = [];

  /** Adds characters to the word being read. */
  add(text: string) {
    this.word += text;
    this.inWord = true;
  }

  endWord() {
    if (this.inWord) {
      this.words.push(this.word);
      this.globs.push(this.glob);
    }
    this.word = "";
    this.inWord = false;
    this.glob = false;
  }

  /** Ends the command being read: false when it has no word. `open`: another command must follow. */
  endCommand(open: boolean): boolean {
    this.endWord();
    if (this.words.length === 0) return false;
    this.out.push({ words: this.words, globs: this.globs });
    this.words = [];
    this.globs = [];
    this.open = open;
    return true;
  }

  /** A line break ends a command, or continues after `|`, `&&` and `||`. */
  lineBreak() {
    this.endWord();
    if (this.words.length > 0) this.endCommand(false);
  }

  /** The commands read, or null when there are none or the last `|`, `&&` or `||` has no command after it. */
  end(): Simple[] | null {
    this.endWord();
    if (this.words.length > 0) this.endCommand(false);
    else if (this.open) return null;
    return this.out.length > 0 ? this.out : null;
  }
}

/**
 * Splits a command line into simple commands, removing quotes as the shell does; null when it uses
 * anything else the dialect's splitter refuses. POSIX: at `|`, `&&`, `||`, `;` and line breaks, also
 * removing backslashes, and refusing redirection and heredocs (`<`, `>`), command and process substitution
 * and every other `$` expansion, backticks, a background `&`, subshells and groups, brace expansion,
 * comments, hidden characters, a quote left open, or an empty command. PowerShell and Command Prompt:
 * see {@link splitPowerShell} and {@link splitCmd}.
 */
export function splitCommands(line: string, dialect: ShellDialect = "posix"): Simple[] | null {
  if (HIDDEN.test(line)) return null;
  if (dialect === "powershell") return splitPowerShell(line);
  if (dialect === "cmd") return splitCmd(line);
  const r = new Reader();
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (c === "'") {
      const end = line.indexOf("'", i + 1);
      if (end < 0) return null;
      r.add(line.slice(i + 1, end));
      i = end;
    } else if (c === '"') {
      let j = i + 1;
      let text = "";
      for (; j < line.length && line[j] !== '"'; j++) {
        const d = line[j];
        if (d === "$" || d === "`") return null;
        if (d === "\\" && j + 1 < line.length && '"\\$`\n'.includes(line[j + 1])) {
          if (line[j + 1] !== "\n") text += line[j + 1];
          j++;
        } else text += d;
      }
      if (j >= line.length) return null;
      r.add(text);
      i = j;
    } else if (c === "\\") {
      const next = line[i + 1];
      if (next === undefined) return null;
      if (next !== "\n") r.add(next);
      i++;
    } else if (c === " " || c === "\t") {
      r.endWord();
    } else if (c === "\n") {
      r.lineBreak();
    } else if (c === ";") {
      if (!r.endCommand(false)) return null;
    } else if (c === "|" || c === "&") {
      const next = line[i + 1];
      if (c === "&" && next !== "&") return null;
      if (c === "|" && next === "&") return null;
      if (next === c) i++;
      if (!r.endCommand(true)) return null;
    } else if ("<>`$(){}#".includes(c)) {
      return null;
    } else {
      if (c === "*" || c === "?" || c === "[") r.glob = true;
      r.add(c);
    }
  }
  return r.end();
}

/** The typographic quotes and dashes PowerShell reads as `'`, `"` and `-`. */
const POWERSHELL_TYPOGRAPHIC = /[\u2013-\u2015\u2018-\u201f]/;

/**
 * PowerShell: splits at `|`, `&&`, `||`, `;` and line breaks, and between words at commas too (a list of
 * arguments). Quotes are removed, a doubled quote inside standing for one; PowerShell expands no globs, so no
 * word is marked. Null for everything else: variables and every other `$` expression, a double-quoted
 * string's backtick escapes, the backtick anywhere, redirection, the call operator and background `&`,
 * parentheses, script blocks, splatting and arrays (`@`), type literals (`[`), comments, stop-parsing
 * (`--%`), typographic quotes and dashes, a quote left open, or an empty command.
 */
function splitPowerShell(line: string): Simple[] | null {
  if (POWERSHELL_TYPOGRAPHIC.test(line)) return null;
  const r = new Reader();
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (c === "'" || c === '"') {
      let j = i + 1;
      let text = "";
      for (; j < line.length; j++) {
        const d = line[j];
        if (c === '"' && (d === "$" || d === "`")) return null;
        if (d !== c) text += d;
        else if (line[j + 1] === c) text += line[++j];
        else break;
      }
      if (j >= line.length) return null;
      r.add(text);
      i = j;
    } else if (c === " " || c === "\t" || c === ",") {
      r.endWord();
    } else if (c === "\n") {
      r.lineBreak();
    } else if (c === ";") {
      if (!r.endCommand(false)) return null;
    } else if (c === "|" || c === "&") {
      const next = line[i + 1];
      if (c === "&" && next !== "&") return null;
      if (next === c) i++;
      if (!r.endCommand(true)) return null;
    } else if ("<>`$(){}#".includes(c) || (!r.inWord && (c === "@" || c === "["))) {
      return null;
    } else {
      r.add(c);
    }
  }
  const out = r.end();
  return out?.some((s) => s.words.includes("--%")) ? null : out;
}

/**
 * Command Prompt, which runs one line: splits at `|`, `||`, `&` and `&&`, and between words at `,`, `;` and
 * `=` too, which separate its own commands' arguments. Quotes are removed; Command Prompt expands no globs.
 * Null for everything else: a line break, `%` and `!` variables (also quoted), the `^` escape, redirection,
 * parentheses, `@` before a command, a quote left open, or an empty command.
 */
function splitCmd(line: string): Simple[] | null {
  if (line.includes("\n")) return null;
  const r = new Reader();
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (c === '"') {
      const end = line.indexOf('"', i + 1);
      if (end < 0) return null;
      const text = line.slice(i + 1, end);
      if (/[%!]/.test(text)) return null;
      r.add(text);
      i = end;
    } else if (" \t,;=".includes(c)) {
      r.endWord();
    } else if (c === "|" || c === "&") {
      if (line[i + 1] === c) i++;
      if (!r.endCommand(true)) return null;
    } else if ("<>^%!()".includes(c) || (!r.inWord && c === "@")) {
      return null;
    } else {
      r.add(c);
    }
  }
  return r.end();
}
