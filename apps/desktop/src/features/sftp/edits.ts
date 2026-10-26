import { create } from "zustand";
import { toast } from "@hatoba/ui";
import { errorMessage } from "@/app/errors";
import { useTabs } from "@/app/tabs";
import { t } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { RemoteEditEvent, RemoteEditView } from "@/ipc/types";

/** SFTP-05: the remote files being edited in a local editor, by edit id. */
interface EditsState {
  items: Record<string, RemoteEditView>;
  /** Conflicts whose question the user put off; one asks again once it is resolved and comes back. */
  deferred: Record<string, true>;
  handle(event: RemoteEditEvent): void;
  load(list: RemoteEditView[]): void;
  defer(editId: string, deferred: boolean): void;
}

export const useEdits = create<EditsState>((set) => ({
  items: {},
  deferred: {},
  handle: ({ edit_id, edit }) =>
    set((s) => {
      const { [edit_id]: _gone, ...items } = s.items;
      const { [edit_id]: _put, ...deferred } = s.deferred;
      // A conflict that was resolved asks again the next time.
      const keep = edit?.state === "conflict" && s.deferred[edit_id];
      return { items: edit ? { ...items, [edit_id]: edit } : items, deferred: keep ? s.deferred : deferred };
    }),
  load: (list) => set({ items: Object.fromEntries(list.map((e) => [e.edit_id, e])) }),
  defer: (editId, deferred) =>
    set((s) => {
      const { [editId]: _put, ...rest } = s.deferred;
      return { deferred: deferred ? { ...s.deferred, [editId]: true } : rest };
    }),
}));

/** The edits opened from a tab, oldest first. */
export function tabEdits(items: Record<string, RemoteEditView>, tabId: string): RemoteEditView[] {
  return Object.values(items)
    .filter((e) => e.tab_id === tabId)
    .sort((a, b) => a.edit_id.localeCompare(b.edit_id));
}

/** The copy's last save is not on the server. */
export const unsent = (e: RemoteEditView) => e.state !== "saved";

const isConnected = (tabId: string) => useTabs.getState().tabs.find((x) => x.id === tabId)?.status === "connected";

/** The tab connected with a new session: saves that waited for it upload. */
export function editSessionChanged(tabId: string, sessionId: string) {
  const items = useEdits.getState().items;
  if (Object.values(items).some((e) => e.tab_id === tabId))
    void api.sftp_edit_session(tabId, sessionId).catch(() => {});
}

function failMessage(e: unknown, name: string): string {
  return toAppError(e).code === "editor" ? t("sftp.edit.noEditor", { name }) : errorMessage(t, e);
}

export async function editFile(tabId: string, sessionId: string, path: string, name: string) {
  try {
    await api.sftp_edit_open(tabId, sessionId, path);
  } catch (e) {
    toast(failMessage(e, name), "error");
  }
}

export async function reopenEdit(edit: RemoteEditView) {
  try {
    await api.sftp_edit_reopen(edit.edit_id);
  } catch (e) {
    toast(failMessage(e, edit.name), "error");
  }
}

/** Retry, or Overwrite after a conflict. A new conflict or failure shows on the edit itself. */
export async function uploadEdit(edit: RemoteEditView, overwrite: boolean) {
  await api.sftp_edit_upload(edit.edit_id, overwrite).catch(() => {});
}

export async function discardEdit(edit: RemoteEditView) {
  try {
    await api.sftp_edit_discard(edit.edit_id);
  } catch (e) {
    toast(errorMessage(t, e), "error");
  }
}

// ───────── Saves that did not upload: Upload or Discard ─────────

export type UnsentChoice = "upload" | "discard";

interface UnsentAsk {
  edits: RemoteEditView[];
  /** The tab is connected, so the saves can upload now. */
  canUpload: boolean;
  /** Hatoba quits after the answer. */
  quit: boolean;
  resolve(choice: UnsentChoice | null): void;
}

export const useUnsentAsk = create<{ pending: UnsentAsk | null }>(() => ({ pending: null }));

/** Asks whether to upload or discard the saves of `edits` that did not upload; null keeps editing. */
export function askUnsent(edits: RemoteEditView[], canUpload: boolean, quit = false): Promise<UnsentChoice | null> {
  useUnsentAsk.getState().pending?.resolve(null);
  return new Promise((resolve) =>
    useUnsentAsk.setState({
      pending: {
        edits,
        canUpload,
        quit,
        resolve: (choice) => {
          useUnsentAsk.setState({ pending: null });
          resolve(choice);
        },
      },
    }),
  );
}

/** Stop Editing. A copy whose last save did not upload asks first. */
export async function stopEditing(edit: RemoteEditView, connected: boolean) {
  let upload = true;
  if (unsent(edit)) {
    const choice = await askUnsent([edit], connected);
    if (!choice) return;
    upload = choice === "upload";
  }
  const ended = await api.sftp_edit_stop(edit.edit_id, upload).catch(() => false);
  // Still uploading means a conflict or a failure that the edit now shows.
  if (!ended && upload && connected && useEdits.getState().items[edit.edit_id]?.state === "failed")
    toast(t("sftp.edit.notUploaded", { n: 1 }), "error");
}

/**
 * Ends a closing tab's edits, after asking about saves that did not upload. False keeps the tab open:
 * the question was cancelled, or a save could not upload (a conflict asks then).
 */
export async function endTabEdits(tabId: string, connected: boolean): Promise<boolean> {
  const edits = tabEdits(useEdits.getState().items, tabId);
  if (edits.length === 0) return true;
  // Connected, a save not noticed yet uploads as editing stops; disconnected, it would be lost, so the
  // copies are asked about it first.
  const changed = connected
    ? edits.map(unsent)
    : await Promise.all(edits.map((e) => unsent(e) || api.sftp_edit_changed(e.edit_id).catch(() => true)));
  const waiting = edits.filter((_, i) => changed[i]);
  let upload = connected;
  if (waiting.length > 0) {
    const choice = await askUnsent(waiting, connected);
    if (!choice) return false;
    upload = choice === "upload";
  }
  // The answer is for the listed copies; a save of another not noticed yet uploads.
  const ended = await Promise.all(
    edits.map((e, i) => api.sftp_edit_stop(e.edit_id, changed[i] ? upload : connected).catch(() => false)),
  );
  const left = ended.filter((done) => !done).length;
  if (left === 0) return true;
  toast(t("sftp.edit.notUploaded", { n: left }), "error");
  return false;
}

/**
 * After the unlock: copies whose last save did not upload, which may have been made behind the lock
 * screen. Editing goes on either way; Discard downloads the server's file into the copy again, or ends
 * editing while the tab is disconnected.
 */
export async function settleHeld(edits: RemoteEditView[], choice: UnsentChoice) {
  await Promise.all(
    edits.map((e) =>
      choice === "upload"
        ? uploadEdit(e, false)
        : isConnected(e.tab_id)
          ? discardEdit(e)
          : api.sftp_edit_stop(e.edit_id, false).catch(() => false),
    ),
  );
}

/**
 * Quitting with files being edited, which the backend holds for this: asks about saves that did not
 * upload, then quits. Behind the lock screen they can only be discarded.
 */
export async function quitEditing() {
  const edits = Object.values(useEdits.getState().items);
  const changed = await Promise.all(edits.map((e) => unsent(e) || api.sftp_edit_changed(e.edit_id).catch(() => true)));
  const waiting = edits.filter((_, i) => changed[i]);
  let upload = false;
  if (waiting.length > 0) {
    // Upload sends what it can; a copy whose tab is disconnected keeps Hatoba open, and quitting
    // again asks about it alone.
    const choice = await askUnsent(
      waiting,
      waiting.some((e) => isConnected(e.tab_id)),
      true,
    );
    if (!choice) return;
    upload = choice === "upload";
  }
  // The answer is for the listed copies; another saved while it was asked uploads like any save.
  const listed = new Set(waiting.map((e) => e.edit_id));
  const ended = await Promise.all(
    Object.values(useEdits.getState().items).map((e) =>
      api.sftp_edit_stop(e.edit_id, listed.has(e.edit_id) ? upload : isConnected(e.tab_id)).catch(() => false),
    ),
  );
  const left = ended.filter((done) => !done).length;
  if (left > 0) {
    toast(t("sftp.edit.notUploaded", { n: left }), "error");
    return;
  }
  await api.sftp_edit_quit();
}
