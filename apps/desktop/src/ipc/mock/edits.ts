import type { HatobaApi } from "../api";
import type { EventMap, RemoteEditState, RemoteEditView } from "../types";

type Emit = <K extends keyof EventMap>(event: K, payload: EventMap[K]) => void;
type StartTransfer = (sid: string, direction: "upload" | "download", name: string, total: number) => string;

/**
 * SFTP-05 in the browser: a file opened for editing is "saved" in its editor a few seconds later and
 * uploads. With `?edit=conflict` that save finds the file changed on the server; with `?edit=fail` the
 * upload fails; with `?edit=held` the files ask after the unlock, as if a save made behind the lock
 * screen had not uploaded.
 */
export function createEditsMock(
  emit: Emit,
  startTransfer: StartTransfer,
  demo: string | null,
): Pick<
  HatobaApi,
  | "sftp_edit_open"
  | "sftp_edit_reopen"
  | "sftp_edit_session"
  | "sftp_edit_upload"
  | "sftp_edit_discard"
  | "sftp_edit_stop"
  | "sftp_edit_changed"
  | "sftp_edit_list"
  | "sftp_edit_quit"
> & { unlock(): void } {
  const edits = new Map<string, RemoteEditView>();
  const sessions = new Map<string, string>();
  let seq = 0;

  const set = (id: string, state: RemoteEditState, error: string | null = null, held = false) => {
    const edit = edits.get(id);
    if (!edit) return;
    const next = { ...edit, state, error, held };
    edits.set(id, next);
    emit("sftp://edit", { edit_id: id, edit: next });
  };

  const end = (id: string) => {
    edits.delete(id);
    emit("sftp://edit", { edit_id: id, edit: null });
  };

  const upload = (id: string, overwrite: boolean) => {
    const edit = edits.get(id);
    const sid = edit && sessions.get(edit.tab_id);
    if (!edit || !sid) return;
    set(id, "uploading");
    setTimeout(() => {
      if (!edits.has(id)) return;
      if (!overwrite && demo === "conflict") return set(id, "conflict");
      if (demo === "fail") return set(id, "failed", `create ${edit.remote_path}.3f9a1c2e.part: Permission denied`);
      startTransfer(sid, "upload", edit.name, 2_400);
      setTimeout(() => set(id, "saved"), 300);
    }, 400);
  };

  return {
    sftp_edit_open: async (tabId, sessionId, path) => {
      sessions.set(tabId, sessionId);
      const open = [...edits.values()].find((e) => e.tab_id === tabId && e.remote_path === path);
      if (open) return open;
      const name = path.split("/").pop() || "file";
      const id = `edit-${++seq}`;
      const edit: RemoteEditView = {
        edit_id: id,
        tab_id: tabId,
        name,
        remote_path: path,
        state: "saved",
        error: null,
        held: false,
      };
      startTransfer(sessionId, "download", name, 2_400);
      await new Promise((r) => setTimeout(r, 300));
      edits.set(id, edit);
      emit("sftp://edit", { edit_id: id, edit });
      // The user saves in the editor.
      setTimeout(() => upload(id, false), 3500);
      return edit;
    },
    sftp_edit_reopen: async () => {},
    sftp_edit_session: async (tabId, sessionId) => {
      if (sessionId) sessions.set(tabId, sessionId);
      else sessions.delete(tabId);
    },
    sftp_edit_upload: async (id, overwrite) => upload(id, overwrite),
    sftp_edit_discard: async (id) => set(id, "saved"),
    sftp_edit_stop: async (id, doUpload) => {
      const edit = edits.get(id);
      if (!edit) return true;
      if (doUpload && edit.state !== "saved") {
        upload(id, false);
        if (demo === "conflict" || demo === "fail") return false;
      }
      end(id);
      return true;
    },
    sftp_edit_changed: async (id) => (edits.get(id)?.state ?? "saved") !== "saved",
    sftp_edit_list: async () => [...edits.values()],
    sftp_edit_quit: async () => {
      for (const id of [...edits.keys()]) end(id);
    },
    unlock: () => {
      if (demo !== "held") return;
      for (const edit of [...edits.values()]) set(edit.edit_id, "pending", null, true);
    },
  };
}
