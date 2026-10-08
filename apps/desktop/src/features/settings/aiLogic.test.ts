import { describe, expect, it } from "vitest";
import type { AiModel, AiProviderView, SearchProviderView } from "@/ipc/types";
import {
  addModels,
  firstModelRef,
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
  searchProviderFor,
  suggestProtocol,
  toDraft,
  toModels,
  validateModels,
  type ModelDraft,
} from "./aiLogic";

const model = (id: string, extra: Partial<AiModel> = {}): AiModel => ({
  id,
  name: id,
  context_window: null,
  max_output_tokens: null,
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
    expect(next).toEqual([{ id: "gpt-5", name: "GPT-5", context: "400K", output: "128K" }]);
  });

  it("keeps what the user entered for a model that is already there", () => {
    const current = [{ id: "m", name: "My Model", context: "64K", output: "" }];
    const next = addModels(current, [model("m", { name: "Fetched", context_window: 200_000, max_output_tokens: 8_192 })]);
    expect(next).toEqual([{ id: "m", name: "My Model", context: "64K", output: "8192" }]);
    expect(current[0].output).toBe("");
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
    const models = toModels([
      { id: " claude-x ", name: "", context: "1M", output: "64k" },
      { id: "", name: "ghost", context: "", output: "" },
      { id: "local", name: "Local", context: "bogus", output: "" },
    ]);
    expect(models).toEqual([
      { id: "claude-x", name: "claude-x", context_window: 1_000_000, max_output_tokens: 64_000 },
      { id: "local", name: "Local", context_window: null, max_output_tokens: null },
    ]);
  });

  it("finds empty and duplicate IDs and bad limits", () => {
    const problems = validateModels([
      { id: "a", name: "a", context: "", output: "" },
      { id: "a ", name: "a", context: "x", output: "-1" },
      { id: " ", name: "", context: "", output: "" },
    ]);
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
    const settings = { default_model: { provider_id: "c", model_id: "c1" }, search_provider_id: "s1", builtin_skill_enabled: false };
    expect(sanitizeSettings(settings, providers, search)).toEqual(settings);
  });

  it("drops a default whose provider, model or search provider is gone", () => {
    expect(sanitizeSettings({ default_model: { provider_id: "z", model_id: "a1" }, search_provider_id: "s9", builtin_skill_enabled: true }, providers, search)).toEqual({
      default_model: null,
      search_provider_id: null,
      builtin_skill_enabled: true,
    });
    expect(
      sanitizeSettings({ default_model: { provider_id: "a", model_id: "gone" }, search_provider_id: null, builtin_skill_enabled: true }, providers, search).default_model,
    ).toBeNull();
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
