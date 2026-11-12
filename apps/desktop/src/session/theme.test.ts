import { describe, expect, it, vi } from "vitest";
import { terminalFontFamily } from "./theme";

// theme.ts builds its settings store at import time, which reads the platform and listens for sync.
vi.mock("@/app/store", () => ({
  useApp: { getState: () => ({ info: { platform: "linux" } }), subscribe: () => () => {} },
  isDarkTheme: () => true,
}));
vi.mock("@/ipc/api", () => ({ api: { settings_get: async () => ({ terminal: {} }), listen: () => () => {} } }));

/** jsdom resolves no CSS custom properties, so the app's mono stack is the fallback of `cssVar`. */
const MONO = "Consolas, monospace";

describe("terminal font stack", () => {
  it("keeps a configured family that measures as a monospace", () => {
    expect(terminalFontFamily("Menlo", (family) => family === "Menlo" || family === "monospace")).toBe(
      `"Menlo", ${MONO}`,
    );
  });

  it("takes the first monospace when the configured family is not monospaced", () => {
    // Menlo on Linux: fontconfig substitutes a proportional face, so W and i do not measure alike.
    expect(terminalFontFamily("Menlo", (family) => family === "monospace")).toBe(`monospace, ${MONO}`);
  });

  it("leaves the app's stack alone when it already leads with a usable family", () => {
    expect(terminalFontFamily("Menlo", (family) => family === "Consolas")).toBe(MONO);
    expect(terminalFontFamily("", (family) => family === "Consolas")).toBe(MONO);
  });

  it("falls back to the family as typed when nothing can be measured", () => {
    expect(terminalFontFamily("Menlo", null)).toBe(`"Menlo", ${MONO}`);
    expect(terminalFontFamily("", null)).toBe(MONO);
    expect(terminalFontFamily('"Some Font", Menlo', null)).toBe(`"Some Font", Menlo, ${MONO}`);
  });
});
