import { describe, expect, it } from "vitest";
import type { AiModel, AiProviderView, SearchProviderView } from "@/ipc/types";
import {
  addModels,
  firstModelRef,
  formatEfforts,
  formatTokenCount,
  keyForInput,
  matchesFilter,
  modelChoices,
  normalizeBaseUrl,
  parseBaseUrl,
  parseModelIds,
  parseTokenCount,
  parseToolLimit,
  removeModel,
  sanitizeSettings,
  sameSettings,
  searchProviderFor,
  suggestProtocol,
  toDraft,
  toggleEffort,
  toModels,
  typedModel,
  validateModels,
  withListedCapabilities,
  type ModelDraft,
} from "./aiLogic";

const model = (id: string, extra: Partial<AiModel> = {}): AiModel => ({
  id,
  name: id,
  context_window: null,
  max_output_tokens: null,
  efforts: null,
  adaptive_thinking: null,
  ...extra,
});

/** A row as the form holds it; nothing is known about its thinking levels unless given. */
const draft = (id: string, name: string, context: string, output: string, extra: Partial<ModelDraft> = {}): ModelDraft => ({
  id,
  name,
  context,
  output,
  efforts: null,
  adaptive: null,
  ...extra,
});

const provider = (id: string, models: AiModel[]): AiProviderView => ({
  id,
  name: id.toUpperCase(),
  protocol: "chat_completions",
  base_url: "https://example.com/v1",
  has_api_key: true,
  auth_header: "x-api-key",
  models,
  updated_at: 1,
});

describe("suggestProtocol", () => {
  it("picks anthropic for api.anthropic.com", () => {
    expect(suggestProtocol("https://api.anthropic.com")).toBe("anthropic");
    expect(suggestProtocol("https://API.Anthropic.com/")).toBe("anthropic");
    expect(suggestProtocol("api.anthropic.com")).toBe("anthropic");
  });

  it("picks anthropic when the path ends in /anthropic", () => {
    expect(suggestProtocol("https://api.deepseek.com/anthropic")).toBe("anthropic");
    expect(suggestProtocol("https://example.com/api/anthropic/")).toBe("anthropic");
  });

  it("keeps chat_completions for everything else", () => {
    expect(suggestProtocol("")).toBe("chat_completions");
    expect(suggestProtocol("https://api.openai.com/v1")).toBe("chat_completions");
    expect(suggestProtocol("http://localhost:11434/v1")).toBe("chat_completions");
    expect(suggestProtocol("https://example.com/anthropic/v1")).toBe("chat_completions");
    expect(suggestProtocol("https://notanthropic.example/foo")).toBe("chat_completions");
    expect(suggestProtocol("https://")).toBe("chat_completions");
    expect(suggestProtocol("htt")).toBe("chat_completions");
  });
});

describe("base URL", () => {
  it("accepts absolute http and https addresses only", () => {
    expect(parseBaseUrl("https://api.openai.com/v1")?.hostname).toBe("api.openai.com");
    expect(parseBaseUrl(" http://localhost:11434/v1 ")?.port).toBe("11434");
    expect(parseBaseUrl("api.openai.com/v1")).toBeNull();
    expect(parseBaseUrl("ftp://example.com")).toBeNull();
    expect(parseBaseUrl("https://")).toBeNull();
    expect(parseBaseUrl("")).toBeNull();
  });

  it("trims trailing slashes", () => {
    expect(normalizeBaseUrl(" https://api.openai.com/v1/ ")).toBe("https://api.openai.com/v1");
    expect(normalizeBaseUrl("http://localhost:1234///")).toBe("http://localhost:1234");
  });
});

describe("keyForInput", () => {
  it("keeps the saved key unless told otherwise", () => {
    expect(keyForInput({ mode: "keep", value: "ignored" }, true)).toBeNull();
    expect(keyForInput({ mode: "replace", value: "" }, true)).toBeNull();
    expect(keyForInput({ mode: "replace", value: "  " }, true)).toBeNull();
  });

  it("replaces and clears", () => {
    expect(keyForInput({ mode: "replace", value: " sk-new \n" }, true)).toBe("sk-new");
    expect(keyForInput({ mode: "clear", value: "sk-typed-before" }, true)).toBe("");
  });

  it("sends the typed key when none is saved", () => {
    expect(keyForInput({ mode: "keep", value: "sk-first" }, false)).toBe("sk-first");
    expect(keyForInput({ mode: "keep", value: "" }, false)).toBeNull();
    expect(keyForInput({ mode: "clear", value: "" }, false)).toBeNull();
  });
});

describe("token counts", () => {
  it("parses digits, separators and K/M suffixes", () => {
    expect(parseTokenCount("")).toEqual({ ok: true, value: null });
    expect(parseTokenCount("  ")).toEqual({ ok: true, value: null });
    expect(parseTokenCount("200000")).toEqual({ ok: true, value: 200_000 });
    expect(parseTokenCount("128,000")).toEqual({ ok: true, value: 128_000 });
    expect(parseTokenCount("200k")).toEqual({ ok: true, value: 200_000 });
    expect(parseTokenCount("1M")).toEqual({ ok: true, value: 1_000_000 });
    expect(parseTokenCount("1.5K")).toEqual({ ok: true, value: 1_500 });
    expect(parseTokenCount("0.5m")).toEqual({ ok: true, value: 500_000 });
  });

  it("rejects everything else", () => {
    for (const bad of ["abc", "0", "-5", "1.5", "12k5", "k", "1e6", "2000000000"]) {
      expect(parseTokenCount(bad), bad).toEqual({ ok: false });
    }
  });

  it("formats what it parses", () => {
    expect(formatTokenCount(null)).toBe("");
    expect(formatTokenCount(1_000_000)).toBe("1M");
    expect(formatTokenCount(200_000)).toBe("200K");
    expect(formatTokenCount(131_072)).toBe("131072");
    expect(formatTokenCount(1_500)).toBe("1500");
    for (const n of [1, 999, 1_000, 8_192, 128_000, 131_072, 1_000_000, 1_048_576, 2_000_000]) {
      expect(parseTokenCount(formatTokenCount(n))).toEqual({ ok: true, value: n });
    }
  });
});

describe("model rows", () => {
  const rows = (...ms: AiModel[]): ModelDraft[] => ms.map(toDraft);

  it("adds new models with their limits", () => {
    const next = addModels([], [model("gpt-5", { name: "GPT-5", context_window: 400_000, max_output_tokens: 128_000 })]);
    expect(next).toEqual([draft("gpt-5", "GPT-5", "400K", "128K")]);
  });

  it("keeps what the user entered for a model that is already there", () => {
    const current = [draft("m", "My Model", "64K", "", { efforts: ["low"] })];
    const next = addModels(current, [model("m", { name: "Fetched", context_window: 200_000, max_output_tokens: 8_192, efforts: ["high"], adaptive_thinking: true })]);
    // AI-05: levels the user set stay, and only unknown capabilities are filled in.
    expect(next).toEqual([draft("m", "My Model", "64K", "8192", { efforts: ["low"], adaptive: true })]);
    expect(current[0].output).toBe("");
  });

  it("carries thinking levels from the list, lowest first (AI-05)", () => {
    const listed = model("claude-opus-5-5", { efforts: ["max", "low", "high"], adaptive_thinking: true });
    expect(toDraft(listed)).toMatchObject({ efforts: ["low", "high", "max"], adaptive: true });
    expect(toModels([toDraft(listed)])[0]).toMatchObject({ efforts: ["low", "high", "max"], adaptive_thinking: true });
    expect(toModels([toDraft(typedModel("gpt-5"))])[0]).toMatchObject({ efforts: null, adaptive_thinking: null });
    expect(toModels([draft("none", "none", "", "", { efforts: [] })])[0].efforts).toEqual([]);
  });

  it("fills unknown levels of rows already in the form from a fetched list", () => {
    const rows = [draft("claude-opus-5-5", "Opus", "", ""), draft("set", "Set", "", "", { efforts: ["low"] }), draft("gone", "Gone", "", "")];
    const list = [model("claude-opus-5-5", { efforts: ["low", "medium"], adaptive_thinking: true }), model("set", { efforts: ["max"] })];
    const next = withListedCapabilities(rows, list);
    expect(next.map((d) => [d.id, d.efforts, d.adaptive])).toEqual([
      ["claude-opus-5-5", ["low", "medium"], true],
      ["set", ["low"], null],
      ["gone", null, null],
    ]);
    expect(rows[0].efforts).toBeNull();
    // Nothing to fill: the same rows come back, so the form does not change.
    expect(withListedCapabilities(next, list)).toBe(next);
  });

  it("ticks levels and describes them", () => {
    expect(toggleEffort(["low", "medium", "high"], "max", true)).toEqual(["low", "medium", "high", "max"]);
    expect(toggleEffort(["low", "medium"], "low", false)).toEqual(["medium"]);
    expect(toggleEffort(["high"], "high", true)).toEqual(["high"]);
    const label = (e: string) => e.toUpperCase();
    expect(formatEfforts(["low", "medium", "high"], label, "none")).toBe("LOW–HIGH");
    expect(formatEfforts(["low", "medium", "high", "max"], label, "none")).toBe("LOW–HIGH · MAX");
    expect(formatEfforts(["max", "low"], label, "none")).toBe("LOW · MAX");
    expect(formatEfforts(["xhigh"], label, "none")).toBe("XHIGH");
    expect(formatEfforts([], label, "none")).toBe("none");
  });

  it("names a typed ID after itself and skips blanks", () => {
    const next = addModels(rows(model("a")), [model("b", { name: "" }), model(" "), model("a")]);
    expect(next.map((d) => [d.id, d.name])).toEqual([
      ["a", "a"],
      ["b", "b"],
    ]);
  });

  it("removes by ID", () => {
    expect(removeModel(rows(model("a"), model("b")), "a").map((d) => d.id)).toEqual(["b"]);
  });

  it("turns rows into models", () => {
    const models = toModels([draft(" claude-x ", "", "1M", "64k"), draft("", "ghost", "", ""), draft("local", "Local", "bogus", "")]);
    expect(models).toEqual([
      model("claude-x", { context_window: 1_000_000, max_output_tokens: 64_000 }),
      model("local", { name: "Local" }),
    ]);
  });

  it("finds empty and duplicate IDs and bad limits", () => {
    const problems = validateModels([draft("a", "a", "", ""), draft("a ", "a", "x", "-1"), draft(" ", "", "", "")]);
    expect(problems).toEqual([null, { id: "duplicate", context: "invalid", output: "invalid" }, { id: "empty" }]);
  });

  it("splits pasted IDs", () => {
    expect(parseModelIds("gpt-5, gpt-5-mini\ngpt-5  gpt-5;o3")).toEqual(["gpt-5", "gpt-5-mini", "o3"]);
    expect(parseModelIds("  ")).toEqual([]);
  });

  it("filters by words in the ID or name", () => {
    const m = model("claude-sonnet-5-5", { name: "Claude Sonnet 5.5" });
    expect(matchesFilter(m, "")).toBe(true);
    expect(matchesFilter(m, "sonnet 5.5")).toBe(true);
    expect(matchesFilter(m, "SONNET opus")).toBe(false);
  });
});

describe("default model", () => {
  const providers = [provider("a", [model("a1"), model("a2")]), provider("b", []), provider("c", [model("c1")])];

  it("lists every model with its provider", () => {
    expect(modelChoices(providers).map((c) => `${c.providerName}/${c.ref.model_id}`)).toEqual(["A/a1", "A/a2", "C/c1"]);
    expect(firstModelRef(providers)).toEqual({ provider_id: "a", model_id: "a1" });
    expect(firstModelRef([provider("b", [])])).toBeNull();
  });

  const search: SearchProviderView[] = [{ id: "s1", kind: "brave", base_url: null, has_api_key: true, updated_at: 5 }];

  it("keeps a default that still exists", () => {
    const settings = { default_model: { provider_id: "c", model_id: "c1" }, default_effort: "max" as const, search_provider_id: "s1", builtin_skill_enabled: false, custom_instructions: "" };
    expect(sanitizeSettings(settings, providers, search)).toEqual(settings);
  });

  it("drops a default whose provider, model or search provider is gone", () => {
    expect(
      sanitizeSettings({ default_model: { provider_id: "z", model_id: "a1" }, default_effort: "low", search_provider_id: "s9", builtin_skill_enabled: true, custom_instructions: "" }, providers, search),
    ).toEqual({
      default_model: null,
      default_effort: "low",
      search_provider_id: null,
      builtin_skill_enabled: true,
      custom_instructions: "",
    });
    expect(
      sanitizeSettings({ default_model: { provider_id: "a", model_id: "gone" }, default_effort: null, search_provider_id: null, builtin_skill_enabled: true, custom_instructions: "" }, providers, search)
        .default_model,
    ).toBeNull();
  });

  it("tells a changed default thinking level apart (AI-05)", () => {
    const base = { default_model: null, default_effort: null, search_provider_id: null, builtin_skill_enabled: true, custom_instructions: "" };
    expect(sameSettings(base, { ...base })).toBe(true);
    expect(sameSettings(base, { ...base, default_effort: "high" })).toBe(false);
  });

  it("keeps and compares the custom instructions (AI-36)", () => {
    const base = { default_model: null, default_effort: null, search_provider_id: "gone", builtin_skill_enabled: true, custom_instructions: "Answer in English." };
    expect(sanitizeSettings(base, providers, search).custom_instructions).toBe("Answer in English.");
    expect(sameSettings(base, { ...base, custom_instructions: "Answer in English. " })).toBe(false);
  });
});

describe("searchProviderFor", () => {
  const list: SearchProviderView[] = [
    { id: "old", kind: "tavily", base_url: null, has_api_key: true, updated_at: 1 },
    { id: "new", kind: "tavily", base_url: null, has_api_key: true, updated_at: 9 },
    { id: "b", kind: "brave", base_url: null, has_api_key: false, updated_at: 3 },
  ];

  it("finds the provider of a kind", () => {
    expect(searchProviderFor(list, "brave", null)?.id).toBe("b");
    expect(searchProviderFor(list, "searxng", null)).toBeNull();
  });

  it("prefers the chosen one, then the newest", () => {
    expect(searchProviderFor(list, "tavily", null)?.id).toBe("new");
    expect(searchProviderFor(list, "tavily", "old")?.id).toBe("old");
    expect(searchProviderFor(list, "tavily", "b")?.id).toBe("new");
  });
});

describe("parseToolLimit", () => {
  it("accepts whole numbers and clamps them to 1-200", () => {
    expect(parseToolLimit("25")).toBe(25);
    expect(parseToolLimit(" 7 ")).toBe(7);
    expect(parseToolLimit("0")).toBe(1);
    expect(parseToolLimit("500")).toBe(200);
  });

  it("rejects anything that is not a number", () => {
    for (const bad of ["", "abc", "-3", "2.5", "1e2"]) expect(parseToolLimit(bad), bad).toBeNull();
  });
});
