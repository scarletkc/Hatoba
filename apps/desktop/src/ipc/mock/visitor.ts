import { detectLocale, LOCALES, type Locale } from "@/i18n";
import type { AppInfo } from "../types";

/**
 * The website's demo (`pnpm build:demo`, or `pnpm dev --mode demo` to try it): what a visitor to
 * hatoba.fog.moe sees instead of the states the other demo parameters select for review. It starts
 * in a tab connected to prod-api-tokyo with the assistant open and a question in its input, on the
 * visitor's own platform, with the release's version. Prefs last until the page reloads, so a
 * reload starts the demo over, and secret fields come filled with a sample.
 */
export const VISITOR = import.meta.env.MODE === "demo";

/** The host the demo connects to, whose terminal the `showcase` fix runs in. */
export const VISITOR_HOST = "prod-api-tokyo";

/** `?lang=en|zh-CN|ja`: the language the page passes in, which the language pref and the sample text follow. */
export function passedLocale(): Locale | null {
  const lang = new URLSearchParams(location.search).get("lang");
  return LOCALES.find((l) => l === lang) ?? null;
}

/** The language of the mock's sample text: the one passed in, else the system's. */
export function mockLocale(): Locale {
  return passedLocale() ?? detectLocale();
}

/** The visitor's OS, so the window and the shortcuts are the ones they would install. */
export function visitorPlatform(): AppInfo["platform"] {
  const agent = navigator.userAgent;
  if (/Mac|iPhone|iPad/.test(agent)) return "macos";
  if (/Windows/.test(agent)) return "windows";
  if (/Linux|X11|CrOS/.test(agent)) return "linux";
  return "windows";
}
