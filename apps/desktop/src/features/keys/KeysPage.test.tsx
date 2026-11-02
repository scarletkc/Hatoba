import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// No Tauri here, so the page lists the mock backend's sample keys.
// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { t } = await import("@/i18n");
const { useVaultData } = await import("@/app/data");
const { ConfirmHost } = await import("@hatoba/ui");
const { KeysPage } = await import("./KeysPage");

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
beforeEach(async () => {
  await act(() => useVaultData.getState().reload());
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() =>
    root.render(
      <>
        <KeysPage />
        <ConfirmHost />
      </>,
    ),
  );
  unmount = () => {
    root.unmount();
    host.remove();
  };
});
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
});

/** Lets the mock backend answer until `find` returns something. */
async function until<T>(find: () => T | null | undefined): Promise<T> {
  for (let i = 0; i < 200; i++) {
    const found = find();
    if (found) return found;
    await act(() => new Promise((r) => setTimeout(r, 10)));
  }
  throw new Error("not found in time");
}

const list = () => document.querySelector<HTMLElement>("[role=listbox]")!;
const search = () => document.querySelector<HTMLInputElement>("input")!;
const rows = () => [...list().querySelectorAll<HTMLElement>("[role=option]")];
const ids = () => rows().map((r) => r.dataset.keyId!);
const selected = () => rows().find((r) => r.getAttribute("aria-selected") === "true")?.dataset.keyId;
const menuItems = () => [...document.querySelectorAll("[role=menu] [role=menuitem]")].map((b) => b.textContent);
const button = (label: string) =>
  [...document.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent === label);

function press(el: Element, key: string) {
  act(() => void el.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true })));
}

function click(el: Element) {
  act(() => void el.dispatchEvent(new MouseEvent("click", { bubbles: true })));
}

describe("KeysPage list", () => {
  it("takes the focus as one list, not row by row, and names the selected key as its active option", () => {
    expect(rows().length).toBeGreaterThan(2);
    expect(list().tabIndex).toBe(0);
    expect(rows().every((r) => !r.hasAttribute("tabindex"))).toBe(true);
    act(() => list().focus());
    press(list(), "ArrowDown");
    expect(document.getElementById(list().getAttribute("aria-activedescendant")!)).toBe(rows()[1]);
  });

  it("moves the selection with ↑/↓ from the list and from the search field", () => {
    const all = ids();
    expect(selected()).toBe(all[0]);
    act(() => list().focus());
    press(list(), "ArrowDown");
    expect(selected()).toBe(all[1]);
    act(() => search().focus());
    press(search(), "ArrowDown");
    expect(selected()).toBe(all[2]);
    press(search(), "ArrowUp");
    expect(selected()).toBe(all[1]);
  });

  it("starts a search when a letter is typed on the list", () => {
    act(() => list().focus());
    press(list(), "d");
    expect(document.activeElement).toBe(search());
  });

  it("selects a key on right-click and offers its actions", () => {
    const all = ids();
    act(() => void rows()[2].dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true })));
    expect(selected()).toBe(all[2]);
    expect(menuItems()).toEqual([
      t("btn.rename"),
      t("keys.action.deploy"),
      t("keys.detail.copyPublic"),
      t("keys.action.delete"),
    ]);
  });

  it("deletes the selected key with Delete after the question", async () => {
    const all = ids();
    act(() => list().focus());
    press(list(), "ArrowDown");
    press(list(), "Delete");
    const confirmButton = await until(() => button(t("keys.delete.confirm")));
    click(confirmButton);
    await until(() => !ids().includes(all[1]) || null);
    expect(ids()).toEqual([all[0], ...all.slice(2)]);
  });
});
