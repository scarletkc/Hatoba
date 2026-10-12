import type { LocalPrefs } from "../types";
import { PREFS } from "./data";

type ChoicePref = "language" | "appearance" | "density" | "ai_permission_mode" | "dashboard_layout";

/** The values this build knows for each pref that is one of a set; the record keys keep them complete. */
const CHOICES: { [K in ChoicePref]: Record<LocalPrefs[K], true> } = {
  language: { system: true, "zh-CN": true, en: true, ja: true },
  appearance: { system: true, light: true, dark: true },
  density: { regular: true, compact: true },
  ai_permission_mode: { manual: true, auto: true, bypass: true },
  dashboard_layout: { cards: true, table: true },
};

/** Whether `value` reads as the pref `key`, as serde reads Rust's `LocalPrefs`. */
function fits(key: keyof LocalPrefs, value: unknown): boolean {
  if (Object.hasOwn(CHOICES, key)) return typeof value === "string" && Object.hasOwn(CHOICES[key as ChoicePref], value);
  if (key === "default_proxy_id") return value === null || typeof value === "string";
  if (key === "dashboard_hidden_columns") return Array.isArray(value) && value.every((v) => typeof v === "string");
  const fallback = PREFS[key];
  // Rust stores the numbers as u32.
  if (typeof fallback === "number") return typeof value === "number" && Number.isInteger(value) && value >= 0 && value <= 0xffff_ffff;
  return typeof value === typeof fallback;
}

/**
 * The prefs the mock stored, read as Rust's `LocalPrefs::from_stored` reads them: a value that is
 * missing, unknown or broken falls back to the default. Dev servers on the same port share their
 * localStorage, so a build of another branch may have stored a value this one does not know, such
 * as a permission mode it lacks, which new conversations would then start in. `read` returns the
 * stored JSON; a browser that denies storage throws from it.
 */
export function readStoredPrefs(read: () => string | null): LocalPrefs {
  let stored: unknown = null;
  try {
    stored = JSON.parse(read() ?? "null");
  } catch {
    // Unreadable prefs are all defaults.
  }
  const prefs: Record<string, unknown> = { ...PREFS };
  if (stored && typeof stored === "object") {
    for (const [key, value] of Object.entries(stored)) {
      if (Object.hasOwn(PREFS, key) && fits(key as keyof LocalPrefs, value)) prefs[key] = value;
    }
  }
  return prefs as unknown as LocalPrefs;
}
