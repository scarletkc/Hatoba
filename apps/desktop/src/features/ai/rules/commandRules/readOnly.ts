import type { ShellDialect, Simple } from "./split";

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

/**
 * What one option takes: `false` no value, `true` a value (after `=` or in the next word, for a short option
 * also in the rest of its word, as in `-n20`), and a pattern an optional value, which is the next word only when
 * the pattern matches it, as journalctl reads `-b -1`.
 */
type Arity = boolean | RegExp;

/** A tool's options that only read, by long name and by short letter. */
interface Options {
  long: Record<string, Arity>;
  short: Record<string, Arity>;
}

/**
 * {@link Options} from their names: the long ones separated by spaces, one that takes a value ending in `=`, and the
 * short ones in getopt's form, a letter that takes a value followed by `:` (`c:n:qv`). `optional` names, by long
 * name or short letter, the options whose value is optional.
 */
function options(long: string, short: string, optional: Record<string, RegExp> = {}): Options {
  const out: Options = {
    long: Object.fromEntries(
      long
        .split(/\s+/)
        .filter(Boolean)
        .map((n) => [n.replace(/=$/, ""), n.endsWith("=")]),
    ),
    short: Object.fromEntries([...short.matchAll(/(.)(:?)/g)].map(([, c, value]) => [c, value === ":"])),
  };
  for (const [name, pattern] of Object.entries(optional)) (name.length === 1 ? out.short : out.long)[name] = pattern;
  return out;
}

/**
 * The operands of `args`, or null when an option is not one of `options`, so that an option nobody checked
 * asks. Long options need their full name; a cluster of short options ends at the first that takes a value.
 * A lone `-` (standard input) is an operand, and so is every word after `--`.
 */
function operands(args: readonly string[], { long, short }: Options): string[] | null {
  const out: string[] = [];
  /** How many words after `i` an option's value takes: none when it is attached or the option takes none. */
  const value = (arity: Arity, attached: boolean, i: number) =>
    attached || arity === false ? 0 : arity === true || arity.test(args[i + 1] ?? "") ? 1 : 0;
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    if (a === "--") {
      out.push(...args.slice(i + 1));
      break;
    }
    if (a.startsWith("--")) {
      const eq = a.indexOf("=");
      const name = a.slice(2, eq < 0 ? undefined : eq);
      if (!Object.hasOwn(long, name)) return null;
      i += value(long[name], eq >= 0, i);
    } else if (a.length > 1 && a.startsWith("-")) {
      for (let j = 1; j < a.length; j++) {
        if (!Object.hasOwn(short, a[j])) return null;
        if (short[a[j]] !== false) {
          i += value(short[a[j]], j < a.length - 1, i);
          break;
        }
      }
    } else out.push(a);
  }
  return out;
}

/** Arguments with only the options of `known`. */
const reads = (known: Options) => (args: string[]) => operands(args, known) !== null;

/** The entry of `table` named `key`, not one of its prototype's. */
const own = <T>(table: Record<string, T>, key: string | undefined): T | undefined =>
  key !== undefined && Object.hasOwn(table, key) ? table[key] : undefined;

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
    else if (
      !["--no-pager", "-P", "--no-optional-locks", "--literal-pathspecs"].includes(a) &&
      !/^--(git-dir|work-tree)=/.test(a)
    )
      return false;
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
      return lists(
        [
          "--delete",
          "--move",
          "--copy",
          "--force",
          "--set-upstream-to",
          "--unset-upstream",
          "--edit-description",
          "--track",
          "--create-reflog",
        ],
        "dDmMcCfut",
        false,
      );
    case "tag":
      return lists(
        [
          "--delete",
          "--annotate",
          "--sign",
          "--local-user",
          "--force",
          "--message",
          "--file",
          "--edit",
          "--create-reflog",
        ],
        "dasufmFe",
        true,
      );
    case "remote": {
      if (rest.every((a) => a === "-v" || a === "--verbose") || rest[0] === "get-url") return true;
      if (rest[0] !== "show") return false;
      // `remote show <name>` asks the remote for its branches, through core.sshCommand or a credential helper,
      // unless -n is given; without a name it lists the remotes.
      const args = rest.slice(1);
      const end = args.indexOf("--");
      const names = operands(args, { long: {}, short: { n: false } });
      return !!names && (names.length === 0 || (end < 0 ? args : args.slice(0, end)).includes("-n"));
    }
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

const DOCKER_PS = options("all filter= format= help last= latest no-trunc quiet size", "af:ln:qs");

const DOCKER_IMAGES = options("all digests filter= format= help no-trunc quiet tree", "af:q");

/** Not `--follow`, and not `--details`, which prints the environment variables a log option names. */
const DOCKER_LOGS = options("help since= tail= timestamps until=", "n:t");

const DOCKER_FORMAT = options("format= help", "f:");

/**
 * `docker` and `podman` subcommands that list containers and images, with the options of each that only read. Not
 * `inspect`, which prints a container's environment, where passwords often are.
 */
const DOCKER_READS: Record<string, Options> = {
  ps: DOCKER_PS,
  images: DOCKER_IMAGES,
  logs: DOCKER_LOGS,
  version: DOCKER_FORMAT,
  info: DOCKER_FORMAT,
};

/** The subcommands of {@link DOCKER_READS} that come after a group's name. */
const DOCKER_GROUPS: Record<string, Record<string, Options>> = {
  container: { ls: DOCKER_PS, list: DOCKER_PS, ps: DOCKER_PS, logs: DOCKER_LOGS },
  image: { ls: DOCKER_IMAGES, list: DOCKER_IMAGES },
  compose: {
    ps: options("all dry-run filter= format= help no-trunc orphans quiet services status=", "aq"),
    ls: options("all dry-run filter= format= help quiet", "aq"),
    images: options("dry-run format= help quiet", "q"),
    version: options("dry-run format= help short", "f:"),
    logs: options("dry-run help index= no-color no-log-prefix since= tail= timestamps until=", "n:t"),
  },
};

/** `docker` or `podman` with a subcommand of {@link DOCKER_READS} or {@link DOCKER_GROUPS}, first, and only its options. */
const dockerReads =
  (podman: boolean) =>
  (args: string[]): boolean => {
    const group = own(DOCKER_GROUPS, args[0]);
    const known = group ? own(group, args[1]) : own(DOCKER_READS, args[0]);
    if (!known) return false;
    // podman's `logs -n` is `--names`, which takes no value. Reading its other `-n` that way too is safe: the
    // value it takes is then checked as an option.
    const short = podman && known.short.n === true ? { ...known.short, n: false } : known.short;
    return operands(args.slice(group ? 2 : 1), { long: known.long, short }) !== null;
  };

const KUBECTL_READS = [
  "get",
  "describe",
  "logs",
  "top",
  "version",
  "explain",
  "api-resources",
  "api-versions",
  "cluster-info",
];

/**
 * The options of {@link KUBECTL_READS} that only read. Every other option asks, among them those that pick the API
 * server, its TLS trust, the credentials or the kubeconfig (whose `exec` plugins run programs), which would send the
 * cluster's credentials elsewhere; those that write files (`--cache-dir`, `--profile-output`, `--output-directory`);
 * those that follow output (`--watch`, `--follow`); and `get`'s raw API paths, manifests and kustomizations, which
 * name objects another way.
 */
const KUBECTL = options(
  `all-containers all-namespaces all-pods allow-missing-template-keys api-group= api-version= cached categories= chunk-size= client container=
  containers context= field-selector= ignore-errors ignore-not-found label-columns= limit-bytes= max-log-requests= namespace= namespaced namespaces=
  no-headers output= pod-running-timeout= prefix previous recursive request-timeout= selector= server-print show-capacity show-events show-kind
  show-labels show-managed-fields since= since-time= sort-by= sum tail= template= timestamps use-protocol-buffers v= verbs=`,
  "Ac:L:l:n:o:pRv:",
);

/** `kubectl` with a verb that only reads, first, and only the options of {@link KUBECTL}. */
function kubectlReads(args: string[]): boolean {
  const [sub, ...rest] = args;
  if (!KUBECTL_READS.includes(sub)) return false;
  const names = operands(rest, KUBECTL);
  // `get secret -o yaml` prints the secrets' values; resource types are case-insensitive.
  return !!names && (sub !== "get" || !names.flatMap((a) => a.split(",")).some((n) => /^secrets?($|[/.])/i.test(n)));
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
    if (
      !/^-(4|6|0|br(ief)?|c(olor)?(=\w+)?|j(son)?|p(retty)?|s(tats?|tatistics)?|d(etails)?|o(neline)?|h(uman(-readable)?)?|r(esolve)?)$/.test(
        args[i],
      )
    )
      return false;
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
  return args.every(
    (a) =>
      /^-[fsdiIAa]+$/.test(a) ||
      [
        "--fqdn",
        "--long",
        "--short",
        "--domain",
        "--ip-address",
        "--all-ip-addresses",
        "--all-fqdns",
        "--alias",
      ].includes(a),
  );
}

/**
 * The parts of a `printf` format that only print: text, the escapes every shell reads alike, `%%`, and a conversion
 * with its flags, width and precision in digits, whose letter is captured. Not `\u0025`, which zsh and
 * ksh read as the `%` of a conversion, `%n`, or a `*` width.
 */
const PRINTF_PART = /[^%\\]+|\\[\\abfnrtv]|%%|%[-+ #0]*\d*(?:\.\d*)?([sbqcdiouxXeEfgG])/g;

/** A number as every shell's `printf` reads it, with nothing to evaluate. */
const PRINTF_NUMBER = /^[-+]?(\d+\.?\d*|\.\d+|0[xX][\da-fA-F]+)$/;

/**
 * `printf` that only prints. The shell's own `printf` also assigns variables, `PATH` and `HOME` among them: with
 * `-v`, with `%n`, and in zsh and ksh with the argument of a number's conversion, which they evaluate as an
 * arithmetic expression (`x=7`). So the format comes first, or after `--`, holds only {@link PRINTF_PART}s, and
 * takes a {@link PRINTF_NUMBER} for each number, also when it repeats for the arguments left.
 */
function printfReads(args: string[]): boolean {
  const [format, ...values] = args[0] === "--" ? args.slice(1) : args;
  if (format === undefined || (format.startsWith("-") && args[0] !== "--")) return false;
  const parts = [...format.matchAll(PRINTF_PART)];
  if (parts.map((p) => p[0]).join("") !== format) return false;
  const numbers = parts.filter((p) => p[1]).map((p) => !"sbqc".includes(p[1]));
  return values.every((v, i) => !numbers[i % numbers.length] || PRINTF_NUMBER.test(v));
}

function dmesgReads(args: string[]): boolean {
  return args.every(
    (a) =>
      /^-[THkxdLe]+$/.test(a) ||
      /^--(ctime|human|kernel|decode|reltime|notime|nopager|raw|color(=\w+)?|time-format=[\w-]+|(level|facility)=[\w,]+)$/.test(
        a,
      ),
  );
}

/** `tail`'s options that only read: not `--follow`, `-f` and `-F`, nor the options that only matter while following. */
const TAIL = options("bytes= help lines= quiet silent verbose version zero-terminated", "c:n:qvz");

/**
 * `tail` with only the options of {@link TAIL}, and the obsolete `tail -20`. GNU tail also reads `+20` and `+f` as
 * the obsolete form, which follows with an `f`.
 */
function tailReads(args: string[]): boolean {
  const files = operands(/^-\d+[bcl]?$/.test(args[0] ?? "") ? args.slice(1) : args, TAIL);
  return !!files && !files.some((f) => /^\+\d*[bcl]?f$/.test(f));
}

/**
 * `sort`'s options that only read: not `--output` and `-o`, `--compress-program`, which runs the program it names,
 * or `--temporary-directory` and `-T`, which pick where it writes.
 */
const SORT = options(
  `batch-size= buffer-size= check debug dictionary-order field-separator= files0-from= general-numeric-sort help human-numeric-sort ignore-case
  ignore-leading-blanks ignore-nonprinting key= merge month-sort numeric-sort parallel= random-sort random-source= reverse sort= stable unique version
  version-sort zero-terminated`,
  "bCcdfghik:MmnRrS:st:uVz",
);

/**
 * `file`'s options that only read: not `--compile` and `-C`, which writes the compiled magic file, `-p`, which sets
 * the files' access times back, `-z` and `-Z`, which may run a decompression program found on `PATH`, or `-S`,
 * which turns its sandbox off.
 */
const FILE = options(
  `apple brief checking-printout debug dereference exclude= exclude-quiet= extension files-from= help keep-going list magic-file= mime mime-encoding
  mime-type no-buffer no-dereference no-pad parameter= print0 raw separator= special-files version`,
  "0bcde:F:f:hikLlm:NnP:rsv",
);

/**
 * `ss`'s options that only read: not `--kill` and `-K`, which close sockets, `--diag` and `-D`, which writes a
 * file, or `--events` and `-E`, which keeps printing until stopped.
 */
const SS = options(
  `all bpf cgroup context contexts dccp extended family= filter= help info inet-sockopt ipv4 ipv6 listening memory mptcp net= no-header numeric
  oneline options packet processes query= raw resolve sctp socket= summary tcp threads tipc tipcinfo tos udp unix version vsock xdp`,
  "046A:abdeF:f:HhilMmN:nOoprSsTtuVwxZz",
);

/** The boot `journalctl -b` takes from the next word: an offset, a boot ID with or without one, or `all`. */
const JOURNAL_BOOT = /^(all|[+-]?\d+|[0-9a-f]{32}([+-]\d+)?)$/i;

/** The count `journalctl -n` takes from the next word. */
const JOURNAL_LINES = /^(all|\+?\d+)$/;

/**
 * `journalctl`'s options that only read: not `--follow` and `-f`, the vacuuming, rotating, flushing, syncing, and
 * relinquishing commands, `--update-catalog`, the sealing keys' `--setup-keys`, `--force`, and `--interval`,
 * `--cursor-file`, which writes the last entry's cursor to the file, or `--image`, which mounts a disk image.
 */
const JOURNALCTL = options(
  `after-cursor= all case-sensitive catalog cursor= directory= disk-usage dmesg dump-catalog exclude-identifier= facility= field= fields file= full
  grep= header help identifier= list-boots list-catalog list-namespaces machine= merge namespace= no-full no-hostname no-pager no-tail output=
  output-fields= pager-end priority= quiet reverse root= show-cursor since= system this-boot truncate-newline unit= until= user user-unit= utc verify
  verify-key= version`,
  "ac:D:eF:g:hklM:mNo:p:qrS:T:t:U:u:x",
  { boot: JOURNAL_BOOT, b: JOURNAL_BOOT, lines: JOURNAL_LINES, n: JOURNAL_LINES },
);

/**
 * `rg`'s options that only read: not `--pre`, which runs a program on each file, `--hostname-bin`, which runs one
 * to look up the host name, or `--search-zip` and `-z`, which run the decompression programs found on `PATH`.
 */
const RG = options(
  `after-context= auto-hybrid-regex before-context= binary block-buffered byte-offset case-sensitive color= colors= column context= context-separator=
  count count-matches crlf debug dfa-size-limit= encoding= engine= field-context-separator= field-match-separator= file= files files-with-matches
  files-without-match fixed-strings follow generate= glob= glob-case-insensitive heading help hidden hyperlink-format= iglob= ignore ignore-case
  ignore-dot ignore-exclude ignore-file= ignore-file-case-insensitive ignore-files ignore-global ignore-messages ignore-parent ignore-vcs include-zero
  invert-match json line-buffered line-number line-regexp max-columns= max-columns-preview max-count= max-depth= max-filesize= maxdepth= messages mmap
  multiline multiline-dotall no-auto-hybrid-regex no-binary no-block-buffered no-byte-offset no-column no-config no-context-separator no-crlf
  no-filename no-fixed-strings no-follow no-glob-case-insensitive no-heading no-hidden no-ignore no-ignore-dot no-ignore-exclude
  no-ignore-file-case-insensitive no-ignore-files no-ignore-global no-ignore-messages no-ignore-parent no-ignore-vcs no-include-zero no-invert-match
  no-json no-line-buffered no-line-number no-max-columns-preview no-messages no-mmap no-multiline no-multiline-dotall no-one-file-system no-pcre2
  no-pcre2-unicode no-require-git no-search-zip no-sort-files no-stats no-text no-trim no-unicode null null-data one-file-system only-matching passthru
  path-separator= pcre2 pcre2-unicode pcre2-version pretty quiet regex-size-limit= regexp= replace= require-git smart-case sort= sort-files sortr=
  stats stop-on-nonmatch text threads= trace trim type= type-add= type-clear= type-list type-not= unicode unrestricted version vimgrep with-filename
  word-regexp`,
  ".0A:aB:bC:cd:E:e:Ff:g:HhIij:LlM:m:NnoPpqr:SsT:t:UuVvwx",
);

/**
 * The primaries of `find`'s expression that only read, by the number of words they take: not `-exec`, `-execdir`,
 * `-ok`, and `-okdir`, which run programs, `-delete`, or `-fprint`, `-fprint0`, `-fprintf`, and `-fls`, which write
 * files. Those that take a word end in `=`.
 */
const FIND_PRIMARIES: Record<string, number> = Object.fromEntries(
  `! ( ) , -a -and -daystart -depth -empty -executable -false -follow -ignore_readdir_race -ls -mount -nogroup -noignore_readdir_race -noleaf -not
  -nouser -nowarn -o -or -print -print0 -prune -quit -readable -true -warn -writable -xdev
  -amin= -anewer= -atime= -cmin= -cnewer= -context= -ctime= -files0-from= -fstype= -gid= -group= -ilname= -iname= -inum= -ipath= -iregex= -iwholename=
  -links= -lname= -maxdepth= -mindepth= -mmin= -mtime= -name= -newer= -path= -perm= -printf= -regex= -regextype= -samefile= -size= -type= -uid= -used=
  -user= -wholename= -xtype=`
    .split(/\s+/)
    .map((p) => [p.replace(/=$/, ""), p.endsWith("=") ? 1 : 0]),
);

/**
 * `find` with only the primaries of {@link FIND_PRIMARIES} and `-newerXY`. Before the starting points it takes `-H`,
 * `-L`, `-P`, `-D` with its debug options, `-O` with a level, and BSD find's `-E`, `-X`, `-s`, and `-x`; the expression
 * starts at the first word that begins with `-` or is `(`, `)`, `!`, or `,`.
 */
function findReads(args: string[]): boolean {
  let i = 0;
  for (; i < args.length && /^-([HLPEXsx]+|O\d*|D)$/.test(args[i]); i++) if (args[i] === "-D") i++;
  while (i < args.length && !args[i].startsWith("-") && !["(", ")", "!", ","].includes(args[i])) i++;
  for (; i < args.length; i++) {
    const words = /^-newer[aBcm][aBcmt]$/.test(args[i]) ? 1 : own(FIND_PRIMARIES, args[i]);
    if (words === undefined) return false;
    i += words;
  }
  return true;
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
  tail: { ok: tailReads, globs: true },
  grep: free,
  egrep: free,
  fgrep: free,
  rg: { ok: reads(RG) },
  find: { ok: findReads },
  wc: free,
  stat: free,
  file: { ok: reads(FILE) },
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
  // No unquoted glob: `[-]v` becomes `-v` in a folder that holds a file of that name.
  printf: { ok: printfReads },
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
  sort: { ok: reads(SORT) },
  uniq: { ok: uniqReads },
  ss: { ok: reads(SS) },
  netstat: free,
  ip: { ok: ipReads },
  dmesg: { ok: dmesgReads },
  systemctl: { ok: systemctlReads },
  journalctl: { ok: reads(JOURNALCTL) },
  git: { ok: gitReads },
  docker: { ok: dockerReads(false) },
  podman: { ok: dockerReads(true) },
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
const each = (names: readonly string[], rule: Rule): Record<string, Rule> =>
  Object.fromEntries(names.map((n) => [n, rule]));

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
  ...each(
    [
      "get-location",
      "gl",
      "pwd",
      "set-location",
      "sl",
      "cd",
      "chdir",
      "push-location",
      "pushd",
      "pop-location",
      "popd",
    ],
    free,
  ),
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
export function reading(commands: readonly Simple[], dialect: ShellDialect): boolean[] {
  const moves = dialect === "cmd" && commands.length > 1;
  return commands.map((c) => readsOnly(c, dialect) && !(moves && CMD_CD.has(lookupName(c.words[0]))));
}
