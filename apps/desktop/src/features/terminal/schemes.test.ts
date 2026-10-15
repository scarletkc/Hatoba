import { describe, expect, it } from "vitest";
import type { TerminalSchemeView } from "@/ipc/types";
import { customScheme, DEFAULT_SCHEME, modeOf, paletteOf, SCHEMES, schemeFor, schemesFor, searchColors, toHex } from "./schemes";

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

  it("convert the colours a palette holds to #rrggbb", () => {
    expect(toHex("#ABC")).toBe("#aabbcc");
    expect(toHex(" #0D0E10 ")).toBe("#0d0e10");
    expect(toHex("#ff000080", "#000000")).toBe("#800000");
    expect(toHex("rgba(122, 183, 255, 0.3)", "#0d0e10")).toBe("#2e4158");
    expect(toHex("rgb(1 2 3)")).toBe("#010203");
    expect(toHex("red")).toBeNull();
    expect(toHex("rgb(300, 0, 0)")).toBeNull();
    expect(modeOf("#fdf6e3")).toBe("light");
    expect(modeOf("#002b36")).toBe("dark");
    // A copy of Hatoba Dark keeps its translucent selection as it looks over the background.
    const copy = paletteOf(schemeFor("dark", null));
    expect(copy.selection).toBe("#2e4158");
    for (const value of Object.values(copy)) expect(value).toMatch(HEX);
  });

  it("read the user's schemes, and fall back where one is missing or has a colour this build cannot read", () => {
    const dracula = paletteOf(schemeFor("dark", "dracula"));
    const view: TerminalSchemeView = {
      id: "0192f0c4-7a9b-7cc1-8d1e-3f5a6b7c8d9e",
      name: "Mine",
      mode: "dark",
      colors: { ...dracula, red: "", green: "oklch(70% 0.1 140)" },
      updated_at: 1,
    };
    const mine = customScheme(view);
    expect(mine).toMatchObject({ id: view.id, name: "Mine", mode: "dark", contrast: 1, custom: true });
    expect(mine.colors.background).toBe("#282a36");
    expect(mine.colors.selectionBackground).toBe(dracula.selection);
    expect(mine.colors.brightMagenta).toBe(dracula.bright_magenta);
    const hatoba = paletteOf(schemeFor("dark", null));
    expect(mine.colors.red).toBe(hatoba.red);
    expect(mine.colors.green).toBe(hatoba.green);
    expect(customScheme({ ...view, mode: "light" }).contrast).toBeGreaterThan(1);

    // Picked by its id among the built-in ones, under its own mode only; deleted, it reads as the default.
    expect(schemeFor("dark", view.id, [mine])).toBe(mine);
    expect(schemeFor("light", view.id, [mine]).id).toBe(DEFAULT_SCHEME.light);
    expect(schemeFor("dark", view.id, []).id).toBe(DEFAULT_SCHEME.dark);
    expect(schemesFor("dark", [mine]).at(-1)).toBe(mine);
    expect(schemesFor("light", [mine])).not.toContain(mine);
  });

  it("derive search highlights xterm can draw", () => {
    for (const s of SCHEMES) {
      for (const value of Object.values(searchColors(s))) expect(value, s.id).toMatch(HEX);
    }
    expect(searchColors(schemeFor("dark", "nord")).matchBackground).toBe("#676157");
  });
});
