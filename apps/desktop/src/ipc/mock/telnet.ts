import { byteCount, HOST_NOTES_MAX_BYTES } from "@/lib/ai/instructions";
import type { FrameHandler, HatobaApi } from "../api";
import { FRAME_CLOSED, FRAME_DATA, type TelnetHostView } from "../types";
import * as D from "./data";
import { delay, fail, frame, type MockDeps } from "./shared";

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

type TelnetApi = Pick<
  HatobaApi,
  | "telnet_hosts_list"
  | "telnet_host_save"
  | "telnet_host_delete"
  | "telnet_host_duplicate"
  | "telnet_host_set_favorite"
  | "telnet_host_copy_password"
  | "telnet_open_target"
  | "telnet_open"
  | "telnet_write"
  | "telnet_write_bytes"
  | "telnet_resize"
  | "telnet_close"
>;

export function createTelnetMock(ctx: MockDeps): TelnetApi {
  const { state } = ctx;
  const telnets = new Map<string, FakeTelnet>();

  /** TELNET-02 in the mock: a fake switch console; a host refused by the probe refuses the connection too. */
  async function openTelnet(host: TelnetHostView, hostId: string | null, onFrame: FrameHandler): Promise<string> {
    ctx.needUnlocked();
    await delay(300);
    // The vault may have locked while connecting.
    ctx.needUnlocked();
    if (hostId && D.PROBE[hostId] === false)
      fail("ssh", `connect to ${host.address}:${host.port}: connection refused`, { ssh_kind: "refused" });
    const sid = ctx.id("t");
    const fake = new FakeTelnet(host, (bytes) => onFrame(frame(FRAME_DATA, bytes)));
    telnets.set(sid, fake);
    if (hostId)
      state.telnetHosts = state.telnetHosts.map((h) => (h.id === hostId ? { ...h, last_connected_at: Date.now() } : h));
    setTimeout(() => {
      ctx.emit("ssh://state", {
        session_id: sid,
        host_id: hostId,
        state: "connected",
        latency_ms: null,
        error: null,
        exit_status: null,
      });
      fake.start();
    }, 0);
    fake.onExit = () => {
      onFrame(frame(FRAME_CLOSED, new Uint8Array()));
      ctx.emit("ssh://state", {
        session_id: sid,
        host_id: hostId,
        state: "disconnected",
        latency_ms: null,
        error: null,
        exit_status: null,
      });
      telnets.delete(sid);
    };
    return sid;
  }

  return {
    telnet_hosts_list: async () => {
      ctx.needUnlocked();
      return state.telnetHosts.map((h) => ({ ...h }));
    },
    telnet_host_save: async (input) => {
      ctx.needUnlocked();
      if (!input.address.trim()) fail("invalid_input", "address is required", { field: "address" });
      if (byteCount(input.ai_notes) > HOST_NOTES_MAX_BYTES)
        fail("invalid_input", "AI notes are limited to 16 KB", { field: "ai_notes" });
      if (input.proxy_mode === "proxy" && !state.proxies.some((p) => p.id === input.proxy_id))
        fail("invalid_input", "proxy not found", { field: "proxy_id" });
      const existing = input.id ? state.telnetHosts.find((h) => h.id === input.id) : undefined;
      const view: TelnetHostView = {
        id: existing?.id ?? ctx.id("t"),
        name: input.name.trim() || Array.from(input.address.trim()).slice(0, 128).join(""),
        address: input.address.trim(),
        port: input.port || 23,
        username: input.username.trim(),
        has_password: input.password !== null ? input.password.length > 0 : !!existing?.has_password,
        group_id: input.group_id,
        tags: input.tags,
        favorite: input.favorite,
        sort: existing?.sort ?? ctx.placeNewHost(),
        proxy_mode: input.proxy_mode,
        proxy_id: input.proxy_mode === "proxy" ? input.proxy_id : null,
        note: input.note,
        ai_notes: input.ai_notes,
        updated_at: Date.now(),
        last_connected_at: existing?.last_connected_at ?? null,
      };
      state.telnetHosts = existing
        ? state.telnetHosts.map((h) => (h.id === view.id ? view : h))
        : [...state.telnetHosts, view];
      ctx.touch();
      return view;
    },
    telnet_host_delete: async (hid) => {
      state.telnetHosts = state.telnetHosts.filter((h) => h.id !== hid);
      ctx.touch();
    },
    telnet_host_duplicate: async (hid) => {
      const h = state.telnetHosts.find((x) => x.id === hid) ?? fail("not_found");
      const copy = {
        ...h,
        id: ctx.id("t"),
        name: `${h.name}-copy`,
        favorite: false,
        sort: ctx.placeNewHost(hid),
        last_connected_at: null,
      };
      state.telnetHosts = [...state.telnetHosts, copy];
      ctx.touch();
      return copy;
    },
    telnet_host_set_favorite: async (hid, favorite) => {
      state.telnetHosts = state.telnetHosts.map((h) => (h.id === hid ? { ...h, favorite } : h));
      ctx.touch();
    },
    telnet_host_copy_password: async () => {},
    telnet_open_target: async (address, port, _options, onFrame) => {
      const host: TelnetHostView = {
        ...D.TELNET_HOSTS[1],
        id: "",
        name: address,
        address,
        port,
        username: "",
        has_password: false,
      };
      return openTelnet(host, null, onFrame);
    },
    telnet_open: async (hostId, _options, onFrame) => {
      ctx.needUnlocked();
      const host = state.telnetHosts.find((h) => h.id === hostId) ?? fail("not_found", "host not found");
      return openTelnet(host, hostId, onFrame);
    },
    telnet_write: async (sid, data) => {
      telnets.get(sid)?.input(data);
    },
    telnet_write_bytes: async () => {},
    telnet_resize: async () => {},
    telnet_close: async (sid) => {
      telnets.get(sid)?.stop();
      telnets.delete(sid);
    },
  };
}
