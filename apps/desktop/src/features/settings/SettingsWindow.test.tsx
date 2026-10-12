import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";

// No Tauri here, so the window talks to the mock backend. The app store follows the system theme
// from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { useApp } = await import("@/app/store");
const { getLocale, t } = await import("@/i18n");
const { api } = await import("@/ipc/api");
const { SettingsWindow } = await import("./SettingsWindow");

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
});

function open() {
  useApp.setState({ settingsTab: "terminal" });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(<SettingsWindow onClose={() => {}} />));
  unmount = () => {
    root.unmount();
    host.remove();
  };
}

/** Lets the mock backend answer until `find` returns something. */
async function until<T>(find: () => T | null | undefined | false | Promise<T | false>): Promise<T> {
  for (let i = 0; i < 200; i++) {
    const found = await find();
    if (found) return found;
    await act(() => new Promise((r) => setTimeout(r, 10)));
  }
  throw new Error("not found in time");
}

const byLabel = <E extends HTMLElement>(label: string) => document.querySelector<E>(`[aria-label="${label}"]`);
const fontField = () => byLabel<HTMLInputElement>(t("settings.term.font"));
const stored = async () => (await api.settings_get()).terminal;
const scrollbackShows = (n: number) =>
  byLabel(t("settings.term.scrollback"))?.textContent === t("settings.term.lines", { n: n.toLocaleString(getLocale()) });
const blur = (field: HTMLElement) =>
  act(() => {
    field.dispatchEvent(new FocusEvent("focusout", { bubbles: true }));
  });

/** Another device changes the settings, and a sync round brings the change into this vault. */
async function syncIn(terminal: Parameters<typeof api.settings_save>[0]["terminal"]) {
  await act(() => api.settings_save({ terminal }));
}

function type(field: HTMLInputElement, text: string) {
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(field, text);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

describe("SettingsWindow", () => {
  it("saves only the field the user changes, over settings that synced in after the window read them", async () => {
    open();
    const bigger = await until(() => byLabel<HTMLButtonElement>(t("settings.term.sizeUp")));
    const before = await stored();
    await syncIn({ scrollback: 50_000, cursor_style: "underline" });

    act(() => bigger.click());

    const after = await until(async () => {
      const now = await stored();
      return now.font_size === before.font_size + 1 && now;
    });
    expect(after.scrollback).toBe(50_000);
    expect(after.cursor_style).toBe("underline");
  });

  it("shows what a sync round brought, but not over the font the user is typing", async () => {
    open();
    const field = await until(fontField);
    await syncIn({ font_family: "Iosevka" });
    act(() => useApp.getState().refreshSynced());
    await until(() => field.value === "Iosevka");

    type(field, "JetBrains Mono");
    await syncIn({ font_family: "Fira Code", scrollback: 5_000 });
    act(() => useApp.getState().refreshSynced());
    await until(() => scrollbackShows(5_000));
    expect(field.value).toBe("JetBrains Mono");

    blur(field);
    const after = await until(async () => {
      const now = await stored();
      return now.font_family === "JetBrains Mono" && now;
    });
    expect(after.scrollback).toBe(5_000);
  });

  it("saves nothing when the user types and changes the font back", async () => {
    open();
    const field = await until(fontField);
    await syncIn({ font_family: "Iosevka" });
    act(() => useApp.getState().refreshSynced());
    await until(() => field.value === "Iosevka");

    type(field, "Iosevka Term");
    await syncIn({ font_family: "Fira Code", scrollback: 1_000 });
    act(() => useApp.getState().refreshSynced());
    await until(() => scrollbackShows(1_000));
    type(field, "Iosevka");
    blur(field);

    expect(field.value).toBe("Fira Code");
    await act(() => new Promise((r) => setTimeout(r, 50)));
    expect((await stored()).font_family).toBe("Fira Code");
  });
});
