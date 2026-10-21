import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppError, HostView, TunnelLostEvent, TunnelsEvent } from "@/ipc/types";

const ipc = vi.hoisted(() => ({
  handlers: new Map<string, (payload: never) => void>(),
  start: vi.fn(),
  notify: vi.fn(async () => {}),
  list: { version: 0, tunnels: [] } as TunnelsEvent,
}));
const ui = vi.hoisted(() => ({
  focused: true,
  toast: vi.fn(),
  navigate: vi.fn(),
  activate: vi.fn(),
  credentials: vi.fn(),
  secret: vi.fn(),
  host: null as HostView | null,
}));

vi.mock("@/ipc/api", () => ({
  api: {
    listen: async (event: string, handler: (payload: never) => void) => {
      ipc.handlers.set(event, handler);
      return () => {};
    },
    tunnels_list: async () => ipc.list,
    tunnel_forward_start: ipc.start,
    tunnel_forward_stop: async () => {},
    notification_show: ipc.notify,
  },
  toAppError: (e: unknown) => e as AppError,
}));
vi.mock("@/lib/native", () => ({ windowFocused: async () => ui.focused }));
vi.mock("@/components/overlay", () => ({ toast: ui.toast }));
vi.mock("@/app/data", () => ({
  hostById: () => ui.host,
  useVaultData: { getState: () => ({ reloadHosts: async () => {} }) },
}));
vi.mock("@/app/store", () => ({ useApp: { getState: () => ({ phase: "unlocked", navigate: ui.navigate }) } }));
vi.mock("@/app/tabs", () => ({ useTabs: { getState: () => ({ activate: ui.activate }) } }));
vi.mock("@/features/terminal/connect", () => ({ collectCredentials: ui.credentials }));
vi.mock("@/features/terminal/prompts", () => ({ askSecret: ui.secret }));
vi.mock("@/features/terminal/session", () => ({
  needsPassphrase: (err: AppError) => err.code === "key_parse",
}));

import { getTunnels, startTunnelForward, startTunnelListeners, tunnelPort } from "./tunnels";

const emit = <T>(event: string, payload: T) => ipc.handlers.get(event)?.(payload as never);
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

const host = (auth_kind: HostView["auth_kind"]) =>
  ({ id: "h1", name: "db", address: "10.0.0.5", port: 22, username: "pg", auth_kind }) as HostView;

/** The backend's versions only grow, across the tests too. */
let version = 0;
const tunnels = (ports: number[], at = ++version): TunnelsEvent => ({
  version: at,
  tunnels: ports.length
    ? [{ host_id: "h1", host_name: "db", forwards: ports.map((p) => ({ forward_id: `f${p}`, kind: "local", port: p, address: `localhost:${p}`, dest: "127.0.0.1:5432" })) }]
    : [],
});
const current = () => getTunnels().find((x) => x.host_id === "h1");

beforeEach(async () => {
  ipc.start.mockReset();
  ipc.notify.mockClear();
  ui.toast.mockClear();
  ui.navigate.mockClear();
  ui.activate.mockClear();
  ui.credentials.mockReset();
  ui.secret.mockReset();
  ui.focused = true;
  ui.host = host("key");
  startTunnelListeners();
  await flush();
  emit("ssh://tunnels", tunnels([]));
});

describe("forwards running without a terminal (FWD-05)", () => {
  it("never lets an older list replace a newer one", () => {
    const older = ++version;
    emit("ssh://tunnels", tunnels([15432]));
    emit("ssh://tunnels", tunnels([], older));
    expect(tunnelPort(current(), "f15432")).toBe(15432);
    emit("ssh://tunnels", tunnels([]));
    expect(current()).toBeUndefined();
  });

  it("asks an ask-every-time host's password for its first forward only", async () => {
    ui.host = host("ask");
    ui.credentials.mockResolvedValue({ password: "pw", passphrase: null });
    ipc.start.mockResolvedValue(15432);
    expect(await startTunnelForward("h1", "f15432")).toBe(15432);
    expect(ui.credentials).toHaveBeenCalledTimes(1);
    expect(ipc.start).toHaveBeenLastCalledWith("h1", "f15432", { password: "pw", passphrase: null });

    emit("ssh://tunnels", tunnels([15432]));
    ipc.start.mockResolvedValue(3000);
    expect(await startTunnelForward("h1", "f3000")).toBe(3000);
    expect(ui.credentials).toHaveBeenCalledTimes(1);
    expect(ipc.start).toHaveBeenLastCalledWith("h1", "f3000", { password: null, passphrase: null });
  });

  it("starts nothing when the password question is cancelled", async () => {
    ui.host = host("ask");
    ui.credentials.mockResolvedValue(null);
    expect(await startTunnelForward("h1", "f15432")).toBeNull();
    expect(ipc.start).not.toHaveBeenCalled();
  });

  it("asks for a key passphrase the vault lacks and tries again with it", async () => {
    ui.credentials.mockResolvedValue({ password: null, passphrase: null });
    ipc.start
      .mockRejectedValueOnce({ code: "key_parse", key_kind: "passphrase_required", detail: "" })
      .mockResolvedValueOnce(15432);
    ui.secret.mockResolvedValue("secret");
    expect(await startTunnelForward("h1", "f15432")).toBe(15432);
    expect(ui.secret).toHaveBeenCalledWith(expect.objectContaining({ kind: "passphrase", hostName: "db", wrong: false }));
    expect(ipc.start).toHaveBeenLastCalledWith("h1", "f15432", { password: null, passphrase: "secret" });
  });

  it("rejects with the failure, which the host editor shows", async () => {
    ui.credentials.mockResolvedValue({ password: null, passphrase: null });
    const busy = { code: "ssh", ssh_kind: "io", detail: "listen on 127.0.0.1:15432: address in use" };
    ipc.start.mockRejectedValue(busy);
    await expect(startTunnelForward("h1", "f15432")).rejects.toBe(busy);
  });

  const lost: TunnelLostEvent = { host_id: "h1", host_name: "db", error: { code: "ssh", ssh_kind: "disconnected", detail: "connection closed" } };

  it("tells about a lost connection in a toast while the window has the focus", async () => {
    emit("ssh://tunnel-lost", lost);
    await flush();
    expect(ui.toast).toHaveBeenCalledWith("db: port forwarding stopped. The connection was closed.", "error");
    expect(ipc.notify).not.toHaveBeenCalled();
  });

  it("notifies when the window is in the background, and the click opens the host's forwards", async () => {
    ui.focused = false;
    emit("ssh://tunnel-lost", lost);
    await flush();
    expect(ui.toast).not.toHaveBeenCalled();
    expect(ipc.notify).toHaveBeenCalledWith({
      title: "db",
      body: "Port forwarding stopped. The connection was closed.",
      target: { tab: null, conversation_id: null, host_id: "h1" },
    });
    emit("app://notification-click", { tab: null, conversation_id: null, host_id: "h1" });
    expect(ui.navigate).toHaveBeenCalledWith({ kind: "forwards", hostId: "h1" });
    expect(ui.activate).toHaveBeenCalledWith("home");
  });

  it("leaves an AI notification's click to the assistant", () => {
    emit("app://notification-click", { tab: "home", conversation_id: "c1", host_id: null });
    expect(ui.navigate).not.toHaveBeenCalled();
  });
});
