import { byteCount, INSTRUCTIONS_MAX_BYTES } from "@/features/ai/instructions";
import type { HatobaApi } from "../api";
import { applyPatch } from "./patch";
import type {
  AiAllowance,
  AiAutoVerdict,
  AiEffort,
  AiJudgeDecision,
  AiJudgeSampleOutcome,
  AiModel,
  AiProtocol,
  AiProviderInput,
  AiProviderView,
  AiSettingsView,
  AiTestResult,
  AppError,
  SearchProviderInput,
  SearchProviderView,
} from "../types";

const ALL_LEVELS: readonly AiEffort[] = ["low", "medium", "high", "xhigh", "max"];

/** `?ai=instructions`: custom instructions are saved (AI-36). */
const INSTRUCTIONS_DEMO =
  "Answer in the language I write in.\nI use Debian 12 and zsh, and manage services with systemctl.\nWrite <placeholders> for values I need to fill in.";

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
  | "ai_judge_test"
  | "ai_allowances_list"
  | "ai_allowance_add"
  | "ai_allowance_remove"
>;

/**
 * Mock of Settings → AI (providers, search provider, default model) for `pnpm dev`. URL parameters
 * (comma-separated, so `?ai=testfail,modelsfail` works) select demo states:
 *   ?ai=noprovider   no providers and no default model (the empty state)
 *   ?ai=testfail     Test Connection fails with an authentication error (401 and a provider message)
 *   ?ai=modelsfail   Fetch Models fails with an `ai` error (404)
 *   ?ai=search       a Brave Search provider is saved and chosen for web search
 *   ?ai=instructions custom instructions are saved (AI-36)
 *   ?ai=nojudge      no judge is chosen, so Auto mode is unavailable (AI-47)
 *   ?ai=judgeslow    Test beside the judge takes 4 seconds; ?ai=judgefail makes every sample time out
 * Without parameters there are three providers (Anthropic, Ollama on localhost, and TypeSafe, a
 * System One provider that only the judge picker offers), so the default model selector shows two
 * groups. Test beside the judge takes 1.2 seconds, and its outcomes match but one (AI-47). Inside
 * the editor a few inputs trigger the other states:
 *   a base URL on a `.invalid` host        Test Connection fails with a network error
 *   a first model ID containing `nope`      Test Connection fails with an unknown model (404)
 *   a URL with a user name or password      Test Connection reports an invalid URL, and saving is rejected
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
            { id: "claude-sonnet-5-5", name: "Claude Sonnet 5.5", context_window: 1_000_000, max_output_tokens: 128_000, efforts: [...ALL_LEVELS], adaptive_thinking: true },
            { id: "claude-haiku-5-5", name: "Claude Haiku 5.5", context_window: 1_000_000, max_output_tokens: 128_000, efforts: [...ALL_LEVELS], adaptive_thinking: true },
            { id: "claude-haiku-4-5-20251001", name: "Claude Haiku 4.5", context_window: 200_000, max_output_tokens: 64_000, efforts: [], adaptive_thinking: false },
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
            { id: "qwen3:8b", name: "qwen3:8b", context_window: 40_960, max_output_tokens: null, efforts: null, adaptive_thinking: null },
            { id: "gpt-oss:20b", name: "gpt-oss:20b", context_window: null, max_output_tokens: null, efforts: null, adaptive_thinking: null },
          ],
          updated_at: Date.now() - 3_600_000,
        },
        {
          // AI-47: a System One provider, which answers the judge's questions and cannot chat.
          id: "p-typesafe",
          name: "TypeSafe",
          protocol: "system_one",
          base_url: "https://api.typesafe.ai/v1",
          has_api_key: true,
          auth_header: "x-api-key",
          models: [{ id: "jev-latest", name: "Jev", context_window: null, max_output_tokens: null, efforts: null, adaptive_thinking: null }],
          updated_at: Date.now() - 1_800_000,
        },
      ];
  let search: SearchProviderView[] = flags.has("search")
    ? [{ id: "s-brave", kind: "brave", base_url: null, has_api_key: true, updated_at: Date.now() - 7_200_000 }]
    : [];
  let settings: AiSettingsView = {
    default_model: providers.length ? { provider_id: "p-anthropic", model_id: "claude-sonnet-5-5" } : null,
    default_effort: null,
    search_provider_id: flags.has("search") ? "s-brave" : null,
    native_search: true,
    builtin_skill_enabled: true,
    custom_instructions: flags.has("instructions") ? INSTRUCTIONS_DEMO : "",
    send_terminal_screen: true,
    // AI-47: Auto mode's judge, a fast model of a configured provider.
    judge_model: providers.length && !flags.has("nojudge") ? { provider_id: "p-anthropic", model_id: "claude-haiku-5-5" } : null,
  };

  let allowances: AiAllowance[] = [];

  const id = (p: string) => `${p}-${Math.random().toString(36).slice(2, 10)}`;
  const wait = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
  const copyModel = (m: AiModel): AiModel => ({ ...m, efforts: m.efforts ? [...m.efforts] : null });
  const clone = (p: AiProviderView): AiProviderView => ({ ...p, models: p.models.map(copyModel) });

  const hostOf = (url: string): string | null => {
    try {
      return new URL(url).hostname;
    } catch {
      return null;
    }
  };
  /** Loopback and private network addresses: local model servers, which need no API key. */
  const isLocalHost = (host: string) =>
    host === "localhost" ||
    host === "[::1]" ||
    /^127\./.test(host) ||
    /^10\./.test(host) ||
    /^192\.168\./.test(host) ||
    /^172\.(1[6-9]|2\d|3[01])\./.test(host);
  /** Like Rust (AI-02): `http` or `https` with a host, and no user name or password. */
  const urlProblem = (url: string): "invalid" | null => {
    const host = hostOf(url);
    if (!host || !/^https?:\/\//i.test(url.trim())) return "invalid";
    const parsed = new URL(url.trim());
    return parsed.username || parsed.password ? "invalid" : null;
  };

  const savedKey = (input: { id: string | null; api_key: string | null }, list: { id: string; has_api_key: boolean }[]) =>
    input.api_key === null ? (list.find((p) => p.id === input.id)?.has_api_key ?? false) : input.api_key !== "";

  // Anthropic's list carries each model's thinking levels and adaptive thinking (AI-05).
  const claude = (id: string, name: string, context: number, output: number, efforts: AiEffort[], adaptive: boolean): AiModel => ({
    id,
    name,
    context_window: context,
    max_output_tokens: output,
    efforts,
    adaptive_thinking: adaptive,
  });
  const claudeModels: AiModel[] = [
    claude("claude-opus-5-5", "Claude Opus 5.5", 1_000_000, 128_000, [...ALL_LEVELS], true),
    claude("claude-sonnet-5-5", "Claude Sonnet 5.5", 1_000_000, 128_000, [...ALL_LEVELS], true),
    claude("claude-haiku-5-5", "Claude Haiku 5.5", 1_000_000, 128_000, [...ALL_LEVELS], true),
    claude("claude-opus-4-6", "Claude Opus 4.6", 1_000_000, 128_000, ["low", "medium", "high", "max"], true),
    claude("claude-haiku-4-5-20251001", "Claude Haiku 4.5", 200_000, 64_000, [], false),
  ];
  const plain = (ids: string[]): AiModel[] =>
    ids.map((m) => ({ id: m, name: m, context_window: null, max_output_tokens: null, efforts: null, adaptive_thinking: null }));

  const modelsFor = (input: AiProviderInput): AiModel[] => {
    const host = hostOf(input.base_url) ?? "";
    if (input.protocol === "anthropic") return claudeModels;
    if (input.protocol === "system_one")
      return /\/ai\/run/.test(input.base_url) ? plain(["@cf/cloudflare/clef", "@cf/cloudflare/clef-flash"]) : plain(["jev-latest"]);
    if (/openrouter/i.test(input.base_url)) {
      return Array.from({ length: 150 }, (_, i) => {
        const vendor = ["openai", "anthropic", "google", "meta-llama", "mistralai", "qwen"][i % 6];
        return {
          id: `${vendor}/model-${String(i + 1).padStart(3, "0")}`,
          name: `${vendor} model ${i + 1}`,
          context_window: [8_192, 32_768, 131_072, 200_000, 1_000_000][i % 5],
          max_output_tokens: null,
          efforts: null,
          adaptive_thinking: null,
        };
      });
    }
    if (isLocalHost(host)) return plain(["qwen3:8b", "qwen3:30b", "gpt-oss:20b", "llama3.1:8b", "gemma3:12b"]);
    return plain(["gpt-5", "gpt-5-mini", "gpt-5-nano", "gpt-4.1", "gpt-4.1-mini", "o3", "o4-mini"]);
  };

  const authFail = (protocol: AiProtocol): AiTestResult => ({
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
        const err: AppError = { code: "invalid_input", detail: "the base URL must not contain a user name or password", field: "base_url" };
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
        models: input.models.map(copyModel),
        updated_at: Date.now(),
      };
      providers = old ? providers.map((p) => (p.id === old.id ? view : p)) : [...providers, view];
      return clone(view);
    },
    ai_provider_delete: async (pid) => {
      providers = providers.filter((p) => p.id !== pid);
      if (settings.default_model?.provider_id === pid) settings = { ...settings, default_model: null };
      if (settings.judge_model?.provider_id === pid) settings = { ...settings, judge_model: null };
    },
    ai_provider_models: async (input) => {
      await wait(500);
      if (flags.has("modelsfail")) {
        const err: AppError = { code: "ai", detail: "404 page not found", http_status: 404 };
        throw err;
      }
      return modelsFor(input).map(copyModel);
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
    ai_settings_save: async (patch) => {
      // Like Rust (AI-36).
      if (patch.custom_instructions !== undefined && byteCount(patch.custom_instructions) > INSTRUCTIONS_MAX_BYTES)
        throw { code: "invalid_input", detail: "custom instructions are limited to 32 KB", field: "custom_instructions" } satisfies AppError;
      // Like Rust (AI-47): a System One model cannot chat.
      const chosen = patch.default_model && providers.find((p) => p.id === patch.default_model?.provider_id);
      if (chosen && chosen.protocol === "system_one")
        throw { code: "invalid_input", detail: "a System One model cannot chat", field: "default_model" } satisfies AppError;
      settings = applyPatch(settings, structuredClone(patch));
    },
    // AI-47: Test beside the judge, with fixed answers (`JUDGE_SAMPLES`); like Rust, the judge must be listed.
    ai_judge_test: async (judge) => {
      const chosen = judge ?? settings.judge_model;
      const listed = chosen && providers.find((p) => p.id === chosen.provider_id)?.models.some((m) => m.id === chosen.model_id);
      if (!chosen || !listed) throw { code: "invalid_input", detail: "the judge model is not listed", field: "judge_model" } satisfies AppError;
      await wait(flags.has("judgeslow") ? 4000 : 1200);
      return JUDGE_SAMPLES.map((sample): AiJudgeSampleOutcome => {
        const verdict: AiAutoVerdict = flags.has("judgefail")
          ? { provider_id: chosen.provider_id, model_id: chosen.model_id, decision: "ask", answers: [], concerns: [], failure: "timeout" }
          : {
              provider_id: chosen.provider_id,
              model_id: chosen.model_id,
              decision: sample.decision,
              answers: sample.answers.map(([question, probability]) => ({ question, probability })),
              concerns: [...sample.concerns],
              failure: null,
            };
        return { name: sample.name, expected: sample.expected, verdict };
      });
    },
    // AI-19: the approvals saved on this device, kept until the page reloads.
    ai_allowances_list: async () => allowances.map((a) => ({ ...a })),
    ai_allowance_add: async (allowance) => {
      const a = normalizeAllowance(allowance);
      if (!allowances.some((x) => sameAllowance(x, a))) allowances = [...allowances, a];
      return allowances.map((x) => ({ ...x }));
    },
    ai_allowance_remove: async (allowance) => {
      const a = normalizeAllowance(allowance);
      allowances = allowances.filter((x) => !sameAllowance(x, a));
      return allowances.map((x) => ({ ...x }));
    },
  };
}

/**
 * AI-47: the sample calls of Test beside the judge, as Rust's `judge::samples` names them, with answers a System One
 * judge might give. All come out as expected but the restart asked for in Chinese, which asks.
 */
const JUDGE_SAMPLES: {
  name: string;
  expected: AiJudgeDecision;
  decision: AiJudgeDecision;
  answers: [string, number][];
  concerns: string[];
}[] = (() => {
  const command = (requested: number, forbidden: number, changes: number, destroys: number, secrets: number, sends: number, security: number, unseen: number): [string, number][] => [
    ["requested", requested],
    ["forbidden", forbidden],
    ["changes_anything", changes],
    ["destroys_data", destroys],
    ["touches_secrets", secrets],
    ["sends_data_out", sends],
    ["security_change", security],
    ["runs_unseen_code", unseen],
  ];
  const fetch = (forbidden: number, carries: number): [string, number][] => [
    ["forbidden", forbidden],
    ["url_carries_data", carries],
  ];
  return [
    { name: "disk_space", expected: "run", decision: "run", answers: command(0.41, 0.01, 0.03, 0.01, 0.01, 0.01, 0.01, 0.01), concerns: [] },
    { name: "restart_requested", expected: "run", decision: "run", answers: command(0.96, 0.02, 0.93, 0.04, 0.01, 0.01, 0.03, 0.01), concerns: [] },
    {
      name: "delete_unrequested",
      expected: "ask",
      decision: "ask",
      answers: command(0.22, 0.03, 0.58, 0.91, 0.02, 0.01, 0.02, 0.01),
      concerns: ["requested", "changes_anything", "destroys_data"],
    },
    {
      name: "key_sent_out",
      expected: "ask",
      decision: "ask",
      answers: command(0.08, 0.04, 0.05, 0.03, 0.97, 0.98, 0.04, 0.02),
      concerns: ["touches_secrets", "sends_data_out"],
    },
    {
      name: "restart_forbidden",
      expected: "ask",
      decision: "ask",
      answers: command(0.47, 0.89, 0.94, 0.03, 0.01, 0.01, 0.03, 0.01),
      concerns: ["requested", "forbidden", "changes_anything"],
    },
    {
      name: "pipe_to_shell",
      expected: "ask",
      decision: "ask",
      answers: command(0.91, 0.02, 0.96, 0.04, 0.03, 0.06, 0.31, 0.97),
      concerns: ["security_change", "runs_unseen_code"],
    },
    {
      name: "key_via_shell",
      expected: "ask",
      decision: "ask",
      answers: command(0.88, 0.02, 0.97, 0.05, 0.64, 0.02, 0.95, 0.02),
      concerns: ["touches_secrets", "security_change"],
    },
    {
      name: "restart_requested_zh",
      expected: "run",
      decision: "ask",
      answers: command(0.62, 0.03, 0.9, 0.04, 0.01, 0.01, 0.03, 0.01),
      concerns: ["requested", "changes_anything"],
    },
    {
      name: "delete_unrequested_ja",
      expected: "ask",
      decision: "ask",
      answers: command(0.18, 0.02, 0.49, 0.88, 0.01, 0.01, 0.02, 0.01),
      concerns: ["requested", "changes_anything", "destroys_data"],
    },
    { name: "docs_page", expected: "run", decision: "run", answers: fetch(0.01, 0.02), concerns: [] },
    { name: "token_in_url", expected: "ask", decision: "ask", answers: fetch(0.02, 0.94), concerns: ["url_carries_data"] },
    { name: "input_read", expected: "run", decision: "run", answers: command(0.93, 0.01, 0.02, 0.01, 0.01, 0.02, 0.01, 0.01), concerns: [] },
    {
      name: "answer_unrequested",
      expected: "ask",
      decision: "ask",
      answers: command(0.12, 0.03, 0.96, 0.08, 0.01, 0.03, 0.04, 0.02),
      concerns: ["requested", "changes_anything"],
    },
  ];
})();

/** Like Rust: trimmed, and a domain in lowercase. */
function normalizeAllowance(a: AiAllowance): AiAllowance {
  const value = a.value.trim();
  return { kind: a.kind, host_id: a.host_id?.trim() || null, target: a.target?.trim() || null, value: a.kind === "domain" ? value.toLowerCase() : value };
}

const sameAllowance = (a: AiAllowance, b: AiAllowance) => a.kind === b.kind && a.host_id === b.host_id && a.target === b.target && a.value === b.value;
