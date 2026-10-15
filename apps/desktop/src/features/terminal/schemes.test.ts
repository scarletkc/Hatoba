import { describe, expect, it } from "vitest";
import { contrast, DEFAULT_SCHEME, MATCH_CONTRAST, SCHEMES, schemeFor, schemesFor, searchColors } from "./schemes";

const HEX = /^#[0-9a-f]{6}$/;
const ANSI = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"] as const;

describe("terminal colour schemes", () => {
  it("have unique ids the settings backend accepts", () => {
    const ids = SCHEMES.map((s) => s.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const id of ids) expect(id).toMatch(/^[a-z0-9-]{1,64}$/);
  });

  it("set every palette colour as #rrggbb", () => {
    for (const { id, colors } of SCHEMES) {
      const named = ["background", "foreground", "cursor", ...ANSI, ...ANSI.map((c) => `bright${c[0].toUpperCase()}${c.slice(1)}`)];
      for (const key of named) expect(colors[key as keyof typeof colors], `${id}.${key}`).toMatch(HEX);
    }
  });

  it("fall back to the mode's own scheme for an unset, unknown, or other-mode id", () => {
    expect(schemeFor("dark", null).id).toBe(DEFAULT_SCHEME.dark);
    expect(schemeFor("dark", "from-the-future").id).toBe(DEFAULT_SCHEME.dark);
    expect(schemeFor("light", "dracula").id).toBe(DEFAULT_SCHEME.light);
    expect(schemeFor("dark", "dracula").name).toBe("Dracula");
    expect(schemesFor("light")[0].id).toBe(DEFAULT_SCHEME.light);
    expect(schemesFor("dark").every((s) => s.mode === "dark")).toBe(true);
  });

  it("raise the contrast floor on light schemes only", () => {
    for (const s of SCHEMES) expect(s.contrast > 1, s.id).toBe(s.mode === "light");
  });

  it("derive search highlights xterm can draw", () => {
    for (const s of SCHEMES) {
      for (const value of Object.values(searchColors(s))) expect(value, s.id).toMatch(HEX);
    }
  });

  it("keep text readable on search highlights, the current match standing out", () => {
    for (const s of SCHEMES) {
      const { matchBackground, activeMatchBackground } = searchColors(s);
      const { foreground, background } = s.colors;
      expect(contrast(foreground, matchBackground), `${s.id} match`).toBeGreaterThanOrEqual(MATCH_CONTRAST);
      expect(contrast(foreground, activeMatchBackground), `${s.id} active match`).toBeGreaterThanOrEqual(MATCH_CONTRAST);
      if (!s.search) expect(contrast(background, activeMatchBackground), s.id).toBeGreaterThan(contrast(background, matchBackground));
    }
  });
});
