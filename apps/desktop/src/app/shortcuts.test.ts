import { describe, expect, it } from "vitest";
import { isAppShortcut, matchShortcut } from "./shortcuts";

const key = (init: KeyboardEventInit & { code?: string }) => new KeyboardEvent("keydown", init);

describe("matchShortcut (WIN-04)", () => {
  it("uses Ctrl+Shift for app shortcuts on Windows", () => {
    expect(matchShortcut(key({ key: "K", code: "KeyK", ctrlKey: true, shiftKey: true }), "windows", true)).toBe("search");
    expect(matchShortcut(key({ key: "T", code: "KeyT", ctrlKey: true, shiftKey: true }), "windows", true)).toBe("newTab");
    expect(matchShortcut(key({ key: "W", code: "KeyW", ctrlKey: true, shiftKey: true }), "windows", true)).toBe("closeTab");
    expect(matchShortcut(key({ key: "L", code: "KeyL", ctrlKey: true, shiftKey: true }), "windows", true)).toBe("lock");
    expect(matchShortcut(key({ key: "A", code: "KeyA", ctrlKey: true, shiftKey: true }), "windows", true)).toBe("aiPanel");
    expect(matchShortcut(key({ key: "A", code: "KeyA", ctrlKey: true, shiftKey: true }), "linux", false)).toBe("aiPanel");
  });

  it("leaves plain Ctrl+letter to the terminal", () => {
    for (const k of ["l", "w", "k", "c", "v", "t", "a"]) {
      expect(matchShortcut(key({ key: k, ctrlKey: true }), "windows", true)).toBeNull();
    }
  });

  it("allows Ctrl+K for search only outside the terminal", () => {
    expect(matchShortcut(key({ key: "k", ctrlKey: true }), "windows", false)).toBe("search");
    expect(matchShortcut(key({ key: "k", ctrlKey: true }), "windows", true)).toBeNull();
  });

  it("cycles tabs and opens settings", () => {
    expect(matchShortcut(key({ key: "Tab", ctrlKey: true }), "windows", true)).toBe("nextTab");
    expect(matchShortcut(key({ key: "Tab", ctrlKey: true, shiftKey: true }), "windows", true)).toBe("prevTab");
    expect(matchShortcut(key({ key: ",", ctrlKey: true }), "windows", true)).toBe("settings");
  });

  it("uses ⌘ on macOS", () => {
    expect(matchShortcut(key({ key: "k", metaKey: true }), "macos", false)).toBe("search");
    expect(matchShortcut(key({ key: "l", metaKey: true }), "macos", true)).toBe("lock");
    expect(matchShortcut(key({ key: "k", ctrlKey: true, shiftKey: true, code: "KeyK" }), "macos", true)).toBeNull();
  });

  it("leaves ⌘K to the terminal, which clears itself, while it has focus on macOS (§9.1)", () => {
    expect(matchShortcut(key({ key: "k", metaKey: true }), "macos", true)).toBeNull();
    expect(matchShortcut(key({ key: "k", metaKey: true }), "macos", false)).toBe("search");
    expect(isAppShortcut(key({ key: "k", metaKey: true }), "macos")).toBe(false);
    expect(isAppShortcut(key({ key: "t", metaKey: true }), "macos")).toBe(true);
  });

  it("searches hosts with ⌘⇧K everywhere on macOS, like Ctrl+Shift+K (§9.1)", () => {
    const cmdShiftK = key({ key: "K", code: "KeyK", metaKey: true, shiftKey: true });
    expect(matchShortcut(cmdShiftK, "macos", true)).toBe("search");
    expect(matchShortcut(cmdShiftK, "macos", false)).toBe("search");
    expect(isAppShortcut(cmdShiftK, "macos")).toBe(true);
  });

  it("toggles the AI panel with Ctrl+Shift+A / ⌘⇧A (§9.1)", () => {
    expect(matchShortcut(key({ key: "A", code: "KeyA", metaKey: true, shiftKey: true }), "macos", true)).toBe("aiPanel");
    expect(matchShortcut(key({ key: "a", code: "KeyA", metaKey: true }), "macos", true)).toBeNull();
    expect(matchShortcut(key({ key: "A", code: "KeyA", ctrlKey: true, shiftKey: true }), "macos", true)).toBeNull();
    expect(matchShortcut(key({ key: "A", code: "KeyA", ctrlKey: true, shiftKey: true, altKey: true }), "windows", true)).toBeNull();
  });
});
