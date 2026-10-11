/**
 * AI-36, AI-37: text the user writes for the assistant, which goes into the system prompt of every
 * request: custom instructions in Settings → AI, and a host's AI notes in the host editor. Rust
 * refuses longer text when it is saved (`MAX_CUSTOM_INSTRUCTIONS_BYTES`, `MAX_HOST_AI_NOTES_BYTES`
 * in crates/hatoba-core/src/model.rs).
 */

import { estimateTokens } from "./meter";

/** The longest custom instructions, in bytes of UTF-8 (AI-36). */
export const INSTRUCTIONS_MAX_BYTES = 32 * 1024;
/** The longest AI notes of a host, in bytes of UTF-8 (AI-37). */
export const HOST_NOTES_MAX_BYTES = 16 * 1024;

/** Characters as Rust counts them (Unicode code points), so an emoji counts once, not twice. */
export function charCount(text: string): number {
  return [...text].length;
}

/** Bytes of UTF-8, as Rust measures the limits above. */
export function byteCount(text: string): number {
  return new TextEncoder().encode(text).length;
}

export interface TextSize {
  bytes: number;
  /** The context meter's estimate (AI-20). */
  tokens: number;
  /** Longer than `max` bytes: it cannot be saved. */
  over: boolean;
}

export function textSize(text: string, max: number): TextSize {
  const bytes = byteCount(text);
  return { bytes, tokens: estimateTokens(text), over: bytes > max };
}
