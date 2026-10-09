import type { MessageKey } from "@/i18n";
import type { AiConversationView, AiEffort, AiModel, AiSettingsView } from "@/ipc/types";

/* AI-05: thinking levels. Rust decides what a request sends (`ModelSpec::effort_for` in hatoba-ai);
 * these mirror it so the selector shows the level a message is sent at. */

/** Every level, lowest first. */
export const EFFORTS: readonly AiEffort[] = ["low", "medium", "high", "xhigh", "max"];

/** What a model offers while its levels are unknown (typed in, or a list without them). */
export const UNKNOWN_EFFORTS: readonly AiEffort[] = ["low", "medium", "high"];

/** A slot's pick in the selector: a level, Default, or null to follow the conversation. */
export type EffortPick = AiEffort | "default" | null;

const rank = (e: AiEffort) => EFFORTS.indexOf(e);

/** The label of a level, or of Default (null). */
export const effortKey = (effort: AiEffort | null): MessageKey => `ai.effort.${effort ?? "default"}`;

/** The levels a model offers, lowest first; Default is always there too. */
export function modelEfforts(model: AiModel | null | undefined): readonly AiEffort[] {
  return model?.efforts ?? UNKNOWN_EFFORTS;
}

/** The level a request sends for `chosen`: the highest offered level not above it, or null (Default) when there is none. */
export function effectiveEffort(chosen: AiEffort | null, offered: readonly AiEffort[]): AiEffort | null {
  if (!chosen) return null;
  let best: AiEffort | null = null;
  for (const e of offered) if (rank(e) <= rank(chosen) && (best === null || rank(e) > rank(best))) best = e;
  return best;
}

/** Known levels only, lowest first, each once. */
export function sortEfforts(list: readonly AiEffort[]): AiEffort[] {
  return EFFORTS.filter((e) => list.includes(e));
}

/**
 * The level the next message of a slot is sent with: the one picked in the selector, else the one the
 * conversation used last, else, for a new conversation, the default from Settings → AI. A conversation
 * without a stored level is Default.
 */
export function chosenEffort(
  pick: EffortPick,
  conversationId: string | null,
  conversation: AiConversationView | null,
  settings: AiSettingsView | null,
): AiEffort | null {
  if (pick === "default") return null;
  if (pick) return pick;
  if (conversationId) return conversation?.effort ?? null;
  return settings?.default_effort ?? null;
}
