import { StrictMode, act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AiProviderView } from "@/ipc/types";

// No Tauri here, so the dialog talks to the mock backend. The app store follows the system theme from
// its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
// Each sign-in the dialog starts opens the browser once.
const signIns = vi.hoisted(() => ({ count: 0 }));
vi.mock("@/ipc/api", async (importOriginal) => {
  const mod = await importOriginal<typeof import("@/ipc/api")>();
  const api = new Proxy(mod.api, {
    get(target, prop, receiver) {
      if (prop === "ai_chatgpt_sign_in") signIns.count++;
      return Reflect.get(target, prop, receiver);
    },
  });
  return { ...mod, api };
});

const { t } = await import("@/i18n");
const { ChatgptSignInDialog } = await import("./ChatgptSignIn");

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
});

const shown = (text: string) => document.body.textContent?.includes(text) ?? false;
const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent === label);
const settle = async (until: () => boolean) => {
  for (let i = 0; i < 400 && !until(); i++) await act(() => new Promise((r) => setTimeout(r, 10)));
};

function render(onDone: (p: AiProviderView) => void, onClose: () => void) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  // StrictMode, as in the app: effects run twice in development.
  act(() =>
    root.render(
      <StrictMode>
        <ChatgptSignInDialog onDone={onDone} onClose={onClose} />
      </StrictMode>,
    ),
  );
  unmount = () => {
    root.unmount();
    host.remove();
  };
}

describe("ChatgptSignInDialog (AI-52)", () => {
  it("waits for the browser, then says once that requests use the ChatGPT plan", { timeout: 10_000 }, async () => {
    const done: AiProviderView[] = [];
    let closed = 0;
    signIns.count = 0;
    render((p) => done.push(p), () => closed++);
    expect(shown(t("aiSettings.chatgpt.waiting.title"))).toBe(true);
    await settle(() => done.length > 0);
    expect(signIns.count).toBe(1);
    expect(done[0].chatgpt).toEqual({ email: "you@example.com", status: "signed_in" });
    expect(done[0].models.length).toBeGreaterThan(0);
    expect(shown(t("aiSettings.chatgpt.welcome.title"))).toBe(true);
    expect(button(t("aiSettings.chatgpt.manageUsage"))).toBeDefined();
    // Re-rendering does not start another sign-in, which would open the browser again.
    await act(() => new Promise((r) => setTimeout(r, 2500)));
    expect(signIns.count).toBe(1);
    expect(new Set(done.map((p) => p.id)).size).toBe(1);
    act(() => button(t("aiSettings.chatgpt.welcome.ok"))?.click());
    expect(closed).toBe(1);
  });

  it("closes without a provider when the sign-in is cancelled", async () => {
    const done: AiProviderView[] = [];
    let closed = 0;
    render((p) => done.push(p), () => closed++);
    act(() => button(t("btn.cancel"))?.click());
    await settle(() => closed > 0);
    await act(() => new Promise((r) => setTimeout(r, 50)));
    expect(closed).toBeGreaterThan(0);
    expect(done).toEqual([]);
  });
});
