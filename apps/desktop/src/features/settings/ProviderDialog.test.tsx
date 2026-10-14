import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AiProviderView } from "@/ipc/types";

// No Tauri here, so the dialog talks to the mock backend. The app store follows the system theme from
// its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
// The mock backend has a ChatGPT provider that is signed in on another device only (AI-52).
history.replaceState(null, "", "/?ai=chatgptelsewhere");

const { t } = await import("@/i18n");
const { ProviderDialog } = await import("./ProviderDialog");

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
});

function render(provider: AiProviderView | null, onSaved: (saved: AiProviderView) => void = () => {}) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(<ProviderDialog provider={provider} onClose={() => {}} onSaved={onSaved} />));
  unmount = () => {
    root.unmount();
    host.remove();
  };
}

function type(field: HTMLInputElement, text: string) {
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(field, text);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

const byLabel = (label: string) => document.querySelector(`[aria-label="${label}"]`);
/** The protocol the popup shows, and choosing one from its menu. */
const protocol = () => byLabel(t("aiSettings.f.protocol"))?.textContent;
function pickProtocol(label: string) {
  act(() => (byLabel(t("aiSettings.f.protocol")) as HTMLButtonElement).click());
  const item = [...document.querySelectorAll<HTMLButtonElement>("[role=menuitemradio]")].find((b) => b.textContent === label);
  act(() => item?.click());
}
const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent === label);
const shown = (text: string) => document.body.textContent?.includes(text) ?? false;

describe("ProviderDialog", () => {
  it("preselects System One from the base URL until a protocol is chosen (AI-47)", () => {
    render(null);
    const url = document.querySelector<HTMLInputElement>('input[placeholder="https://api.openai.com/v1"]')!;
    type(url, "https://api.typesafe.ai/v1");
    expect(protocol()).toBe(t("aiSettings.protocol.system_one"));
    expect(shown(t("aiSettings.f.protocol.system_one"))).toBe(true);
    expect(shown(t("aiSettings.f.baseUrl.system_one"))).toBe(true);
    type(url, "https://api.cloudflare.com/client/v4/accounts/abc123/ai/run");
    expect(protocol()).toBe(t("aiSettings.protocol.system_one"));
    type(url, "https://openrouter.ai/api/v1");
    expect(protocol()).toBe(t("aiSettings.protocol.chat_completions"));
    expect(shown(t("aiSettings.f.protocol.system_one"))).toBe(false);

    type(url, "https://api.openai.com/v1");
    expect(protocol()).toBe(t("aiSettings.protocol.responses"));
    expect(shown(t("aiSettings.f.baseUrl.responses"))).toBe(true);

    // A protocol the user picks stays.
    pickProtocol(t("aiSettings.protocol.system_one"));
    type(url, "https://api.openai.com/v1");
    expect(protocol()).toBe(t("aiSettings.protocol.system_one"));
  });

  it("hides what a decision model does not have, and saves its models without it (AI-47)", async () => {
    let saved: AiProviderView | null = null;
    render(
      {
        id: "p-edit",
        name: "Haiku",
        protocol: "anthropic",
        base_url: "https://api.anthropic.com",
        has_api_key: true,
        auth_header: "x-api-key",
        models: [{ id: "claude-haiku-5-5", name: "Claude Haiku 5.5", context_window: 1_000_000, max_output_tokens: 128_000, efforts: ["low"], adaptive_thinking: true }],
        updated_at: 1,
      },
      (view) => (saved = view),
    );
    const columns = [t("aiSettings.models.col.context"), t("aiSettings.models.col.output"), t("aiSettings.models.effort.label", { id: "claude-haiku-5-5" })];
    for (const label of columns) expect(byLabel(label)).not.toBeNull();
    expect(byLabel(t("aiSettings.f.authHeader"))).not.toBeNull();

    pickProtocol(t("aiSettings.protocol.system_one"));
    for (const label of columns) expect(byLabel(label)).toBeNull();
    expect(byLabel(t("aiSettings.f.authHeader"))).toBeNull();
    expect(shown(t("aiSettings.models.col.context"))).toBe(false);
    expect(byLabel(t("aiSettings.models.col.id"))).not.toBeNull();

    act(() => button(t("btn.save"))?.click());
    for (let i = 0; i < 100 && !saved; i++) await act(() => new Promise((r) => setTimeout(r, 10)));
    const view = saved as AiProviderView | null;
    expect(view?.protocol).toBe("system_one");
    expect(view?.models).toEqual([
      { id: "claude-haiku-5-5", name: "Claude Haiku 5.5", context_window: null, max_output_tokens: null, efforts: null, adaptive_thinking: null },
    ]);
  });

  it("shows a ChatGPT provider's account and signs it in and out on this device (AI-52)", async () => {
    const views: AiProviderView[] = [];
    const chatgpt: AiProviderView = {
      id: "p-chatgpt",
      name: "ChatGPT",
      protocol: "responses",
      base_url: "https://api.openai.com/v1",
      has_api_key: false,
      auth_header: "x-api-key",
      models: [],
      chatgpt: { email: "you@example.com", status: "signed_out" },
      updated_at: 1,
    };
    render(chatgpt, (view) => views.push(view));
    // No protocol, base URL or key: the account signs in instead.
    expect(byLabel(t("aiSettings.f.protocol"))).toBeNull();
    expect(shown(t("aiSettings.f.baseUrl"))).toBe(false);
    expect(shown(t("aiSettings.f.apiKey"))).toBe(false);
    expect(shown("you@example.com")).toBe(true);
    expect(shown(t("aiSettings.chatgpt.status.signed_out"))).toBe(true);
    expect(button(t("aiSettings.chatgpt.signOut"))).toBeUndefined();

    // Cancelling the wait for the browser leaves it signed out.
    act(() => button(t("aiSettings.chatgpt.continue"))?.click());
    expect(shown(t("aiSettings.chatgpt.waiting.inline"))).toBe(true);
    act(() => button(t("btn.cancel"))?.click());
    for (let i = 0; i < 100 && shown(t("aiSettings.chatgpt.waiting.inline")); i++) await act(() => new Promise((r) => setTimeout(r, 10)));
    expect(button(t("aiSettings.chatgpt.continue"))).toBeDefined();

    act(() => button(t("aiSettings.chatgpt.continue"))?.click());
    for (let i = 0; i < 400 && !shown(t("aiSettings.chatgpt.status.signed_in")); i++) await act(() => new Promise((r) => setTimeout(r, 10)));
    expect(shown(t("aiSettings.chatgpt.status.signed_in"))).toBe(true);
    expect(views.at(-1)?.id).toBe("p-chatgpt");
    expect(views.at(-1)?.chatgpt?.status).toBe("signed_in");
    // The provider had no models, so it takes the account's from the sign-in, in the form too.
    expect(views.at(-1)?.models.map((m) => m.id)).toEqual(["gpt-6.1-sol", "gpt-6.1-sol-mini", "gpt-6.1-codex"]);
    expect([...document.querySelectorAll("input")].some((i) => i.value === "gpt-6.1-sol")).toBe(true);

    act(() => button(t("aiSettings.chatgpt.signOut"))?.click());
    for (let i = 0; i < 100 && !shown(t("aiSettings.chatgpt.status.signed_out")); i++) await act(() => new Promise((r) => setTimeout(r, 10)));
    expect(shown(t("aiSettings.chatgpt.status.signed_out"))).toBe(true);
    expect(views.at(-1)?.chatgpt?.status).toBe("signed_out");
  });
});
