/**
 * AI-36, AI-37: text the user writes for the assistant, which goes into the system prompt of every
 * request: custom instructions in Settings → AI, and a host's AI notes in the host editor. Rust
 * refuses longer text when it is saved (`MAX_CUSTOM_INSTRUCTIONS_CHARS`, `MAX_HOST_AI_NOTES_CHARS`
 * in crates/hatoba-core/src/model.rs).
 */

import { estimateTokens } from "./meter";

/** The longest custom instructions, in characters (AI-36). */
export const INSTRUCTIONS_MAX_CHARS = 4_000;
/** The longest AI notes of a host, in characters (AI-37). */
export const HOST_NOTES_MAX_CHARS = 2_000;

/** Characters as Rust counts them (Unicode code points), so an emoji counts once, not twice. */
export function charCount(text: string): number {
  return [...text].length;
}

export interface TextSize {
  chars: number;
  /** The context meter's estimate (AI-20). */
  tokens: number;
  /** Longer than `max`: it cannot be saved. */
  over: boolean;
}

export function textSize(text: string, max: number): TextSize {
  const chars = charCount(text);
  return { chars, tokens: estimateTokens(text), over: chars > max };
}
