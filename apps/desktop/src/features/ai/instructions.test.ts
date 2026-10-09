import { describe, expect, it } from "vitest";
import { charCount, HOST_NOTES_MAX_CHARS, INSTRUCTIONS_MAX_CHARS, textSize } from "./instructions";

describe("custom instructions and host notes (AI-36, AI-37)", () => {
  it("counts characters the way Rust does", () => {
    expect(charCount("")).toBe(0);
    expect(charCount("abc")).toBe(3);
    expect(charCount("日本語")).toBe(3);
    // One code point, two UTF-16 units.
    expect(charCount("😀")).toBe(1);
  });

  it("estimates tokens like the context meter and flags text over the limit", () => {
    expect(textSize("Answer in English.", INSTRUCTIONS_MAX_CHARS)).toEqual({ chars: 18, tokens: 5, over: false });
    expect(textSize("x".repeat(INSTRUCTIONS_MAX_CHARS), INSTRUCTIONS_MAX_CHARS).over).toBe(false);
    expect(textSize("x".repeat(INSTRUCTIONS_MAX_CHARS + 1), INSTRUCTIONS_MAX_CHARS).over).toBe(true);
    // At the limit in characters, though longer in UTF-16 units.
    expect(textSize("😀".repeat(HOST_NOTES_MAX_CHARS), HOST_NOTES_MAX_CHARS)).toEqual({ chars: HOST_NOTES_MAX_CHARS, tokens: 1_000, over: false });
  });
});
