import { reading } from "./readOnly";
import { HIDDEN, splitCommands, type ShellDialect, type Simple } from "./split";

/**
 * A command that sets environment variables before it runs, or runs with sudo, never matches a prefix, and
 * neither does a `printf` that does not only print ({@link printfReads}): it may set a variable for the commands
 * after it. In PowerShell and Command Prompt the splitter refuses variables, and sudo counts by its
 * {@link windowsName}.
 */
function plain(words: string[], dialect: ShellDialect): boolean {
  if (dialect !== "posix") return windowsName(words[0]) !== "sudo";
  return !/^[A-Za-z_][A-Za-z0-9_]*=/.test(words[0]) && words[0] !== "sudo" && words[0] !== "printf";
}

/** The words of a prefix the user allowed. */
const prefixWords = (prefix: string) => prefix.trim().split(/\s+/).filter(Boolean);

/** Whether a command starts with a prefix's words; PowerShell and Command Prompt take the command's name in any case. */
function startsWith(words: readonly string[], prefix: readonly string[], dialect: ShellDialect): boolean {
  if (prefix.length > words.length) return false;
  return prefix.every(
    (w, i) => words[i] === w || (i === 0 && dialect !== "posix" && words[0].toLowerCase() === w.toLowerCase()),
  );
}

/**
 * The commands of a `run_command` call: its line's, after a `cd` to its `workdir` when it gives one, so that the
 * directory counts as in `cd workdir && …` (in Command Prompt, a program of that folder runs before one on PATH;
 * in PowerShell, a network path is another computer). Null as for `splitCommands`, and for a directory with
 * hidden characters.
 */
function callCommands(line: string, dialect: ShellDialect, workdir: string): Simple[] | null {
  const commands = splitCommands(line, dialect);
  if (!commands || !workdir) return commands;
  return HIDDEN.test(workdir) ? null : [{ words: ["cd", workdir], globs: [false, false], workdir: true }, ...commands];
}

/**
 * Whether a `run_command` command line, run in `workdir` (none: the home directory), runs without asking: every
 * command in it only reads, or starts with one of `prefixes` (whole words: `git pull` covers `git pull origin`,
 * not `git pull-request`). A line that uses anything `splitCommands` refuses never does.
 */
export function commandAllowed(
  line: string,
  prefixes: readonly string[],
  dialect: ShellDialect = "posix",
  workdir = "",
): boolean {
  const commands = callCommands(line, dialect, workdir);
  if (!commands) return false;
  const allowed = prefixes.map(prefixWords).filter((p) => p.length > 0);
  const reads = reading(commands, dialect);
  return commands.every(
    (c, i) => reads[i] || (plain(c.words, dialect) && allowed.some((p) => startsWith(c.words, p, dialect))),
  );
}

/** Whether the command line, run in `workdir`, only reads (§13.5), and so runs without asking in manual approval mode. */
export function readsOnlyCommand(line: string, dialect: ShellDialect = "posix", workdir = ""): boolean {
  return commandAllowed(line, [], dialect, workdir);
}

/** Tools whose first argument is a subcommand, which an allowance names too: `git pull`, not all of `git`. */
const SUBCOMMANDS = new Set([
  "git",
  "docker",
  "podman",
  "systemctl",
  "service",
  "kubectl",
  "helm",
  "npm",
  "pnpm",
  "yarn",
  "npx",
  "apt",
  "apt-get",
  "dnf",
  "yum",
  "zypper",
  "brew",
  "snap",
  "cargo",
  "go",
  "pip",
  "pip3",
  "gem",
  "make",
  "terraform",
  "ufw",
]);

/** {@link SUBCOMMANDS} in PowerShell and Command Prompt, with Windows' own tools that take one. */
const WINDOWS_SUBCOMMANDS = new Set([
  ...SUBCOMMANDS,
  "winget",
  "choco",
  "scoop",
  "dotnet",
  "net",
  "netsh",
  "reg",
  "sc",
]);

/**
 * A command's name as PowerShell and Command Prompt find it: in lowercase, without its directory and
 * without `.exe`, `.com`, `.bat` or `.cmd`. `C:\Windows\System32\shutdown.exe` is `shutdown`.
 */
export function windowsName(word: string): string {
  const base = word.slice(Math.max(word.lastIndexOf("/"), word.lastIndexOf("\\")) + 1).toLowerCase();
  return base.replace(/\.(exe|com|bat|cmd)$/, "");
}

/** Whether a command's name without its directory is one of `names`, also with a version: `python3.12` is `python`. */
function inNames(names: ReadonlySet<string>, base: string): boolean {
  return names.has(base) || names.has(base.replace(/[\d.]+$/, ""));
}

/**
 * Whether `word` names a command of `names`, also with a directory or a version: `/usr/bin/sudo` is
 * `sudo`, and `python3.12` is `python`. In PowerShell and Command Prompt by its {@link windowsName}.
 */
function named(names: ReadonlySet<string>, word: string, dialect: ShellDialect = "posix"): boolean {
  return inNames(names, dialect === "posix" ? word.slice(word.lastIndexOf("/") + 1) : windowsName(word));
}

/** Commands that run another command, which a prefix of theirs would allow whatever it is. */
const WRAPPERS = new Set([
  "sudo",
  "doas",
  "su",
  "env",
  "xargs",
  "nohup",
  "timeout",
  "nice",
  "ionice",
  "time",
  "watch",
  "exec",
  "eval",
  "command",
  "builtin",
  "sh",
  "bash",
  "zsh",
  "dash",
  "ksh",
  "fish",
  "busybox",
]);

/**
 * {@link WRAPPERS} in PowerShell and Command Prompt, with the shells, `start`, `call`, dot-sourcing, and the
 * cmdlets that run a command, a script block or what a file opens with.
 */
const WINDOWS_WRAPPERS = new Set([
  ...WRAPPERS,
  "cmd",
  "powershell",
  "pwsh",
  "wsl",
  "runas",
  "gsudo",
  "start",
  "call",
  ".",
  "invoke-expression",
  "iex",
  "invoke-command",
  "icm",
  "start-process",
  "saps",
  "start-job",
  "invoke-item",
  "ii",
  "foreach-object",
  "foreach",
  "%",
]);

/**
 * Commands that a prefix saved for good would let run anything, delete or overwrite data, or send it
 * elsewhere on the host: interpreters, deleting and formatting, permissions, processes and power, and
 * network transfers, Windows' among them. Their cards offer Allow for this conversation, not Always allow.
 */
const NOT_SAVED = new Set([
  "python",
  "python3",
  "node",
  "perl",
  "ruby",
  "php",
  "lua",
  "deno",
  "bun",
  "rm",
  "rmdir",
  "shred",
  "dd",
  "truncate",
  "wipefs",
  "fdisk",
  "sfdisk",
  "parted",
  "chmod",
  "chown",
  "chgrp",
  "kill",
  "killall",
  "pkill",
  "reboot",
  "shutdown",
  "poweroff",
  "halt",
  "curl",
  "wget",
  "scp",
  "rsync",
  "nc",
  "ncat",
  "socat",
  "ssh",
  "sftp",
  "ftp",
  "powershell",
  "pwsh",
  "cmd",
  "cscript",
  "wscript",
  "mshta",
  "rundll32",
  "regsvr32",
  "remove-item",
  "ri",
  "del",
  "erase",
  "rd",
  "clear-content",
  "clc",
  "format",
  "diskpart",
  "format-volume",
  "clear-disk",
  "remove-partition",
  "cipher",
  "icacls",
  "cacls",
  "takeown",
  "set-acl",
  "stop-process",
  "spps",
  "taskkill",
  "stop-computer",
  "restart-computer",
  "invoke-webrequest",
  "iwr",
  "invoke-restmethod",
  "irm",
  "start-bitstransfer",
  "bitsadmin",
  "certutil",
]);

/**
 * Whether Always allow may save these prefixes (`allowancePrefixes`), which `NOT_SAVED` decides by the
 * prefix's {@link windowsName} in every dialect: a computer's approvals hold in all of its local shells
 * (§7.5), and Git Bash and WSL run Windows programs too.
 */
export function savablePrefixes(prefixes: readonly string[]): boolean {
  return prefixes.every((p) => {
    const base = windowsName(prefixWords(p)[0] ?? "");
    return !inNames(NOT_SAVED, base) && !base.startsWith("mkfs");
  });
}

/** The most prefixes one allowance on a card names. */
const MAX_PREFIXES = 3;

/**
 * What Allow for this conversation and Always allow on the approval card of a `run_command` call allow
 * (AI-19): for each command of the line that does not only read, its name, and for a tool such as `git`
 * its subcommand too. Null when the card offers neither: the line uses what `splitCommands` refuses,
 * a command runs another one (`sudo`, `xargs`, `sh -c`, `Start-Process`), sets variables first, is a `printf`
 * that does not only print, or gives no subcommand (in PowerShell and Command Prompt also a `/` switch in its
 * place), or the line would need more than three prefixes. A `workdir` counts as a `cd` before the line; when
 * that `cd` does not only read (in Command Prompt beside any command, in PowerShell to a network path), it is
 * offered for that folder alone (`cd C:\src`), so that allowing it never lets a program of every other folder
 * run, and not for a folder whose name has a space, which a prefix cannot name.
 */
export function allowancePrefixes(line: string, dialect: ShellDialect = "posix", workdir = ""): string[] | null {
  const commands = callCommands(line, dialect, workdir);
  if (!commands) return null;
  const windows = dialect !== "posix";
  const reads = reading(commands, dialect);
  const out: string[] = [];
  for (const [i, c] of commands.entries()) {
    if (reads[i]) continue;
    if (c.workdir) {
      if (/\s/.test(c.words[1])) return null;
      out.push(`cd ${c.words[1]}`);
      continue;
    }
    const [name, sub] = c.words;
    if (
      !plain(c.words, dialect) ||
      named(windows ? WINDOWS_WRAPPERS : WRAPPERS, name, dialect) ||
      c.globs[0] ||
      /\s/.test(name)
    )
      return null;
    let prefix = name;
    if (named(windows ? WINDOWS_SUBCOMMANDS : SUBCOMMANDS, name, dialect)) {
      if (!sub || sub.startsWith("-") || (windows && sub.startsWith("/")) || c.globs[1] || /\s/.test(sub)) return null;
      prefix = `${name} ${sub}`;
    }
    if (!out.includes(prefix)) out.push(prefix);
  }
  return out.length > 0 && out.length <= MAX_PREFIXES ? out : null;
}
