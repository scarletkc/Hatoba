import { isTauri } from "@/ipc/api";

export const DEPLOY_URL =
  "https://deploy.workers.cloudflare.com/?url=https://github.com/scarletkc/Hatoba/tree/main/workers/sync";
export const DEPLOY_GUIDE_URL = "https://github.com/scarletkc/Hatoba/blob/main/workers/sync/README.md";
export const API_TOKENS_URL = "https://dash.cloudflare.com/profile/api-tokens";

/** Opens a link in the system browser (plugin-opener in the app, a new tab in a plain browser). */
export async function openExternal(url: string): Promise<void> {
  if (isTauri()) {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
  } else {
    window.open(url, "_blank", "noopener,noreferrer");
  }
}
