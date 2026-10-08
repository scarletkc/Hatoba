import type { AppInfo } from "@/ipc/types";

export const REPO_URL = "https://github.com/scarletkc/Hatoba";

const OS_NAMES: Partial<Record<AppInfo["platform"], string>> = { windows: "Windows", macos: "macOS", linux: "Linux" };

/**
 * The bug report form with the version and OS filled in through the field ids of
 * `.github/ISSUE_TEMPLATE/bug_report.yml`. The user sees both before submitting.
 */
export function bugReportUrl(info: Pick<AppInfo, "version" | "platform">): string {
  const query = new URLSearchParams({ template: "bug_report.yml", version: info.version });
  const os = OS_NAMES[info.platform];
  if (os) query.set("os", os);
  return `${REPO_URL}/issues/new?${query}`;
}
