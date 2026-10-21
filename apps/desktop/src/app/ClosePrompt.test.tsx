import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LocalPrefs, PackageManager, TrayMenuInput } from "@/ipc/types";

const backend = vi.hoisted(() => ({
  prompt: null as (() => void) | null,
  trayUpdate: null as (() => void) | null,
  calls: [] as string[],
  menus: [] as TrayMenuInput[],
}));

const app = vi.hoisted(() => ({
  phase: "unlocked",
  info: { package_manager: null as PackageManager | null },
  prefs: { close_action: "ask" } as Partial<LocalPrefs>,
  saved: [] as Partial<LocalPrefs>[],
  setPrefs: async (prefs: Partial<LocalPrefs>) => {
    backend.calls.push("save");
    app.saved.push(prefs);
  },
  openSettings: (open: boolean, tab?: string) => void backend.calls.push(`settings:${open}:${tab}`),
}));

vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: {
    listen: async (event: string, handler: () => void) => {
      if (event === "window://close-prompt") backend.prompt = handler;
      if (event === "tray://update") backend.trayUpdate = handler;
      return () => {
        if (event === "window://close-prompt") backend.prompt = null;
        if (event === "tray://update") backend.trayUpdate = null;
      };
    },
    update_install: () => {
      backend.calls.push("install");
      return new Promise(() => {});
    },
    window_hide_to_tray: async () => void backend.calls.push("hide"),
    app_quit: async () => void backend.calls.push("quit"),
    tray_set_menu: async (menu: TrayMenuInput) => void backend.menus.push(menu),
  },
}));

vi.mock("./store", () => ({
  useApp: Object.assign((select: (st: typeof app) => unknown) => select(app), { getState: () => app }),
}));

import { ClosePrompt, useTrayMenu } from "./ClosePrompt";
import { useUpdate } from "./update";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
beforeEach(() => {
  backend.prompt = null;
  backend.trayUpdate = null;
  backend.calls = [];
  backend.menus = [];
  app.phase = "unlocked";
  app.info.package_manager = null;
  app.saved = [];
  useUpdate.setState({ result: null, install: null, checking: false });
});
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
});

async function render(ui: React.ReactNode) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  await act(async () => root.render(ui));
  unmount = () => {
    root.unmount();
    host.remove();
  };
}

const button = (text: string) =>
  [...document.querySelectorAll<HTMLElement>("button")].find((b) => b.textContent === text) ?? null;

async function click(el: HTMLElement | null) {
  expect(el).not.toBeNull();
  await act(async () => el!.click());
}

async function ask() {
  await act(async () => backend.prompt!());
}

describe("ClosePrompt", () => {
  it("shows only when the backend asks", async () => {
    await render(<ClosePrompt />);
    expect(document.querySelector("[role=alertdialog]")).toBeNull();
    await ask();
    expect(document.querySelector("[role=alertdialog]")?.textContent).toContain("Minimize Hatoba to the tray?");
  });

  it("minimizes to the tray without changing the pref", async () => {
    await render(<ClosePrompt />);
    await ask();
    await click(button("Minimize to Tray"));
    expect(backend.calls).toEqual(["hide"]);
    expect(document.querySelector("[role=alertdialog]")).toBeNull();
  });

  it("stores a remembered choice before quitting", async () => {
    await render(<ClosePrompt />);
    await ask();
    await click(document.querySelector<HTMLElement>("[role=checkbox]"));
    await click(button("Quit Hatoba"));
    expect(backend.calls).toEqual(["save", "quit"]);
    expect(app.saved).toEqual([{ close_action: "quit" }]);
  });

  it("asks again without the box ticked after a cancel", async () => {
    await render(<ClosePrompt />);
    await ask();
    await click(document.querySelector<HTMLElement>("[role=checkbox]"));
    await click(button("Cancel"));
    expect(backend.calls).toEqual([]);
    await ask();
    expect(document.querySelector("[role=checkbox]")?.getAttribute("aria-checked")).toBe("false");
  });
});

describe("useTrayMenu", () => {
  function Menu() {
    useTrayMenu();
    return null;
  }

  const found = (version: string) => ({
    current_version: "0.1.0",
    update: { version, notes: null, published_at: null, release_url: `https://github.com/scarletkc/Hatoba/releases/tag/v${version}` },
  });

  it("labels the menu in the app's language and enables Lock only while unlocked", async () => {
    app.phase = "locked";
    await render(<Menu />);
    expect(backend.menus).toEqual([
      {
        open: "Open Hatoba",
        lock: "Lock",
        quit: "Quit Hatoba",
        can_lock: false,
        update: null,
        can_update: false,
        forwards: "Port Forwarding",
        copy_address: "Copy Address",
        stop_forward: "Stop",
        on_server: "{addr} on the server",
        socks_proxy: "SOCKS proxy",
      },
    ]);
  });

  it("offers a newer release, except while it installs", async () => {
    await render(<Menu />);
    await act(async () => useUpdate.setState({ result: found("0.2.0") }));
    expect(backend.menus.at(-1)).toMatchObject({ update: "Update to v0.2.0", can_update: true });
    await act(async () => useUpdate.setState({ install: { kind: "installing" } }));
    expect(backend.menus.at(-1)).toMatchObject({ update: "Update to v0.2.0", can_update: false });
    await act(async () => useUpdate.setState({ result: found("0.2.0"), install: null }));
    await act(async () => useUpdate.setState({ result: { current_version: "0.2.0", update: null } }));
    expect(backend.menus.at(-1)).toMatchObject({ update: null });
  });

  it("offers no update when a package manager installed Hatoba", async () => {
    app.info.package_manager = "scoop";
    useUpdate.setState({ result: found("0.2.0") });
    await render(<Menu />);
    expect(backend.menus.at(-1)).toMatchObject({ update: null });
  });

  it("opens Settings → About and installs when the update item is clicked", async () => {
    useUpdate.setState({ result: found("0.2.0") });
    await render(<Menu />);
    await act(async () => backend.trayUpdate!());
    // No session or transfer would end, so it installs without asking.
    expect(backend.calls).toEqual(["settings:true:about", "install"]);
  });

  it("ignores the update item while the vault is locked", async () => {
    app.phase = "locked";
    useUpdate.setState({ result: found("0.2.0") });
    await render(<Menu />);
    await act(async () => backend.trayUpdate!());
    expect(backend.calls).toEqual([]);
  });
});
