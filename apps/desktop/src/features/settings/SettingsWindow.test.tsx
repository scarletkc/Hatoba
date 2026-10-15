import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";

// No Tauri here, so the window talks to the mock backend. The app store follows the system theme
// from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { useApp } = await import("@/app/store");
const { getLocale, t } = await import("@/i18n");
const { api } = await import("@/ipc/api");
const { ConfirmHost, ToastHost } = await import("@/components/overlay");
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
  act(() =>
    root.render(
      <>
        <SettingsWindow onClose={() => {}} />
        <ConfirmHost />
        <ToastHost />
      </>,
    ),
  );
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

/** A scheme card reads as its name; the miniature terminal above it is hidden from assistive tech. */
const card = (name: string) =>
  [...document.querySelectorAll<HTMLElement>("[role=radio]")].find((el) => el.querySelector(":scope > :not([aria-hidden])")?.textContent === name);
const checked = (name: string) => card(name)?.getAttribute("aria-checked") === "true";
const button = (text: string, within: ParentNode = document) =>
  [...within.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent === text);
const editor = () => [...document.querySelectorAll<HTMLElement>("[role=dialog]")].at(-1)!;
/** The scheme editor's name field, named by its form label. */
const nameField = () => {
  const label = [...document.querySelectorAll("label")].find((l) => l.textContent === t("settings.scheme.name"));
  return label?.htmlFor ? (document.getElementById(label.htmlFor) as HTMLInputElement | null) : null;
};

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
    await until(() => card("Hatoba Dark"));
    // Nothing picked yet: Hatoba's own scheme, and only the dark ones while the terminal is always dark.
    expect(checked("Hatoba Dark")).toBe(true);
    expect(card("Solarized Light")).toBeUndefined();

    act(() => card("Dracula")!.click());
    await until(async () => (await stored()).dark_scheme === "dracula");
    expect(checked("Dracula")).toBe(true);

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

  it("duplicates a scheme, edits the copy, puts it to use, and deletes it", async () => {
    await syncIn({ theme: "dark", dark_scheme: "dracula" });
    open("appearance");
    await until(() => checked("Dracula"));
    // A built-in scheme cannot be edited, only duplicated.
    expect(button(t("settings.scheme.edit"))).toBeUndefined();
    act(() => button(t("settings.scheme.duplicate"))!.click());

    const copyName = t("settings.scheme.copyName", { name: "Dracula" });
    const name = await until(nameField);
    expect(name.value).toBe(copyName);
    const red = editor().querySelector<HTMLInputElement>(`input[aria-label="${t("settings.scheme.color.red")}"]:not([type=color])`)!;
    expect(red.value).toBe("#ff5555");
    // The preview follows each valid code as it is typed; a partial one changes nothing yet.
    type(red, "#12");
    expect(red.getAttribute("aria-invalid")).toBe("true");
    type(red, "123456");
    const preview = editor().querySelector<HTMLElement>("[aria-hidden]")!;
    expect([...preview.querySelectorAll<HTMLElement>("span")].some((el) => el.style.color === "rgb(18, 52, 86)")).toBe(true);
    act(() => button(t("btn.save"), editor())!.click());

    // Saved, listed after the built-in schemes, and put to use for dark terminals.
    const saved = await until(async () => (await api.terminal_schemes_list()).find((s) => s.name === copyName) ?? false);
    expect(saved).toMatchObject({ mode: "dark", colors: { red: "#123456", background: "#282a36" } });
    await until(async () => (await stored()).dark_scheme === saved.id);
    await until(() => checked(copyName));

    // A second copy of Dracula gets a name of its own.
    act(() => card("Dracula")!.click());
    await until(() => checked("Dracula"));
    act(() => button(t("settings.scheme.duplicate"))!.click());
    await until(() => nameField()?.value === `${copyName} 2`);
    act(() => button(t("btn.cancel"), editor())!.click());

    act(() => card(copyName)!.click());
    await until(() => checked(copyName));
    act(() => button(t("settings.scheme.edit"))!.click());
    const remove = await until(() => nameField() && button(t("btn.delete"), editor()));
    act(() => remove.click());
    const ok = await until(() => [...document.querySelectorAll<HTMLElement>("[role=alertdialog]")].at(-1));
    act(() => button(t("btn.delete"), ok)!.click());

    await until(async () => (await api.terminal_schemes_list()).length === 0);
    // The settings keep the id, which now names nothing and reads as Hatoba's own scheme.
    await until(() => checked("Hatoba Dark"));
    expect(card(copyName)).toBeUndefined();
    expect((await stored()).dark_scheme).toBe(saved.id);
  });

  it("imports a scheme file into the editor, choosing among the schemes it holds", async () => {
    await syncIn({ theme: "light" });
    open("appearance");
    await until(() => card("Hatoba Light"));
    const input = document.querySelector<HTMLInputElement>("input[type=file]")!;
    const pick = (file: File) =>
      act(() => {
        Object.defineProperty(input, "files", { value: [file], configurable: true });
        input.dispatchEvent(new Event("change", { bubbles: true }));
      });
    const scheme = (name: string, background: string) => ({ name, background, foreground: "#333333", red: "#cc0000", purple: "#aa00aa" });

    act(() => button(t("settings.scheme.import"))!.click());
    pick(new File([JSON.stringify({ schemes: [scheme("Paper", "#f8f8f8"), scheme("Ink", "#101010")] })], "settings.json"));
    const ink = await until(() => [...document.querySelectorAll<HTMLElement>("[role=menuitem]")].find((el) => el.textContent?.includes("Ink")));
    act(() => ink.click());

    // Imported where the user asked, though its background is dark.
    const name = await until(nameField);
    expect(name.value).toBe("Ink");
    act(() => button(t("btn.save"), editor())!.click());
    const saved = await until(async () => (await api.terminal_schemes_list())[0] ?? false);
    expect(saved).toMatchObject({ name: "Ink", mode: "light", colors: { background: "#101010", magenta: "#aa00aa" } });
    await until(async () => (await stored()).light_scheme === saved.id);

    act(() => button(t("settings.scheme.import"))!.click());
    pick(new File(["not a scheme"], "notes.txt"));
    await until(() => document.body.textContent?.includes(t("settings.scheme.importFailed.unreadable", { name: "notes.txt" })));
    await act(() => api.terminal_scheme_delete(saved.id));
  });

  it("saves the line height and the cursor blink", async () => {
    open();
    const taller = await until(() => byLabel<HTMLButtonElement>(t("settings.term.lineHeightUp")));
    act(() => taller.click());
    await until(async () => (await stored()).line_height === 1.4);
    act(() => byLabel<HTMLButtonElement>(t("settings.term.cursorBlink"))!.click());
    await until(async () => (await stored()).cursor_blink);
  });
});
