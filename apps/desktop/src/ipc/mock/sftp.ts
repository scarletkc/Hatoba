import type { HatobaApi } from "../api";
import type { FileEntry } from "../types";
import * as D from "./data";
import { delay, type MockDeps } from "./shared";

type SftpApi = Pick<
  HatobaApi,
  | "sftp_home"
  | "sftp_list"
  | "sftp_download"
  | "sftp_upload"
  | "sftp_rename"
  | "sftp_remove"
  | "sftp_mkdir"
  | "transfer_cancel"
>;

export type SftpMockDeps = Pick<MockDeps, "emit" | "id">;

export function createSftpMock(ctx: SftpMockDeps): SftpApi & {
  startTransfer(sid: string, direction: "upload" | "download", name: string, total: number): string;
} {
  const { emit, id } = ctx;

  function startTransfer(sid: string, direction: "upload" | "download", name: string, total: number) {
    const tid = id("t");
    let bytes = 0;
    const rate = 3_100_000;
    const timer = setInterval(() => {
      bytes = Math.min(total, bytes + rate / 5);
      const done = bytes >= total;
      emit("transfer://progress", {
        transfer_id: tid,
        session_id: sid,
        direction,
        name,
        bytes,
        total,
        bytes_per_sec: rate,
        state: done ? "done" : "running",
        error: null,
      });
      if (done) clearInterval(timer);
    }, 200);
    transfers.set(tid, () => {
      clearInterval(timer);
      emit("transfer://progress", {
        transfer_id: tid,
        session_id: sid,
        direction,
        name,
        bytes,
        total,
        bytes_per_sec: 0,
        state: "cancelled",
        error: null,
      });
    });
    return tid;
  }

  return {
    startTransfer,
    sftp_home: async () => "/srv/api",
    sftp_list: async (_sid, path) => {
      await delay(120);
      if (path === "/srv/api") return D.API_FILES;
      if (path === "/home/deploy") return D.HOME_FILES;
      return [
        {
          name: "README",
          path: `${path.replace(/\/$/, "")}/README`,
          is_dir: false,
          is_symlink: false,
          size: 120,
          modified: Date.now(),
          permissions: "-rw-r--r--",
        },
      ] satisfies FileEntry[];
    },
    sftp_download: async (sid, remote) => startTransfer(sid, "download", remote.split("/").pop() ?? remote, 18_400_000),
    sftp_upload: async (sid, local) => startTransfer(sid, "upload", local.split(/[\\/]/).pop() ?? local, 61_600_000),
    sftp_rename: async () => {},
    sftp_remove: async () => {},
    sftp_mkdir: async () => {},
    transfer_cancel: async (tid) => {
      transfers.get(tid)?.();
    },
  };
}

const transfers = new Map<string, () => void>();
