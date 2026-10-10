import { useEffect } from "react";
import type { Platform } from "@/lib/platform";

export type ShortcutAction = "search" | "newTab" | "closeTab" | "nextTab" | "prevTab" | "settings" | "lock" | "aiPanel";

/**
 * App-level shortcuts (WIN-04). On Windows/Linux every app shortcut uses Ctrl+Shift so that plain
 * Ctrl+letter always reaches the remote shell (Ctrl+L, Ctrl+W, Ctrl+K…). Exceptions are Ctrl+Tab,
 * Ctrl+, and Ctrl+K while focus is outside the terminal. On macOS ⌘K likewise searches hosts only
 * outside the terminal; inside it clears the terminal (session.ts). ⌘⇧K searches hosts everywhere,
 * like Ctrl+Shift+K.
 */
export function matchShortcut(e: KeyboardEvent, platform: Platform, inTerminal: boolean): ShortcutAction | null {
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  if (platform === "macos") {
    if (e.ctrlKey && key === "Tab") return e.shiftKey ? "prevTab" : "nextTab";
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
    }
    return null;
  }
  if (key === "k" && !inTerminal) return "search";
  return null;
}

/** True when the event is an app shortcut, so the terminal must not consume it. */
export function isAppShortcut(e: KeyboardEvent, platform: Platform): boolean {
  return matchShortcut(e, platform, true) !== null;
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
