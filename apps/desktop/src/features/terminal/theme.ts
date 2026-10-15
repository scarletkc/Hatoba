import type { ITerminalOptions, ITheme } from "@xterm/xterm";
import { create } from "zustand";
import { isDarkTheme, useApp } from "@/app/store";
import { api } from "@/ipc/api";
import type { TerminalSettings } from "@/ipc/types";
import { defaultRightClick, defaultTerminalFont, type Platform } from "@/lib/platform";
import { customScheme, DEFAULT_SCHEME, schemeFor, type TermMode, type TermScheme } from "./schemes";

/** Used until `settings_get` answers, and when it fails. */
export function defaultTerminalSettings(platform: Platform): TerminalSettings {
  return {
    font_family: defaultTerminalFont(platform),
    font_size: 13,
    theme: "dark",
    dark_scheme: null,
    light_scheme: null,
    line_height: 1.3,
    cursor_style: "block",
    cursor_blink: false,
    scrollback: 10_000,
    right_click: defaultRightClick(platform),
    confirm_multiline_paste: true,
  };
}

interface TermSettingsState {
  settings: TerminalSettings;
  /** The colour schemes the user made (TERM-06). */
  schemes: TermScheme[];
  /** Bumped when the app theme (light/dark) flips, so terminals that follow it re-theme. */
  appearanceTick: number;
  reload(): Promise<void>;
}

/** Synced terminal settings (TERM-04/05/06, WIN-05), cached for every terminal. Re-read when the settings window closes. */
export const useTermSettings = create<TermSettingsState>((set) => ({
  settings: defaultTerminalSettings(useApp.getState().info.platform),
  schemes: [],
  appearanceTick: 0,
  reload: async () => {
    try {
      const [{ terminal }, schemes] = await Promise.all([api.settings_get(), api.terminal_schemes_list()]);
      set({ settings: { ...defaultTerminalSettings(useApp.getState().info.platform), ...terminal }, schemes: schemes.map(customScheme) });
    } catch {
      /* locked or unavailable: keep what we have */
    }
  },
}));

/** "system" follows the app's resolved light/dark theme. */
export function resolveMode(setting: TerminalSettings["theme"]): TermMode {
  if (setting === "system") return isDarkTheme() ? "dark" : "light";
  return setting;
}

type SchemeSettings = Pick<TerminalSettings, "theme" | "dark_scheme" | "light_scheme">;

/** The colour scheme for the current mode (TERM-06), built in or one of the user's `custom` schemes. */
export function resolveScheme(settings: SchemeSettings, custom: readonly TermScheme[] = []): TermScheme {
  const mode = resolveMode(settings.theme);
  return schemeFor(mode, mode === "dark" ? settings.dark_scheme : settings.light_scheme, custom);
}

function cssVar(name: string, fallback: string): string {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v || fallback;
}

/** xterm's theme for `scheme`. Hatoba Dark sits on the app's own terminal background. */
export function terminalTheme(scheme: TermScheme): ITheme {
  const background = scheme.id === DEFAULT_SCHEME.dark ? cssVar("--term", scheme.colors.background) : scheme.colors.background;
  return { cursorAccent: background, ...scheme.colors, background };
}

/** Terminal chrome colours (background behind the padding, find bar…) for `scheme`. */
export function terminalChrome(scheme: TermScheme): { background: string; foreground: string } {
  const th = terminalTheme(scheme);
  return { background: th.background!, foreground: th.foreground! };
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

export function terminalOptions(settings: TerminalSettings, custom: readonly TermScheme[] = []): ITerminalOptions {
  const scheme = resolveScheme(settings, custom);
  return {
    fontFamily: terminalFontFamily(settings.font_family),
    fontSize: settings.font_size,
    lineHeight: settings.line_height,
    cursorStyle: settings.cursor_style,
    cursorBlink: settings.cursor_blink,
    scrollback: settings.scrollback,
    theme: terminalTheme(scheme),
    minimumContrastRatio: scheme.contrast,
  };
}

/** React hook: terminal background / foreground for the current settings and app theme. */
export function useTerminalChrome(): { mode: TermMode; background: string; foreground: string } {
  const theme = useTermSettings((s) => s.settings.theme);
  const dark = useTermSettings((s) => s.settings.dark_scheme);
  const light = useTermSettings((s) => s.settings.light_scheme);
  const custom = useTermSettings((s) => s.schemes);
  useTermSettings((s) => s.appearanceTick);
  const scheme = resolveScheme({ theme, dark_scheme: dark, light_scheme: light }, custom);
  return { mode: scheme.mode, ...terminalChrome(scheme) };
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
