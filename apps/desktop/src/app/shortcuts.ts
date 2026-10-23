import { useEffect } from "react";
import type { Platform } from "@/lib/platform";

export type ShortcutAction =
  "search" | "newTab" | "newLocal" | "closeTab" | "nextTab" | "prevTab" | "settings" | "lock" | "aiPanel";

/**
 * App-level shortcuts (WIN-04). On Windows/Linux every app shortcut uses Ctrl+Shift so that plain
 * Ctrl+letter always reaches the remote shell (Ctrl+L, Ctrl+W, Ctrl+K…). Exceptions are Ctrl+Tab,
 * Ctrl+, and Ctrl+K while focus is outside the terminal. On macOS ⌘K likewise searches hosts only
 * outside the terminal; inside it clears the terminal (session.ts). ⌘⇧K searches hosts everywhere,
 * like Ctrl+Shift+K. A new local terminal (LOCAL-01) is Ctrl+Shift+` everywhere, ⌃⇧` on macOS, as in
 * VS Code.
 */
export function matchShortcut(e: KeyboardEvent, platform: Platform, inTerminal: boolean): ShortcutAction | null {
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  if (platform === "macos") {
    if (e.ctrlKey && key === "Tab") return e.shiftKey ? "prevTab" : "nextTab";
    if (e.ctrlKey && e.shiftKey && !e.metaKey && !e.altKey && e.code === "Backquote") return "newLocal";
    if (!e.metaKey || e.altKey || e.ctrlKey) return null;
    if (key === "k") return e.shiftKey || !inTerminal ? "search" : null;
    if (key === "t" && !e.shiftKey) return "newTab";
    if (key === "w" && !e.shiftKey) return "closeTab";
    if (key === "," && !e.shiftKey) return "settings";
    if (key === "l" && !e.shiftKey) return "lock";
    if (key === "a" && e.shiftKey) return "aiPanel";
    return null;
  }
  if (!e.ctrlKey || e.altKey || e.metaKey) return null;
  if (key === "Tab") return e.shiftKey ? "prevTab" : "nextTab";
  if (key === "," && !e.shiftKey) return "settings";
  if (e.shiftKey) {
    // With Shift held, e.key is the shifted character; use e.code for letters.
    switch (e.code) {
      case "KeyK":
        return "search";
      case "KeyT":
        return "newTab";
      case "KeyW":
        return "closeTab";
      case "KeyL":
        return "lock";
      case "KeyA":
        return "aiPanel";
      case "Backquote":
        return "newLocal";
    }
    return null;
  }
  if (key === "k" && !inTerminal) return "search";
  return null;
}

export type ZoomAction = "in" | "out" | "reset";

/**
 * The zoom keys (TERM-13): Ctrl+= or Ctrl++ makes larger, Ctrl+- smaller, Ctrl+0 the default size;
 * ⌘ on macOS. They go by the character typed, so the number pad's + - 0 count and so do layouts
 * that type = with Shift. Ctrl+Shift+- types `_` and still reaches the terminal as `^_`, and Ctrl+0
 * on the number pad with Num Lock off is Ctrl+Insert, which copies (WIN-05).
 */
export function matchZoom(e: KeyboardEvent, platform: Platform): ZoomAction | null {
  const mod = platform === "macos" ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  if (!mod || e.altKey || e.isComposing) return null;
  switch (e.key) {
    case "=":
    case "+":
      return "in";
    case "-":
      return "out";
    case "0":
      return "reset";
  }
  return null;
}

/** True when the event is an app shortcut or a zoom key, so the terminal must not consume it. */
export function isAppShortcut(e: KeyboardEvent, platform: Platform): boolean {
  return matchShortcut(e, platform, true) !== null || matchZoom(e, platform) !== null;
}

export function useShortcuts(platform: Platform, handler: (action: ShortcutAction) => void) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      const inTerminal = !!target?.closest?.(".xterm");
      const action = matchShortcut(e, platform, inTerminal);
      if (!action) return;
      e.preventDefault();
      e.stopPropagation();
      handler(action);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [platform, handler]);
}
