import { describe, expect, it } from "vitest";
import type { AiConversationView, AiEntryView, AiProviderView } from "@/ipc/types";
import { conversationModel, hasModels, sortConversations } from "./models";

const provider = (id: string, models: string[]): AiProviderView => ({
  id,
  name: id,
  protocol: "chat_completions",
  base_url: "https://example.com/v1",
  has_api_key: true,
  auth_header: "authorization",
  models: models.map((m) => ({ id: m, name: m, context_window: null, max_output_tokens: null, efforts: null, adaptive_thinking: null })),
  updated_at: 0,
});

const reply = (provider_id: string, model_id: string): AiEntryView => ({
  role: "assistant",
  entry_id: "e1",
  created_at: 0,
  provider_id,
  model_id,
  text: "",
  reasoning: null,
  tool_calls: [],
  finish: "stop",
  usage: null,
});

describe("conversationModel (AI-05)", () => {
  const providers = [provider("a", []), provider("b", ["b1", "b2"]), provider("c", ["c1"])];
  const settings = { default_model: { provider_id: "c", model_id: "c1" }, default_effort: null, search_provider_id: null, builtin_skill_enabled: true };

  it("uses the default model for a new conversation", () => {
    expect(conversationModel([], providers, settings)).toEqual({ provider_id: "c", model_id: "c1" });
  });

  it("keeps the model a conversation used last", () => {
    expect(conversationModel([reply("b", "b2")], providers, settings)).toEqual({ provider_id: "b", model_id: "b2" });
  });

  it("prefers the user's pick, and falls back when a model is gone", () => {
    expect(conversationModel([reply("b", "b2")], providers, settings, { provider_id: "b", model_id: "b1" })).toEqual({ provider_id: "b", model_id: "b1" });
    expect(conversationModel([reply("x", "gone")], providers, settings)).toEqual({ provider_id: "c", model_id: "c1" });
    expect(conversationModel([], providers, null)).toEqual({ provider_id: "b", model_id: "b1" });
    expect(conversationModel([], [provider("a", [])], null)).toBeNull();
  });

  it("knows whether any model exists", () => {
    expect(hasModels(providers)).toBe(true);
    expect(hasModels([provider("a", [])])).toBe(false);
  });
});

describe("sortConversations (AI-23)", () => {
  const conv = (id: string, pinned: boolean, last_activity: number): AiConversationView => ({
    id,
    title: id,
    host_id: null,
    pinned,
    context_start: null,
    effort: null,
    created_at: 0,
    updated_at: 0,
    last_activity,
  });

  it("puts pinned conversations first, then the newest", () => {
    const sorted = sortConversations([conv("old", false, 1), conv("pin-old", true, 0), conv("new", false, 9), conv("pin-new", true, 5)]);
    expect(sorted.map((c) => c.id)).toEqual(["pin-new", "pin-old", "new", "old"]);
  });
});
