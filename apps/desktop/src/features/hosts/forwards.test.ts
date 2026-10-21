import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppError, ConnectionsEvent, ForwardsLostEvent, ForwardView, HostView } from "@/ipc/types";

const ipc = vi.hoisted(() => ({
  handlers: new Map<string, (payload: never) => void>(),
  start: vi.fn(),
  notify: vi.fn(async () => {}),
  rules: vi.fn(),
  list: { version: 0, connections: [] } as ConnectionsEvent,
}));
const ui = vi.hoisted(() => ({
  focused: true,
  toast: vi.fn(),
  navigate: vi.fn(),
  activate: vi.fn(),
  credentials: vi.fn(),
  password: vi.fn(),
  secret: vi.fn(),
  host: null as HostView | null,
}));

vi.mock("@/ipc/api", () => ({
  api: {
    listen: async (event: string, handler: (payload: never) => void) => {
      ipc.handlers.set(event, handler);
      return () => {};
    },
    connections_list: async () => ipc.list,
    forwards_list_all: ipc.rules,
    forward_start: ipc.start,
    forward_stop: async () => {},
    notification_show: ipc.notify,
  },
  toAppError: (e: unknown) => e as AppError,
}));
vi.mock("@/lib/native", () => ({ windowFocused: async () => ui.focused }));
vi.mock("@/components/overlay", () => ({ toast: ui.toast }));
vi.mock("@/app/data", () => ({
  hostById: () => ui.host,
  useVaultData: { getState: () => ({ reloadHosts: async () => {} }), subscribe: () => () => {} },
}));
vi.mock("@/app/store", () => ({ useApp: { getState: () => ({ phase: "unlocked", navigate: ui.navigate }) } }));
vi.mock("@/app/tabs", () => ({ useTabs: { getState: () => ({ activate: ui.activate }) } }));
vi.mock("@/features/terminal/connect", () => ({ collectCredentials: ui.credentials }));
vi.mock("@/features/terminal/prompts", () => ({ askPassword: ui.password, askSecret: ui.secret }));
vi.mock("@/features/terminal/session", () => ({
  needsPassphrase: (err: AppError) => err.code === "key_parse",
  needsPassword: (err: AppError) => err.code === "invalid_input" && err.field === "password",
}));

import { forwardFailure, getConnections, hostConnected, runningForward } from "./connections";
import { getForwardRules, loadForwardRules, putForwardRule, startForward, startForwardListeners } from "./forwards";

const emit = <T>(event: string, payload: T) => ipc.handlers.get(event)?.(payload as never);
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

const host = (auth_kind: HostView["auth_kind"]) =>
  ({ id: "h1", name: "db", address: "10.0.0.5", port: 22, username: "pg", auth_kind }) as HostView;

/** The backend's versions only grow, across the tests too. */
let version = 0;
const connected = (ports: number[] | null, at = ++version): ConnectionsEvent => ({
  version: at,
  connections: ports
    ? [
        {
          host_id: "h1",
          host_name: "db",
          forwards: ports.map((p) => ({ forward_id: `f${p}`, kind: "local", port: p, address: `localhost:${p}`, dest: "127.0.0.1:5432" })),
          failed: [],
        },
      ]
    : [],
});
const current = () => getConnections().find((x) => x.host_id === "h1");

const rule = (id: string, bind_port: number): ForwardView => ({
  id,
  host_id: "h1",
  kind: "local",
  bind_address: "127.0.0.1",
  bind_port,
  dest_host: "127.0.0.1",
  dest_port: 5432,
  auto_start: false,
});

beforeEach(async () => {
  ipc.start.mockReset();
  ipc.notify.mockClear();
  ipc.rules.mockReset();
  ipc.rules.mockResolvedValue([]);
  ui.toast.mockClear();
  ui.navigate.mockClear();
  ui.activate.mockClear();
  ui.credentials.mockReset();
  ui.credentials.mockResolvedValue({ password: null, passphrase: null });
  ui.password.mockReset();
  ui.secret.mockReset();
  ui.focused = true;
  ui.host = host("key");
  startForwardListeners();
  await flush();
  emit("ssh://connections", connected(null));
});

describe("the hosts' shared connections (SSH-16)", () => {
  it("never lets an older list replace a newer one", () => {
    const older = ++version;
    emit("ssh://connections", connected([15432]));
    emit("ssh://connections", connected(null, older));
    expect(runningForward(current(), "f15432")?.address).toBe("localhost:15432");
    emit("ssh://connections", connected(null));
    expect(current()).toBeUndefined();
  });

  it("counts a connection with no forwards as connected", () => {
    expect(hostConnected("h1")).toBe(false);
    emit("ssh://connections", connected([]));
    expect(hostConnected("h1")).toBe(true);
    expect(runningForward(current(), "f15432")).toBeNull();
  });

  it("keeps why a forward failed to start on the connection", () => {
    const busy: AppError = { code: "ssh", ssh_kind: "io", detail: "address in use" };
    emit("ssh://connections", { version: ++version, connections: [{ host_id: "h1", host_name: "db", forwards: [], failed: [{ forward_id: "f1", error: busy }] }] });
    expect(forwardFailure(current(), "f1")).toEqual(busy);
    expect(forwardFailure(current(), "f2")).toBeNull();
  });
});

describe("starting a forward (FWD-05)", () => {
  it("signs in with what the host's credentials ask for", async () => {
    ui.host = host("ask");
    ui.credentials.mockResolvedValue({ password: "pw", passphrase: null });
    ipc.start.mockResolvedValue(15432);
    expect(await startForward("h1", "f15432")).toBe(15432);
    expect(ipc.start).toHaveBeenLastCalledWith("h1", "f15432", { password: "pw", passphrase: null });
  });

  it("starts nothing when the password question is cancelled", async () => {
    ui.host = host("ask");
    ui.credentials.mockResolvedValue(null);
    expect(await startForward("h1", "f15432")).toBeNull();
    expect(ipc.start).not.toHaveBeenCalled();
  });

  it("asks for the password when the host's connection closed after it was skipped", async () => {
    ui.host = host("ask");
    ipc.start
      .mockRejectedValueOnce({ code: "invalid_input", field: "password", detail: "a password is required for this host" })
      .mockResolvedValueOnce(15432);
    ui.password.mockResolvedValue("pw");
    expect(await startForward("h1", "f15432")).toBe(15432);
    expect(ipc.start).toHaveBeenLastCalledWith("h1", "f15432", { password: "pw", passphrase: null });
  });

  it("asks for a key passphrase the vault lacks and tries again with it", async () => {
    ipc.start
      .mockRejectedValueOnce({ code: "key_parse", key_kind: "passphrase_required", detail: "" })
      .mockResolvedValueOnce(15432);
    ui.secret.mockResolvedValue("secret");
    expect(await startForward("h1", "f15432")).toBe(15432);
    expect(ui.secret).toHaveBeenCalledWith(expect.objectContaining({ kind: "passphrase", hostName: "db", wrong: false }));
    expect(ipc.start).toHaveBeenLastCalledWith("h1", "f15432", { password: null, passphrase: "secret" });
  });

  it("rejects with the failure, which the page shows", async () => {
    const busy = { code: "ssh", ssh_kind: "io", detail: "listen on 127.0.0.1:15432: address in use" };
    ipc.start.mockRejectedValue(busy);
    await expect(startForward("h1", "f15432")).rejects.toBe(busy);
  });
});

describe("the saved forwards", () => {
  it("keeps a save that lands while an earlier read is on its way", async () => {
    let answer: (rules: ForwardView[]) => void = () => {};
    ipc.rules.mockReturnValueOnce(new Promise<ForwardView[]>((resolve) => (answer = resolve)));
    const reading = loadForwardRules();
    putForwardRule(rule("f1", 15432));
    answer([]);
    await reading;
    expect(getForwardRules()).toEqual([rule("f1", 15432)]);
  });
});

describe("a lost connection", () => {
  const lost: ForwardsLostEvent = { host_id: "h1", host_name: "db", error: { code: "ssh", ssh_kind: "disconnected", detail: "connection closed" } };

  it("is told in a toast while the window has the focus", async () => {
    emit("ssh://forwards-lost", lost);
    await flush();
    expect(ui.toast).toHaveBeenCalledWith("db: port forwarding stopped. The connection was closed.", "error");
    expect(ipc.notify).not.toHaveBeenCalled();
  });

  it("notifies when the window is in the background, and the click opens the host's forwards", async () => {
    ui.focused = false;
    emit("ssh://forwards-lost", lost);
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
