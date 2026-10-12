import type { McpToolInfo } from "@/ipc/types";
import { HIDDEN } from "./commandRules";
import type { ToolKind } from "./tools";

/**
 * AI-47: the hard rules of Auto mode. A call that manual approval would ask about goes to the judge first,
 * except one these send straight to the approval card, so that no model decides it:
 *
 * - an MCP tool whose server is set to **Always ask** (AI-31);
 * - a `run_command` command or `send_input` text with hidden or bidirectional characters, which could show
 *   the judge and the user something other than what runs;
 * - a `run_command` command or `send_input` text that names one of {@link ALWAYS_ASK_COMMANDS};
 * - `send_input` while the screen's last line asks for a password or passphrase.
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

/** Why a call in Auto mode goes straight to the approval card (AI-47). */
export type HardRule =
  /** The MCP tool's server is set to Always ask (AI-31). */
  | { kind: "mcp_always_ask" }
  /** The command or the text has hidden or bidirectional characters. */
  | { kind: "hidden" }
  /** The command or the text names `command`: one of {@link ALWAYS_ASK_COMMANDS}, or `rm -r`. */
  | { kind: "command"; command: string }
  /** `send_input` while the screen asks for a password or passphrase. */
  | { kind: "password" };

export interface HardRuleInput {
  kind: ToolKind;
  /** The call's arguments; a `call_mcp_tool` call's are those of the tool it names. */
  args: Record<string, unknown>;
  /** An MCP call's tool (`mcp_tool_info`). */
  mcp?: McpToolInfo | null;
  /** For `send_input`, the screen's last lines, the cursor's line last. */
  screen?: string | null;
}

/** The hard rule that sends a call in Auto mode straight to the approval card, or null when the judge decides it (AI-47). */
export function hardRule({ kind, args, mcp, screen }: HardRuleInput): HardRule | null {
  if (kind === "mcp") return mcp?.always_ask ? { kind: "mcp_always_ask" } : null;
  if (kind !== "run_command" && kind !== "send_input") return null;
  const value = kind === "run_command" ? args.command : args.text;
  const text = typeof value === "string" ? value : "";
  if (HIDDEN.test(text)) return { kind: "hidden" };
  if (kind === "send_input" && screen && passwordPrompt(lastLine(screen))) return { kind: "password" };
  const command = namedCommand(text);
  return command ? { kind: "command", command } : null;
}

/**
 * What separates words for {@link namedCommand}: white space and the shell's punctuation, so that a command
 * joined to others (`a;rm`), substituted (`$(rm …)`), in a group or brace expansion (`{rm,-rf,/}`), or set as
 * a value (`alias x=reboot`) is a word of its own.
 */
const SEPARATORS = /[\s;|&()<>{}`$=,]+/;

/**
 * The first of {@link ALWAYS_ASK_COMMANDS} that `line` names, as named (without its directory), or `rm -r` for
 * `rm` with a recursive option after it: a cluster of short options with `r` or `R` (`-r`, `-rf`, `-fR`), or
 * `--recursive`, also abbreviated as getopt accepts it (`--rec`). Quotes and backslashes are removed first, as
 * the shell joins what they quote (`'r'm`, `r\m`), so a command inside `sh -c '…'` is found too. The option may
 * belong to a later command of the line (`rm a; ls -R`), which asks too.
 */
export function namedCommand(line: string): string | null {
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
