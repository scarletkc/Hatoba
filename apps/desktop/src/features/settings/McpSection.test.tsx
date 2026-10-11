import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { McpSection } from "./McpSection";

// No Tauri here, so the section talks to the mock backend.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
});

function render(ui: React.ReactNode): HTMLElement {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(ui));
  unmount = () => {
    root.unmount();
    host.remove();
  };
  return host;
}

/** Lets the mock backend answer until `find` returns something. */
async function until<T>(find: () => T | null | undefined): Promise<T> {
  for (let i = 0; i < 200; i++) {
    const found = find();
    if (found) return found;
    await act(() => new Promise((r) => setTimeout(r, 10)));
  }
  throw new Error("not found in time");
}

describe("McpSection", () => {
  it("gives the focus back to Add from Template when a dialog opened from its menu closes", async () => {
    const host = render(<McpSection />);
    const button = await until(() => host.querySelector<HTMLButtonElement>("button[aria-haspopup=menu]"));
    act(() => button.click());
    const item = [...document.querySelectorAll<HTMLButtonElement>("[role=menuitem]")].find((x) => x.textContent === "GitHub");
    act(() => {
      // A click focuses the menu item first, as it does in the app.
      item?.focus();
      item?.click();
    });
    expect(document.querySelector("[role=menu]")).toBeNull();
    expect(document.querySelector("[role=dialog]")).not.toBeNull();

    act(() => void window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
    expect(document.querySelector("[role=dialog]")).toBeNull();
    expect(document.activeElement).toBe(button);
  });
});
