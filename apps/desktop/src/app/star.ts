import { useEffect, useReducer } from "react";
import { create } from "zustand";
import { api } from "@/ipc/api";
import type { StarPrompt } from "@/ipc/types";

/** How long after the first read of the state the prompt waits (spec §9). */
export const STAR_PROMPT_DELAY_MS = 24 * 60 * 60 * 1000;

interface StarPromptState {
  /** Null until loaded, or when loading failed. */
  prompt: StarPrompt | null;
  /** Loads the state once per launch. */
  load(): Promise<void>;
  /** The user starred, opened the bug report form, or closed the prompt: it never shows again. */
  finish(): void;
}

let loading: Promise<void> | null = null;

export const useStarPrompt = create<StarPromptState>((set, get) => ({
  prompt: null,
  load: () => {
    loading ??= api.star_prompt_get().then(
      (prompt) => set({ prompt }),
      () => {},
    );
    return loading;
  },
  finish: () => {
    const { prompt } = get();
    if (prompt) set({ prompt: { ...prompt, done: true } });
    void api.star_prompt_done().catch(() => {});
  },
}));

/** A day after the first read of the state, once a session has connected in this launch. */
export function starPromptDue(prompt: StarPrompt | null, connectedOnce: boolean, now: number): boolean {
  return prompt !== null && !prompt.done && connectedOnce && now - prompt.first_seen_at >= STAR_PROMPT_DELAY_MS;
}

/** {@link starPromptDue} now. A timer renders again when the day is up, since no state changes then. */
export function useStarPromptDue(connectedOnce: boolean): boolean {
  const prompt = useStarPrompt((s) => s.prompt);
  const [, recheck] = useReducer((n: number) => n + 1, 0);
  const dueAt = prompt && !prompt.done ? prompt.first_seen_at + STAR_PROMPT_DELAY_MS : null;
  useEffect(() => {
    const wait = dueAt === null ? 0 : dueAt - Date.now();
    if (wait <= 0) return;
    const timer = setTimeout(recheck, wait);
    return () => clearTimeout(timer);
  }, [dueAt]);
  return starPromptDue(prompt, connectedOnce, Date.now());
}
