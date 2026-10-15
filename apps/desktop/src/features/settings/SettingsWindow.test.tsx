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

function open(tab: "terminal" | "appearance" = "terminal") {
  useApp.setState({ settingsTab: tab });
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

  it("switches to a tab asked for while it is open, again after the user moved on", async () => {
    open();
    const selected = () => document.querySelector('[aria-selected="true"]')?.textContent;
    await until(() => selected() === t("settings.tab.terminal"));

    act(() => useApp.getState().openSettings(true, "about"));
    await until(() => selected() === t("settings.tab.about"));
    const general = [...document.querySelectorAll<HTMLElement>("[role=tab]")].find((el) => el.textContent === t("settings.tab.general"));
    act(() => general!.click());
    await until(() => selected() === t("settings.tab.general"));
    act(() => useApp.getState().openSettings(true, "about"));
    await until(() => selected() === t("settings.tab.about"));
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

  it("picks a colour scheme for each terminal mode", async () => {
    await syncIn({ theme: "dark" });
    open("appearance");
    // A card reads as its name; the miniature terminal above it is hidden from assistive tech.
    const card = (name: string) =>
      [...document.querySelectorAll<HTMLElement>("[role=radio]")].find((el) => el.querySelector(":scope > :not([aria-hidden])")?.textContent === name);
    const checked = (name: string) => card(name)?.getAttribute("aria-checked") === "true";
    await until(() => card("Hatoba Dark"));
    // Nothing picked yet: Hatoba's own scheme, and only the dark ones while the terminal is always dark.
    expect(checked("Hatoba Dark")).toBe(true);
    expect(card("Solarized Light")).toBeUndefined();

    act(() => card("Dracula")!.click());
    await until(async () => (await stored()).dark_scheme === "dracula");
    expect(checked("Dracula")).toBe(true);

    // The group is one tab stop, and the arrow keys move the pick.
    expect([...card("Dracula")!.parentElement!.querySelectorAll("[tabindex='0']")]).toEqual([card("Dracula")]);
    act(() => {
      card("Dracula")!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    });
    await until(async () => (await stored()).dark_scheme === "tokyo-night");
    expect(document.activeElement).toBe(card("Tokyo Night"));
    act(() => {
      card("Tokyo Night")!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }));
    });
    await until(async () => (await stored()).dark_scheme === "dracula");

    const follow = [...document.querySelectorAll<HTMLElement>("[role=radio]")].find((el) => el.textContent === t("settings.termColors.follow"));
    act(() => follow!.click());
    await until(() => byLabel(t("settings.termColors.forLight")));
    expect(byLabel(t("settings.termColors.forDark"))).not.toBeNull();
    act(() => card("Solarized Light")!.click());
    const after = await until(async () => {
      const now = await stored();
      return now.light_scheme === "solarized-light" && now;
    });
    expect(after).toMatchObject({ theme: "system", dark_scheme: "dracula" });
  });
});
