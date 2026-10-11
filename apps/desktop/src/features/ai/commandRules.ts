/**
 * §13.5: which `run_command` command lines run without asking in manual approval mode. A command line
 * runs without asking when every command in it only reads (`ls`, `cat`, `grep`, `git log`, …) or starts
 * with a prefix the user allowed (AI-19). Whatever this cannot tell for sure asks.
 */

/** One simple command of a command line: its words after quote removal. */
interface Simple {
  words: string[];
  /** Whether a word has an unquoted `*`, `?` or `[`, which the shell may expand into other words. */
  globs: boolean[];
}

/** Controls but tab and line break, and the bidirectional and zero-width format characters. */
const HIDDEN = /[\u0000-\u0008\u000b-\u001f\u007f-\u009f\u061c\u200b-\u200f\u202a-\u202e\u2060-\u2069\ufeff]/;

/**
 * Splits a command line into simple commands at `|`, `&&`, `||`, `;` and line breaks, removing quotes
 * and backslashes as the shell does. Null when it uses anything else: redirection and heredocs (`<`,
 * `>`), command and process substitution and every other `$` expansion, backticks, a background `&`,
 * subshells and groups, brace expansion, comments, hidden characters, a quote left open, or an empty
 * command.
 */
export function splitCommands(line: string): Simple[] | null {
  if (HIDDEN.test(line)) return null;
  const out: Simple[] = [];
  let words: string[] = [];
  let globs: boolean[] = [];
  let word = "";
  let inWord = false;
  let glob = false;
  // `|`, `&&` or `||` was last, so another command must follow.
  let open = false;
  const endWord = () => {
    if (inWord) {
      words.push(word);
      globs.push(glob);
    }
    word = "";
    inWord = false;
    glob = false;
  };
  const endCommand = (): boolean => {
    endWord();
    if (words.length === 0) return false;
    out.push({ words, globs });
    words = [];
    globs = [];
    return true;
  };
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (c === "'") {
      const end = line.indexOf("'", i + 1);
      if (end < 0) return null;
      word += line.slice(i + 1, end);
      inWord = true;
      i = end;
    } else if (c === '"') {
      let j = i + 1;
      for (; j < line.length && line[j] !== '"'; j++) {
        const d = line[j];
        if (d === "$" || d === "`") return null;
        if (d === "\\" && j + 1 < line.length && '"\\$`\n'.includes(line[j + 1])) {
          if (line[j + 1] !== "\n") word += line[j + 1];
          j++;
        } else word += d;
      }
      if (j >= line.length) return null;
      inWord = true;
      i = j;
    } else if (c === "\\") {
      const next = line[i + 1];
      if (next === undefined) return null;
      if (next !== "\n") {
        word += next;
        inWord = true;
      }
      i++;
    } else if (c === " " || c === "\t") {
      endWord();
    } else if (c === "\n") {
      // A line break ends a command, or continues after `|`, `&&` and `||`.
      endWord();
      if (words.length > 0) {
        endCommand();
        open = false;
      }
    } else if (c === ";") {
      if (!endCommand()) return null;
      open = false;
    } else if (c === "|" || c === "&") {
      const next = line[i + 1];
      if (c === "&" && next !== "&") return null;
      if (c === "|" && next === "&") return null;
      if (next === c) i++;
      if (!endCommand()) return null;
      open = true;
    } else if ("<>`$(){}#".includes(c)) {
      return null;
    } else {
      if (c === "*" || c === "?" || c === "[") glob = true;
      word += c;
      inWord = true;
    }
  }
  endWord();
  if (words.length > 0) endCommand();
  else if (open) return null;
  return out.length > 0 ? out : null;
}

/**
 * Whether an option is one of `longs` (a long option, also abbreviated, as getopt and git accept it,
 * or with a value after `=`) or a cluster of short options with one of `shorts`.
 */
function refused(arg: string, longs: readonly string[], shorts = ""): boolean {
  if (arg.startsWith("--")) {
    const name = arg.split("=")[0];
    return longs.some((l) => name.startsWith(l) || (name.length >= 3 && l.startsWith(name)));
  }
  return arg.length > 1 && arg.startsWith("-") && [...arg.slice(1)].some((c) => shorts.includes(c));
}

const without =
  (longs: readonly string[], shorts = "") =>
  (args: string[]) =>
    !args.some((a) => refused(a, longs, shorts));

const anyArgs = () => true;

/** The first word that is no option, and what follows it. */
function firstWord(args: string[]): [string | undefined, string[]] {
  const at = args.findIndex((a) => !a.startsWith("-"));
  return at < 0 ? [undefined, []] : [args[at], args.slice(at + 1)];
}

/** `git` with a subcommand that only reads. */
function gitReads(args: string[]): boolean {
  let i = 0;
  // Options before the subcommand: only these. `-c` can name a program to run.
  for (; i < args.length && args[i].startsWith("-"); i++) {
    const a = args[i];
    if (a === "-C") i++;
    else if (!["--no-pager", "-P", "--no-optional-locks", "--literal-pathspecs"].includes(a) && !/^--(git-dir|work-tree)=/.test(a)) return false;
  }
  const sub = args[i];
  const rest = args.slice(i + 1);
  // Write their output to a file, or run a program the repository's configuration names.
  if (rest.some((a) => refused(a, ["--output", "--ext-diff", "--open-files-in-pager"]))) return false;
  // `branch` and `tag` only list: with no name given, or with --list. Git before 2.20 reads `branch -l`
  // as --create-reflog, so with a name only `tag -l` lists.
  const lists = (longs: string[], shorts: string, shortList: boolean) =>
    without(longs, shorts)(rest) &&
    (rest.every((a) => a.startsWith("-")) || rest.includes("--list") || (shortList && rest.includes("-l")));
  switch (sub) {
    case "status":
    case "log":
    case "diff":
    case "show":
    case "rev-parse":
    case "ls-files":
    case "ls-tree":
    case "blame":
    case "describe":
    case "shortlog":
    case "cat-file":
      return true;
    case "grep":
      return without([], "O")(rest);
    case "branch":
      return lists(["--delete", "--move", "--copy", "--force", "--set-upstream-to", "--unset-upstream", "--edit-description", "--track", "--create-reflog"], "dDmMcCfut", false);
    case "tag":
      return lists(["--delete", "--annotate", "--sign", "--local-user", "--force", "--message", "--file", "--edit", "--create-reflog"], "dasufmFe", true);
    case "remote":
      return rest.every((a) => a === "-v" || a === "--verbose") || rest[0] === "get-url" || rest[0] === "show";
    case "stash":
      return rest[0] === "list" || rest[0] === "show";
    case "reflog": {
      const [verb] = firstWord(rest);
      return verb === undefined || verb === "show";
    }
    default:
      return false;
  }
}

/**
 * `docker` or `podman` listing containers and images. Not `inspect`, which prints a container's
 * environment, where passwords often are.
 */
function dockerReads(args: string[]): boolean {
  const [sub, ...rest] = args;
  const follows = (xs: string[]) => xs.some((a) => refused(a, ["--follow"], "f"));
  switch (sub) {
    case "ps":
    case "images":
    case "version":
    case "info":
      return true;
    case "logs":
      return !follows(rest);
    case "container":
      return ["ls", "list", "ps"].includes(rest[0]) || (rest[0] === "logs" && !follows(rest));
    case "image":
      return ["ls", "list"].includes(rest[0]);
    case "compose":
      return ["ps", "ls", "images", "version"].includes(rest[0]) || (rest[0] === "logs" && !follows(rest));
    default:
      return false;
  }
}

const KUBECTL_READS = ["get", "describe", "logs", "top", "version", "explain", "api-resources", "api-versions", "cluster-info"];

function kubectlReads(args: string[]): boolean {
  const sub = args[0];
  if (!KUBECTL_READS.includes(sub)) return false;
  // `get secret -o yaml` prints the secrets' values (resource types are case-insensitive), and so can
  // `get` with a raw API path, a manifest or a kustomization, which name the objects another way.
  const names = args.slice(1).filter((a) => !a.startsWith("-")).flatMap((a) => a.split(","));
  if (sub === "get" && names.some((n) => /^secrets?($|[/.])/i.test(n))) return false;
  if (sub === "get" && args.some((a) => refused(a, ["--raw", "--filename", "--kustomize"], "k"))) return false;
  return !args.some((a) => refused(a, ["--watch", "--follow", "--output-directory"], "wf"));
}

const SYSTEMCTL_READS = [
  "status",
  "show",
  "cat",
  "list-units",
  "list-unit-files",
  "list-timers",
  "list-sockets",
  "list-dependencies",
  "list-jobs",
  "is-active",
  "is-enabled",
  "is-failed",
  "is-system-running",
  "get-default",
];

function systemctlReads(args: string[]): boolean {
  const [verb] = firstWord(args);
  // Without a verb, systemctl lists units.
  return verb === undefined || SYSTEMCTL_READS.includes(verb);
}

/** `ip addr`, `ip route` and `ip link`, and their show forms. */
function ipReads(args: string[]): boolean {
  let i = 0;
  for (; i < args.length && args[i].startsWith("-"); i++) {
    if (!/^-(4|6|0|br(ief)?|c(olor)?(=\w+)?|j(son)?|p(retty)?|s(tats?|tatistics)?|d(etails)?|o(neline)?|h(uman(-readable)?)?|r(esolve)?)$/.test(args[i])) return false;
  }
  const object = args[i];
  // ip takes any abbreviation of an object's name.
  const is = (name: string) => !!object && name.startsWith(object);
  if (!is("address") && !is("route") && !is("link")) return false;
  const verb = args[i + 1];
  return verb === undefined || ["show", "list", "lst"].includes(verb) || (is("route") && verb === "get");
}

/** `date` that shows a date: not `-s`, and no positional argument, which would set the clock. */
function dateReads(args: string[]): boolean {
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    if (a.startsWith("+") || /^-I\w*$/.test(a)) continue;
    if (!a.startsWith("-") || refused(a, ["--set"], "s")) return false;
    if (a === "-d" || a === "-r" || a === "-f") i++;
  }
  return true;
}

/** `hostname` that shows a name; with an argument, it sets it. */
function hostnameReads(args: string[]): boolean {
  return args.every((a) => /^-[fsdiIAa]+$/.test(a) || ["--fqdn", "--long", "--short", "--domain", "--ip-address", "--all-ip-addresses", "--all-fqdns", "--alias"].includes(a));
}

function dmesgReads(args: string[]): boolean {
  return args.every((a) => /^-[THkxdLe]+$/.test(a) || /^--(ctime|human|kernel|decode|reltime|notime|nopager|raw|color(=\w+)?|time-format=[\w-]+|(level|facility)=[\w,]+)$/.test(a));
}

interface Rule {
  ok: (args: string[]) => boolean;
  /** An unquoted glob may stand among the arguments: no option of the command writes or runs anything. */
  globs?: boolean;
}

const free: Rule = { ok: anyArgs, globs: true };

/** The commands that only read, and the arguments they may take for it. */
const READ_ONLY: Record<string, Rule> = {
  ls: free,
  cat: free,
  head: free,
  tail: { ok: without(["--follow"], "fF"), globs: true },
  grep: free,
  egrep: free,
  fgrep: free,
  rg: { ok: without(["--pre", "--hostname-bin"]) },
  find: { ok: (args) => !args.some((a) => ["-exec", "-execdir", "-ok", "-okdir", "-delete", "-fprint", "-fprint0", "-fprintf", "-fls"].includes(a)) },
  wc: free,
  stat: free,
  file: { ok: without(["--compile"], "C") },
  du: free,
  df: free,
  pwd: free,
  whoami: free,
  id: free,
  uname: free,
  hostname: { ok: hostnameReads },
  uptime: free,
  date: { ok: dateReads },
  which: free,
  whereis: free,
  type: free,
  ps: free,
  free: free,
  lsblk: free,
  lscpu: free,
  nproc: free,
  w: free,
  who: free,
  last: free,
  lsof: free,
  getent: free,
  echo: free,
  printf: free,
  cd: free,
  true: free,
  realpath: free,
  readlink: free,
  basename: free,
  dirname: free,
  cut: free,
  tr: free,
  nl: free,
  column: free,
  jq: free,
  md5sum: free,
  sha1sum: free,
  sha256sum: free,
  sha512sum: free,
  sort: { ok: without(["--output", "--compress-program"], "o") },
  uniq: { ok: (args) => args.filter((a) => !a.startsWith("-")).length <= 1 },
  ss: { ok: without(["--kill", "--diag"], "KD") },
  netstat: free,
  ip: { ok: ipReads },
  dmesg: { ok: dmesgReads },
  systemctl: { ok: systemctlReads },
  journalctl: {
    ok: without(
      ["--vacuum", "--rotate", "--follow", "--flush", "--sync", "--relinquish-var", "--smart-relinquish-var", "--setup-keys", "--update-catalog", "--cursor-file"],
      "f",
    ),
  },
  git: { ok: gitReads },
  docker: { ok: dockerReads },
  podman: { ok: dockerReads },
  kubectl: { ok: kubectlReads },
};

function readsOnly({ words, globs }: Simple): boolean {
  const rule = Object.hasOwn(READ_ONLY, words[0]) ? READ_ONLY[words[0]] : undefined;
  if (!rule || globs[0] || (!rule.globs && globs.some(Boolean))) return false;
  return rule.ok(words.slice(1));
}

/** A command that sets environment variables before it runs, or runs with sudo, never matches a prefix. */
function plain(words: string[]): boolean {
  return !/^[A-Za-z_][A-Za-z0-9_]*=/.test(words[0]) && words[0] !== "sudo";
}

/** The words of a prefix the user allowed. */
const prefixWords = (prefix: string) => prefix.trim().split(/\s+/).filter(Boolean);

/**
 * Whether a `run_command` command line runs without asking: every command in it only reads, or starts
 * with one of `prefixes` (whole words: `git pull` covers `git pull origin`, not `git pull-request`).
 * A line that uses anything `splitCommands` refuses never does.
 */
export function commandAllowed(line: string, prefixes: readonly string[]): boolean {
  const commands = splitCommands(line);
  if (!commands) return false;
  const allowed = prefixes.map(prefixWords).filter((p) => p.length > 0);
  return commands.every((c) => readsOnly(c) || (plain(c.words) && allowed.some((p) => p.length <= c.words.length && p.every((w, i) => c.words[i] === w))));
}

/** Whether the command line only reads (§13.5), and so runs without asking in manual approval mode. */
export function readsOnlyCommand(line: string): boolean {
  return commandAllowed(line, []);
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

/**
 * Whether `word` names a command of `names`, also with a directory or a version: `/usr/bin/sudo` is
 * `sudo`, and `python3.12` is `python`.
 */
function named(names: ReadonlySet<string>, word: string): boolean {
  const base = word.slice(word.lastIndexOf("/") + 1);
  return names.has(base) || names.has(base.replace(/[\d.]+$/, ""));
}

/** Commands that run another command, which a prefix of theirs would allow whatever it is. */
const WRAPPERS = new Set(["sudo", "doas", "su", "env", "xargs", "nohup", "timeout", "nice", "ionice", "time", "watch", "exec", "eval", "command", "builtin", "sh", "bash", "zsh", "dash", "ksh", "fish", "busybox"]);

/**
 * Commands that a prefix saved for good would let run anything, delete or overwrite data, or send it
 * elsewhere on the host: interpreters, deleting and formatting, permissions, processes and power, and
 * network transfers. Their cards offer Allow for this conversation, not Always allow.
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
]);

/** Whether Always allow may save these prefixes (`allowancePrefixes`), which `NOT_SAVED` decides. */
export function savablePrefixes(prefixes: readonly string[]): boolean {
  return prefixes.every((p) => {
    const name = prefixWords(p)[0] ?? "";
    return !named(NOT_SAVED, name) && !name.slice(name.lastIndexOf("/") + 1).startsWith("mkfs");
  });
}

/** The most prefixes one allowance on a card names. */
const MAX_PREFIXES = 3;

/**
 * What Allow for this conversation and Always allow on the approval card of a `run_command` call allow
 * (AI-19): for each command of the line that does not only read, its name, and for a tool such as `git`
 * its subcommand too. Null when the card offers neither: the line uses what `splitCommands` refuses,
 * a command runs another one (`sudo`, `xargs`, `sh -c`), sets variables first, or gives no subcommand,
 * or the line would need more than three prefixes.
 */
export function allowancePrefixes(line: string): string[] | null {
  const commands = splitCommands(line);
  if (!commands) return null;
  const out: string[] = [];
  for (const c of commands) {
    if (readsOnly(c)) continue;
    const [name, sub] = c.words;
    if (!plain(c.words) || named(WRAPPERS, name) || c.globs[0] || /\s/.test(name)) return null;
    let prefix = name;
    if (named(SUBCOMMANDS, name)) {
      if (!sub || sub.startsWith("-") || c.globs[1] || /\s/.test(sub)) return null;
      prefix = `${name} ${sub}`;
    }
    if (!out.includes(prefix)) out.push(prefix);
  }
  return out.length > 0 && out.length <= MAX_PREFIXES ? out : null;
}
