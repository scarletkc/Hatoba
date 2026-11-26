import { beforeEach, describe, expect, it, vi } from "vitest";
import type { HostView, TelnetHostView } from "@/ipc/types";

const mockedApi = vi.hoisted(() => ({
  host_set_favorite: vi.fn(async () => {}),
  telnet_host_set_favorite: vi.fn(async () => {}),
  host_duplicate: vi.fn(),
  telnet_host_duplicate: vi.fn(),
  host_delete: vi.fn(async () => {}),
  telnet_host_delete: vi.fn(async () => {}),
  host_copy_password: vi.fn(async () => {}),
  telnet_host_copy_password: vi.fn(async () => {}),
}));

const connectHost = vi.hoisted(() => vi.fn(async () => {}));
const connectTelnetHost = vi.hoisted(() => vi.fn());

vi.mock("@/ipc/api", async (importActual) => {
  const actual = await importActual<typeof import("@/ipc/api")>();
  return {
    ...actual,
    api: new Proxy(actual.api, {
      get: (target, prop) =>
        prop in mockedApi ? mockedApi[prop as keyof typeof mockedApi] : Reflect.get(target, prop),
    }),
  };
});

vi.mock("@/session/connect", () => ({ connectHost, connectTelnetHost }));

import { hostApi } from "./hostApi";

function sshHost(overrides: Partial<HostView> = {}): HostView {
  return {
    id: "ssh-1",
    name: "Prod",
    address: "10.0.0.1",
    port: 22,
    username: "ops",
    auth_kind: "password",
    has_password: true,
    key_id: null,
    group_id: null,
    tags: [],
    favorite: false,
    sort: 0,
    jump_host_id: null,
    proxy_mode: "device_default",
    proxy_id: null,
    note: "",
    ai_notes: "",
    updated_at: 0,
    last_connected_at: null,
    os: null,
    show_stats: false,
    dashboard: true,
    env: [],
    ...overrides,
  };
}

function telnetHost(overrides: Partial<TelnetHostView> = {}): TelnetHostView {
  return {
    id: "tel-1",
    name: "Core Switch",
    address: "10.0.0.2",
    port: 23,
    username: "admin",
    has_password: true,
    group_id: null,
    tags: [],
    favorite: false,
    sort: 0,
    proxy_mode: "device_default",
    proxy_id: null,
    note: "",
    ai_notes: "",
    updated_at: 0,
    last_connected_at: null,
    ...overrides,
  };
}

describe("hostApi", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("dispatches SSH host actions and formats SSH address strings", async () => {
    const h = sshHost();
    const ops = hostApi(h);
    expect(ops.editRoute("g1")).toEqual({ kind: "host-edit", hostId: "ssh-1", groupId: null, back: "g1" });
    expect(ops.addressText()).toBe("ops@10.0.0.1");
    expect(hostApi(sshHost({ port: 2222, username: "" })).addressText()).toBe("10.0.0.1 -p 2222");

    await ops.connect();
    await ops.setFavorite(true);
    await ops.duplicate();
    await ops.remove();
    await ops.copyPassword();

    expect(connectHost).toHaveBeenCalledWith("ssh-1");
    expect(mockedApi.host_set_favorite).toHaveBeenCalledWith("ssh-1", true);
    expect(mockedApi.host_duplicate).toHaveBeenCalledWith("ssh-1");
    expect(mockedApi.host_delete).toHaveBeenCalledWith("ssh-1");
    expect(mockedApi.host_copy_password).toHaveBeenCalledWith("ssh-1");
  });

  it("dispatches Telnet host actions and formats Telnet address strings", async () => {
    const h = telnetHost();
    const ops = hostApi(h);
    expect(ops.editRoute(null)).toEqual({ kind: "telnet-edit", hostId: "tel-1", groupId: null, back: null });
    expect(ops.addressText()).toBe("10.0.0.2");
    expect(hostApi(telnetHost({ port: 2323 })).addressText()).toBe("10.0.0.2 2323");

    await ops.connect();
    await ops.setFavorite(false);
    await ops.duplicate();
    await ops.remove();
    await ops.copyPassword();

    expect(connectTelnetHost).toHaveBeenCalledWith("tel-1");
    expect(mockedApi.telnet_host_set_favorite).toHaveBeenCalledWith("tel-1", false);
    expect(mockedApi.telnet_host_duplicate).toHaveBeenCalledWith("tel-1");
    expect(mockedApi.telnet_host_delete).toHaveBeenCalledWith("tel-1");
    expect(mockedApi.telnet_host_copy_password).toHaveBeenCalledWith("tel-1");
  });
});
