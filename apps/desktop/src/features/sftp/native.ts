import { isTauri } from "@/ipc/api";
import type { Platform } from "@/lib/platform";

/** Native file pickers. In a plain browser (mock backend) they fall back to the file input / file name. */

/** Upload picker (SFTP-02). Resolves to local paths, or file names in the browser. */
export async function pickUploadFiles(): Promise<string[]> {
  if (isTauri()) {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({ multiple: true, directory: false });
    return picked == null ? [] : Array.isArray(picked) ? picked : [picked];
  }
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.multiple = true;
    input.onchange = () => resolve(Array.from(input.files ?? [], (f) => f.name));
    input.oncancel = () => resolve([]);
    input.click();
  });
}

/** Save dialog for a download. `null` when the user cancels. */
export async function pickSavePath(fileName: string): Promise<string | null> {
  if (isTauri()) {
    const { save } = await import("@tauri-apps/plugin-dialog");
    return save({ defaultPath: fileName });
  }
  return fileName;
}

/** SFTP-05: picks the program that edits remote files: an app bundle on macOS, a program elsewhere. */
export async function pickProgram(platform: Platform): Promise<string | null> {
  if (!isTauri())
    return platform === "macos"
      ? "/Applications/Visual Studio Code.app"
      : "C:\\Program Files\\Microsoft VS Code\\Code.exe";
  const { open } = await import("@tauri-apps/plugin-dialog");
  const picked = await open({
    multiple: false,
    directory: false,
    defaultPath: platform === "macos" ? "/Applications" : undefined,
    filters: platform === "windows" ? [{ name: "Programs", extensions: ["exe", "cmd", "bat"] }] : undefined,
  });
  return typeof picked === "string" ? picked : null;
}

/** A program's name as a menu shows it: `Code.exe` and `Visual Studio Code.app` without the extension. */
export function programName(path: string): string {
  const name = path.split(/[\\/]/).filter(Boolean).pop() ?? path;
  return name.replace(/\.(exe|cmd|bat|app)$/i, "");
}
