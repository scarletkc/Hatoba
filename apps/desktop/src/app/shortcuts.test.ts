import { describe, expect, it } from "vitest";
import { isAppShortcut, matchShortcut, matchZoom } from "./shortcuts";

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

  it("opens a local terminal with Ctrl+Shift+`, ⌃⇧` on macOS (LOCAL-01)", () => {
    expect(matchShortcut(key({ key: "~", code: "Backquote", ctrlKey: true, shiftKey: true }), "windows", true)).toBe("newLocal");
    expect(matchShortcut(key({ key: "~", code: "Backquote", ctrlKey: true, shiftKey: true }), "linux", false)).toBe("newLocal");
    expect(matchShortcut(key({ key: "~", code: "Backquote", ctrlKey: true, shiftKey: true }), "macos", true)).toBe("newLocal");
    expect(matchShortcut(key({ key: "`", code: "Backquote", ctrlKey: true }), "windows", true)).toBeNull();
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

describe("matchZoom (TERM-13)", () => {
  it("zooms with Ctrl+= / Ctrl+- / Ctrl+0, the number pad and Ctrl++ included", () => {
    expect(matchZoom(key({ key: "=", code: "Equal", ctrlKey: true }), "windows")).toBe("in");
    expect(matchZoom(key({ key: "+", code: "Equal", ctrlKey: true, shiftKey: true }), "windows")).toBe("in");
    expect(matchZoom(key({ key: "+", code: "NumpadAdd", ctrlKey: true }), "linux")).toBe("in");
    expect(matchZoom(key({ key: "-", code: "Minus", ctrlKey: true }), "windows")).toBe("out");
    expect(matchZoom(key({ key: "-", code: "NumpadSubtract", ctrlKey: true }), "windows")).toBe("out");
    expect(matchZoom(key({ key: "0", code: "Digit0", ctrlKey: true }), "windows")).toBe("reset");
    expect(matchZoom(key({ key: "0", code: "Numpad0", ctrlKey: true }), "linux")).toBe("reset");
  });

  it("uses ⌘ on macOS", () => {
    expect(matchZoom(key({ key: "=", metaKey: true }), "macos")).toBe("in");
    expect(matchZoom(key({ key: "-", metaKey: true }), "macos")).toBe("out");
    expect(matchZoom(key({ key: "0", metaKey: true }), "macos")).toBe("reset");
    expect(matchZoom(key({ key: "=", ctrlKey: true }), "macos")).toBeNull();
  });

  it("leaves ^_, Ctrl+Insert and AltGr to the terminal", () => {
    // Ctrl+Shift+- types _, readline's undo.
    expect(matchZoom(key({ key: "_", code: "Minus", ctrlKey: true, shiftKey: true }), "windows")).toBeNull();
    // Ctrl+Numpad0 with Num Lock off copies (WIN-05).
    expect(matchZoom(key({ key: "Insert", code: "Numpad0", ctrlKey: true }), "windows")).toBeNull();
    // AltGr is Ctrl+Alt on Windows.
    expect(matchZoom(key({ key: "0", ctrlKey: true, altKey: true }), "windows")).toBeNull();
    expect(matchZoom(key({ key: "=" }), "windows")).toBeNull();
  });

  it("keeps the zoom keys from the terminal", () => {
    expect(isAppShortcut(key({ key: "=", ctrlKey: true }), "windows")).toBe(true);
    expect(isAppShortcut(key({ key: "-", metaKey: true }), "macos")).toBe(true);
    expect(isAppShortcut(key({ key: "_", ctrlKey: true, shiftKey: true }), "windows")).toBe(false);
  });
});
