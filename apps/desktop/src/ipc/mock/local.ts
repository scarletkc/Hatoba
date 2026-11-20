import type { Platform } from "@/lib/platform";
import type { FrameHandler, HatobaApi } from "../api";
import { FRAME_CLOSED, FRAME_DATA, type LocalShellView } from "../types";
import { delay, fail, frame, type MockDeps } from "./shared";
import { FakeShell } from "./shell";

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

type LocalApi = Pick<
  HatobaApi,
  "local_shells" | "local_open" | "local_write" | "local_write_bytes" | "local_resize" | "local_close"
>;

export interface LocalMockDeps extends Pick<MockDeps, "emit" | "needUnlocked" | "id"> {
  platform: Platform;
  shells: Map<string, FakeShell>;
  q: URLSearchParams;
}

export function createLocalMock(ctx: LocalMockDeps): LocalApi {
  const { emit, needUnlocked, id, platform, shells, q } = ctx;

  return {
    local_shells: async () => mockLocalShells(platform),
    local_open: async (shellId, _options, onFrame: FrameHandler) => {
      needUnlocked();
      const shell = mockLocalShells(platform).find((s) => s.id === shellId) ?? fail("not_found", "shell not found");
      // `?local=fail`: the shell does not start, as when a WSL distribution is broken.
      if (q.get("local") === "fail") fail("io", `the shell could not be started: ${shell.name} exited at once`);
      await delay(120);
      const sid = id("l");
      const fake = new FakeShell(null, (bytes) => onFrame(frame(FRAME_DATA, bytes)), {
        shell: shell.kind,
        user: "demo",
        machine: platform === "macos" ? "Demo-MacBook" : "DEMO-PC",
      });
      shells.set(sid, fake);
      setTimeout(() => {
        emit("ssh://state", {
          session_id: sid,
          host_id: null,
          state: "connected",
          latency_ms: null,
          error: null,
          exit_status: null,
        });
        fake.start();
      }, 0);
      fake.onExit = () => {
        onFrame(frame(FRAME_CLOSED, new Uint8Array()));
        emit("ssh://state", {
          session_id: sid,
          host_id: null,
          state: "disconnected",
          latency_ms: null,
          error: null,
          exit_status: 0,
        });
        shells.delete(sid);
      };
      return sid;
    },
    local_write: async (sid, data) => {
      shells.get(sid)?.input(data);
    },
    local_write_bytes: async () => {},
    local_resize: async () => {},
    local_close: async (sid) => {
      shells.get(sid)?.stop();
      shells.delete(sid);
    },
  };
}
