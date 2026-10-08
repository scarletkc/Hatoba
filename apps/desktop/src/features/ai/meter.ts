import type { AiEntryView, AiModelRef } from "@/ipc/types";

/** The rough text-to-token ratio used for estimates (AI-20). */
export const CHARS_PER_TOKEN = 4;

/** At this share of the context window the meter warns and offers Compact (AI-21). */
export const WARN_RATIO = 0.8;

export function estimateTokens(text: string): number {
  return Math.ceil(text.length / CHARS_PER_TOKEN);
}

function entryText(e: AiEntryView): string {
  switch (e.role) {
    case "user":
    case "summary":
      return e.text;
    case "tool":
      return e.content;
    case "assistant":
      return e.text + e.tool_calls.map((c) => c.name + c.arguments).join("");
  }
}

/** The entries sent to the model: from `context_start` on (AI-21). */
export function contextEntries(entries: AiEntryView[], contextStart: string | null): AiEntryView[] {
  if (!contextStart) return entries;
  const i = entries.findIndex((e) => e.entry_id === contextStart);
  return i < 0 ? entries : entries.slice(i);
}

export interface MeterInput {
  entries: AiEntryView[];
  contextStart: string | null;
  /** The model of the next request. */
  model: AiModelRef | null;
  /** That model's context window, when known. */
  contextWindow: number | null;
  /** Text streamed but not stored yet. */
  pending?: string;
}

export interface MeterState {
  tokens: number;
  window: number | null;
  /** tokens / window, or null when the window is unknown. */
  ratio: number | null;
  /** Shown with ≈: the usage was estimated, nothing reported it yet, or the model changed since. */
  estimated: boolean;
  warn: boolean;
}

/**
 * AI-20: the last response's input and output tokens plus an estimate from text length for what was
 * added since. Without a reported usage in the context the whole count is an estimate, and after a
 * model switch it is one until the next response.
 */
export function contextUsage({ entries, contextStart, model, contextWindow, pending = "" }: MeterInput): MeterState {
  const inContext = contextEntries(entries, contextStart);
  let base = 0;
  let estimated = false;
  let rest = inContext;
  for (let i = inContext.length - 1; i >= 0; i--) {
    const e = inContext[i];
    if (e.role === "assistant" && e.usage) {
      base = e.usage.input_tokens + e.usage.output_tokens;
      estimated = e.usage.estimated;
      rest = inContext.slice(i + 1);
      break;
    }
  }
  if (rest === inContext && inContext.length > 0) estimated = true;
  const added = rest.reduce((n, e) => n + estimateTokens(entryText(e)), 0) + estimateTokens(pending);

  const last = [...entries].reverse().find((e) => e.role === "assistant");
  if (model && last?.role === "assistant" && (last.provider_id !== model.provider_id || last.model_id !== model.model_id)) estimated = true;

  const tokens = base + added;
  const window = contextWindow && contextWindow > 0 ? contextWindow : null;
  const ratio = window ? tokens / window : null;
  return { tokens, window, ratio, estimated, warn: ratio !== null && ratio >= WARN_RATIO };
}

/** Compact token counts: 950, 12.3k, 128k, 1.2M. */
export function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 10_000) return `${(n / 1000).toFixed(1).replace(/\.0$/, "")}k`;
  if (n < 1_000_000) return `${Math.round(n / 1000)}k`;
  return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
}
