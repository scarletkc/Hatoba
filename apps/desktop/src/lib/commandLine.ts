/* Command lines of `stdio` MCP servers (AI-29): how one is shown, and how a pasted one is split. No React, no i18n. */

const CONTROL = /[\u0000-\u001f\u007f]/g;
const PLAIN_ARG = /^[A-Za-z0-9_@%+=:,./\\-]+$/;

const ESCAPES: Record<string, string> = { "\n": "\\n", "\r": "\\r", "\t": "\\t" };
const escapeControl = (text: string): string =>
  text.replace(CONTROL, (c) => ESCAPES[c] ?? `\\x${c.charCodeAt(0).toString(16).padStart(2, "0")}`);

/**
 * One argument as it is shown in a command line: unchanged when it has only plain characters, otherwise in double
 * quotes with `"` escaped. Control characters show as escapes, so nothing hides in the line. For display only: a
 * backslash is left alone, so a Windows path reads as it is.
 */
export function quoteArg(arg: string): string {
  if (PLAIN_ARG.test(arg)) return arg;
  return `"${escapeControl(arg.replace(/"/g, '\\"'))}"`;
}

/** The command and its arguments as one line, which the user confirms before a `stdio` server is saved. */
export function formatCommandLine(command: string, args: readonly string[]): string {
  return [escapeControl(command), ...args.map(quoteArg)].join(" ");
}

/**
 * Splits a pasted command line into words the way a shell does for quotes: single and double quotes group words,
 * and a backslash escapes a double quote inside double quotes, and a quote or space outside quotes. Any other
 * backslash stays, so Windows paths survive. Null when a quote is not closed.
 */
export function splitCommandLine(text: string): string[] | null {
  const words: string[] = [];
  let word = "";
  let started = false;
  let quote: '"' | "'" | null = null;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quote) {
      if (c === quote) quote = null;
      else if (c === "\\" && quote === '"' && text[i + 1] === '"') word += text[++i];
      else word += c;
    } else if (c === '"' || c === "'") {
      quote = c;
      started = true;
    } else if (/\s/.test(c)) {
      if (started || word) words.push(word);
      word = "";
      started = false;
    } else if (c === "\\" && (text[i + 1] === '"' || text[i + 1] === "'" || text[i + 1] === " ")) {
      word += text[++i];
      started = true;
    } else {
      word += c;
      started = true;
    }
  }
  if (quote) return null;
  if (started || word) words.push(word);
  return words;
}

/**
 * Whether the command field holds a whole command line (`npx -y server`) instead of a program: it has a space and
 * the first word is not a path. A program path with spaces, such as `C:\Program Files\node.exe`, does not count.
 */
export function looksLikeCommandLine(command: string): boolean {
  const text = command.trim();
  if (!/\s/.test(text)) return false;
  if (/^["']/.test(text)) return false;
  return !/[\\/]/.test(text.split(/\s+/)[0]);
}
