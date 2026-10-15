import type { LocalShell } from "@/app/tabs";
import type { MessageKey } from "@/i18n";
import { api } from "@/ipc/api";
import type { LocalShellView } from "@/ipc/types";

/** The Windows shells' names in the app's language; WSL distributions and Unix shells keep their own. */
const NAMES: Record<string, MessageKey> = {
  pwsh: "terminal.shell.pwsh",
  powershell: "terminal.shell.powershell",
  cmd: "terminal.shell.cmd",
  "git-bash": "terminal.shell.gitBash",
};

/** The name a shell shows in the picker, the tab and the status bar. */
export function shellName(shell: Pick<LocalShell, "id" | "name">, t: (key: MessageKey) => string): string {
  const key = NAMES[shell.id];
  return key ? t(key) : shell.name;
}

/** The shells this computer has now (LOCAL-02), the system default first. */
export function loadLocalShells(): Promise<LocalShellView[]> {
  return api.local_shells();
}

/** The shell a new local tab runs: the one picked in Settings → Terminal while it exists, else the system default. */
export function preferredShell(shells: LocalShellView[], preferred: string | null): LocalShellView | undefined {
  return shells.find((s) => s.id === preferred) ?? shells.find((s) => s.default) ?? shells[0];
}
