import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type NativeDrop = (event: { payload: { type: "drop"; paths: string[]; position: { x: number; y: number } } }) => void;

const backend = vi.hoisted(() => ({
  locked: false,
  homes: 0,
  lists: [] as string[],
  uploads: [] as string[],
  tauri: false,
  nativeDrop: null as NativeDrop | null,
}));

vi.mock("@/ipc/api", async (importActual) => {
  const actual = await importActual<typeof import("@/ipc/api")>();
  const calls: Record<string, (...args: never[]) => Promise<unknown>> = {
    // Rust refuses every SFTP command behind the lock screen (SEC-03).
    sftp_home: async () => {
      backend.homes++;
      if (backend.locked) throw actual.appError("locked", "the vault is locked");
      return "/home/me";
    },
    sftp_list: async (_session: string, dir: string) => {
      backend.lists.push(dir);
      if (backend.locked) throw actual.appError("locked", "the vault is locked");
      const src = { name: "src", path: "/home/me/src", is_dir: true, is_symlink: false, size: 0 };
      return dir === "/home/me" ? [{ ...src, modified: null, permissions: "drwxr-xr-x" }] : [];
    },
    sftp_upload: async (_session: string, path: string) => {
      backend.uploads.push(path);
      return "transfer";
    },
  };
  return {
    ...actual,
    isTauri: () => backend.tauri,
    api: new Proxy(calls, { get: (target, name: string) => target[name] ?? (async () => () => {}) }),
  };
});

// In the app, the webview hands over the paths of a drop anywhere on the window.
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: async (handler: NativeDrop) => {
      backend.nativeDrop = handler;
      return () => {
        if (backend.nativeDrop === handler) backend.nativeDrop = null;
      };
    },
  }),
}));
// jsdom lays nothing out, so every drop counts as one on the panel.
vi.mock("@/lib/dropPoint", () => ({ isDropInside: () => true }));

// Without Tauri, drops come as HTML5 events. The app store follows the system theme from its
// first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { useApp } = await import("@/app/store");
const { t } = await import("@/i18n");
const { SftpPanel } = await import("./SftpPanel");

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
beforeEach(() => {
  backend.locked = false;
  backend.homes = 0;
  backend.lists = [];
  backend.tauri = false;
  backend.nativeDrop = null;
  backend.uploads = [];
  useApp.setState({ phase: "unlocked" });
});
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
});

async function render() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  await act(async () => root.render(<SftpPanel tabId="tab" sessionId="s1" hostName="web" active />));
  unmount = () => {
    root.unmount();
    host.remove();
  };
  return host;
}

async function setLocked(locked: boolean) {
  backend.locked = locked;
  await act(async () => useApp.setState({ phase: locked ? "locked" : "unlocked" }));
}

async function drop(host: HTMLElement, name: string) {
  const event = new Event("drop", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "dataTransfer", { value: { files: [{ name }], types: ["Files"] } });
  await act(async () => void host.firstElementChild!.dispatchEvent(event));
}

describe("SftpPanel behind the lock screen", () => {
  it("takes no file drops", async () => {
    const host = await render();
    await setLocked(true);
    await drop(host, "locked.sh");
    expect(backend.uploads).toEqual([]);

    await setLocked(false);
    await drop(host, "unlocked.sh");
    expect(backend.uploads).toEqual(["unlocked.sh"]);
  });

  it("takes no native file drops in the app", async () => {
    backend.tauri = true;
    await render();
    await vi.waitFor(() => expect(backend.nativeDrop).not.toBeNull());
    await setLocked(true);
    await vi.waitFor(() => expect(backend.nativeDrop).toBeNull());

    await setLocked(false);
    await vi.waitFor(() => expect(backend.nativeDrop).not.toBeNull());
    const payload = { type: "drop" as const, paths: ["/tmp/unlocked.sh"], position: { x: 1, y: 1 } };
    await act(async () => backend.nativeDrop!({ payload }));
    expect(backend.uploads).toEqual(["/tmp/unlocked.sh"]);
  });

  it("loads the folder the lock refused after the unlock", async () => {
    await setLocked(true);
    const host = await render();
    expect(host.textContent).toContain(t("sftp.loadFailed"));

    await setLocked(false);
    expect(backend.homes).toBe(2);
    expect(host.textContent).not.toContain(t("sftp.loadFailed"));
  });

  it("opens the folder the lock refused after the unlock", async () => {
    const host = await render();
    await setLocked(true);
    const src = [...host.querySelectorAll("*")].find((el) => el.textContent === "src")!;
    await act(async () => void src.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    expect(host.textContent).toContain(t("sftp.loadFailed"));

    await setLocked(false);
    expect(backend.lists.at(-1)).toBe("/home/me/src");
    expect(host.textContent).not.toContain(t("sftp.loadFailed"));
  });
});
