import type { ITerminalOptions, ITheme } from "@xterm/xterm";
import { create } from "zustand";
import { isDarkTheme, useApp } from "@/app/store";
import { api } from "@/ipc/api";
import type { TerminalSettings } from "@/ipc/types";
import { defaultRightClick, defaultTerminalFont, type Platform } from "@/lib/platform";

/** Used until `settings_get` answers, and when it fails. */
export function defaultTerminalSettings(platform: Platform): TerminalSettings {
  return {
    font_family: defaultTerminalFont(platform),
    font_size: 13,
    theme: "dark",
    cursor_style: "block",
    scrollback: 10_000,
    right_click: defaultRightClick(platform),
    confirm_multiline_paste: true,
  };
}

interface TermSettingsState {
  settings: TerminalSettings;
  /** Bumped when the app theme (light/dark) flips, so terminals that follow it re-theme. */
  appearanceTick: number;
  reload(): Promise<void>;
}

/** Synced terminal settings (TERM-04/05/06, WIN-05), cached for every terminal. Re-read when the settings window closes. */
export const useTermSettings = create<TermSettingsState>((set) => ({
  settings: defaultTerminalSettings(useApp.getState().info.platform),
  appearanceTick: 0,
  reload: async () => {
    try {
      const { terminal } = await api.settings_get();
      set({ settings: { ...defaultTerminalSettings(useApp.getState().info.platform), ...terminal } });
    } catch {
      /* locked or unavailable: keep what we have */
    }
  },
}));

export type TermMode = "dark" | "light";

/** "system" follows the app's resolved light/dark theme. */
export function resolveMode(setting: TerminalSettings["theme"]): TermMode {
  if (setting === "system") return isDarkTheme() ? "dark" : "light";
  return setting;
}

const DARK: ITheme = {
  foreground: "#d5d8de",
  cursor: "#d5d8de",
  selectionBackground: "rgba(122, 183, 255, 0.3)",
  selectionInactiveBackground: "rgba(122, 183, 255, 0.18)",
  black: "#2b2e36",
  red: "#f07178",
  green: "#8bd49c",
  yellow: "#e5c07b",
  blue: "#7ab7ff",
  magenta: "#c792ea",
  cyan: "#7fd1d6",
  white: "#d5d8de",
  brightBlack: "#6b717c",
  brightRed: "#ff8b92",
  brightGreen: "#a4e5b3",
  brightYellow: "#f0d08f",
  brightBlue: "#9cc9ff",
  brightMagenta: "#d7a8f0",
  brightCyan: "#9fe3e7",
  brightWhite: "#ffffff",
};

const LIGHT: ITheme = {
  background: "#ffffff",
  foreground: "#1d1d1f",
  cursor: "#1d1d1f",
  cursorAccent: "#ffffff",
  selectionBackground: "rgba(0, 122, 255, 0.25)",
  selectionInactiveBackground: "rgba(0, 122, 255, 0.14)",
  black: "#1d1d1f",
  red: "#c9302c",
  green: "#1f8a3b",
  yellow: "#946200",
  blue: "#0b62d6",
  magenta: "#8a3fb8",
  cyan: "#0f7f89",
  white: "#c9ccd2",
  brightBlack: "#6b717c",
  brightRed: "#e5342b",
  brightGreen: "#28a745",
  brightYellow: "#a87a00",
  brightBlue: "#007aff",
  brightMagenta: "#a24dd6",
  brightCyan: "#12919d",
  brightWhite: "#ffffff",
};

/** Search-match highlight colours (xterm needs #RRGGBB here). */
export const SEARCH_DECORATIONS = {
  dark: {
    matchBackground: "#4a3f1d",
    matchBorder: "#7a6a2e",
    matchOverviewRuler: "#e5c07b",
    activeMatchBackground: "#8a6d1f",
    activeMatchBorder: "#e5c07b",
    activeMatchColorOverviewRuler: "#e5c07b",
  },
  light: {
    matchBackground: "#fff1a8",
    matchBorder: "#e0c34a",
    matchOverviewRuler: "#e0b400",
    activeMatchBackground: "#ffd54a",
    activeMatchBorder: "#b88700",
    activeMatchColorOverviewRuler: "#b88700",
  },
} as const;

function cssVar(name: string, fallback: string): string {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v || fallback;
}

export function terminalTheme(mode: TermMode): ITheme {
  if (mode === "light") return LIGHT;
  const background = cssVar("--term", "#0d0e10");
  return { ...DARK, background, cursorAccent: background };
}

/** Terminal chrome colours (background behind the padding, find bar…) for the resolved mode. */
export function terminalChrome(mode: TermMode): { background: string; foreground: string } {
  const th = terminalTheme(mode);
  return { background: th.background ?? "#0d0e10", foreground: th.foreground ?? "#d5d8de" };
}

/**
 * Font stack: the configured family first, then the app's mono stack, which already ends with the
 * CJK fallbacks (Microsoft YaHei UI, Yu Gothic UI) that TERM-02 needs for wide characters.
 * xterm needs a concrete string, so `var(--font-mono)` is resolved here.
 */
export function terminalFontFamily(family: string): string {
  const mono = cssVar("--font-mono", "Consolas, monospace");
  const configured = family.trim();
  if (!configured) return mono;
  const head = configured.includes(",") || /^["'].*["']$/.test(configured) ? configured : `"${configured}"`;
  return `${head}, ${mono}`;
}

export function terminalOptions(settings: TerminalSettings): ITerminalOptions {
  const mode = resolveMode(settings.theme);
  return {
    fontFamily: terminalFontFamily(settings.font_family),
    fontSize: settings.font_size,
    lineHeight: 1.3,
    cursorStyle: settings.cursor_style,
    cursorBlink: false,
    scrollback: settings.scrollback,
    theme: terminalTheme(mode),
    // Programs written for dark terminals stay readable on the light palette.
    minimumContrastRatio: mode === "light" ? 4.5 : 1,
  };
}

/** React hook: terminal background / foreground for the current settings and app theme. */
export function useTerminalChrome(): { mode: TermMode; background: string; foreground: string } {
  const theme = useTermSettings((s) => s.settings.theme);
  useTermSettings((s) => s.appearanceTick);
  const mode = resolveMode(theme);
  return { mode, ...terminalChrome(mode) };
}

let watching = false;

/** Start tracking app theme flips, the settings window closing and synced settings. Idempotent. */
export function watchTerminalAppearance() {
  if (watching || typeof document === "undefined") return;
  watching = true;
  const reload = () => void useTermSettings.getState().reload();
  new MutationObserver(() => useTermSettings.setState((s) => ({ appearanceTick: s.appearanceTick + 1 }))).observe(
    document.documentElement,
    { attributes: true, attributeFilter: ["data-theme"] },
  );
  useApp.subscribe((s, prev) => {
    if (prev.settingsOpen && !s.settingsOpen) reload();
  });
  let lastSync = "";
  void api.listen("sync://status", (status) => {
    // A finished pull may have brought new terminal settings.
    if (status.state === "idle" && lastSync !== "idle") reload();
    lastSync = status.state;
  });
  reload();
}
