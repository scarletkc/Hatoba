import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AiProviderView } from "@/ipc/types";

// No Tauri here, so the dialog talks to the mock backend. The app store follows the system theme from
// its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

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
const radio = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("[role=radio]")].find((b) => b.textContent === label);
const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent === label);
const shown = (text: string) => document.body.textContent?.includes(text) ?? false;

describe("ProviderDialog", () => {
  it("preselects System One from the base URL until a protocol is chosen (AI-47)", () => {
    render(null);
    const url = document.querySelector<HTMLInputElement>('input[placeholder="https://api.openai.com/v1"]')!;
    type(url, "https://api.typesafe.ai/v1");
    expect(radio(t("aiSettings.protocol.system_one"))?.getAttribute("aria-checked")).toBe("true");
    expect(shown(t("aiSettings.f.protocol.system_one"))).toBe(true);
    expect(shown(t("aiSettings.f.baseUrl.system_one"))).toBe(true);
    type(url, "https://api.cloudflare.com/client/v4/accounts/abc123/ai/run");
    expect(radio(t("aiSettings.protocol.system_one"))?.getAttribute("aria-checked")).toBe("true");
    type(url, "https://openrouter.ai/api/v1");
    expect(radio(t("aiSettings.protocol.chat_completions"))?.getAttribute("aria-checked")).toBe("true");
    expect(shown(t("aiSettings.f.protocol.system_one"))).toBe(false);

    // A protocol the user picks stays.
    act(() => radio(t("aiSettings.protocol.system_one"))?.click());
    type(url, "https://api.openai.com/v1");
    expect(radio(t("aiSettings.protocol.system_one"))?.getAttribute("aria-checked")).toBe("true");
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

    act(() => radio(t("aiSettings.protocol.system_one"))?.click());
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
});
