import { isTauri } from "@/ipc/api";

export const DEPLOY_URL =
  "https://deploy.workers.cloudflare.com/?url=https://github.com/scarletkc/Hatoba/tree/main/workers/sync";
export const DEPLOY_GUIDE_URL = "https://github.com/scarletkc/Hatoba/blob/main/workers/sync/README.md";
export const API_TOKENS_URL = "https://dash.cloudflare.com/profile/api-tokens";
/** The dashboard's token form with Workers Scripts · Edit and D1 · Edit filled in (spec §6.7, API token). */
export const DEPLOY_TOKEN_URL =
  "https://dash.cloudflare.com/profile/api-tokens?permissionGroupKeys=%5B%7B%22key%22%3A%22workers_scripts%22%2C%22type%22%3A%22edit%22%7D%2C%7B%22key%22%3A%22d1%22%2C%22type%22%3A%22edit%22%7D%5D&accountId=%2A&zoneId=all&name=Hatoba%20Sync%20Deploy";

/** Opens a link in the system browser (plugin-opener in the app, a new tab in a plain browser). */
export async function openExternal(url: string): Promise<void> {
  if (isTauri()) {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
  } else {
    window.open(url, "_blank", "noopener,noreferrer");
  }
}
