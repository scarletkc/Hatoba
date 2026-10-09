import type { HostView } from "../types";

const G = "\x1b[38;2;139;212;156m"; // design green  #8bd49c
const B = "\x1b[38;2;122;183;255m"; // design blue   #7ab7ff
const Y = "\x1b[38;2;229;192;123m"; // design yellow #e5c07b
const R = "\x1b[38;2;240;113;120m"; // design red    #f07178
const D = "\x1b[38;2;107;113;124m"; // design dim    #6b717c
const X = "\x1b[0m";

/** A tiny line-editing fake shell that replays the design's terminal screen (mock only). */
export class FakeShell {
  onExit: (() => void) | null = null;
  private line = "";
  private cwd = "~";
  private stopped = false;
  private enc = new TextEncoder();

  constructor(
    private host: HostView,
    private out: (bytes: Uint8Array) => void,
  ) {}

  private write(s: string) {
    if (!this.stopped) this.out(this.enc.encode(s));
  }

  private prompt() {
    this.write(`${G}${this.host.username}@${this.host.name}${X}:${B}${this.cwd}${X}$ `);
  }

  start() {
    this.write(`${D}Last login: Thu Oct  8 09:41:12 2026 from 10.0.0.4${X}\r\n`);
    if (this.host.name === "prod-api-tokyo") {
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
    const [name, ...args] = cmd.split(/\s+/);
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
        this.write(`${this.cwd === "~" ? `/home/${this.host.username}` : this.cwd}\r\n`);
        return;
      case "cd":
        this.cwd = args[0] ?? "~";
        return;
      case "echo":
        this.write(`${args.join(" ")}\r\n`);
        return;
      case "whoami":
        this.write(`${this.host.username}\r\n`);
        return;
      case "date":
        this.write(`${new Date().toString()}\r\n`);
        return;
      case "ls":
        this.write(`${B}migrations${X}  ${B}src${X}  ${B}target${X}  Cargo.lock  Cargo.toml  Dockerfile  README.md  wrangler.toml\r\n`);
        return;
      case "uname":
        this.write("Linux prod-api-tokyo 6.8.0-45-generic #45-Ubuntu SMP x86_64 GNU/Linux\r\n");
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
        this.write(`${name}: command not found\r\n`);
    }
  }

  stop() {
    this.stopped = true;
  }
}
