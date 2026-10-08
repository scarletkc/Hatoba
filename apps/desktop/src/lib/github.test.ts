import { describe, expect, it } from "vitest";
import { bugReportUrl } from "./github";

describe("bugReportUrl", () => {
  it("fills in the version and OS of the bug report form", () => {
    const url = new URL(bugReportUrl({ version: "0.2.0", platform: "macos" }));
    expect(url.origin + url.pathname).toBe("https://github.com/scarletkc/Hatoba/issues/new");
    expect(Object.fromEntries(url.searchParams)).toEqual({ template: "bug_report.yml", version: "0.2.0", os: "macOS" });
  });

  it("leaves the OS empty in a plain browser", () => {
    const url = new URL(bugReportUrl({ version: "0.1.0-dev", platform: "web" }));
    expect(url.searchParams.has("os")).toBe(false);
  });
});
