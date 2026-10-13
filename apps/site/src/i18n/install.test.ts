import { describe, expect, it } from "vitest";
import { installerNames } from "../../../../scripts/release/release.mjs";
import { INSTALL_STEPS, PLATFORMS } from "../lib/release";
import { TRANSLATED_STEPS, installSteps } from "./install";

const names = installerNames("1.2.3");

describe("install steps", () => {
  it.each(PLATFORMS)("translate the current English steps for %s", (platform) => {
    // When this fails, INSTALL_STEPS in scripts/release/release.mjs changed: update the Chinese and
    // Japanese steps in install.ts, then copy the new English into `source`.
    expect(TRANSLATED_STEPS.source[platform](names)).toBe(INSTALL_STEPS[platform].steps(names));
  });

  it.each(PLATFORMS)("name every file of %s that the English names", (platform) => {
    const files = INSTALL_STEPS[platform].bundles.map((bundle) => names[bundle]);
    for (const locale of ["zh", "ja"] as const) {
      for (const file of files) expect(installSteps(locale, platform, names)).toContain(file);
    }
  });

  it("start the English steps with a capital", () => {
    expect(installSteps("en", "linux", names)).toMatch(/^On Debian/);
  });
});
