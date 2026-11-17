import { describe, expect, it } from "vitest";
import { byteCount, charCount, HOST_NOTES_MAX_BYTES, INSTRUCTIONS_MAX_BYTES, textSize } from "./instructions";

describe("custom instructions and host notes (AI-36, AI-37)", () => {
  it("counts characters the way Rust does", () => {
    expect(charCount("")).toBe(0);
    expect(charCount("abc")).toBe(3);
    expect(charCount("日本語")).toBe(3);
    // One code point, two UTF-16 units.
    expect(charCount("😀")).toBe(1);
  });

  it("measures the limit in bytes of UTF-8, as Rust does", () => {
    expect(byteCount("abc")).toBe(3);
    expect(byteCount("日本語")).toBe(9);
    expect(byteCount("😀")).toBe(4);
  });

  it("estimates tokens like the context meter and flags text over the limit", () => {
    expect(textSize("Answer in English.", INSTRUCTIONS_MAX_BYTES)).toEqual({ bytes: 18, tokens: 5, over: false });
    expect(textSize("x".repeat(INSTRUCTIONS_MAX_BYTES), INSTRUCTIONS_MAX_BYTES).over).toBe(false);
    expect(textSize("x".repeat(INSTRUCTIONS_MAX_BYTES + 1), INSTRUCTIONS_MAX_BYTES).over).toBe(true);
    // Within the limit in characters, but over it in bytes.
    expect(textSize("日".repeat(HOST_NOTES_MAX_BYTES / 2), HOST_NOTES_MAX_BYTES).over).toBe(true);
  });
});
