import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SearchProviderView } from "@/ipc/types";

// No Tauri here, so the pane talks to the mock backend. The app store follows the system theme from
// its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { useApp } = await import("@/app/store");
const { effortKey } = await import("@/features/ai/effort");
const { t } = await import("@/i18n");
const { api } = await import("@/ipc/api");
const { AiPane } = await import("./AiPane");

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
});

function render() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(<AiPane />));
  unmount = () => {
    root.unmount();
    host.remove();
  };
}

/** Lets the mock backend answer until `find` returns something. */
async function until<T>(find: () => T | null | undefined | false | Promise<T | false>): Promise<T> {
  for (let i = 0; i < 300; i++) {
    const found = await find();
    if (found) return found;
    await act(() => new Promise((r) => setTimeout(r, 10)));
  }
  throw new Error("not found in time");
}

const byLabel = <E extends HTMLElement>(label: string) => document.querySelector<E>(`[aria-label="${label}"]`);
const instructions = () => document.querySelector<HTMLTextAreaElement>("#ai-custom-instructions");

function type(field: HTMLInputElement | HTMLTextAreaElement, text: string) {
  act(() => {
    Object.getOwnPropertyDescriptor(Object.getPrototypeOf(field), "value")?.set?.call(field, text);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

/** Opens the menu of a popup button and picks the option labelled `label`. */
function pick(button: HTMLButtonElement, label: string) {
  act(() => button.click());
  const option = [...document.querySelectorAll<HTMLButtonElement>("[role=menu] button")].find((b) => b.textContent === label);
  if (!option) throw new Error(`no option ${label}`);
  act(() => option.click());
}

describe("AiPane", () => {
  it("saves only the setting the user changes, over settings that synced in after the pane read them", async () => {
    render();
    const effort = await until(() => byLabel<HTMLButtonElement>(t("aiSettings.default.effort")));
    // Another device changes the settings, and a sync round brings the change into this vault.
    await act(() => api.ai_settings_save({ custom_instructions: "Answer in Japanese.", builtin_skill_enabled: false }));

    pick(effort, t(effortKey("high")));

    const after = await until(async () => {
      const stored = await api.ai_settings_get();
      return stored.default_effort === "high" && stored;
    });
    expect(after.custom_instructions).toBe("Answer in Japanese.");
    expect(after.builtin_skill_enabled).toBe(false);
  });

  it("shows instructions synced from another device, but not over what the user is typing", async () => {
    render();
    const box = await until(instructions);
    await act(() => api.ai_settings_save({ custom_instructions: "Be brief." }));
    act(() => useApp.getState().refreshSynced());
    await until(() => box.value === "Be brief.");

    type(box, "Be thorough.");
    await act(() => api.ai_settings_save({ custom_instructions: "Be very brief.", default_effort: "low" }));
    act(() => useApp.getState().refreshSynced());
    await until(() => byLabel(t("aiSettings.default.effort"))?.textContent === t(effortKey("low")));
    expect(box.value).toBe("Be thorough.");

    // Saved a moment after typing stops.
    const after = await until(async () => {
      const stored = await api.ai_settings_get();
      return stored.custom_instructions === "Be thorough." && stored;
    });
    expect(after.default_effort).toBe("low");
  });

  it("follows a search provider chosen on another device, but not over a kind the user picked", async () => {
    render();
    const search = await until(() => byLabel<HTMLButtonElement>(t("aiSettings.search.row")));
    expect(search.textContent).toBe(t("aiSettings.search.none"));
    let tavily!: SearchProviderView;
    await act(async () => {
      tavily = await api.search_provider_save({ id: null, kind: "tavily", base_url: null, api_key: "tvly-test" });
      await api.ai_settings_save({ search_provider_id: tavily.id });
    });
    act(() => useApp.getState().refreshSynced());
    await until(() => search.textContent === t("aiSettings.search.tavily"));

    // Brave is not saved yet, so it waits for Save; a choice synced meanwhile leaves it.
    pick(search, t("aiSettings.search.brave"));
    await act(() => api.ai_settings_save({ search_provider_id: null, custom_instructions: "Search less." }));
    act(() => useApp.getState().refreshSynced());
    await until(() => instructions()?.value === "Search less.");
    expect(search.textContent).toBe(t("aiSettings.search.brave"));
  });

  it("saves an address to the search provider it was typed for, after a sync chose another one", async () => {
    let first!: SearchProviderView;
    let second!: SearchProviderView;
    await act(async () => {
      first = await api.search_provider_save({ id: null, kind: "searxng", base_url: "http://127.0.0.1:8001", api_key: null });
      second = await api.search_provider_save({ id: null, kind: "searxng", base_url: "http://127.0.0.1:8002", api_key: null });
      await api.ai_settings_save({ search_provider_id: first.id });
    });
    render();
    const url = await until(() => {
      const field = document.querySelector<HTMLInputElement>("#ai-search-url");
      return field?.value === first.base_url && field;
    });

    type(url, "http://127.0.0.1:8003");
    await act(() => api.ai_settings_save({ search_provider_id: second.id, custom_instructions: "Use the other instance." }));
    act(() => useApp.getState().refreshSynced());
    await until(() => instructions()?.value === "Use the other instance.");
    expect(url.value).toBe("http://127.0.0.1:8003");
    const save = [...document.querySelectorAll<HTMLButtonElement>("button")].filter((b) => b.textContent === t("btn.save"));
    expect(save).toHaveLength(1);
    act(() => save[0].click());

    await until(async () => (await api.ai_settings_get()).search_provider_id === first.id);
    const stored = await api.search_providers_list();
    expect(stored.find((p) => p.id === first.id)?.base_url).toBe("http://127.0.0.1:8003");
    expect(stored.find((p) => p.id === second.id)?.base_url).toBe("http://127.0.0.1:8002");
  });

  it("offers Auto as the default mode once a judge is chosen (AI-47)", async () => {
    await act(() => api.ai_settings_save({ judge_model: null }));
    render();
    const auto = () => [...document.querySelectorAll<HTMLButtonElement>("[role=radio]")].find((b) => b.textContent === t("aiSettings.mode.auto"));
    await until(() => document.body.textContent?.includes(t("aiSettings.mode.hintNoJudge")));
    expect(auto()?.disabled).toBe(true);

    // A judge chosen on another device arrives with a sync.
    await act(() => api.ai_settings_save({ judge_model: { provider_id: "p-anthropic", model_id: "claude-haiku-5-5" } }));
    act(() => useApp.getState().refreshSynced());
    await until(() => auto()?.disabled === false);
    expect(document.body.textContent).not.toContain(t("aiSettings.mode.hintNoJudge"));
    act(() => auto()?.click());
    await until(() => useApp.getState().prefs.ai_permission_mode === "auto");
  });

  it("picks the judge among every provider's models, tests it, and clears it (AI-47)", async () => {
    await act(() => api.ai_settings_save({ judge_model: null }));
    render();
    const judge = await until(() => byLabel<HTMLButtonElement>(t("aiSettings.judge.row")));
    expect(judge.textContent).toBe(t("aiSettings.judge.placeholder"));
    const test = () => [...document.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent === t("aiSettings.judge.test"));
    expect(test()?.disabled).toBe(true);

    // The mock's System One provider, TypeSafe, may judge but not chat.
    const defaults = byLabel<HTMLButtonElement>(t("aiSettings.default.row"))!;
    act(() => defaults.click());
    expect(document.querySelector("[role=menu]")?.textContent).not.toContain("TypeSafe");
    act(() => defaults.click());
    act(() => judge.click());
    expect(document.querySelector("[role=menu]")?.textContent).toContain("TypeSafe");
    act(() => judge.click());
    pick(judge, "Jev");
    await until(async () => (await api.ai_settings_get()).judge_model?.provider_id === "p-typesafe");
    expect(judge.textContent).toBe("JevTypeSafe");

    act(() => test()?.click());
    await until(() => document.body.textContent?.includes(t("aiSettings.judge.test.running")));
    await until(() => document.body.textContent?.includes(t("aiSettings.judge.test.summary", { n: 10, total: 11 })));
    const marks = (label: string) => document.querySelectorAll(`[role=img][aria-label="${label}"]`).length;
    expect(marks(t("aiSettings.judge.test.match"))).toBe(10);
    expect(marks(t("aiSettings.judge.test.mismatch"))).toBe(1);
    const text = document.body.textContent ?? "";
    for (const name of ["disk_space", "restart_requested_zh", "pipe_to_shell", "key_via_shell", "token_in_url"] as const) {
      expect(text).toContain(t(`aiSettings.judge.sample.${name}`));
    }
    // The mismatch: a restart asked for in Chinese, which the judge asked about.
    expect(text).toContain(t("aiSettings.judge.test.expected", { decision: t("aiSettings.judge.decision.run") }));
    expect(text).toContain(`${t("ai.auto.q.requested")} 62%`);

    // None leaves Auto without a judge, and the result of the judge that was tested goes.
    pick(judge, t("aiSettings.judge.none"));
    await until(async () => (await api.ai_settings_get()).judge_model === null);
    expect(judge.textContent).toBe(t("aiSettings.judge.placeholder"));
    expect(document.body.textContent).not.toContain(t("aiSettings.judge.test.summary", { n: 10, total: 11 }));
    expect(document.body.textContent).toContain(t("aiSettings.mode.hintNoJudge"));
    expect(test()?.disabled).toBe(true);
  });
});
