import type { AppInfo } from "@/ipc/types";

export type Platform = AppInfo["platform"];

/** Windows / Linux use Ctrl+Shift app shortcuts (WIN-04); macOS uses ⌘. */
export function isMac(platform: Platform): boolean {
  return platform === "macos";
}

/** The terminal font a new vault starts with. Keep in step with `DEFAULT_FONT_FAMILY` in hatoba-core. */
export function defaultTerminalFont(platform: Platform): string {
  return isMac(platform) ? "Menlo" : "Cascadia Mono";
}

/**
 * What right-click does in the terminal while the setting is unset: copy or paste (the PuTTY habit)
 * everywhere but macOS, where a secondary click that pastes surprises users. Keep in step with
 * `RightClick::platform_default` in hatoba-core.
 */
export function defaultRightClick(platform: Platform): "copy_paste" | "menu" {
  return isMac(platform) ? "menu" : "copy_paste";
}

/** Human-readable shortcut label, e.g. shortcutLabel("Shift+K") → "Ctrl+Shift+K" or "⌘K". */
export function shortcutLabel(platform: Platform, win: string, mac: string): string {
  return isMac(platform) ? mac : win;
}
