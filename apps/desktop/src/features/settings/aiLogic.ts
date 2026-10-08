import type {
  AiModel,
  AiModelRef,
  AiProtocol,
  AiProviderView,
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
 * `api.anthropic.com` and for base URLs whose path ends in `/anthropic`, `chat_completions` otherwise.
 * Text that is not a URL yet (the user is still typing) gets the default.
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
  const path = url.pathname.replace(/\/+$/, "").toLowerCase();
  return url.hostname.toLowerCase() === "api.anthropic.com" || path.endsWith("/anthropic") ? "anthropic" : "chat_completions";
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
  return { id: m.id, name: m.name, context: formatTokenCount(m.context_window), output: formatTokenCount(m.max_output_tokens) };
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
 * there keeps what the user entered; only a limit that is still empty is filled in.
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
    } else {
      rows.push(toDraft({ ...m, id, name: m.name.trim() || id }));
    }
  }
  return rows;
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

/** Every provider's models, in provider order, for the default model selector. */
export function modelChoices(providers: readonly AiProviderView[]): ModelChoice[] {
  return providers.flatMap((p) =>
    p.models.map((model) => ({ ref: { provider_id: p.id, model_id: model.id }, providerName: p.name, model })),
  );
}

export function sameRef(a: AiModelRef | null, b: AiModelRef | null): boolean {
  return a === b || (!!a && !!b && a.provider_id === b.provider_id && a.model_id === b.model_id);
}

/** Drops a default model or search provider whose provider or model is gone. */
export function sanitizeSettings(
  settings: AiSettingsView,
  providers: readonly AiProviderView[],
  searchProviders: readonly SearchProviderView[],
): AiSettingsView {
  const known = modelChoices(providers);
  const defaultModel = settings.default_model && known.some((c) => sameRef(c.ref, settings.default_model)) ? settings.default_model : null;
  const searchId = settings.search_provider_id && searchProviders.some((p) => p.id === settings.search_provider_id) ? settings.search_provider_id : null;
  return { default_model: defaultModel, search_provider_id: searchId };
}

/** The first model of the first provider, as the default when none is chosen yet. */
export function firstModelRef(providers: readonly AiProviderView[]): AiModelRef | null {
  return modelChoices(providers)[0]?.ref ?? null;
}

/**
 * The settings to store after the providers changed: a default model or search provider that is
 * gone is dropped, and a missing default becomes the first model there is.
 */
export function resolveSettings(
  settings: AiSettingsView,
  providers: readonly AiProviderView[],
  searchProviders: readonly SearchProviderView[],
): AiSettingsView {
  const clean = sanitizeSettings(settings, providers, searchProviders);
  return clean.default_model ? clean : { ...clean, default_model: firstModelRef(providers) };
}

export function sameSettings(a: AiSettingsView, b: AiSettingsView): boolean {
  return sameRef(a.default_model, b.default_model) && a.search_provider_id === b.search_provider_id;
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

// ───────────────────────── Device settings (AI-16, AI-18) ─────────────────────────

export const TOOL_LIMIT_MIN = 1;
export const TOOL_LIMIT_MAX = 200;
export const TOOL_LIMIT_DEFAULT = 25;

/** A tool call limit as typed, kept inside the allowed range; null when it is not a number. */
export function parseToolLimit(text: string): number | null {
  const value = text.trim();
  if (!/^\d{1,9}$/.test(value)) return null;
  return Math.min(TOOL_LIMIT_MAX, Math.max(TOOL_LIMIT_MIN, Number(value)));
}

// ───────────────────────── Test results (AI-04) ─────────────────────────

/** The failure kind to explain; a failed result without one counts as `other`. */
export const failureOf = (failure: AiTestFailure | null): AiTestFailure => failure ?? "other";
