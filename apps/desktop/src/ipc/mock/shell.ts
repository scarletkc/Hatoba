import type { HostView, LocalShellKind } from "../types";
import { VISITOR } from "./visitor";

/** A local tab's shell (§7.5): its kind decides the prompt. */
export interface LocalStyle {
  shell: LocalShellKind;
  user: string;
  machine: string;
}

const G = "\x1b[38;2;139;212;156m"; // design green  #8bd49c
const B = "\x1b[38;2;122;183;255m"; // design blue   #7ab7ff
const Y = "\x1b[38;2;229;192;123m"; // design yellow #e5c07b
const R = "\x1b[38;2;240;113;120m"; // design red    #f07178
const D = "\x1b[38;2;107;113;124m"; // design dim    #6b717c
const X = "\x1b[0m";

/** What `help` lists: the commands the fake shell answers. */
const COMMANDS = ["cat /etc/os-release", "cd", "clear", "curl", "date", "df", "echo", "exit", "free", "git log", "git status", "hostname", "ls", "ps", "pwd", "systemctl status api", "uname", "uptime", "whoami"];

/** A tiny line-editing fake shell that replays the design's terminal screen (mock only). */
export class FakeShell {
  onExit: (() => void) | null = null;
  private line = "";
  private cwd = "~";
  private stopped = false;
  private enc = new TextEncoder();

  constructor(
    /** The host the session is connected to; null for a local tab. */
    readonly host: HostView | null,
    private out: (bytes: Uint8Array) => void,
    /** A local tab's shell. */
    readonly local: LocalStyle | null = null,
  ) {}

  /** The server OS the host recorded (HOST-11), for the mock resource usage (TERM-12). */
  get os(): string | null {
    return this.host?.os ?? null;
  }

  private get user(): string {
    return this.host?.username ?? this.local?.user ?? "demo";
  }

  private get machine(): string {
    return this.host?.name ?? this.local?.machine ?? "localhost";
  }

  private get windowsPath(): string {
    return this.cwd === "~" ? `C:\\Users\\${this.user}` : this.cwd;
  }

  private get windows(): boolean {
    return this.local?.shell === "powershell" || this.local?.shell === "cmd";
  }

  private write(s: string) {
    if (!this.stopped) this.out(this.enc.encode(s));
  }

  private prompt() {
    if (this.local?.shell === "powershell") this.write(`PS ${this.windowsPath}> `);
    else if (this.local?.shell === "cmd") this.write(`${this.windowsPath}>`);
    else this.write(`${G}${this.user}@${this.machine}${X}:${B}${this.cwd}${X}$ `);
  }

  start() {
    if (this.local?.shell === "cmd") {
      this.write("Microsoft Windows [Version 10.0.26100.4652]\r\n(c) Microsoft Corporation. All rights reserved.\r\n\r\n");
    }
    if (!this.local) this.write(`${D}Last login: Thu Oct  8 09:41:12 2026 from 10.0.0.4${X}\r\n`);
    // The website's demo says what its shell answers.
    if (VISITOR) this.write(`${D}A demo shell with sample output: try ls, df -h, git log, or help.${X}\r\n`);
    if (this.host?.name === "prod-api-tokyo") {
      this.cwd = "/srv/api";
      const p = `${G}${this.host.username}@${this.host.name}${X}:`;
      this.write(
        [
          `${p}${B}~${X}$ cd /srv/api && git log --oneline -3`,
          `${Y}a3f91c2${X} (${B}HEAD -> main${X}, ${R}origin/main${X}) fix: retry D1 writes on 429`,
          `${Y}7be0d14${X} chore: bump tokio to 1.41`,
          `${Y}e02c5aa${X} feat: add /healthz latency histogram`,
          `${p}${B}/srv/api${X}$ sudo systemctl status api --no-pager`,
          `${G}●${X} api.service - Tokyo API (axum)`,
          `     Loaded: loaded (/etc/systemd/system/api.service; enabled)`,
          `     Active: ${G}active (running)${X} since Wed 2026-10-07 22:14:03 JST; 11h ago`,
          `   Main PID: 48211 (api)`,
          `     Memory: 212.4M`,
          `        CPU: 41min 12.330s`,
          ``,
          `${D}Oct 08 09:40:58 prod-api-tokyo api[48211]: GET /v1/hosts 200 4.1ms${X}`,
          `${D}Oct 08 09:41:03 prod-api-tokyo api[48211]: GET /healthz 200 0.3ms${X}`,
          `${p}${B}/srv/api${X}$ df -h /srv`,
          `Filesystem      Size  Used Avail Use% Mounted on`,
          `/dev/nvme1n1    196G   88G   99G  48% /srv`,
          ``,
        ].join("\r\n"),
      );
    }
    this.prompt();
  }

  input(data: string) {
    for (const ch of data) {
      if (ch === "\r") {
        this.write("\r\n");
        this.run(this.line.trim());
        this.line = "";
        if (!this.stopped) this.prompt();
      } else if (ch === "\x7f") {
        if (this.line) {
          this.line = this.line.slice(0, -1);
          this.write("\b \b");
        }
      } else if (ch === "\x03") {
        this.write("^C\r\n");
        this.line = "";
        this.prompt();
      } else if (ch === "\x0c") {
        this.write("\x1b[H\x1b[2J");
        this.prompt();
        this.write(this.line);
      } else if (ch >= " ") {
        this.line += ch;
        this.write(ch);
      }
    }
  }

  private run(cmd: string) {
    if (!cmd) return;
    const [first, ...rest] = cmd.split(/\s+/);
    // sudo runs the command, with no password to ask for.
    const [name, ...args] = first === "sudo" && rest.length > 0 ? rest : [first, ...rest];
    const lines = (text: string[]) => this.write(`${text.join("\r\n")}\r\n`);
    switch (name) {
      case "exit":
      case "logout":
        this.write("logout\r\n");
        this.stopped = true;
        this.onExit?.();
        return;
      case "clear":
        this.write("\x1b[H\x1b[2J");
        return;
      case "pwd":
        this.write(`${this.windows ? this.windowsPath : this.cwd === "~" ? `/home/${this.user}` : this.cwd}\r\n`);
        return;
      case "cd":
        this.cwd = args[0] ?? "~";
        return;
      case "echo":
        this.write(`${args.join(" ")}\r\n`);
        return;
      case "whoami":
        this.write(`${this.user}\r\n`);
        return;
      case "date":
        this.write(`${new Date().toString()}\r\n`);
        return;
      case "ls":
      case "dir":
        this.write(`${B}migrations${X}  ${B}src${X}  ${B}target${X}  Cargo.lock  Cargo.toml  Dockerfile  README.md  wrangler.toml\r\n`);
        return;
      case "uname":
        this.write(args.includes("-a") ? `Linux ${this.machine} 6.8.0-45-generic #45-Ubuntu SMP x86_64 GNU/Linux\r\n` : "Linux\r\n");
        return;
      case "hostname":
        this.write(`${this.machine}\r\n`);
        return;
      case "uptime":
        this.write(" 09:41:12 up 41 days,  3:12,  1 user,  load average: 0.21, 0.18, 0.12\r\n");
        return;
      case "df":
        lines([
          "Filesystem      Size  Used Avail Use% Mounted on",
          "/dev/nvme0n1p1   49G   23G   24G  48% /",
          "/dev/nvme1n1    196G   88G   99G  48% /srv",
          "tmpfs           7.8G     0  7.8G   0% /dev/shm",
        ]);
        return;
      case "free":
        lines([
          "               total        used        free      shared  buff/cache   available",
          "Mem:            15Gi       9.1Gi       1.2Gi       112Mi       5.3Gi       6.2Gi",
          "Swap:          4.0Gi          0B       4.0Gi",
        ]);
        return;
      case "ps":
        lines([
          "    PID TTY          TIME CMD",
          "  48211 ?        00:41:12 api",
          "    812 ?        00:03:27 nginx",
          "  51920 pts/0    00:00:00 bash",
          "  52003 pts/0    00:00:00 ps",
        ]);
        return;
      case "cat":
        if (args[0] === "/etc/os-release")
          lines(['PRETTY_NAME="Ubuntu 24.04.1 LTS"', 'NAME="Ubuntu"', 'VERSION_ID="24.04"', 'VERSION="24.04.1 LTS (Noble Numbat)"', "ID=ubuntu"]);
        else this.write(`cat: ${args[0] ?? ""}: No such file or directory\r\n`);
        return;
      case "git":
        if (args[0] === "log")
          lines([
            `${Y}a3f91c2${X} (${B}HEAD -> main${X}, ${R}origin/main${X}) fix: retry D1 writes on 429`,
            `${Y}7be0d14${X} chore: bump tokio to 1.41`,
            `${Y}e02c5aa${X} feat: add /healthz latency histogram`,
          ]);
        else if (args[0] === "status") lines(["On branch main", "Your branch is up to date with 'origin/main'.", "", "nothing to commit, working tree clean"]);
        else this.write(`git: '${args[0] ?? ""}' is not a git command in this demo. Try git log or git status.\r\n`);
        return;
      case "systemctl":
        if (args[0] === "status" && (args[1] ?? "api").replace(/\.service$/, "") === "api")
          lines([
            `${G}●${X} api.service - Tokyo API (axum)`,
            "     Loaded: loaded (/etc/systemd/system/api.service; enabled)",
            `     Active: ${G}active (running)${X} since Wed 2026-10-07 22:14:03 JST; 11h ago`,
            "   Main PID: 48211 (api)",
            "     Memory: 212.4M",
          ]);
        else this.write(`Unit ${args[1] ?? ""}.service could not be found.\r\n`);
        return;
      case "help":
        this.write(`This demo shell answers: ${COMMANDS.join(", ")}.\r\n`);
        return;
      case "curl":
        this.write(
          [
            `HTTP/1.1 ${G}200 OK${X}`,
            "Server: nginx/1.24.0 (Ubuntu)",
            "Date: Thu, 08 Oct 2026 00:42:07 GMT",
            "Content-Type: application/json",
            "Content-Length: 2817",
            "",
          ].join("\r\n"),
        );
        return;
      case "cjk":
        this.write("中文宽字符测试：波止場 · 日本語テスト · 한국어\r\n");
        return;
      default:
        this.write(`${name}: command not found\r\n${D}This demo shell knows only a few commands; type help to list them.${X}\r\n`);
    }
  }

  stop() {
    this.stopped = true;
  }
}
