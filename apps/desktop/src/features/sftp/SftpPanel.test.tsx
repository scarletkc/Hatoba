import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const backend = vi.hoisted(() => ({ locked: false, homes: 0, uploads: [] as string[] }));

vi.mock("@/ipc/api", async (importActual) => {
  const actual = await importActual<typeof import("@/ipc/api")>();
  const calls: Record<string, (...args: never[]) => Promise<unknown>> = {
    // Rust refuses every SFTP command behind the lock screen (SEC-03).
    sftp_home: async () => {
      backend.homes++;
      if (backend.locked) throw actual.appError("locked", "the vault is locked");
      return "/home/me";
    },
    sftp_list: async () => [],
    sftp_upload: async (_session: string, path: string) => {
      backend.uploads.push(path);
      return "transfer";
    },
  };
  return {
    ...actual,
    api: new Proxy(calls, { get: (target, name: string) => target[name] ?? (async () => () => {}) }),
  };
});

// No Tauri here, so drops come as HTML5 events. The app store follows the system theme from its
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

  it("loads the folder the lock refused after the unlock", async () => {
    await setLocked(true);
    const host = await render();
    expect(host.textContent).toContain(t("sftp.loadFailed"));

    await setLocked(false);
    expect(backend.homes).toBe(2);
    expect(host.textContent).not.toContain(t("sftp.loadFailed"));
  });
});
