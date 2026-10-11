import { confirm } from "@/components/overlay";
import { t, useT } from "@/i18n";
import type { AiFileChange, AiFilePreview } from "@/ipc/types";
import { FileChange } from "./FileChange";
import s from "./RevertConfirm.module.css";

/**
 * AI-41: asks before a revert writes to the server. The body names the server and shows the diff the revert
 * writes back, from the file as the call wrote it to what it held before (`preview` is the call's change, null
 * when it could not be read), and says what a revert does not undo. A file the call created is deleted.
 */
export function confirmRevert(change: AiFileChange, preview: AiFilePreview | null): Promise<boolean> {
  return confirm({
    title: t(change.created ? "ai.revert.deleteTitle" : "ai.revert.confirmTitle", { path: change.path }),
    body: <RevertBody change={change} preview={preview} />,
    confirmLabel: t(change.created ? "ai.revert.delete" : "ai.revert.action"),
    danger: true,
    width: 560,
  });
}

function RevertBody({ change, preview }: { change: AiFileChange; preview: AiFilePreview | null }) {
  const tr = useT();
  const back = preview && { ...preview, before: preview.after, after: preview.before ?? "" };
  return (
    <div className={s.body}>
      <div className={s.server}>{tr("ai.revert.where", { server: change.server })}</div>
      {back && (
        <div className={s.diff}>
          <FileChange state={{ preview: back, loading: false, failure: null }} />
        </div>
      )}
      <div>{tr(change.created ? "ai.revert.deleteBody" : "ai.revert.hint")}</div>
    </div>
  );
}
