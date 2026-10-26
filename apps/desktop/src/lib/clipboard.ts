import { isTauri } from "@/ipc/api";

/** Clipboard access: the Tauri plugin in the app, the async clipboard API in a browser. */
export async function writeClipboard(text: string): Promise<void> {
  if (isTauri()) {
    const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
    await writeText(text);
    return;
  }
  await navigator.clipboard.writeText(text);
}

export async function readClipboard(): Promise<string> {
  if (isTauri()) {
    const { readText } = await import("@tauri-apps/plugin-clipboard-manager");
    return (await readText()) ?? "";
  }
  return navigator.clipboard.readText();
}

/** TERM-08: open a clicked link in the system browser. Only http(s) links are ever opened. */
export async function openExternal(uri: string): Promise<void> {
  if (!/^https?:\/\//i.test(uri)) return;
  if (isTauri()) {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(uri);
    return;
  }
  window.open(uri, "_blank", "noopener,noreferrer");
}
