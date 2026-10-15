import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AiProviderView, AiSettingsView } from "@/ipc/types";

// The mode switch calls no command; this keeps the real IPC layer out of the test.
vi.mock("@/ipc/api", async (importActual) => ({ ...(await importActual<typeof import("@/ipc/api")>()), api: {} }));
// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { useApp } = await import("@/app/store");
const { t } = await import("@/i18n");
const { Composer } = await import("./Composer");
const { blankSlot, getSlot, HOME_SLOT, useAi } = await import("./store");

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const provider: AiProviderView = {
  id: "p1",
  name: "Mock",
  protocol: "chat_completions",
  base_url: "http://127.0.0.1/v1",
  has_api_key: false,
  auth_header: "x-api-key",
  models: [
    { id: "m1", name: "Model 1", context_window: null, max_output_tokens: null, efforts: null, adaptive_thinking: null },
    { id: "fast", name: "Fast", context_window: null, max_output_tokens: null, efforts: null, adaptive_thinking: null },
  ],
  updated_at: 0,
};
const settings = (judge_model: AiSettingsView["judge_model"]): AiSettingsView => ({
  default_model: null,
  default_effort: null,
  search_provider_id: null,
  native_search: true,
  builtin_skill_enabled: true,
  custom_instructions: "",
  send_terminal_screen: true,
  judge_model,
});

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
});

beforeEach(() => {
  useAi.setState({ slots: { [HOME_SLOT]: blankSlot("manual") }, catalog: { providers: [provider], settings: settings(null), loaded: true } });
});

function Harness() {
  const s = useAi((st) => st.slots[HOME_SLOT]);
  return <Composer slotId={HOME_SLOT} slot={s} model={{ provider_id: "p1", model_id: "m1" }} tools={false} />;
}

function render(): HTMLButtonElement {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(<Harness />));
  unmount = () => {
    root.unmount();
    host.remove();
  };
  return host.querySelector<HTMLButtonElement>(`button[aria-label^="${t("ai.mode")}:"]`)!;
}

const item = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("[role=menu] button")].find((b) => b.textContent?.startsWith(label));

describe("the permission mode switch (AI-16, AI-47)", () => {
  it("offers Auto only once a judge is chosen, and links to Settings → AI until then", () => {
    const button = render();
    act(() => button.click());
    expect(item(t("ai.mode.auto"))?.disabled).toBe(true);
    expect(item(t("ai.mode.auto"))?.textContent).toContain(t("ai.mode.autoNeedsJudge"));
    act(() => item(t("ai.mode.chooseJudge"))?.click());
    expect(useApp.getState()).toMatchObject({ settingsOpen: true, settingsTab: "ai" });
    act(() => useApp.getState().openSettings(false));
    expect(getSlot(HOME_SLOT).mode).toBe("manual");
  });

  it("switches to Auto without asking, and shows its name beside its icon", () => {
    useAi.setState({ catalog: { providers: [provider], settings: settings({ provider_id: "p1", model_id: "fast" }), loaded: true } });
    const button = render();
    act(() => button.click());
    expect(item(t("ai.mode.chooseJudge"))).toBeUndefined();
    act(() => item(t("ai.mode.auto"))?.click());
    expect(getSlot(HOME_SLOT).mode).toBe("auto");
    expect(button.textContent).toBe(t("ai.mode.autoShort"));
    expect(button.title).toBe(`${t("ai.mode")}: ${t("ai.mode.auto")}`);
    expect(document.querySelector("[role=dialog], [role=alertdialog]")).toBeNull();
  });

  it("takes a judge whose model is gone for none", () => {
    useAi.setState({ catalog: { providers: [provider], settings: settings({ provider_id: "p1", model_id: "gone" }), loaded: true } });
    const button = render();
    act(() => button.click());
    expect(item(t("ai.mode.auto"))?.disabled).toBe(true);
  });
});
