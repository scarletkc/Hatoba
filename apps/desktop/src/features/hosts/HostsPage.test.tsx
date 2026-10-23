import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// No Tauri here, so the page lists the mock backend's sample hosts.
// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { t } = await import("@/i18n");
const { useVaultData } = await import("@/app/data");
const { ConfirmHost } = await import("@/components/overlay");
const { HostsPage } = await import("./HostsPage");
const { useHostsUi } = await import("./ui");

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
beforeEach(async () => {
  await act(() => useVaultData.getState().reload());
  useHostsUi.setState({ selection: [] });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() =>
    root.render(
      <>
        <HostsPage group={null} />
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
const rows = () => [...list().querySelectorAll<HTMLElement>("[role=option]")];
const ids = () => rows().map((r) => r.dataset.id!);
const selected = () => rows().filter((r) => r.getAttribute("aria-selected") === "true").map((r) => r.dataset.id);
const row = (i: number) => rows()[i];
const menuItems = () => [...document.querySelectorAll("[role=menu] [role=menuitem]")].map((b) => b.textContent);
const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent === label);

function click(el: Element, keys: MouseEventInit = {}) {
  act(() => void el.dispatchEvent(new MouseEvent("click", { bubbles: true, ...keys })));
}

function rightClick(el: Element) {
  act(() => void el.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true })));
}

describe("HostsPage selection", () => {
  it("selects no host until one is clicked, and none again after a click beside the rows", () => {
    expect(rows().length).toBeGreaterThan(4);
    expect(selected()).toEqual([]);
    click(row(1));
    expect(selected()).toEqual([ids()[1]]);
    click(list());
    expect(selected()).toEqual([]);
  });

  it("picks several hosts with Ctrl and Shift", () => {
    const all = ids();
    click(row(0));
    click(row(2), { ctrlKey: true });
    expect(selected()).toEqual([all[0], all[2]]);
    click(row(4), { shiftKey: true });
    expect(selected()).toEqual([all[2], all[3], all[4]]);
    click(row(3), { ctrlKey: true });
    expect(selected()).toEqual([all[2], all[4]]);
  });

  it("selects every listed host with Ctrl+A, and none with Escape", () => {
    act(() => list().focus());
    act(() => void list().dispatchEvent(new KeyboardEvent("keydown", { key: "a", ctrlKey: true, bubbles: true })));
    expect(selected()).toEqual(ids());
    act(() => void list().dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(selected()).toEqual([]);
  });

  it("offers what applies to all of several selected hosts, and the full menu for one", () => {
    const all = ids();
    click(row(0));
    click(row(1), { ctrlKey: true });
    rightClick(row(1));
    expect(menuItems()).toHaveLength(3);
    expect(menuItems()).toContain(t("hosts.menu.connect"));
    expect(menuItems()).toContain(t("hosts.menu.delete"));
    expect(menuItems()).not.toContain(t("hosts.menu.edit"));
    expect(selected()).toEqual([all[0], all[1]]);

    // A right-click on a host outside the selection selects it alone.
    click(document.body);
    rightClick(row(3));
    expect(selected()).toEqual([all[3]]);
    expect(menuItems()).toContain(t("hosts.menu.edit"));
  });

  it("deletes the selected hosts after one question", async () => {
    const all = ids();
    click(row(0));
    click(row(1), { shiftKey: true });
    rightClick(row(0));
    click(button(t("hosts.menu.delete"))!);
    await until(() => button(t("btn.delete")));
    expect(document.body.textContent).toContain(t("hosts.deleteManyTitle", { n: 2 }));
    click(button(t("btn.delete"))!);
    await until(() => !ids().includes(all[0]) || null);
    expect(ids()).toEqual(all.slice(2));
  });
});
