import { Button, Dialog, FooterSpacer } from "@hatoba/ui";
import { useTabs } from "@/app/tabs";
import { useT } from "@/i18n";
import type { RemoteEditView } from "@/ipc/types";
import { discardEdit, settleHeld, uploadEdit, useEdits, useUnsentAsk, type UnsentChoice } from "./edits";
import s from "./EditDialogs.module.css";

/**
 * SFTP-05's questions, mounted once while the vault is unlocked: a file that changed on the server, saves
 * that did not upload when editing stops or the tab closes, and those the lock kept.
 */
export function EditDialogs() {
  const items = useEdits((st) => st.items);
  const deferred = useEdits((st) => st.deferred);
  const ask = useUnsentAsk((st) => st.pending);
  const held = Object.values(items).filter((e) => e.held);
  const conflict = Object.values(items).find((e) => e.state === "conflict" && !e.held && !deferred[e.edit_id]);
  if (ask) return <UnsentDialog edits={ask.edits} canUpload={ask.canUpload} onChoose={ask.resolve} />;
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
 * Upload or Discard for saves that did not upload. Stopping or closing a tab can be cancelled; the copies
 * the lock kept wait for an answer.
 */
function UnsentDialog({
  edits,
  canUpload,
  held,
  onChoose,
}: {
  edits: RemoteEditView[];
  canUpload: boolean;
  held?: boolean;
  onChoose(choice: UnsentChoice | null): void;
}) {
  const t = useT();
  const host = useTabTitle(edits[0]?.tab_id ?? "");
  const one = edits.length === 1;
  const title = one
    ? t("sftp.edit.unsent.titleOne", { name: edits[0].name })
    : t("sftp.edit.unsent.title", { n: edits.length });
  const body = held
    ? t("sftp.edit.unsent.locked", { n: edits.length })
    : canUpload
      ? t("sftp.edit.unsent.body", { n: edits.length, host })
      : t("sftp.edit.unsent.offline", { n: edits.length, host });
  return (
    <Dialog
      icon="cloud-arrow-up"
      title={title}
      body={body}
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
      {!one && (
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
