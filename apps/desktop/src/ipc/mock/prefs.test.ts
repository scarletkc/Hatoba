import { describe, expect, it } from "vitest";
import { PREFS } from "./data";
import { readStoredPrefs as readPrefs } from "./prefs";

const readStoredPrefs = (json: string | null) => readPrefs(() => json);

describe("readStoredPrefs", () => {
  it("reads stored values as Rust's LocalPrefs::from_stored does", () => {
    expect(readStoredPrefs(null)).toEqual(PREFS);
    expect(readStoredPrefs("{not json")).toEqual(PREFS);
    // A browser that denies storage.
    expect(
      readPrefs(() => {
        throw new DOMException("denied", "SecurityError");
      }),
    ).toEqual(PREFS);

    const prefs = readStoredPrefs(
      JSON.stringify({
        language: "ja",
        ai_permission_mode: "auto",
        ai_panel_width: 520,
        default_proxy_id: "px1",
        host_probe: false,
      }),
    );
    expect(prefs).toEqual({
      ...PREFS,
      language: "ja",
      ai_permission_mode: "auto",
      ai_panel_width: 520,
      default_proxy_id: "px1",
      host_probe: false,
    });
  });

  it("falls back to the default for a value this build does not know", () => {
    // A mode or language a build of another branch stored, a broken number, and a pref it dropped.
    const prefs = readStoredPrefs(
      JSON.stringify({
        ai_permission_mode: "ask_twice",
        language: "ko",
        ai_panel_width: -1,
        auto_lock_minutes: "15",
        ai_tool_call_limit: 40,
        ai_bypass_confirmed: true,
      }),
    );
    expect(prefs).toEqual({ ...PREFS, ai_bypass_confirmed: true });
  });
});
