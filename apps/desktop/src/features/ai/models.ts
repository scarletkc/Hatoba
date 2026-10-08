import type { AiConversationView, AiEntryView, AiModel, AiModelRef, AiProviderView, AiSettingsView } from "@/ipc/types";

export function sameModel(a: AiModelRef | null | undefined, b: AiModelRef | null | undefined): boolean {
  return !!a && !!b && a.provider_id === b.provider_id && a.model_id === b.model_id;
}

export function findModel(providers: AiProviderView[], ref: AiModelRef | null | undefined): { provider: AiProviderView; model: AiModel } | null {
  if (!ref) return null;
  const provider = providers.find((p) => p.id === ref.provider_id);
  const model = provider?.models.find((m) => m.id === ref.model_id);
  return provider && model ? { provider, model } : null;
}

/** Some provider has at least one model, so the panel can send. */
export function hasModels(providers: AiProviderView[]): boolean {
  return providers.some((p) => p.models.length > 0);
}

/** The model of the newest assistant entry. */
export function lastModel(entries: AiEntryView[]): AiModelRef | null {
  for (let i = entries.length - 1; i >= 0; i--) {
    const e = entries[i];
    if (e.role === "assistant") return { provider_id: e.provider_id, model_id: e.model_id };
  }
  return null;
}

/**
 * AI-05: the model for the next request. A conversation keeps the model it used last; a new one uses
 * the default from Settings → AI. A model that no longer exists falls back to the default, and
 * without a usable default to the first model of the first provider that has one.
 */
export function conversationModel(
  entries: AiEntryView[],
  providers: AiProviderView[],
  settings: AiSettingsView | null,
  chosen: AiModelRef | null = null,
): AiModelRef | null {
  for (const ref of [chosen, lastModel(entries), settings?.default_model ?? null]) {
    if (findModel(providers, ref)) return ref;
  }
  const first = providers.find((p) => p.models.length > 0);
  return first ? { provider_id: first.id, model_id: first.models[0].id } : null;
}

/** AI-23: pinned conversations first, then by last activity, newest first. */
export function sortConversations(list: AiConversationView[]): AiConversationView[] {
  return [...list].sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.last_activity - a.last_activity);
}
