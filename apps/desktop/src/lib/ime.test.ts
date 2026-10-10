import { describe, expect, it } from "vitest";
import { isImeEvent } from "./ime";

describe("isImeEvent", () => {
  it("is false for a plain Enter", () => {
    expect(isImeEvent(new KeyboardEvent("keydown", { key: "Enter", keyCode: 13 }))).toBe(false);
    expect(isImeEvent({ keyCode: 13, nativeEvent: { isComposing: false } })).toBe(false);
  });

  it("is true while a composition is open (Chromium)", () => {
    expect(isImeEvent(new KeyboardEvent("keydown", { key: "Enter", keyCode: 13, isComposing: true }))).toBe(true);
    expect(isImeEvent({ keyCode: 13, nativeEvent: { isComposing: true } })).toBe(true);
  });

  it("is true for keyCode 229 after compositionend (WKWebView)", () => {
    expect(isImeEvent(new KeyboardEvent("keydown", { key: "Enter", keyCode: 229, isComposing: false }))).toBe(true);
    expect(isImeEvent({ keyCode: 229, nativeEvent: { isComposing: false } })).toBe(true);
  });
});
