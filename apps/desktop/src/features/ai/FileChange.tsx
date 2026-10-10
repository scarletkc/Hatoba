import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import { Icon, Spinner } from "@/components/controls";
import { useT } from "@/i18n";
import type { AiFilePreview } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { aiErrorMessage, previewFileCall } from "./actions";
import { diffLines, diffStats, withContext } from "./diff";
import { shownText } from "./tools";
import ms from "./Messages.module.css";
import s from "./FileChange.module.css";

/** How long the card waits after the last keystroke in Edit before it asks for the diff again. */
const EDIT_PAUSE_MS = 300;

export interface FilePreviewState {
  preview: AiFilePreview | null;
  /** The preview is being read; Run waits for it, so the user approves what they saw. */
  loading: boolean;
  /** The preview could not be asked for at all (as opposed to a call that cannot apply). */
  failure: string | null;
}

/**
 * AI-39, AI-40: the change an `edit_file` or `write_file` call waiting for approval would make. Rust
 * reads the file once per call; `edited` (the arguments changed in Edit, or null) asks again after a
 * pause in typing, against the same read. Nothing is asked while `enabled` is false (a card of another
 * tool).
 */
export function useFilePreview(slotId: string, callId: string, edited: string | null, enabled: boolean): FilePreviewState {
  const [state, setState] = useState<FilePreviewState>({ preview: null, loading: enabled, failure: null });
  const asked = useRef(false);
  useEffect(() => {
    if (!enabled) return;
    let current = true;
    setState((st) => ({ ...st, loading: true }));
    const timer = window.setTimeout(
      () => {
        asked.current = true;
        previewFileCall(slotId, callId, edited).then(
          (preview) => current && setState({ preview, loading: false, failure: null }),
          (e: unknown) => current && setState({ preview: null, loading: false, failure: aiErrorMessage(e) }),
        );
      },
      asked.current ? EDIT_PAUSE_MS : 0,
    );
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
  }, [slotId, callId, edited, enabled]);
  return state;
}

/** A path or a line of the file as the approval card shows it: hidden characters as escapes (AI-17). */
function shown(text: string): { text: string; escaped: boolean } {
  const r = shownText(text);
  return { text: r.lines.join("↵"), escaped: r.escaped };
}

/**
 * The file a call changes and the change, on its approval card: the path, then a unified diff with 3
 * lines of context, or the whole content of a new file; or why the call cannot apply.
 */
export function FileChange({ state, replaceAll }: { state: FilePreviewState; replaceAll?: boolean }) {
  const t = useT();
  const { preview, loading, failure } = state;
  const lines = useMemo(() => (preview?.after != null ? diffLines(preview.before ?? "", preview.after) : null), [preview]);
  const rows = useMemo(() => (lines ? withContext(lines) : []), [lines]);
  const stats = lines ? diffStats(lines) : null;
  let escaped = false;
  const cells = rows.map((row) => {
    if (row.kind === "gap") return row;
    const r = shown(row.text);
    escaped ||= r.escaped;
    return { ...row, text: r.text };
  });

  if (!preview && loading)
    return (
      <div className={s.status}>
        <Spinner size={12} />
        {t("ai.approval.readingFile")}
      </div>
    );
  if (!preview) return <div className={ms.fieldError}>{failure}</div>;
  const path = shown(preview.path);

  return (
    <div className={s.change}>
      {path.text && (
        <div className={s.head}>
          <Icon name="file-text" size={13} className={s.fileIcon} />
          <span className={cx(s.path, "selectable")}>
            {path.text}
          </span>
          {loading && <Spinner size={11} />}
          {stats && (stats.added > 0 || stats.removed > 0) && (
            <span className={s.stats} title={t("ai.approval.diffStats", stats)} aria-label={t("ai.approval.diffStats", stats)}>
              <span className={s.added}>+{stats.added}</span>
              <span className={s.removed}>−{stats.removed}</span>
            </span>
          )}
        </div>
      )}
      {!preview.error && (
        <div className={ms.inputNotes}>
          {preview.before === null && <span className={cx(ms.chip, ms.chipOk)}>{t("ai.approval.newFile")}</span>}
          {replaceAll && <span className={cx(ms.chip, ms.chipWarn)}>{t("ai.approval.replaceAll")}</span>}
          {preview.crlf && <span className={ms.chip}>{t("ai.approval.crlf")}</span>}
          {(path.escaped || escaped) && <span className={cx(ms.chip, ms.chipWarn)}>{t("ai.approval.escaped")}</span>}
        </div>
      )}
      {preview.error ? (
        <div className={s.error} role="alert">
          <div>{t("ai.approval.cannotApply")}</div>
          <div className={cx(s.errorText, "selectable")}>{preview.error}</div>
        </div>
      ) : cells.length === 0 ? (
        <div className={ms.meta}>{t("ai.approval.noChanges")}</div>
      ) : (
        <div className={cx(s.diff, "selectable")} role="group" aria-label={t("ai.approval.diff")}>
          {cells.map((row, i) =>
            row.kind === "gap" ? (
              <div key={i} className={s.gap}>
                {t("ai.approval.unchangedLines", { n: row.count })}
              </div>
            ) : (
              <Fragment key={i}>
                <span className={cx(s.num, s[row.kind])} aria-hidden>
                  {row.old ?? ""}
                </span>
                <span className={cx(s.num, s[row.kind])} aria-hidden>
                  {row.new ?? ""}
                </span>
                <span className={cx(s.sign, s[row.kind])} aria-hidden>
                  {row.kind === "add" ? "+" : row.kind === "del" ? "−" : " "}
                </span>
                <span className={cx(s.text, s[row.kind])}>
                  {row.text}
                  {row.noEol && (
                    <span className={s.noEol} title={t("ai.approval.noEol")} aria-label={t("ai.approval.noEol")}>
                      {" ⊘"}
                    </span>
                  )}
                </span>
              </Fragment>
            ),
          )}
        </div>
      )}
    </div>
  );
}
