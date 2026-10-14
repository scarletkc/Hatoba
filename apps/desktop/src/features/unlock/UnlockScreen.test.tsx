import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setLocale, t } from "@/i18n";
import type { VaultStatus } from "@/ipc/types";

vi.mock("@/app/TitleBar", () => ({ WindowChrome: () => null }));

/** What the screen asked the backend for, and how the PIN answers. */
const backend = vi.hoisted(() => ({
  calls: [] as string[],
  status: null as unknown,
  pinError: null as unknown,
}));
vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: {
    vault_status: async () => backend.status,
    vault_unlock: async () => void backend.calls.push("password"),
    vault_unlock_pin: async (pin: string) => {
      backend.calls.push(`pin:${pin}`);
      if (backend.pinError) throw backend.pinError;
    },
    // The user cancels Windows Hello, which leaves the lock screen as it was.
    vault_unlock_biometric: async () => {
      backend.calls.push("hello");
      throw { code: "cancelled", detail: "Windows Hello was cancelled" };
    },
  },
}));
vi.mock("@/app/boot", () => ({ enterUnlocked: async () => void backend.calls.push("unlocked") }));
// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { UnlockScreen } = await import("./UnlockScreen");
const { useApp } = await import("@/app/store");

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
  useApp.setState({ vault: null, lockReason: null });
  backend.calls = [];
  backend.status = null;
  backend.pinError = null;
  vi.restoreAllMocks();
});

/** A locked vault with these unlock methods. */
function locked(status: Partial<VaultStatus>, lockReason: "manual" | "idle" | "sleep" | null = null) {
  const vault = { state: "locked", retry_at: null, biometric_enabled: false, pin_unlock: false, ...status } as VaultStatus;
  backend.status = vault;
  useApp.setState({ vault, lockReason });
}

/** The window in front, as when the user is at it. */
function focused(on = true) {
  vi.spyOn(document, "hasFocus").mockReturnValue(on);
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
}

/** Types into the field the way React sees it. */
function type(field: HTMLInputElement, value: string) {
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

const field = () => document.querySelector<HTMLInputElement>("form input")!;
const link = (key: Parameters<typeof t>[0]) => [...document.querySelectorAll("button")].find((b) => b.textContent === t(key));

async function submit() {
  await act(async () => {
    document.querySelector<HTMLButtonElement>('button[type="submit"]')!.click();
  });
}

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

describe("PIN unlock", () => {
  it("asks for the PIN while it can unlock, and for the master password on request", async () => {
    locked({ pin_unlock: true });
    show();
    expect(field().getAttribute("aria-label")).toBe(t("vault.unlock.pinField"));
    expect(link("vault.unlock.forgot")).toBeUndefined();
    type(field(), "2468");
    await submit();
    expect(backend.calls).toEqual(["pin:2468", "unlocked"]);

    act(() => link("vault.unlock.usePassword")!.click());
    expect(field().getAttribute("aria-label")).toBe(t("vault.unlock.field"));
    expect(field().value).toBe("");
    expect(link("vault.unlock.forgot")).toBeDefined();
    act(() => link("vault.unlock.usePin")!.click());
    expect(field().getAttribute("aria-label")).toBe(t("vault.unlock.pinField"));
  });

  it("says how many tries are left, then asks for the master password", async () => {
    locked({ pin_unlock: true });
    show();
    backend.pinError = { code: "wrong_pin", detail: "wrong PIN", attempts_left: 2 };
    type(field(), "0000");
    await submit();
    expect(document.querySelector('[role="alert"]')!.textContent).toBe(t("err.wrong_pin", { n: 2 }));
    expect(field().getAttribute("aria-label")).toBe(t("vault.unlock.pinField"));

    backend.pinError = { code: "wrong_pin", detail: "wrong PIN", attempts_left: 0 };
    await submit();
    expect(document.querySelector('[role="alert"]')!.textContent).toBe(t("err.pin_locked"));
    expect(field().getAttribute("aria-label")).toBe(t("vault.unlock.field"));
    expect(field().value).toBe("");
  });
});

describe("Windows Hello on its own", () => {
  const hello = () => backend.calls.filter((c) => c === "hello").length;
  const fire = async (event: string) => {
    await act(async () => {
      window.dispatchEvent(new Event(event));
    });
  };

  it("asks once when the app starts in front", async () => {
    focused();
    locked({ biometric_enabled: true });
    await act(async () => void show());
    expect(hello()).toBe(1);
    await fire("focus");
    await fire("pointermove");
    expect(hello()).toBe(1);
  });

  it("waits for the window when the app starts behind another one", async () => {
    focused(false);
    locked({ biometric_enabled: true });
    await act(async () => void show());
    expect(hello()).toBe(0);
    vi.mocked(document.hasFocus).mockReturnValue(true);
    await fire("focus");
    expect(hello()).toBe(1);
  });

  it("after a manual lock, waits until the window is left and comes back", async () => {
    focused();
    locked({ biometric_enabled: true }, "manual");
    await act(async () => void show());
    await fire("pointermove");
    await fire("pointerdown");
    expect(hello()).toBe(0);
    await fire("focus");
    expect(hello()).toBe(1);
  });

  it("after an idle lock, asks at the first pointer movement", async () => {
    focused();
    locked({ biometric_enabled: true }, "idle");
    await act(async () => void show());
    expect(hello()).toBe(0);
    await fire("pointermove");
    await fire("pointermove");
    expect(hello()).toBe(1);
  });

  it("never asks without Windows Hello", async () => {
    focused();
    locked({ biometric_enabled: false });
    await act(async () => void show());
    await fire("focus");
    expect(hello()).toBe(0);
  });
});

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
