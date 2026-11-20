import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useVaultData } from "@/app/data";
import { useApp } from "@/app/store";
import { useTabs } from "@/app/tabs";
import { appError } from "@/ipc/api";
import { FRAME_SESSION, type HostView, type TelnetHostView } from "@/ipc/types";

vi.hoisted(() => {
  Object.defineProperty(window, "matchMedia", {
    value: () => ({ matches: false, addEventListener() {} }),
    configurable: true,
  });
  class MockResizeObserver {
    observe = vi.fn();
    disconnect = vi.fn();
  }
  Object.defineProperty(globalThis, "ResizeObserver", {
    value: MockResizeObserver,
    configurable: true,
  });
});

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    element: HTMLElement | null = null;
    textarea: HTMLTextAreaElement | null = null;
    loadAddon = vi.fn();
    attachCustomKeyEventHandler = vi.fn();
    parser = { registerOscHandler: vi.fn() };
    onData = vi.fn();
    onBinary = vi.fn();
    onResize = vi.fn();
    onWriteParsed = vi.fn();
    onCursorMove = vi.fn();
    onScroll = vi.fn();
    open = vi.fn((el: HTMLElement) => {
      this.element = el;
    });
    write = vi.fn();
    paste = vi.fn();
    focus = vi.fn();
    dispose = vi.fn();
    options = {};
    modes = { mouseTrackingMode: "none", applicationCursorKeysMode: false };
    buffer = { active: { viewportY: 0, cursorX: 0, cursorY: 0, baseY: 0, type: "normal" } };
    scrollToLine = vi.fn();
    scrollToBottom = vi.fn();
  },
}));

vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    proposeDimensions = vi.fn(() => ({ cols: 80, rows: 24 }));
    fit = vi.fn();
  },
}));

vi.mock("@xterm/addon-webgl", () => ({
  WebglAddon: class {
    onContextLoss = vi.fn();
    dispose = vi.fn();
  },
}));

const toast = vi.hoisted(() => vi.fn());
vi.mock("@hatoba/ui", async (importActual) => ({
  ...(await importActual<typeof import("@hatoba/ui")>()),
  toast,
}));

const askPassword = vi.hoisted(() => vi.fn());
const askSecret = vi.hoisted(() => vi.fn());
const cancelSessionPrompts = vi.hoisted(() => vi.fn());
vi.mock("./prompts", async (importActual) => ({
  ...(await importActual<typeof import("./prompts")>()),
  askPassword,
  askSecret,
  cancelSessionPrompts,
}));

const mockedApi = vi.hoisted(() => ({
  ssh_connect: vi.fn(),
  ssh_connect_target: vi.fn(),
  ssh_disconnect: vi.fn(async () => {}),
  ssh_suggestions: vi.fn(async () => null),
  telnet_open: vi.fn(),
  telnet_open_target: vi.fn(),
  telnet_close: vi.fn(async () => {}),
  local_open: vi.fn(),
  local_close: vi.fn(async () => {}),
  hosts_list: vi.fn(async () => []),
  telnet_hosts_list: vi.fn(async () => []),
  recent_targets_list: vi.fn(async () => []),
  command_history_get: vi.fn(async () => []),
  listen: vi.fn(async () => () => {}),
}));

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

import { reconnectSession } from "./connect";
import { getInfo } from "./info";
import { ensureSession, getSession, LiveSession, routeStateEvent } from "./session";
import { useTransfers } from "./transfers";

function makeHost(overrides: Partial<HostView> = {}): HostView {
  return {
    id: "h1",
    name: "Prod",
    address: "10.0.0.1",
    port: 22,
    username: "ops",
    auth_kind: "password",
    has_password: true,
    key_id: null,
    jump_host_id: null,
    proxy_mode: "inherit",
    proxy_id: null,
    group_id: null,
    tags: [],
    notes: "",
    ai_notes: "",
    env: [],
    favorite: false,
    sort: 0,
    last_connected: null,
    os: null,
    show_stats: true,
    ...overrides,
  };
}

function makeTelnetHost(overrides: Partial<TelnetHostView> = {}): TelnetHostView {
  return {
    id: "t1",
    name: "Switch",
    address: "10.0.0.2",
    port: 23,
    username: "admin",
    has_password: false,
    proxy_mode: "inherit",
    proxy_id: null,
    group_id: null,
    tags: [],
    notes: "",
    favorite: false,
    sort: 0,
    last_connected: null,
    ...overrides,
  };
}

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (err: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (err: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function sessionFrame(sid: string): Uint8Array {
  const bytes = new TextEncoder().encode(sid);
  const frame = new Uint8Array(1 + bytes.length);
  frame[0] = FRAME_SESSION;
  frame.set(bytes, 1);
  return frame;
}

async function flush(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

describe("LiveSession connect and reconnect", () => {
  const sessions: LiveSession[] = [];
  let container: HTMLDivElement;

  const track = (s: LiveSession) => {
    sessions.push(s);
    return s;
  };

  beforeEach(() => {
    vi.resetAllMocks();
    const defaultHost = makeHost();
    const defaultTelnet = makeTelnetHost();
    mockedApi.ssh_disconnect.mockResolvedValue(undefined);
    mockedApi.ssh_suggestions.mockResolvedValue(null);
    mockedApi.telnet_close.mockResolvedValue(undefined);
    mockedApi.local_close.mockResolvedValue(undefined);
    mockedApi.hosts_list.mockResolvedValue([defaultHost]);
    mockedApi.telnet_hosts_list.mockResolvedValue([defaultTelnet]);
    mockedApi.recent_targets_list.mockResolvedValue([]);
    mockedApi.command_history_get.mockResolvedValue([]);
    mockedApi.listen.mockResolvedValue(() => {});
    useApp.setState({ phase: "unlocked" });
    useTabs.setState({ tabs: [], active: "home" });
    useVaultData.setState({ hosts: [defaultHost], telnetHosts: [defaultTelnet] });
    useTransfers.setState({ items: [], uploadsDone: {} });
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(async () => {
    while (sessions.length > 0) {
      await sessions.pop()!.dispose();
    }
    container.remove();
  });

  it("connects all five session sources through their backend commands", async () => {
    mockedApi.ssh_connect.mockResolvedValueOnce("ssh-1");
    const savedSsh = track(new LiveSession("tab-ssh", { hostId: "h1", target: null }));
    savedSsh.attach(container);
    await flush();
    expect(mockedApi.ssh_connect).toHaveBeenCalledWith(
      "h1",
      { cols: 80, rows: 24, password: null, passphrase: null },
      expect.any(Function),
    );
    expect(savedSsh.status).toBe("connected");
    expect(savedSsh.sessionId).toBe("ssh-1");

    const target = { address: "example.com", port: 2222, username: "alice" };
    mockedApi.ssh_connect_target.mockResolvedValueOnce("ssh-q1");
    const quickSsh = track(new LiveSession("tab-qssh", { hostId: null, target }));
    quickSsh.attach(container);
    await flush();
    expect(mockedApi.ssh_connect_target).toHaveBeenCalledWith(
      target,
      { cols: 80, rows: 24, password: null, passphrase: null },
      expect.any(Function),
    );
    expect(quickSsh.status).toBe("connected");

    mockedApi.telnet_open.mockResolvedValueOnce("tel-1");
    const savedTelnet = track(new LiveSession("tab-tel", { hostId: null, target: null, telnet: { hostId: "t1" } }));
    savedTelnet.attach(container);
    await flush();
    expect(mockedApi.telnet_open).toHaveBeenCalledWith("t1", { cols: 80, rows: 24 }, expect.any(Function));
    expect(savedTelnet.status).toBe("connected");

    mockedApi.telnet_open_target.mockResolvedValueOnce("tel-q1");
    const quickTelnet = track(
      new LiveSession("tab-qtel", {
        hostId: null,
        target: null,
        telnet: { hostId: null, address: "10.0.0.9", port: 2323 },
      }),
    );
    quickTelnet.attach(container);
    await flush();
    expect(mockedApi.telnet_open_target).toHaveBeenCalledWith(
      "10.0.0.9",
      2323,
      { cols: 80, rows: 24 },
      expect.any(Function),
    );
    expect(quickTelnet.status).toBe("connected");

    mockedApi.local_open.mockResolvedValueOnce("loc-1");
    const local = track(
      new LiveSession("tab-loc", {
        hostId: null,
        target: null,
        local: { id: "bash", name: "Bash", kind: "bash" },
      }),
    );
    local.attach(container);
    await flush();
    expect(mockedApi.local_open).toHaveBeenCalledWith("bash", { cols: 80, rows: 24 }, expect.any(Function));
    expect(local.status).toBe("connected");
  });

  it("retries SSH password lazily for ask-every-time hosts and fails if cancelled", async () => {
    const askHost = makeHost({ id: "h-ask", auth_kind: "ask", has_password: false });
    useVaultData.setState({ hosts: [askHost] });
    const pwErr = appError("invalid_input", "password required", { field: "password" });

    mockedApi.ssh_connect.mockRejectedValueOnce(pwErr).mockResolvedValueOnce("ssh-pw");
    askPassword.mockResolvedValueOnce("secret-pw");

    const s1 = track(new LiveSession("tab-pw-ok", { hostId: "h-ask", target: null }));
    s1.attach(container);
    await flush();
    expect(askPassword).toHaveBeenCalledWith(askHost);
    expect(mockedApi.ssh_connect).toHaveBeenNthCalledWith(
      2,
      "h-ask",
      { cols: 80, rows: 24, password: "secret-pw", passphrase: null },
      expect.any(Function),
    );
    expect(s1.status).toBe("connected");

    mockedApi.ssh_connect.mockRejectedValueOnce(pwErr);
    askPassword.mockResolvedValueOnce(null);
    const s2 = track(new LiveSession("tab-pw-cancel", { hostId: "h-ask", target: null }));
    s2.attach(container);
    await flush();
    expect(s2.status).toBe("failed");
    expect(getInfo("tab-pw-cancel").error?.code).toBe("invalid_input");
  });

  it("retries key passphrase prompts with wrong flag up to three rounds", async () => {
    const reqErr = appError("key_parse", "passphrase required", { key_kind: "passphrase_required" });
    const wrongErr = appError("key_parse", "wrong passphrase", { key_kind: "wrong_passphrase" });

    mockedApi.ssh_connect
      .mockRejectedValueOnce(reqErr)
      .mockRejectedValueOnce(wrongErr)
      .mockResolvedValueOnce("ssh-key");
    askSecret.mockResolvedValueOnce("bad").mockResolvedValueOnce("good");

    const s1 = track(new LiveSession("tab-key-ok", { hostId: "h1", target: null }));
    s1.attach(container);
    await flush();
    expect(askSecret).toHaveBeenNthCalledWith(1, {
      kind: "passphrase",
      hostName: "Prod",
      target: "ops@10.0.0.1:22",
      wrong: false,
    });
    expect(askSecret).toHaveBeenNthCalledWith(2, {
      kind: "passphrase",
      hostName: "Prod",
      target: "ops@10.0.0.1:22",
      wrong: true,
    });
    expect(s1.status).toBe("connected");

    // Exhausting round < 3 fails without a fourth prompt.
    mockedApi.ssh_connect
      .mockRejectedValueOnce(reqErr)
      .mockRejectedValueOnce(wrongErr)
      .mockRejectedValueOnce(wrongErr)
      .mockRejectedValueOnce(wrongErr);
    askSecret.mockResolvedValue("still-wrong");

    const s2 = track(new LiveSession("tab-key-limit", { hostId: "h1", target: null }));
    s2.attach(container);
    await flush();
    expect(askSecret).toHaveBeenCalledTimes(5); // 2 from s1 + 3 from s2
    expect(s2.status).toBe("failed");
  });

  it("tracks FRAME_SESSION while connecting and replays early state events on connect", async () => {
    const pending = deferred<string>();
    let frameHandler: ((frame: Uint8Array) => void) | null = null;
    mockedApi.ssh_connect.mockImplementationOnce((_id, _opts, onFrame) => {
      frameHandler = onFrame;
      return pending.promise;
    });

    const s = track(new LiveSession("tab-early", { hostId: "h1", target: null }));
    s.attach(container);
    expect(s.connectingAs("sid-early")).toBe(false);

    frameHandler!(sessionFrame("sid-early"));
    expect(s.connectingAs("sid-early")).toBe(true);

    routeStateEvent({ session_id: "sid-early", state: "connected", latency_ms: 42, exit_status: null, error: null });
    routeStateEvent({
      session_id: "sid-early",
      state: "disconnected",
      latency_ms: null,
      exit_status: 1,
      error: null,
    });

    pending.resolve("sid-early");
    await flush();

    expect(getInfo("tab-early").latencyMs).toBe(42);
    expect(s.status).toBe("disconnected");
  });

  it("closes orphaned sessions when connect resolves after disposal", async () => {
    const sshPending = deferred<string>();
    mockedApi.ssh_connect.mockReturnValueOnce(sshPending.promise);
    const ssh = track(new LiveSession("tab-stale-ssh", { hostId: "h1", target: null }));
    ssh.attach(container);
    await ssh.dispose();
    sshPending.resolve("orphan-ssh");
    await flush();
    expect(mockedApi.ssh_disconnect).toHaveBeenCalledWith("orphan-ssh");

    const telnetPending = deferred<string>();
    mockedApi.telnet_open.mockReturnValueOnce(telnetPending.promise);
    const telnet = track(new LiveSession("tab-stale-tel", { hostId: null, target: null, telnet: { hostId: "t1" } }));
    telnet.attach(container);
    await telnet.dispose();
    telnetPending.resolve("orphan-tel");
    await flush();
    expect(mockedApi.telnet_close).toHaveBeenCalledWith("orphan-tel");

    const localPending = deferred<string>();
    mockedApi.local_open.mockReturnValueOnce(localPending.promise);
    const local = track(
      new LiveSession("tab-stale-loc", {
        hostId: null,
        target: null,
        local: { id: "bash", name: "Bash", kind: "bash" },
      }),
    );
    local.attach(container);
    await local.dispose();
    localPending.resolve("orphan-loc");
    await flush();
    expect(mockedApi.local_close).toHaveBeenCalledWith("orphan-loc");
  });

  it("reconnect closes old session, purges transfers, respects guards and missing hosts", async () => {
    mockedApi.ssh_connect.mockResolvedValueOnce("sid-1").mockResolvedValueOnce("sid-2");
    const s = track(new LiveSession("tab-reconn", { hostId: "h1", target: null }));
    s.attach(container);
    await flush();
    expect(s.sessionId).toBe("sid-1");

    useTransfers.setState({
      items: [
        {
          transfer_id: "tr-1",
          session_id: "sid-1",
          direction: "upload",
          name: "a.txt",
          transferred: 1,
          total: 2,
          state: "running",
          error: null,
          fading: false,
        },
      ],
    });

    // Cancelled credential collection leaves the existing session untouched.
    await s.reconnect(async () => null);
    expect(mockedApi.ssh_disconnect).not.toHaveBeenCalled();
    expect(s.sessionId).toBe("sid-1");

    await s.reconnect(async () => ({ password: "next", passphrase: null }));
    expect(mockedApi.ssh_disconnect).toHaveBeenCalledWith("sid-1");
    expect(useTransfers.getState().items).toEqual([]);
    expect(s.sessionId).toBe("sid-2");
    expect(mockedApi.ssh_connect).toHaveBeenLastCalledWith(
      "h1",
      { cols: 80, rows: 24, password: "next", passphrase: null },
      expect.any(Function),
    );
    expect(s.term.write).toHaveBeenCalledWith(expect.stringContaining("── "));

    // Deleted saved host toasts an error instead of connecting.
    useVaultData.setState({ hosts: [], telnetHosts: [] });
    await s.reconnect(async () => ({ password: null, passphrase: null }));
    expect(toast).toHaveBeenCalledWith(expect.any(String), "error");
    expect(mockedApi.ssh_connect).toHaveBeenCalledTimes(2);
  });

  it("reconnectSession adopts a saved host created from a quick-connect target", async () => {
    const target = { address: "10.0.0.1", port: 22, username: "ops" };
    const tabId = useTabs.getState().openSession({ hostId: null, target }, "ops@10.0.0.1");
    const s = track(ensureSession(tabId)!);

    mockedApi.ssh_connect_target.mockResolvedValueOnce("sid-quick");
    s.attach(container);
    await flush();
    expect(s.target).toEqual(target);

    mockedApi.ssh_connect.mockResolvedValueOnce("sid-saved");
    await reconnectSession(tabId);
    await flush();

    expect(s.hostId).toBe("h1");
    expect(s.target).toBeNull();
    expect(mockedApi.ssh_connect).toHaveBeenCalledWith(
      "h1",
      { cols: 80, rows: 24, password: null, passphrase: null },
      expect.any(Function),
    );
    expect(getSession(tabId)?.sessionId).toBe("sid-saved");
  });
});
