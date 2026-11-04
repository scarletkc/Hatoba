/**
 * Command suggestions at a shell prompt (TERM-14) and the command history they come from (TERM-15).
 * The xterm buffer is read through the minimal interfaces below so the logic is testable without a
 * terminal.
 */

export interface SuggestCell {
  getChars(): string;
  getWidth(): number;
  isDim(): number;
  isFgPalette(): boolean;
  getFgColor(): number;
}

export interface SuggestLine {
  /** The row continues the previous one (soft wrap). */
  readonly isWrapped: boolean;
  readonly length: number;
  translateToString(trimRight?: boolean, startColumn?: number, endColumn?: number): string;
  getCell(x: number): SuggestCell | undefined;
}

export interface SuggestBuffer {
  readonly type: "normal" | "alternate";
  /** The first row of the screen (rows above it are scrollback). */
  readonly baseY: number;
  /** The first row the view shows; equal to `baseY` while it is scrolled to the bottom. */
  readonly viewportY: number;
  readonly cursorX: number;
  /** The cursor's row on the screen. */
  readonly cursorY: number;
  getLine(y: number): SuggestLine | undefined;
}

/** Commands kept per host or local shell (TERM-15). */
export const HISTORY_LIMIT = 1000;
/** Longer commands are not kept. */
export const COMMAND_MAX_CHARS = 4096;

/** Characters a prompt ends with, with or without a space after them (TERM-14). */
const PROMPT_END = /[$#%>❯]\s*$/u;
/** A line's text up to its first prompt character and the space after it. */
const PROMPT_IN_LINE = /^[^$#%>❯]*[$#%>❯] ?/u;
/**
 * A program asking for a secret, such as `Token> `, whose answer may be echoed: no prompt (TERM-14). A
 * folder of that name at the end of a path, as in PowerShell's `PS C:\token> `, is not one.
 */
const SECRET_PROMPT =
  /(?<![\\/:])(?:(?<![\p{L}\p{N}])(?:password|passphrase|passcode|pass|token|secret|otp|pin|key)|密码|密碼|口令|パスワード)\s*>\s*$/iu;

/** Whether the history keeps `command` (TERM-15): not empty, not starting with a space, not too long. */
export function keepsCommand(command: string): boolean {
  return command.trim() !== "" && !/^\s/.test(command) && command.length <= COMMAND_MAX_CHARS;
}

/** `list` with `command` added at the front, each command once, at most {@link HISTORY_LIMIT}. */
export function withCommand(list: readonly string[], command: string): string[] {
  if (!keepsCommand(command)) return [...list];
  return [command, ...list.filter((c) => c !== command)].slice(0, HISTORY_LIMIT);
}

/** The first word of a prompt, which stays the same in prompts that change with the directory. */
function firstWord(prompt: string): string {
  return prompt.trim().split(/\s+/u)[0] ?? "";
}

/**
 * Whether `prompt`, the text before the input on its line, is a shell's prompt: it ends with a prompt
 * character, or it starts with the same word as the line Enter was last pressed on, as oh-my-zsh's
 * default `➜  dir git:(main) ✗ ` does. A program's question for a secret, such as `Token> `, is none.
 */
export function isPrompt(prompt: string, lastWord: string | null): boolean {
  if (prompt.trim() === "" || SECRET_PROMPT.test(prompt)) return false;
  return PROMPT_END.test(prompt) || (lastWord !== null && firstWord(prompt) === lastWord);
}

/** The rows of the logical line the cursor is on: from the row it starts on to the last row wrapping from it. */
function cursorLine(buf: SuggestBuffer): { first: number; cursor: number; last: number } {
  const cursor = buf.baseY + buf.cursorY;
  let first = cursor;
  while (first > 0 && buf.getLine(first)?.isWrapped) first--;
  let last = cursor;
  while (buf.getLine(last + 1)?.isWrapped) last++;
  return { first, cursor, last };
}

/** The text of the cursor's line before the cursor, its earlier soft-wrapped rows included. */
export function textBeforeCursor(buf: SuggestBuffer): string {
  const { first, cursor } = cursorLine(buf);
  let text = "";
  for (let y = first; y < cursor; y++) text += buf.getLine(y)?.translateToString(false) ?? "";
  return text + (buf.getLine(cursor)?.translateToString(false, 0, buf.cursorX) ?? "");
}

/** Cells drawn as a shell's own suggestion: dim (PSReadLine), or in palette colour 8 (fish, zsh-autosuggestions). */
function looksSuggested(cell: SuggestCell): boolean {
  return cell.isDim() !== 0 || (cell.isFgPalette() && cell.getFgColor() === 8);
}

/**
 * What the cursor's line holds from the cursor on, and whether all of it looks like the shell's own
 * suggestion. A right prompt (zsh's `RPROMPT`), set apart by spaces and ending at the right edge of
 * the cursor's row, counts as nothing.
 */
export function textAfterCursor(buf: SuggestBuffer): { text: string; suggested: boolean } {
  const { cursor, last } = cursorLine(buf);
  if (last === cursor) {
    const rest = buf.getLine(cursor)?.translateToString(false, buf.cursorX) ?? "";
    if (/^\s{2,}\S/u.test(rest) && rest.trimEnd().length >= rest.length - 2) return { text: "", suggested: false };
  }
  let text = "";
  let suggested = true;
  for (let y = cursor; y <= last; y++) {
    const line = buf.getLine(y);
    if (!line) continue;
    const from = y === cursor ? buf.cursorX : 0;
    for (let x = from; x < line.length; x++) {
      const cell = line.getCell(x);
      if (!cell || cell.getWidth() === 0) continue;
      const chars = cell.getChars();
      if (chars !== "" && chars !== " " && !looksSuggested(cell)) suggested = false;
    }
    text += line.translateToString(false, from);
  }
  text = text.trimEnd();
  return { text, suggested: text !== "" && suggested };
}

/** The rest of the newest command in `history` that starts with `typed` (TERM-14), or null. */
export function suggestionFor(typed: string, history: readonly string[]): string | null {
  if (typed === "") return null;
  const hit = history.find((c) => c.length > typed.length && c.startsWith(typed));
  return hit === undefined ? null : hit.slice(typed.length);
}

/**
 * The rest of the shortest command name that starts with `typed`, the first word of a command, of the
 * server's sorted `commands` (TERM-16), or null. A word that already is a command suggests no longer
 * name, or `ls` would suggest `lsblk`.
 */
export function commandNameFor(typed: string, commands: readonly string[]): string | null {
  if (typed === "" || /\s/u.test(typed)) return null;
  let best: string | null = null;
  for (const name of commands) {
    if (name === typed) return null;
    if (name.startsWith(typed) && (best === null || name.length < best.length)) best = name;
  }
  return best === null ? null : best.slice(typed.length);
}

/** The commands in `history` that contain `filter`, ignoring case: those that start with it first, newest first. */
export function listMatches(filter: string, history: readonly string[]): string[] {
  const needle = filter.toLocaleLowerCase();
  const starts: string[] = [];
  const contains: string[] = [];
  for (const c of history) {
    const hay = c.toLocaleLowerCase();
    if (hay.startsWith(needle)) starts.push(c);
    else if (hay.includes(needle)) contains.push(c);
  }
  return [...starts, ...contains];
}

/** The next word of a suggestion's rest, with the spaces before it, which Ctrl+→ (⌥→ on macOS) types. */
export function nextWord(rest: string): string {
  // A backslash stays with what it escapes, as in the rest of a file name with spaces.
  return /^\s*(?:\\.|[^\s\\]|\\$)+/u.exec(rest)?.[0] ?? rest;
}

/** Keystrokes that erase `typed` before the cursor: one Backspace per character. */
export function eraseKeys(typed: string): string {
  return "\x7f".repeat(Array.from(typed).length);
}

/** What the tracker found when Enter went to the shell. */
export interface EnterResult {
  /** The command run at a prompt, for the history; null when there is none. */
  command: string | null;
}

/**
 * Where the input starts on the current prompt (TERM-15). The prompt is the text before the cursor
 * when the first key is pressed after the previous Enter; the input is what follows it on the
 * cursor's line, which survives the shell drawing the line again (Ctrl+L, a Tab completion list).
 */
export class InputTracker {
  private prompt: string | null = null;
  private lastWord: string | null = null;

  /** Forgets the prompt, as after Ctrl+C or a disconnect. */
  reset() {
    this.prompt = null;
  }

  /** Called with each piece of input before it goes to the shell. Returns the command when it is Enter. */
  onInput(data: string, buf: SuggestBuffer): EnterResult {
    if (buf.type === "alternate") {
      this.prompt = null;
      return { command: null };
    }
    if (data === "\r") {
      // Reading the input first may find the prompt again on the line.
      const typed = this.typed(buf);
      const prompt = this.prompt ?? textBeforeCursor(buf);
      const atPrompt = isPrompt(prompt, this.lastWord);
      this.lastWord = firstWord(prompt) || null;
      this.prompt = null;
      return { command: atPrompt && typed !== null && keepsCommand(typed) ? typed : null };
    }
    if (data === "\x03" || data.includes("\r") || data.includes("\n")) {
      // Ctrl+C, or a paste of several lines, which runs what it holds.
      this.prompt = null;
      return { command: null };
    }
    this.prompt ??= textBeforeCursor(buf);
    return { command: null };
  }

  /**
   * The whole input of the current prompt, as Enter would run it: what precedes the cursor and what
   * follows it, unless what follows is the shell's own suggestion. Null when the cursor's line no
   * longer starts with the prompt.
   */
  typed(buf: SuggestBuffer): string | null {
    const before = this.beforeCursor(buf);
    if (before === null) return null;
    const after = textAfterCursor(buf);
    return after.suggested ? before : before + after.text;
  }

  /**
   * Keys that clear the input of a recognized prompt, for the list to put a command in its place: `right`
   * (the → key) to its end, past text the user typed after the cursor, then one Backspace per character.
   * Empty when there is no input, or no prompt it is known to follow.
   */
  clearKeys(buf: SuggestBuffer, right: string): string {
    if (buf.type === "alternate" || this.prompt === null) return "";
    const before = this.beforeCursor(buf);
    if (before === null || !isPrompt(this.prompt, this.lastWord)) return "";
    const after = textAfterCursor(buf);
    // The shell's own suggestion goes with the input's last character; → could take it.
    if (after.suggested || after.text === "") return eraseKeys(before);
    return right.repeat(Array.from(after.text).length) + eraseKeys(before + after.text);
  }

  /** The input before the cursor at a recognized prompt, with nothing but blanks after the cursor; null otherwise. */
  typedAtEnd(buf: SuggestBuffer): string | null {
    if (buf.type === "alternate" || this.prompt === null) return null;
    const before = this.beforeCursor(buf);
    if (before === null || !isPrompt(this.prompt, this.lastWord) || textAfterCursor(buf).text !== "") return null;
    return before;
  }

  /**
   * What follows the prompt before the cursor. When the line does not start with the prompt taken at
   * the first key, as when that key was typed before the shell drew its prompt, the prompt is found
   * again on the line: up to its first prompt character.
   */
  private beforeCursor(buf: SuggestBuffer): string | null {
    if (this.prompt === null) return null;
    const line = textBeforeCursor(buf);
    if (this.prompt.trim() !== "" && line.startsWith(this.prompt)) return line.slice(this.prompt.length);
    const found = PROMPT_IN_LINE.exec(line)?.[0];
    if (found === undefined) return null;
    this.prompt = found;
    return line.slice(found.length);
  }
}
