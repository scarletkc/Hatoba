import type { McpToolInfo } from "@/ipc/types";
import { HIDDEN, type ShellDialect, windowsName } from "./commandRules";
import type { ToolKind } from "./tools";

/**
 * AI-47: the hard rules of Auto mode. A call that manual approval would ask about goes to the judge first,
 * except one these send straight to the approval card, so that no model decides it:
 *
 * - an MCP tool whose server is set to **Always ask** (AI-31), or a tool that cannot be found (its server was
 *   deleted, say), which asks in every mode;
 * - a `run_command` command, `send_input` text or string of an MCP call's arguments with hidden or
 *   bidirectional characters, which could show the judge and the user something other than what runs (an MCP
 *   call's strings may hold the tabs and line breaks of code and text: {@link MCP_HIDDEN});
 * - a `run_command` command, `send_input` text or MCP call's arguments that name one of
 *   {@link ALWAYS_ASK_COMMANDS}: an MCP call's strings at any depth, as its server may run them as a command;
 *   on a PowerShell or Command Prompt tab, a command or text that names one of {@link WINDOWS_ALWAYS_ASK_COMMANDS}
 *   or deletes recursively too;
 * - `send_input` while the screen's last line asks for a password or passphrase;
 * - any call after the user rejected an earlier call of the same response (`after_reject`, which the turn
 *   applies).
 *
 * A `send_input` call that submits a line (its text has a line break, or its key is Enter) is also checked
 * with the cursor's line before it, so that a command split across calls is caught: `rm ` typed by one call
 * and `-rf /data` plus Enter by the next, or `shut` completed by Tab and then entered. The cursor's line
 * starts with the prompt, which {@link namedCommand} reads past.
 *
 * `sudo` is not one of them: the judge sees what the command does.
 */

/**
 * Commands Auto always asks about (AI-47): they destroy data or a disk, or stop the machine, so a judge's
 * mistake would cost too much. A line asks when it names one anywhere: inside quotes, `sh -c '…'`, `eval`,
 * `xargs` or `ssh host …` too, named with a directory (`/sbin/reboot`) or in any case (a case-insensitive file
 * system runs `RM`), and also as a mere argument (`man shutdown`), since a false alarm only costs a click.
 * `mkfs` counts with any suffix (`mkfs.ext4`). `rm` counts only when it is recursive: see {@link namedCommand}.
 */
export const ALWAYS_ASK_COMMANDS: readonly string[] = ["shred", "dd", "mkfs", "wipefs", "fdisk", "sfdisk", "parted", "shutdown", "reboot", "poweroff", "halt"];

/**
 * What a PowerShell or Command Prompt tab also always asks about, in lowercase: formatting and wiping disks and
 * partitions, and stopping or restarting the computer. `format` counts only with a drive after it (`format D:`),
 * as it is always run, since `npm run format` and `dotnet format` format code. Deleting asks when it is
 * recursive: see {@link namedCommand}.
 */
export const WINDOWS_ALWAYS_ASK_COMMANDS: readonly string[] = ["diskpart", "format-volume", "clear-disk", "remove-partition", "stop-computer", "restart-computer"];

/** PowerShell's and Command Prompt's commands that delete files and folders: `Remove-Item` and its aliases, `del`, `erase`, `rd` and `rmdir`. */
const WINDOWS_DELETES: readonly string[] = ["remove-item", "ri", "rm", "rmdir", "rd", "del", "erase"];

/** Why a call in Auto mode goes straight to the approval card (AI-47). */
export type HardRule =
  /** The MCP tool's server is set to Always ask (AI-31). */
  | { kind: "mcp_always_ask" }
  /** The MCP call's tool cannot be found, so it asks in every mode (AI-31). */
  | { kind: "mcp_unknown" }
  /** The command, the text or the MCP call's arguments have hidden or bidirectional characters. */
  | { kind: "hidden" }
  /** The command, the line the text submits or the MCP call's arguments name `command`: one of {@link ALWAYS_ASK_COMMANDS}, or `rm -r`. */
  | { kind: "command"; command: string }
  /** `send_input` while the screen asks for a password or passphrase. */
  | { kind: "password" }
  /** The user rejected an earlier call of the same response, so Auto asks about the rest of its calls. */
  | { kind: "after_reject" };

export interface HardRuleInput {
  kind: ToolKind;
  /** The call's arguments; a `call_mcp_tool` call's are those of the tool it names. */
  args: Record<string, unknown>;
  /** An MCP call's tool (`mcp_tool_info`), null when it cannot be found. */
  mcp?: McpToolInfo | null;
  /** For `send_input`, the screen's last lines, the cursor's line last; without them, only the text is checked. */
  screen?: string | null;
  /** The shell of the call's tab (`dialectOf`), which `run_command` and `send_input` are read in; POSIX when absent. */
  dialect?: ShellDialect;
}

/**
 * The hard rule that sends a call in Auto mode straight to the approval card, or null when the judge decides it
 * (AI-47). `after_reject` is not decided here: it depends on the response's earlier calls.
 */
export function hardRule({ kind, args, mcp, screen, dialect = "posix" }: HardRuleInput): HardRule | null {
  if (kind === "mcp") {
    if (!mcp) return { kind: "mcp_unknown" };
    if (mcp.always_ask) return { kind: "mcp_always_ask" };
    const { keys, values } = mcpStrings(args);
    if (keys.some((s) => MCP_HIDDEN.test(s)) || values.some((s) => MCP_HIDDEN.test(s))) return { kind: "hidden" };
    // The values in order as one text, so that a command given as a list of words (`["rm", "-rf", "/srv"]`) asks too.
    return commandRule([values.join("\n")], "posix");
  }
  if (kind !== "run_command" && kind !== "send_input") return null;
  const value = kind === "run_command" ? args.command : args.text;
  const text = typeof value === "string" ? value : "";
  if (HIDDEN.test(text)) return { kind: "hidden" };
  if (kind !== "send_input" || !screen) return commandRule([text], dialect);
  const cursor = lastLine(screen);
  if (passwordPrompt(cursor)) return { kind: "password" };
  if (!submits(args, text)) return commandRule([text], dialect);
  if (HIDDEN.test(cursor)) return { kind: "hidden" };
  // The line the shell receives: the text continues what the cursor's line holds, whose trailing spaces the
  // screen does not keep, so the two are joined both without and with a space.
  return commandRule([text, cursor + text, `${cursor} ${text}`], dialect);
}

function commandRule(lines: readonly string[], dialect: ShellDialect): HardRule | null {
  for (const line of lines) {
    const command = namedCommand(line, dialect);
    if (command) return { kind: "command", command };
  }
  return null;
}

/** Whether a `send_input` call submits the line it types: its text has a line break, or its key is Enter. */
function submits(args: Record<string, unknown>, text: string): boolean {
  return text.includes("\n") || (typeof args.key === "string" && args.key.toLowerCase() === "enter");
}

/**
 * What is hidden in an MCP call's strings: {@link HIDDEN} without carriage return. Tab, line feed, carriage
 * return and the zero-width joiner and non-joiner are allowed, as the code and text an MCP tool takes often
 * hold them (the joiners in emoji and in Persian or Indic text); every other control character, and the
 * bidirectional and other zero-width format characters, are not.
 */
const MCP_HIDDEN = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f\u061c\u200b\u200e\u200f\u202a-\u202e\u2060-\u2069\ufeff]/;

/** The strings of an MCP call's arguments, through nested objects and arrays: the objects' keys, and the values. */
function mcpStrings(args: unknown): { keys: string[]; values: string[] } {
  const keys: string[] = [];
  const values: string[] = [];
  const visit = (value: unknown) => {
    if (typeof value === "string") values.push(value);
    else if (Array.isArray(value)) value.forEach(visit);
    else if (value && typeof value === "object") {
      for (const [key, inner] of Object.entries(value)) {
        keys.push(key);
        visit(inner);
      }
    }
  };
  visit(args);
  return { keys, values };
}

/**
 * What separates words for {@link namedCommand}: white space and the shell's punctuation, so that a command
 * joined to others (`a;rm`), substituted (`$(rm …)`), in a group or brace expansion (`{rm,-rf,/}`), or set as
 * a value (`alias x=reboot`) is a word of its own; and `#` and `%`, which with `$` and `>` end prompts, so that
 * a command typed right after a prompt without a space (`root@db1:~#rm`) is one too.
 */
const SEPARATORS = /[\s;|&()<>{}`$=,#%]+/;

/**
 * The first of {@link ALWAYS_ASK_COMMANDS} that `line` names, as named (without its directory), or `rm -r` for
 * `rm` with a recursive option after it: a cluster of short options with `r` or `R` (`-r`, `-rf`, `-fR`), or
 * `--recursive`, also abbreviated as getopt accepts it (`--rec`). Quotes and backslashes are removed first, as
 * the shell joins what they quote (`'r'm`, `r\m`), so a command inside `sh -c '…'` is found too. The option may
 * belong to a later command of the line (`rm a; ls -R`), which asks too. In PowerShell and Command Prompt see
 * {@link windowsCommand}.
 */
export function namedCommand(line: string, dialect: ShellDialect = "posix"): string | null {
  if (dialect !== "posix") return windowsCommand(line);
  const words = line.replace(/['"\\]/g, "").split(SEPARATORS);
  let rm = false;
  for (const word of words) {
    if (!word) continue;
    if (rm && recursiveOption(word)) return "rm -r";
    const name = word.slice(word.lastIndexOf("/") + 1);
    const lower = name.toLowerCase();
    if (lower === "rm") rm = true;
    else if (ALWAYS_ASK_COMMANDS.includes(lower) || lower.startsWith("mkfs.")) return name;
  }
  return null;
}

function recursiveOption(word: string): boolean {
  if (word.startsWith("--")) return word.length > 2 && "--recursive".startsWith(word);
  return /^-[A-Za-z]*[rR]/.test(word);
}

/**
 * {@link namedCommand} in PowerShell and Command Prompt: the first of {@link ALWAYS_ASK_COMMANDS} and
 * {@link WINDOWS_ALWAYS_ASK_COMMANDS} (and `format` with a drive) that `line` names, as named (without its directory), or one of
 * {@link WINDOWS_DELETES} with a recursive option anywhere in the line, before it too (`gci -Recurse |
 * Remove-Item`): `/s`, `-Recurse` also abbreviated (`-r`), or `rm`'s own (`-rf`, `--recursive`). Escapes
 * (`` ` ``, `^`) and quotes, the typographic ones too, are removed first, and typographic dashes read as
 * `-`. A word's first and last parts between `\` and before `/` name commands too, so that one named with a
 * directory (`C:\Windows\System32\shutdown.exe`) or with switches right after it (`rd/s/q`) is found, by its
 * {@link windowsName}.
 */
function windowsCommand(line: string): string | null {
  const words = line
    .replace(/['"`^\u2018-\u201f]/g, "")
    .replace(/[\u2013-\u2015]/g, "-")
    .split(SEPARATORS)
    .filter(Boolean);
  let deletes: string | null = null;
  let recursive: string | null = null;
  for (const [i, word] of words.entries()) {
    const parts = word.split(/\\|(?=\/)/).filter(Boolean);
    for (const part of parts) if (!recursive && windowsRecursive(part)) recursive = part;
    for (const part of new Set([parts[0], parts.at(-1)])) {
      if (!part) continue;
      const shown = part.replace(/^\/+/, "");
      const name = windowsName(shown);
      const formats = name === "format" && /^[a-z]:/i.test(words[i + 1] ?? "");
      if (WINDOWS_DELETES.includes(name)) deletes ??= shown;
      else if (formats || ALWAYS_ASK_COMMANDS.includes(name) || WINDOWS_ALWAYS_ASK_COMMANDS.includes(name) || name.startsWith("mkfs.")) return shown;
    }
  }
  return deletes && recursive ? `${deletes} ${recursive}` : null;
}

/** A recursive option of {@link WINDOWS_DELETES}: `/s`, `-Recurse` or an abbreviation of it, `--recursive` or a cluster of `rm`'s options with `r`. */
function windowsRecursive(part: string): boolean {
  const lower = part.toLowerCase();
  const name = lower.split(":")[0];
  if (lower === "/s" || (name.length >= 2 && "-recurse".startsWith(name))) return true;
  if (lower.startsWith("--")) return lower.length > 2 && "--recursive".startsWith(lower);
  return /^-[dfiv]*r[dfiv]*$/.test(lower);
}

/**
 * A line that asks for a password or passphrase: it names one and ends with a colon, as `Password:`, `[sudo]
 * password for kc:`, `Enter passphrase for key '/home/kc/.ssh/id_ed25519':`, `密码：` and `パスワード:` do.
 */
const PASSWORD_PROMPT = /(password|passphrase|passcode|passwd|密码|口令|パスワード|パスフレーズ).*[:：]\s*$/i;

/** Whether a screen line asks for a password or passphrase (AI-47). */
export function passwordPrompt(line: string): boolean {
  return PASSWORD_PROMPT.test(line);
}

function lastLine(text: string): string {
  return text.slice(text.lastIndexOf("\n") + 1);
}
