import type { TelnetHostView } from "../types";

/**
 * A switch's console over Telnet (mock only): it asks for a user name and a password, which a
 * host with saved ones gets typed for it (TELNET-03), then answers a few IOS-style commands.
 */
export class FakeTelnet {
  onExit: (() => void) | null = null;
  private line = "";
  private stage: "user" | "password" | "exec" = "user";
  private enabled = false;
  private stopped = false;
  private enc = new TextEncoder();

  constructor(
    readonly host: TelnetHostView,
    private out: (bytes: Uint8Array) => void,
  ) {}

  private write(s: string) {
    if (!this.stopped) this.out(this.enc.encode(s));
  }

  private get hostname(): string {
    return this.host.name.replace(/[^\w-]/g, "") || "Switch";
  }

  private prompt() {
    this.write(`${this.hostname}${this.enabled ? "#" : ">"}`);
  }

  start() {
    this.write("\r\n\r\nUser Access Verification\r\n\r\nUsername: ");
    // What the client types for a saved login, as the device echoes it, unless the user typed first.
    if (this.host.username)
      setTimeout(() => {
        if (this.stage !== "user") return;
        this.write(`${this.host.username}\r\n`);
        this.askPassword();
      }, 200);
  }

  /** The password prompt, where the client types a saved password, also after a user name typed by hand. */
  private askPassword() {
    this.stage = "password";
    this.write("Password: ");
    if (this.host.has_password)
      setTimeout(() => {
        // The user is typing a password of their own.
        if (this.stage !== "password" || this.line) return;
        this.write("\r\n");
        this.stage = "exec";
        this.prompt();
      }, 150);
  }

  stop() {
    this.stopped = true;
  }

  input(data: string) {
    for (const ch of data) {
      if (ch === "\r") {
        const line = this.line;
        this.line = "";
        this.write("\r\n");
        this.enter(line.trim());
      } else if (ch === "\x7f") {
        if (this.line && this.stage !== "password") this.write("\b \b");
        this.line = this.line.slice(0, -1);
      } else if (ch >= " ") {
        this.line += ch;
        if (this.stage !== "password") this.write(ch);
      }
    }
  }

  private enter(line: string) {
    if (this.stage === "user") {
      this.askPassword();
      return;
    }
    if (this.stage === "password") {
      this.stage = "exec";
      this.prompt();
      return;
    }
    if (line === "exit" || line === "quit" || line === "logout") {
      this.stopped = true;
      this.onExit?.();
      return;
    }
    if (line === "enable") this.enabled = true;
    else if (line === "disable") this.enabled = false;
    else if (/^sh(ow)? ver/.test(line)) {
      this.write(
        [
          "Cisco IOS Software, C2960X Software (C2960X-UNIVERSALK9-M), Version 15.2(7)E8",
          `${this.hostname} uptime is 41 weeks, 3 days, 2 hours, 17 minutes`,
          'System image file is "flash:c2960x-universalk9-mz.152-7.E8.bin"',
          "",
        ].join("\r\n"),
      );
    } else if (/^sh(ow)? ip int(erface)? br/.test(line)) {
      this.write(
        [
          "Interface              IP-Address      OK? Method Status                Protocol",
          "Vlan1                  10.0.0.2        YES NVRAM  up                    up",
          "GigabitEthernet1/0/1   unassigned      YES unset  up                    up",
          "GigabitEthernet1/0/2   unassigned      YES unset  down                  down",
          "",
        ].join("\r\n"),
      );
    } else if (line) this.write(`% Invalid input detected at '^' marker.\r\n`);
    this.prompt();
  }
}
