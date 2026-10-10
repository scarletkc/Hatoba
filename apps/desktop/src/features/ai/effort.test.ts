import { describe, expect, it } from "vitest";
import { translate } from "@/i18n";
import type { AiConversationView, AiModel, AiSettingsView } from "@/ipc/types";
import { EFFORTS, UNKNOWN_EFFORTS, chosenEffort, effectiveEffort, effortKey, modelEfforts, sortEfforts } from "./effort";

const model = (efforts: AiModel["efforts"]): AiModel => ({
  id: "m",
  name: "m",
  context_window: null,
  max_output_tokens: null,
  efforts,
  adaptive_thinking: null,
});

const conversation = (effort: AiConversationView["effort"]): AiConversationView => ({
  id: "c1",
  title: "t",
  host_id: null,
  quick_target: null,
  pinned: false,
  context_start: null,
  effort,
  created_at: 0,
  updated_at: 0,
  last_activity: 0,
});

const settings = (default_effort: AiSettingsView["default_effort"]): AiSettingsView => ({
  default_model: null,
  default_effort,
  search_provider_id: null,
  builtin_skill_enabled: true,
  custom_instructions: "",
});

describe("thinking levels (AI-05)", () => {
  it("offers the model's levels, or Low to High when they are unknown", () => {
    expect(modelEfforts(model(["low", "max"]))).toEqual(["low", "max"]);
    expect(modelEfforts(model([]))).toEqual([]);
    expect(modelEfforts(model(null))).toEqual(UNKNOWN_EFFORTS);
    expect(modelEfforts(null)).toEqual(UNKNOWN_EFFORTS);
  });

  it("uses the highest offered level that is not above the chosen one, as Rust does", () => {
    expect(effectiveEffort(null, EFFORTS)).toBeNull();
    expect(effectiveEffort("max", UNKNOWN_EFFORTS)).toBe("high");
    expect(effectiveEffort("medium", UNKNOWN_EFFORTS)).toBe("medium");
    // Opus 4.6 has Max but no Extra High.
    expect(effectiveEffort("xhigh", ["low", "medium", "high", "max"])).toBe("high");
    expect(effectiveEffort("high", [])).toBeNull();
    expect(effectiveEffort("low", ["high", "max"])).toBeNull();
  });

  it("follows the pick, then the conversation, then the default for new conversations", () => {
    // A new conversation: the default from Settings → AI.
    expect(chosenEffort(null, null, null, settings("xhigh"))).toBe("xhigh");
    expect(chosenEffort(null, null, null, null)).toBeNull();
    // A conversation keeps its level; one without a stored level is Default.
    expect(chosenEffort(null, "c1", conversation("low"), settings("xhigh"))).toBe("low");
    expect(chosenEffort(null, "c1", conversation(null), settings("xhigh"))).toBeNull();
    expect(chosenEffort(null, "c1", null, settings("xhigh"))).toBeNull();
    // The user's pick wins, Default included.
    expect(chosenEffort("max", "c1", conversation("low"), null)).toBe("max");
    expect(chosenEffort("default", null, null, settings("high"))).toBeNull();
  });

  it("sorts levels and labels every one in every language", () => {
    expect(sortEfforts(["max", "low", "high", "low"])).toEqual(["low", "high", "max"]);
    for (const locale of ["zh-CN", "en", "ja"] as const) {
      const labels = [null, ...EFFORTS].map((e) => translate(locale, effortKey(e)));
      expect(new Set(labels).size).toBe(6);
      for (const label of labels) expect(label).not.toMatch(/^ai\.effort/);
    }
    expect([null, ...EFFORTS].map((e) => translate("en", effortKey(e)))).toEqual(["Default", "Low", "Medium", "High", "Extra High", "Max"]);
    expect([null, ...EFFORTS].map((e) => translate("zh-CN", effortKey(e)))).toEqual(["默认", "低", "中", "高", "超高", "最高"]);
  });
});
