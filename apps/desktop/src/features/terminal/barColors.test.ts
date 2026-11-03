import { describe, expect, it } from "vitest";
import { contrast, SCHEMES, schemeFor } from "@/session/schemes";
import { barColors } from "./barColors";

describe("barColors", () => {
  it("keeps the bar's text at AA on every scheme", () => {
    for (const scheme of SCHEMES) {
      const { background, foreground } = scheme.colors;
      const c = barColors(background, foreground, scheme.mode);
      expect(contrast(c.text, background), scheme.id).toBeGreaterThanOrEqual(4.5);
      expect(contrast(c.secondary, background), scheme.id).toBeGreaterThanOrEqual(4.5);
      expect(contrast(c.secondary, background), scheme.id).toBeLessThanOrEqual(contrast(c.text, background));
      expect(contrast(c.muted, background), scheme.id).toBeLessThan(contrast(c.secondary, background));
    }
  });

  it("takes the scheme's foreground as it is when it reads well", () => {
    const { background, foreground } = schemeFor("dark", "dracula").colors;
    expect(barColors(background, foreground, "dark").text).toBe(foreground);
  });

  it("darkens a light scheme's foreground that reads below AA", () => {
    const { background, foreground } = schemeFor("light", "solarized-light").colors;
    const { text } = barColors(background, foreground, "light");
    expect(contrast(foreground, background)).toBeLessThan(4.5);
    expect(text).not.toBe(foreground);
    expect(contrast(text, background)).toBeGreaterThanOrEqual(4.5);
  });

  it("leaves colors it can't read to CSS", () => {
    const c = barColors("rgb(0 0 0)", "#ffffff", "dark");
    expect(c.text).toBe("#ffffff");
    expect(c.secondary).toBe("color-mix(in srgb, #ffffff 72%, rgb(0 0 0))");
  });
});
