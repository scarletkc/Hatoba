import type { HatobaApi } from "../api";
import type {
  AiModel,
  AiProviderInput,
  AiProviderView,
  AiSettingsView,
  AiTestResult,
  AppError,
  SearchProviderInput,
  SearchProviderView,
} from "../types";

type AiSettingsApi = Pick<
  HatobaApi,
  | "ai_providers_list"
  | "ai_provider_save"
  | "ai_provider_delete"
  | "ai_provider_models"
  | "ai_provider_test"
  | "search_providers_list"
  | "search_provider_save"
  | "search_provider_delete"
  | "search_provider_test"
  | "ai_settings_get"
  | "ai_settings_save"
>;

/**
 * Mock of Settings → AI (providers, search provider, default model) for `pnpm dev`. URL parameters
 * (comma-separated, so `?ai=testfail,modelsfail` works) select demo states:
 *   ?ai=noprovider   no providers and no default model (the empty state)
 *   ?ai=testfail     Test Connection fails with an authentication error (401 and a provider message)
 *   ?ai=modelsfail   Fetch Models fails with an `ai` error (404)
 *   ?ai=search       a Brave Search provider is saved and chosen for web search
 * Without parameters there are two providers (Anthropic, and Ollama on localhost), so the default
 * model selector shows two groups. Inside the editor a few inputs trigger the other states:
 *   a base URL on a `.invalid` host        Test Connection fails with a network error
 *   a first model ID containing `nope`      Test Connection fails with an unknown model (404)
 *   an `http://` URL outside localhost      Test Connection reports an invalid URL, and saving is rejected
 *   a non-local provider with no API key    Test Connection fails with an authentication error
 *   a base URL containing `openrouter`      Fetch Models returns 150 models, to try the picker's filter
 * Fetch Models takes half a second and Test Connection 700 ms, so the busy states can be seen.
 * Nothing here is secure; it never runs inside the Tauri app.
 */
export function createAiSettingsMock(): AiSettingsApi {
  const flags = new Set((new URLSearchParams(location.search).get("ai") ?? "").split(",").map((x) => x.trim()));

  let providers: AiProviderView[] = flags.has("noprovider")
    ? []
    : [
        {
          id: "p-anthropic",
          name: "Anthropic",
          protocol: "anthropic",
          base_url: "https://api.anthropic.com",
          has_api_key: true,
          auth_header: "x-api-key",
          models: [
            { id: "claude-sonnet-5-5", name: "Claude Sonnet 5.5", context_window: 1_000_000, max_output_tokens: 128_000 },
            { id: "claude-haiku-5-5", name: "Claude Haiku 5.5", context_window: 1_000_000, max_output_tokens: 128_000 },
          ],
          updated_at: Date.now() - 86_400_000,
        },
        {
          id: "p-ollama",
          name: "Ollama (local)",
          protocol: "chat_completions",
          base_url: "http://localhost:11434/v1",
          has_api_key: false,
          auth_header: "x-api-key",
          models: [
            { id: "qwen3:8b", name: "qwen3:8b", context_window: 40_960, max_output_tokens: null },
            { id: "gpt-oss:20b", name: "gpt-oss:20b", context_window: null, max_output_tokens: null },
          ],
          updated_at: Date.now() - 3_600_000,
        },
      ];
  let search: SearchProviderView[] = flags.has("search")
    ? [{ id: "s-brave", kind: "brave", base_url: null, has_api_key: true, updated_at: Date.now() - 7_200_000 }]
    : [];
  let settings: AiSettingsView = {
    default_model: providers.length ? { provider_id: "p-anthropic", model_id: "claude-sonnet-5-5" } : null,
    search_provider_id: flags.has("search") ? "s-brave" : null,
  };

  const id = (p: string) => `${p}-${Math.random().toString(36).slice(2, 10)}`;
  const wait = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
  const clone = (p: AiProviderView): AiProviderView => ({ ...p, models: p.models.map((m) => ({ ...m })) });

  const hostOf = (url: string): string | null => {
    try {
      return new URL(url).hostname;
    } catch {
      return null;
    }
  };
  /** Loopback and private network addresses, the only ones that may use plain HTTP (AI-02). */
  const isLocalHost = (host: string) =>
    host === "localhost" ||
    host === "[::1]" ||
    /^127\./.test(host) ||
    /^10\./.test(host) ||
    /^192\.168\./.test(host) ||
    /^172\.(1[6-9]|2\d|3[01])\./.test(host);
  const urlProblem = (url: string): "invalid" | null => {
    const host = hostOf(url);
    if (!host || !/^https?:\/\//i.test(url.trim())) return "invalid";
    return /^http:/i.test(url.trim()) && !isLocalHost(host) ? "invalid" : null;
  };

  const savedKey = (input: { id: string | null; api_key: string | null }, list: { id: string; has_api_key: boolean }[]) =>
    input.api_key === null ? (list.find((p) => p.id === input.id)?.has_api_key ?? false) : input.api_key !== "";

  const claudeModels: AiModel[] = [
    { id: "claude-opus-5-1", name: "Claude Opus 5.1", context_window: 1_000_000, max_output_tokens: 128_000 },
    { id: "claude-sonnet-5-5", name: "Claude Sonnet 5.5", context_window: 1_000_000, max_output_tokens: 128_000 },
    { id: "claude-haiku-5-5", name: "Claude Haiku 5.5", context_window: 1_000_000, max_output_tokens: 128_000 },
    { id: "claude-sonnet-4-5-20250929", name: "Claude Sonnet 4.5", context_window: 200_000, max_output_tokens: 64_000 },
    { id: "claude-haiku-4-5-20251001", name: "Claude Haiku 4.5", context_window: 200_000, max_output_tokens: 64_000 },
  ];
  const plain = (ids: string[]): AiModel[] => ids.map((m) => ({ id: m, name: m, context_window: null, max_output_tokens: null }));

  const modelsFor = (input: AiProviderInput): AiModel[] => {
    const host = hostOf(input.base_url) ?? "";
    if (input.protocol === "anthropic") return claudeModels;
    if (/openrouter/i.test(input.base_url)) {
      return Array.from({ length: 150 }, (_, i) => {
        const vendor = ["openai", "anthropic", "google", "meta-llama", "mistralai", "qwen"][i % 6];
        return {
          id: `${vendor}/model-${String(i + 1).padStart(3, "0")}`,
          name: `${vendor} model ${i + 1}`,
          context_window: [8_192, 32_768, 131_072, 200_000, 1_000_000][i % 5],
          max_output_tokens: null,
        };
      });
    }
    if (isLocalHost(host)) return plain(["qwen3:8b", "qwen3:30b", "gpt-oss:20b", "llama3.1:8b", "gemma3:12b"]);
    return plain(["gpt-5", "gpt-5-mini", "gpt-5-nano", "gpt-4.1", "gpt-4.1-mini", "o3", "o4-mini"]);
  };

  const authFail = (protocol: "anthropic" | "chat_completions"): AiTestResult => ({
    ok: false,
    failure: "auth",
    status: 401,
    message:
      protocol === "anthropic"
        ? "invalid x-api-key"
        : "Incorrect API key provided: sk-proj-********a1b2. You can find your API key at https://platform.openai.com/account/api-keys.",
  });

  return {
    ai_providers_list: async () => providers.map(clone),
    ai_provider_save: async (input) => {
      if (urlProblem(input.base_url)) {
        const err: AppError = { code: "invalid_input", detail: "the base URL must use https unless it is a local address", field: "base_url" };
        throw err;
      }
      await wait(150);
      const old = providers.find((p) => p.id === input.id);
      const view: AiProviderView = {
        id: old?.id ?? id("p"),
        name: input.name,
        protocol: input.protocol,
        base_url: input.base_url,
        has_api_key: savedKey(input, providers),
        auth_header: input.auth_header,
        models: input.models.map((m) => ({ ...m })),
        updated_at: Date.now(),
      };
      providers = old ? providers.map((p) => (p.id === old.id ? view : p)) : [...providers, view];
      return clone(view);
    },
    ai_provider_delete: async (pid) => {
      providers = providers.filter((p) => p.id !== pid);
      if (settings.default_model?.provider_id === pid) settings = { ...settings, default_model: null };
    },
    ai_provider_models: async (input) => {
      await wait(500);
      if (flags.has("modelsfail")) {
        const err: AppError = { code: "ai", detail: "404 page not found", http_status: 404 };
        throw err;
      }
      return modelsFor(input).map((m) => ({ ...m }));
    },
    ai_provider_test: async (input) => {
      await wait(700);
      if (flags.has("testfail")) return authFail(input.protocol);
      if (urlProblem(input.base_url)) return { ok: false, failure: "invalid_url", status: null, message: null };
      const host = hostOf(input.base_url) ?? "";
      if (host.endsWith(".invalid")) {
        return { ok: false, failure: "network", status: null, message: `error sending request for url (${input.base_url}): dns error` };
      }
      const hasKey = savedKey(input, providers);
      if (!isLocalHost(host) && !hasKey && !(input.api_key ?? "")) {
        return { ok: false, failure: "auth", status: 401, message: "No API key provided." };
      }
      const first = input.models[0]?.id;
      if (first && /nope/i.test(first)) {
        return { ok: false, failure: "unknown_model", status: 404, message: `The model \`${first}\` does not exist or you do not have access to it.` };
      }
      return { ok: true, failure: null, status: 200, message: null };
    },
    search_providers_list: async () => search.map((s) => ({ ...s })),
    search_provider_save: async (input: SearchProviderInput) => {
      await wait(150);
      const old = search.find((s) => s.id === input.id);
      const view: SearchProviderView = {
        id: old?.id ?? id("s"),
        kind: input.kind,
        base_url: input.base_url,
        has_api_key: savedKey(input, search),
        updated_at: Date.now(),
      };
      search = old ? search.map((s) => (s.id === old.id ? view : s)) : [...search, view];
      return { ...view };
    },
    search_provider_delete: async (sid) => {
      search = search.filter((s) => s.id !== sid);
      if (settings.search_provider_id === sid) settings = { ...settings, search_provider_id: null };
    },
    search_provider_test: async (input) => {
      await wait(700);
      if (flags.has("testfail")) return { ok: false, failure: "auth", status: 401, message: "Invalid subscription token." };
      if (input.kind === "searxng" && ((input.base_url ?? "").includes(".invalid") || !hostOf(input.base_url ?? ""))) {
        return { ok: false, failure: "network", status: null, message: "error sending request: dns error" };
      }
      return { ok: true, failure: null, status: 200, message: null };
    },
    ai_settings_get: async () => structuredClone(settings),
    ai_settings_save: async (next) => {
      settings = structuredClone(next);
    },
  };
}
