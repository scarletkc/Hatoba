import { EFFORTS, sortEfforts } from "@/features/ai/effort";
import { chats } from "@/features/ai/models";
import type {
  AiEffort,
  AiModel,
  AiModelRef,
  AiProtocol,
  AiProviderView,
  AiSettingsPatch,
  AiSettingsView,
  AiTestFailure,
  SearchKind,
  SearchProviderView,
} from "@/ipc/types";

/* Pure logic behind Settings → AI (spec §13.2, §13.4, §13.5): no React, no i18n. */

// ───────────────────────── Base URL and protocol (AI-01, AI-02) ─────────────────────────

/** The URL if it is an absolute `http` or `https` address, else null. */
export function parseBaseUrl(text: string): URL | null {
  const value = text.trim();
  if (!/^https?:\/\//i.test(value)) return null;
  try {
    const url = new URL(value);
    return url.hostname ? url : null;
  } catch {
    return null;
  }
}

/** What the backend stores and appends paths to: trimmed, without trailing slashes. */
export function normalizeBaseUrl(text: string): string {
  return text.trim().replace(/\/+$/, "");
}

/**
 * AI-01: the protocol the form preselects while the user has not chosen one. `anthropic` for
 * `api.anthropic.com` and for base URLs whose path ends in `/anthropic`; `system_one` (AI-47) for
 * `api.typesafe.ai`, a path that ends in `/systemone`, and a Workers AI path with `/ai/run`;
 * `chat_completions` otherwise. Text that is not a URL yet (the user is still typing) gets the default.
 */
export function suggestProtocol(baseUrl: string): AiProtocol {
  const value = baseUrl.trim();
  if (!value) return "chat_completions";
  let url: URL;
  try {
    url = new URL(/^[a-z][a-z0-9+.-]*:\/\//i.test(value) ? value : `https://${value}`);
  } catch {
    return "chat_completions";
  }
  const host = url.hostname.toLowerCase();
  const path = url.pathname.replace(/\/+$/, "").toLowerCase();
  if (host === "api.anthropic.com" || path.endsWith("/anthropic")) return "anthropic";
  if (host === "api.typesafe.ai" || path.endsWith("/systemone") || /\/ai\/run(\/|$)/.test(path)) return "system_one";
  return "chat_completions";
}

// ───────────────────────── Saved secrets (AI-01, HOST-08) ─────────────────────────

/**
 * What the user did with an API key field. A key that is saved is never loaded into the page, so
 * the field either keeps it, replaces it with `value`, or clears it.
 */
export type KeyMode = "keep" | "replace" | "clear";

export interface KeyDraft {
  mode: KeyMode;
  value: string;
}

/**
 * The `api_key` of `AiProviderInput` / `SearchProviderInput`: null keeps (or tests with) the saved
 * key, "" clears it, anything else replaces it. An empty replacement keeps the saved key.
 */
export function keyForInput(draft: KeyDraft, hasSaved: boolean): string | null {
  const typed = draft.value.trim();
  if (hasSaved && draft.mode === "clear") return "";
  if (hasSaved && draft.mode === "keep") return null;
  return typed === "" ? null : typed;
}

// ───────────────────────── Models (AI-03) ─────────────────────────

/** A model row while it is being edited: the token limits are text until the form is saved. */
export interface ModelDraft {
  id: string;
  name: string;
  context: string;
  output: string;
  /** The thinking levels it accepts (AI-05), null when unknown. */
  efforts: AiEffort[] | null;
  /** Adaptive thinking, from the provider's list; the form does not edit it. */
  adaptive: boolean | null;
}

export type TokenCount = { ok: true; value: number | null } | { ok: false };

const MAX_TOKENS = 1_000_000_000;

/**
 * A token limit as typed: empty (unknown), digits with optional separators ("128,000"), or a
 * number with a `K` or `M` suffix ("200K", "1.5M").
 */
export function parseTokenCount(text: string): TokenCount {
  const value = text.trim().replace(/[\s,_]/g, "").toLowerCase();
  if (value === "") return { ok: true, value: null };
  const m = /^(\d+(?:\.\d+)?)([km]?)$/.exec(value);
  if (!m) return { ok: false };
  const factor = m[2] === "k" ? 1_000 : m[2] === "m" ? 1_000_000 : 1;
  const n = Number(m[1]) * factor;
  const whole = Math.round(n);
  // "1.5K" is 1,500, but "1.5" is not a number of tokens.
  if (Math.abs(n - whole) > 1e-6) return { ok: false };
  return whole >= 1 && whole <= MAX_TOKENS ? { ok: true, value: whole } : { ok: false };
}

/** A token limit for a field: whole thousands and millions are abbreviated, and `parseTokenCount` reads it back. */
export function formatTokenCount(n: number | null): string {
  if (n === null) return "";
  if (n >= 1_000_000 && n % 1_000_000 === 0) return `${n / 1_000_000}M`;
  if (n >= 1_000 && n % 1_000 === 0) return `${n / 1_000}K`;
  return String(n);
}

export function toDraft(m: AiModel): ModelDraft {
  return {
    id: m.id,
    name: m.name,
    context: formatTokenCount(m.context_window),
    output: formatTokenCount(m.max_output_tokens),
    efforts: m.efforts ? sortEfforts(m.efforts) : null,
    adaptive: m.adaptive_thinking,
  };
}

/** A model typed in by its id: nothing else is known about it. */
export function typedModel(id: string): AiModel {
  return { id, name: id, context_window: null, max_output_tokens: null, efforts: null, adaptive_thinking: null };
}

/**
 * The models to send to the backend. Rows without an ID are dropped, an empty display name falls
 * back to the ID, and a limit that does not parse counts as unknown. Use {@link validateModels}
 * first when saving.
 */
export function toModels(drafts: readonly ModelDraft[]): AiModel[] {
  return drafts
    .filter((d) => d.id.trim() !== "")
    .map((d) => {
      const id = d.id.trim();
      const ctx = parseTokenCount(d.context);
      const out = parseTokenCount(d.output);
      return {
        id,
        name: d.name.trim() || id,
        context_window: ctx.ok ? ctx.value : null,
        max_output_tokens: out.ok ? out.value : null,
        efforts: d.efforts ? sortEfforts(d.efforts) : null,
        adaptive_thinking: d.adaptive,
      };
    });
}

export interface ModelProblem {
  id?: "empty" | "duplicate";
  context?: "invalid";
  output?: "invalid";
}

/** The problems of each row (null when it is fine), index for index with `drafts`. */
export function validateModels(drafts: readonly ModelDraft[]): (ModelProblem | null)[] {
  const seen = new Set<string>();
  return drafts.map((d) => {
    const problem: ModelProblem = {};
    const id = d.id.trim();
    if (id === "") problem.id = "empty";
    else if (seen.has(id)) problem.id = "duplicate";
    seen.add(id);
    if (!parseTokenCount(d.context).ok) problem.context = "invalid";
    if (!parseTokenCount(d.output).ok) problem.output = "invalid";
    return Object.keys(problem).length ? problem : null;
  });
}

/**
 * Adds models (typed IDs, or picked from the provider's list) to the rows. A model that is already
 * there keeps what the user entered; only a limit that is still empty, and thinking levels or adaptive
 * thinking that are still unknown, are filled in.
 */
export function addModels(drafts: readonly ModelDraft[], incoming: readonly AiModel[]): ModelDraft[] {
  const rows = drafts.map((d) => ({ ...d }));
  for (const m of incoming) {
    const id = m.id.trim();
    if (!id) continue;
    const existing = rows.find((d) => d.id.trim() === id);
    if (existing) {
      if (existing.context.trim() === "") existing.context = formatTokenCount(m.context_window);
      if (existing.output.trim() === "") existing.output = formatTokenCount(m.max_output_tokens);
      fillCapabilities(existing, m);
    } else {
      rows.push(toDraft({ ...m, id, name: m.name.trim() || id }));
    }
  }
  return rows;
}

function fillCapabilities(row: ModelDraft, m: AiModel): boolean {
  const efforts = row.efforts === null && m.efforts !== null;
  const adaptive = row.adaptive === null && m.adaptive_thinking !== null;
  if (efforts) row.efforts = sortEfforts(m.efforts ?? []);
  if (adaptive) row.adaptive = m.adaptive_thinking;
  return efforts || adaptive;
}

/**
 * AI-05: after the provider's list was fetched, the rows already in the form whose thinking levels or
 * adaptive thinking are unknown (typed in, or saved before the list had them) take them from the list.
 * Returns the same array when nothing changed.
 */
export function withListedCapabilities(drafts: ModelDraft[], list: readonly AiModel[]): ModelDraft[] {
  let changed = false;
  const rows = drafts.map((d) => {
    const listed = list.find((m) => m.id === d.id.trim());
    if (!listed) return d;
    const row = { ...d };
    if (!fillCapabilities(row, listed)) return d;
    changed = true;
    return row;
  });
  return changed ? rows : drafts;
}

/** Ticks or unticks one thinking level of a row; an unknown row starts from what it offers (Low to High). */
export function toggleEffort(current: readonly AiEffort[], effort: AiEffort, on: boolean): AiEffort[] {
  return sortEfforts(on ? [...current, effort] : current.filter((e) => e !== effort));
}

/**
 * A row's levels in a few words: runs of neighbouring levels as `first–last`, separate runs joined by
 * ` · ` ("Low–High · Max"), or `none` when it has none.
 */
export function formatEfforts(levels: readonly AiEffort[], label: (e: AiEffort) => string, none: string): string {
  const sorted = sortEfforts(levels);
  if (sorted.length === 0) return none;
  const runs: AiEffort[][] = [];
  for (const e of sorted) {
    const run = runs[runs.length - 1];
    if (run && EFFORTS.indexOf(e) === EFFORTS.indexOf(run[run.length - 1]) + 1) run.push(e);
    else runs.push([e]);
  }
  return runs.map((r) => (r.length === 1 ? label(r[0]) : `${label(r[0])}–${label(r[r.length - 1])}`)).join(" · ");
}

/**
 * AI-47: the rows as a System One provider keeps them. Its models only answer the judge's questions, so they have no
 * token limits or thinking levels, which the form hides and does not check.
 */
export function decisionDrafts(drafts: readonly ModelDraft[]): ModelDraft[] {
  return drafts.map((d) => ({ ...d, context: "", output: "", efforts: null, adaptive: null }));
}

export function removeModel(drafts: readonly ModelDraft[], id: string): ModelDraft[] {
  return drafts.filter((d) => d.id.trim() !== id);
}

/** Model IDs from pasted or typed text, separated by commas, spaces, or lines, without repeats. */
export function parseModelIds(text: string): string[] {
  return [...new Set(text.split(/[\s,;]+/).map((x) => x.trim()).filter(Boolean))];
}

/** Whether a model passes the picker's filter: every word is in its ID or display name. */
export function matchesFilter(m: AiModel, filter: string): boolean {
  const words = filter.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return true;
  const hay = `${m.id} ${m.name}`.toLowerCase();
  return words.every((w) => hay.includes(w));
}

// ───────────────────────── Default model and search provider (AI-05, AI-14) ─────────────────────────

export interface ModelChoice {
  ref: AiModelRef;
  providerName: string;
  model: AiModel;
}

function choicesOf(providers: readonly AiProviderView[]): ModelChoice[] {
  return providers.flatMap((p) =>
    p.models.map((model) => ({ ref: { provider_id: p.id, model_id: model.id }, providerName: p.name, model })),
  );
}

/** The models of every provider that chats, in provider order, for the default model selector (System One ones cannot, AI-47). */
export function modelChoices(providers: readonly AiProviderView[]): ModelChoice[] {
  return choicesOf(providers.filter(chats));
}

/** AI-47: every provider's models, System One ones included, for the judge of Auto mode. */
export function judgeChoices(providers: readonly AiProviderView[]): ModelChoice[] {
  return choicesOf(providers);
}

export function sameRef(a: AiModelRef | null, b: AiModelRef | null): boolean {
  return a === b || (!!a && !!b && a.provider_id === b.provider_id && a.model_id === b.model_id);
}

/**
 * Drops a default model, judge (AI-47) or search provider whose provider or model is gone, and a default model on a
 * provider that cannot chat.
 */
export function sanitizeSettings(
  settings: AiSettingsView,
  providers: readonly AiProviderView[],
  searchProviders: readonly SearchProviderView[],
): AiSettingsView {
  const listed = (choices: ModelChoice[], ref: AiModelRef | null) => (ref && choices.some((c) => sameRef(c.ref, ref)) ? ref : null);
  const defaultModel = listed(modelChoices(providers), settings.default_model);
  const judgeModel = listed(judgeChoices(providers), settings.judge_model);
  const searchId = settings.search_provider_id && searchProviders.some((p) => p.id === settings.search_provider_id) ? settings.search_provider_id : null;
  return {
    default_model: defaultModel,
    default_effort: settings.default_effort,
    search_provider_id: searchId,
    native_search: settings.native_search,
    builtin_skill_enabled: settings.builtin_skill_enabled,
    custom_instructions: settings.custom_instructions,
    send_terminal_screen: settings.send_terminal_screen,
    judge_model: judgeModel,
  };
}

/** The first model of the first provider that chats, as the default when none is chosen yet. */
export function firstModelRef(providers: readonly AiProviderView[]): AiModelRef | null {
  return modelChoices(providers)[0]?.ref ?? null;
}

/**
 * The settings to store after the providers changed: a default model, judge or search provider that
 * is gone is dropped, and a missing default becomes the first model there is that chats.
 */
export function resolveSettings(
  settings: AiSettingsView,
  providers: readonly AiProviderView[],
  searchProviders: readonly SearchProviderView[],
): AiSettingsView {
  const clean = sanitizeSettings(settings, providers, searchProviders);
  return clean.default_model ? clean : { ...clean, default_model: firstModelRef(providers) };
}

/** The fields of `next` that differ from `prev`, to save; null when none does. */
export function changedSettings(prev: AiSettingsView, next: AiSettingsView): AiSettingsPatch | null {
  const patch: AiSettingsPatch = {};
  if (!sameRef(prev.default_model, next.default_model)) patch.default_model = next.default_model;
  if (prev.default_effort !== next.default_effort) patch.default_effort = next.default_effort;
  if (prev.search_provider_id !== next.search_provider_id) patch.search_provider_id = next.search_provider_id;
  if (prev.native_search !== next.native_search) patch.native_search = next.native_search;
  if (prev.builtin_skill_enabled !== next.builtin_skill_enabled) patch.builtin_skill_enabled = next.builtin_skill_enabled;
  if (prev.custom_instructions !== next.custom_instructions) patch.custom_instructions = next.custom_instructions;
  if (prev.send_terminal_screen !== next.send_terminal_screen) patch.send_terminal_screen = next.send_terminal_screen;
  if (!sameRef(prev.judge_model, next.judge_model)) patch.judge_model = next.judge_model;
  return Object.keys(patch).length > 0 ? patch : null;
}

export const SEARCH_KINDS: readonly SearchKind[] = ["brave", "tavily", "searxng"];

/** SearXNG is an instance the user runs, so it needs an address and no key; Brave and Tavily need a key. */
export const searchNeedsUrl = (kind: SearchKind): boolean => kind === "searxng";

/**
 * The saved provider of a kind. Settings hold one provider per kind; when a sync produced several,
 * the chosen one wins, then the newest.
 */
export function searchProviderFor(
  list: readonly SearchProviderView[],
  kind: SearchKind,
  chosenId: string | null,
): SearchProviderView | null {
  const ofKind = list.filter((p) => p.kind === kind);
  return ofKind.find((p) => p.id === chosenId) ?? [...ofKind].sort((a, b) => b.updated_at - a.updated_at)[0] ?? null;
}

// ───────────────────────── Test results (AI-04) ─────────────────────────

/** The failure kind to explain; a failed result without one counts as `other`. */
export const failureOf = (failure: AiTestFailure | null): AiTestFailure => failure ?? "other";
