import { Icon, IconButton, Spinner } from "@hatoba/ui";
import { useT } from "@/i18n";
import type { RemoteEditView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { reopenEdit, stopEditing, tabEdits, uploadEdit, useEdits } from "./edits";
import s from "./EditList.module.css";

/** SFTP-05: the files the tab is editing in a local editor, between the file list and the transfers. */
export function EditList({ tabId }: { tabId: string }) {
  const t = useT();
  const items = useEdits((st) => st.items);
  const edits = tabEdits(items, tabId);
  if (edits.length === 0) return null;
  return (
    <div className={s.section}>
      <div className={s.header}>{t("sftp.editing")}</div>
      {edits.map((edit) => (
        <EditRow key={edit.edit_id} edit={edit} />
      ))}
    </div>
  );
}

function EditRow({ edit }: { edit: RemoteEditView }) {
  const t = useT();
  const bad = edit.state === "failed";
  const status =
    edit.state === "saved"
      ? t("sftp.edit.state.saved")
      : edit.state === "pending"
        ? t("sftp.edit.state.pending")
        : edit.state === "uploading"
          ? t("sftp.edit.state.uploading")
          : edit.state === "conflict"
            ? t("sftp.edit.state.conflict")
            : (edit.error ?? t("sftp.edit.state.cancelled"));
  return (
    <div className={s.row}>
      <button
        type="button"
        className={s.open}
        title={`${t("sftp.edit.openAgain")}\n${edit.remote_path}`}
        onClick={() => void reopenEdit(edit)}
      >
        <Icon name="note-pencil" size={14} color="var(--accent)" />
        <span className={s.name}>{edit.name}</span>
      </button>
      <span
        className={cx(s.status, bad && s.statusFailed, edit.state === "conflict" && s.statusConflict)}
        title={bad ? status : undefined}
      >
        {edit.state === "uploading" && <Spinner size={10} />}
        {status}
      </span>
      {bad && (
        <IconButton
          small
          icon="arrows-clockwise"
          label={t("sftp.edit.retry")}
          className={s.action}
          onClick={() => void uploadEdit(edit, false)}
        />
      )}
      {edit.state === "conflict" && (
        <IconButton
          small
          icon="warning"
          label={t("sftp.edit.resolve")}
          className={s.action}
          onClick={() => useEdits.getState().defer(edit.edit_id, false)}
        />
      )}
      <IconButton
        small
        icon="x"
        label={t("sftp.edit.stop")}
        className={s.action}
        onClick={() => void stopEditing(edit, true)}
      />
    </div>
  );
}
