import type { Platform } from "@/lib/platform";
import type { LocalShellView } from "../types";

/** The shells the mock computer has (LOCAL-02), as the real detection lists them, the default first. */
export function mockLocalShells(platform: Platform): LocalShellView[] {
  if (platform === "windows") {
    return [
      { id: "pwsh", name: "PowerShell", kind: "powershell", default: true },
      { id: "powershell", name: "Windows PowerShell", kind: "powershell", default: false },
      { id: "cmd", name: "Command Prompt", kind: "cmd", default: false },
      { id: "git-bash", name: "Git Bash", kind: "git_bash", default: false },
      { id: "wsl:Ubuntu", name: "Ubuntu", kind: "wsl", default: false },
    ];
  }
  if (platform === "macos") {
    return [
      { id: "/bin/zsh", name: "zsh", kind: "unix", default: true },
      { id: "/bin/bash", name: "bash", kind: "unix", default: false },
      { id: "/bin/sh", name: "sh", kind: "unix", default: false },
    ];
  }
  return [
    { id: "/bin/bash", name: "bash", kind: "unix", default: true },
    { id: "/usr/bin/zsh", name: "zsh", kind: "unix", default: false },
    { id: "/usr/bin/fish", name: "fish", kind: "unix", default: false },
  ];
}
