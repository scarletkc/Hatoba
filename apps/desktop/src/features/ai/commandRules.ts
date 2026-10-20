import type { LocalShellKind } from "@/ipc/types";

/**
 * §13.5: which `run_command` command lines run without asking in manual approval mode. A command line
 * runs without asking when every command in it only reads (`ls`, `cat`, `grep`, `git log`, …) or starts
 * with a prefix the user allowed (AI-19). Whatever this cannot tell for sure asks.
 */

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
interface Simple {
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
    // These run what the repository's configuration names (core.fsmonitor, diff.external, textconv drivers,
    // filters), as when the user types them. Writing that configuration asks first, unless an edit, write or
    // terminal allowance already lets the model run code on the host (§13.5).
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

/**
 * The options of {@link KUBECTL_READS} that only read, by long name: true for one that takes a value (after `=`,
 * in the next word, or for a short one in the rest of its word, as in `-owide`). Every other option asks,
 * among them those that pick the API server, its TLS trust, the credentials or the kubeconfig (whose `exec`
 * plugins run programs), which would send the cluster's credentials elsewhere; those that write files
 * (`--cache-dir`, `--profile-output`, `--output-directory`); those that follow output (`--watch`, `--follow`);
 * and `get`'s raw API paths, manifests and kustomizations, which name objects another way.
 */
const KUBECTL_LONG: Record<string, boolean> = {
  "all-containers": false,
  "all-namespaces": false,
  "all-pods": false,
  "allow-missing-template-keys": false,
  "api-group": true,
  "api-version": true,
  cached: false,
  categories: true,
  "chunk-size": true,
  client: false,
  container: true,
  containers: false,
  context: true,
  "field-selector": true,
  "ignore-errors": false,
  "ignore-not-found": false,
  "label-columns": true,
  "limit-bytes": true,
  "max-log-requests": true,
  namespace: true,
  namespaced: false,
  namespaces: true,
  "no-headers": false,
  output: true,
  "pod-running-timeout": true,
  prefix: false,
  previous: false,
  recursive: false,
  "request-timeout": true,
  selector: true,
  "server-print": false,
  "show-capacity": false,
  "show-events": false,
  "show-kind": false,
  "show-labels": false,
  "show-managed-fields": false,
  since: true,
  "since-time": true,
  "sort-by": true,
  sum: false,
  tail: true,
  template: true,
  timestamps: false,
  "use-protocol-buffers": false,
  v: true,
  verbs: true,
};

/** The short forms of {@link KUBECTL_LONG}'s options. */
const KUBECTL_SHORT: Record<string, boolean> = { n: true, v: true, A: false, o: true, l: true, L: true, R: false, c: true, p: false };

/** `kubectl` with a verb that only reads, first, and only options of {@link KUBECTL_LONG} and {@link KUBECTL_SHORT}. */
function kubectlReads(args: string[]): boolean {
  const [sub, ...rest] = args;
  if (!KUBECTL_READS.includes(sub)) return false;
  const names: string[] = [];
  for (let i = 0; i < rest.length; i++) {
    const a = rest[i];
    if (a === "--") {
      names.push(...rest.slice(i + 1));
      break;
    }
    if (a.startsWith("--")) {
      const eq = a.indexOf("=");
      const name = a.slice(2, eq < 0 ? undefined : eq);
      if (!Object.hasOwn(KUBECTL_LONG, name)) return false;
      if (KUBECTL_LONG[name] && eq < 0) i++;
    } else if (a.length > 1 && a.startsWith("-")) {
      // A cluster of short options, the first that takes a value taking the rest of the word or the next one.
      for (let j = 1; j < a.length; j++) {
        if (!Object.hasOwn(KUBECTL_SHORT, a[j])) return false;
        if (KUBECTL_SHORT[a[j]]) {
          if (j === a.length - 1) i++;
          break;
        }
      }
    } else names.push(a);
  }
  // `get secret -o yaml` prints the secrets' values; resource types are case-insensitive.
  return sub !== "get" || !names.flatMap((a) => a.split(",")).some((n) => /^secrets?($|[/.])/i.test(n));
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

/**
 * `uniq` with one input at most: a second operand is the file it writes. Options end at `--` or at the first
 * operand, a lone `-` (standard input) included, as in BSD uniq, so every word after it is an operand.
 */
function uniqReads(args: string[]): boolean {
  const first = args.findIndex((a) => a === "-" || !a.startsWith("-") || a === "--");
  if (first < 0) return true;
  return args.length - (args[first] === "--" ? first + 1 : first) <= 1;
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
  uniq: { ok: uniqReads },
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

/**
 * A PowerShell parameter (`-Name` or `-Name:value`) that is `name` or an abbreviation of it, as PowerShell
 * accepts them, in any case.
 */
function psParam(arg: string, name: string): boolean {
  const given = arg.startsWith("-") ? arg.slice(1).split(":")[0].toLowerCase() : "";
  return given.length > 0 && name.startsWith(given);
}

/** `ipconfig` that shows the configuration: not `/release`, `/renew`, `/flushdns` and the like. */
function ipconfigReads(args: string[]): boolean {
  return args.every((a) => /^[-/](all|allcompartments|displaydns)$/i.test(a));
}

/** `arp` that shows the table: not `-d` or `-s`, which delete and add entries. */
function arpReads(args: string[]): boolean {
  return !args.some((a) => /^[-/][ds]$/i.test(a));
}

/** `route print`; `-f`, also before it, clears the routing table. */
function routeReads(args: string[]): boolean {
  const [verb] = firstWord(args);
  return verb?.toLowerCase() === "print" && args.every((a) => !a.startsWith("-") || a === "-4" || a === "-6");
}

/** Command Prompt's `date` and `time` with `/t`; without it they ask for a new date or time to set. */
function showsOnly(args: string[]): boolean {
  return args.length === 1 && args[0].toLowerCase() === "/t";
}

/** The same rule under each of `names`: a cmdlet and its aliases, say. */
const each = (names: readonly string[], rule: Rule): Record<string, Rule> => Object.fromEntries(names.map((n) => [n, rule]));

/** Windows programs that only read, which PowerShell and Command Prompt run alike, and the tools of {@link READ_ONLY} found there too. */
const WINDOWS_READS: Record<string, Rule> = {
  ...each(["where", "tree", "findstr", "whoami", "systeminfo", "tasklist", "getmac", "jq"], free),
  hostname: { ok: hostnameReads },
  ipconfig: { ok: ipconfigReads },
  // A number repeats the output at that interval until stopped.
  netstat: { ok: (args) => !args.some((a) => /^\d+$/.test(a)) },
  arp: { ok: arpReads },
  route: { ok: routeReads },
  git: READ_ONLY.git,
  rg: READ_ONLY.rg,
  docker: READ_ONLY.docker,
  podman: READ_ONLY.podman,
  kubectl: READ_ONLY.kubectl,
};

/**
 * PowerShell's cmdlets that only read, by name and alias (lowercase): those Claude Code and Codex run
 * without asking, and the network and system queries beside them. `-Wait` keeps `Get-Content` reading.
 */
const POWERSHELL_READS: Record<string, Rule> = {
  ...WINDOWS_READS,
  ...each(["get-childitem", "gci", "ls", "dir", "select-string", "sls"], free),
  ...each(["get-content", "gc", "cat", "type"], { ok: (args) => !args.some((a) => psParam(a, "wait")) }),
  ...each(
    [
      "get-item",
      "gi",
      "get-itemproperty",
      "gp",
      "get-itempropertyvalue",
      "gpv",
      "test-path",
      "resolve-path",
      "rvpa",
      "convert-path",
      "cvpa",
      "split-path",
      "join-path",
      "get-filehash",
      "get-acl",
      "format-hex",
      "fhx",
    ],
    free,
  ),
  ...each(["get-location", "gl", "pwd", "set-location", "sl", "cd", "chdir", "push-location", "pushd", "pop-location", "popd"], free),
  ...each(
    [
      "write-output",
      "echo",
      "write",
      "write-host",
      "select-object",
      "select",
      "where-object",
      "where",
      "sort-object",
      "sort",
      "group-object",
      "group",
      "measure-object",
      "measure",
      "get-unique",
      "gu",
      "compare-object",
      "compare",
      "diff",
      "get-member",
      "gm",
      "format-table",
      "ft",
      "format-list",
      "fl",
      "format-wide",
      "fw",
      "format-custom",
      "fc",
      "out-string",
      "out-host",
      "oh",
      "convertto-json",
      "convertfrom-json",
      "convertto-csv",
      "convertfrom-csv",
      "convertto-xml",
      "convertto-html",
      "join-string",
    ],
    free,
  ),
  ...each(
    [
      "get-process",
      "ps",
      "gps",
      "get-service",
      "gsv",
      "get-command",
      "gcm",
      "get-alias",
      "gal",
      "get-module",
      "gmo",
      "get-psdrive",
      "gdr",
      "get-psprovider",
      "get-date",
      "get-host",
      "get-culture",
      "get-uiculture",
      "get-timezone",
      "gtz",
      "get-uptime",
      "get-computerinfo",
      "gin",
      "get-hotfix",
      "get-ciminstance",
      "gcim",
      "get-wmiobject",
      "gwmi",
      "get-winevent",
      "get-eventlog",
      "get-executionpolicy",
      "get-volume",
      "get-disk",
      "get-partition",
      "get-localuser",
      "get-localgroup",
      "get-localgroupmember",
      "get-scheduledtask",
    ],
    free,
  ),
  ...each(
    [
      "get-netadapter",
      "get-netipaddress",
      "get-netipconfiguration",
      "gip",
      "get-netroute",
      "get-nettcpconnection",
      "get-netudpendpoint",
      "get-dnsclient",
      "get-dnsclientserveraddress",
      "get-dnsclientcache",
      "test-netconnection",
      "tnc",
      "resolve-dnsname",
    ],
    free,
  ),
};

/** Command Prompt's own commands that only read, and the Windows programs of {@link WINDOWS_READS}. */
const CMD_READS: Record<string, Rule> = {
  ...WINDOWS_READS,
  ...each(["dir", "type", "echo", "cd", "chdir", "ver", "vol", "find", "fc"], free),
  date: { ok: showsOnly },
  time: { ok: showsOnly },
  // `/o` writes the sorted lines to a file.
  sort: { ok: (args) => !args.some((a) => /^\/o/i.test(a)) },
};

/** Command Prompt's commands that change the current folder. */
const CMD_CD = new Set(["cd", "chdir"]);

/**
 * The name a PowerShell or Command Prompt command is looked up by: in lowercase, and without `.exe` or
 * `.com` unless it names a directory too, which none of the tables trusts.
 */
function lookupName(word: string): string {
  const lower = word.toLowerCase();
  return /[\\/]/.test(lower) ? lower : lower.replace(/\.(exe|com)$/, "");
}

/**
 * A UNC path (`\\server\share`, `//server/share`, or with the slashes mixed, as in `/\server\share`): Windows
 * reaches the computer it names, and signs in there as the user.
 */
const UNC = /\\\\|(^|[^:])[\\/]{2}/;

function readsOnly({ words, globs }: Simple, dialect: ShellDialect): boolean {
  if (dialect !== "posix") {
    const table = dialect === "powershell" ? POWERSHELL_READS : CMD_READS;
    const name = lookupName(words[0]);
    const rule = Object.hasOwn(table, name) ? table[name] : undefined;
    return !!rule && !words.some((w) => UNC.test(w)) && rule.ok(words.slice(1));
  }
  const rule = Object.hasOwn(READ_ONLY, words[0]) ? READ_ONLY[words[0]] : undefined;
  if (!rule || globs[0] || (!rule.globs && globs.some(Boolean))) return false;
  return rule.ok(words.slice(1));
}

/**
 * Whether each of a line's commands only reads (§13.5). In Command Prompt a `cd` only reads alone: it runs a
 * program of the current folder before one on PATH, so a command after `cd` could run one the folder holds.
 */
function reading(commands: readonly Simple[], dialect: ShellDialect): boolean[] {
  const moves = dialect === "cmd" && commands.length > 1;
  return commands.map((c) => readsOnly(c, dialect) && !(moves && CMD_CD.has(lookupName(c.words[0]))));
}

/**
 * A command that sets environment variables before it runs, or runs with sudo, never matches a prefix. In
 * PowerShell and Command Prompt the splitter refuses variables, and sudo counts by its {@link windowsName}.
 */
function plain(words: string[], dialect: ShellDialect): boolean {
  if (dialect !== "posix") return windowsName(words[0]) !== "sudo";
  return !/^[A-Za-z_][A-Za-z0-9_]*=/.test(words[0]) && words[0] !== "sudo";
}

/** The words of a prefix the user allowed. */
const prefixWords = (prefix: string) => prefix.trim().split(/\s+/).filter(Boolean);

/** Whether a command starts with a prefix's words; PowerShell and Command Prompt take the command's name in any case. */
function startsWith(words: readonly string[], prefix: readonly string[], dialect: ShellDialect): boolean {
  if (prefix.length > words.length) return false;
  return prefix.every((w, i) => words[i] === w || (i === 0 && dialect !== "posix" && words[0].toLowerCase() === w.toLowerCase()));
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
export function commandAllowed(line: string, prefixes: readonly string[], dialect: ShellDialect = "posix", workdir = ""): boolean {
  const commands = callCommands(line, dialect, workdir);
  if (!commands) return false;
  const allowed = prefixes.map(prefixWords).filter((p) => p.length > 0);
  const reads = reading(commands, dialect);
  return commands.every((c, i) => reads[i] || (plain(c.words, dialect) && allowed.some((p) => startsWith(c.words, p, dialect))));
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
const WINDOWS_SUBCOMMANDS = new Set([...SUBCOMMANDS, "winget", "choco", "scoop", "dotnet", "net", "netsh", "reg", "sc"]);

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
const WRAPPERS = new Set(["sudo", "doas", "su", "env", "xargs", "nohup", "timeout", "nice", "ionice", "time", "watch", "exec", "eval", "command", "builtin", "sh", "bash", "zsh", "dash", "ksh", "fish", "busybox"]);

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
 * a command runs another one (`sudo`, `xargs`, `sh -c`, `Start-Process`), sets variables first, or gives no
 * subcommand (in PowerShell and Command Prompt also a `/` switch in its place), or the line would need more
 * than three prefixes. A `workdir` counts as a `cd` before the line; when that `cd` does not only read (in
 * Command Prompt beside any command, in PowerShell to a network path), it is offered for that folder alone
 * (`cd C:\src`), so that allowing it never lets a program of every other folder run, and not for a folder whose
 * name has a space, which a prefix cannot name.
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
    if (!plain(c.words, dialect) || named(windows ? WINDOWS_WRAPPERS : WRAPPERS, name, dialect) || c.globs[0] || /\s/.test(name)) return null;
    let prefix = name;
    if (named(windows ? WINDOWS_SUBCOMMANDS : SUBCOMMANDS, name, dialect)) {
      if (!sub || sub.startsWith("-") || (windows && sub.startsWith("/")) || c.globs[1] || /\s/.test(sub)) return null;
      prefix = `${name} ${sub}`;
    }
    if (!out.includes(prefix)) out.push(prefix);
  }
  return out.length > 0 && out.length <= MAX_PREFIXES ? out : null;
}
