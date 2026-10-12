import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setLocale, t } from "@/i18n";
import type { VaultStatus } from "@/ipc/types";

vi.mock("@/app/TitleBar", () => ({ WindowChrome: () => null }));
// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { UnlockScreen } = await import("./UnlockScreen");
const { useApp } = await import("@/app/store");

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
  useApp.setState({ vault: null });
});

function show() {
  setLocale("en");
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(<UnlockScreen />));
  unmount = () => {
    root.unmount();
    host.remove();
  };
  const field = document.querySelector<HTMLInputElement>(`input[aria-label="${t("vault.unlock.field")}"]`)!;
  const reveal = () => document.querySelector<HTMLButtonElement>(`button[aria-label="${t("field.show")}"], button[aria-label="${t("field.hide")}"]`)!;
  return { field, reveal };
}

describe("unlock screen", () => {
  it("shows the password with its own button and keeps the focus in the field", () => {
    const { field, reveal } = show();
    act(() => field.focus());
    expect(field.type).toBe("password");
    expect(reveal().getAttribute("aria-label")).toBe(t("field.show"));

    const down = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    reveal().dispatchEvent(down);
    expect(down.defaultPrevented).toBe(true);
    act(() => reveal().click());
    expect(field.type).toBe("text");
    expect(reveal().getAttribute("aria-label")).toBe(t("field.hide"));

    act(() => reveal().click());
    expect(field.type).toBe("password");
  });

  it("hides the password again when the window loses focus", () => {
    const { field, reveal } = show();
    act(() => reveal().click());
    expect(field.type).toBe("text");
    act(() => {
      window.dispatchEvent(new Event("blur"));
    });
    expect(field.type).toBe("password");
  });

  it("hides the password when a delay starts, since the field and its button are disabled", () => {
    const { field, reveal } = show();
    act(() => reveal().click());
    expect(field.type).toBe("text");
    act(() => useApp.setState({ vault: { retry_at: Date.now() + 30_000 } as VaultStatus }));
    expect(field.disabled).toBe(true);
    expect(reveal().disabled).toBe(true);
    expect(field.type).toBe("password");
  });
});
