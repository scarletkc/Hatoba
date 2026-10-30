import { Button, Dialog, FooterSpacer } from "@hatoba/ui";
import { useTabs } from "@/app/tabs";
import { useT } from "@/i18n";
import type { RemoteEditView } from "@/ipc/types";
import { discardEdit, settleHeld, uploadEdit, useEdits, useUnsentAsk, type UnsentChoice } from "@/session/edits";
import s from "./EditDialogs.module.css";

/**
 * SFTP-05's questions: a file that changed on the server, saves that did not upload when editing stops,
 * the tab closes or Hatoba quits, and those found after the unlock. Behind the lock screen only quitting
 * asks, naming no file or host, and its saves can only be discarded.
 */
export function EditDialogs({ locked }: { locked: boolean }) {
  const items = useEdits((st) => st.items);
  const deferred = useEdits((st) => st.deferred);
  const ask = useUnsentAsk((st) => st.pending);
  if (locked)
    return ask?.quit ? <UnsentDialog edits={ask.edits} canUpload={false} quit locked onChoose={ask.resolve} /> : null;
  const held = Object.values(items).filter((e) => e.held);
  const conflict = Object.values(items).find((e) => e.state === "conflict" && !e.held && !deferred[e.edit_id]);
  if (ask) return <UnsentDialog edits={ask.edits} canUpload={ask.canUpload} quit={ask.quit} onChoose={ask.resolve} />;
  if (held.length > 0)
    return <UnsentDialog edits={held} canUpload held onChoose={(c) => c && void settleHeld(held, c)} />;
  if (conflict) return <ConflictDialog key={conflict.edit_id} edit={conflict} />;
  return null;
}

/** The host a tab is connected to, as its tab names it. */
function useTabTitle(tabId: string): string {
  return useTabs((st) => st.tabs.find((x) => x.id === tabId)?.title ?? "");
}

function ConflictDialog({ edit }: { edit: RemoteEditView }) {
  const t = useT();
  const host = useTabTitle(edit.tab_id);
  const later = () => useEdits.getState().defer(edit.edit_id, true);
  return (
    <Dialog
      icon="warning"
      iconColor="var(--orange)"
      title={t("sftp.edit.conflict.title", { name: edit.name })}
      body={t("sftp.edit.conflict.body", { host })}
      onClose={later}
      actions={
        <>
          <Button onClick={() => void discardEdit(edit)}>{t("sftp.edit.discardMine")}</Button>
          <Button variant="primary" autoFocus onClick={() => void uploadEdit(edit, true)}>
            {t("sftp.edit.overwrite")}
          </Button>
        </>
      }
    />
  );
}

/**
 * Upload or Discard for saves that did not upload. Stopping, closing a tab and quitting can be cancelled;
 * the copies found after the unlock wait for an answer.
 */
function UnsentDialog({
  edits,
  canUpload,
  held,
  quit,
  locked,
  onChoose,
}: {
  edits: RemoteEditView[];
  canUpload: boolean;
  held?: boolean;
  quit?: boolean;
  /** Over the lock screen, which hides the files' names. */
  locked?: boolean;
  onChoose(choice: UnsentChoice | null): void;
}) {
  const t = useT();
  const host = useTabTitle(edits[0]?.tab_id ?? "");
  const n = edits.length;
  const one = n === 1;
  const title = locked
    ? t("sftp.edit.unsent.quitLockedTitle")
    : one
      ? t("sftp.edit.unsent.titleOne", { name: edits[0].name })
      : t("sftp.edit.unsent.title", { n });
  const body = locked
    ? t("sftp.edit.unsent.quitLocked", { n })
    : held
      ? t("sftp.edit.unsent.locked", { n })
      : quit && canUpload
        ? t("sftp.edit.unsent.quit", { n })
        : canUpload
          ? t("sftp.edit.unsent.body", { n, host })
          : t("sftp.edit.unsent.offline", { n, host });
  return (
    <Dialog
      icon="cloud-arrow-up"
      title={title}
      body={body}
      whileLocked={locked}
      onClose={held ? undefined : () => onChoose(null)}
      actions={
        <>
          <Button onClick={() => onChoose("discard")}>{t("sftp.edit.discard")}</Button>
          {!held && (
            <>
              <FooterSpacer />
              <Button onClick={() => onChoose(null)}>{t("btn.cancel")}</Button>
            </>
          )}
          {canUpload && (
            <Button variant="primary" autoFocus onClick={() => onChoose("upload")}>
              {t("sftp.edit.upload")}
            </Button>
          )}
        </>
      }
    >
      {!one && !locked && (
        <ul className={s.files}>
          {edits.map((e) => (
            <li key={e.edit_id} className={s.file} title={e.remote_path}>
              {e.name}
            </li>
          ))}
        </ul>
      )}
    </Dialog>
  );
}
