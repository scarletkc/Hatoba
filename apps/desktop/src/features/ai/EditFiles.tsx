import { useEffect, useState } from "react";
import { create } from "zustand";
import { Button, Icon, Spinner } from "@/components/controls";
import { Dialog, useOverlaysLocked } from "@/components/overlay";
import { useT } from "@/i18n";
import type { AiFileChange, AiFileRevert } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { revertFailure } from "./revertText";
import s from "./EditFiles.module.css";

/** An edit (AI-26) that deletes calls whose file changes this device can revert (AI-41). */
interface Ask {
  /** The messages after the edited one, which the edit deletes. */
  later: number;
  /** Oldest call first. */
  changes: AiFileChange[];
  revert: (change: AiFileChange) => Promise<AiFileRevert>;
  resolve: (send: boolean) => void;
}

const useAsk = create<{ ask: Ask | null }>(() => ({ ask: null }));

/**
 * Asks before an edit deletes the calls that made `changes`: revert them and send, keep the files as they
 * are and send, or cancel. Resolves true when the edit goes ahead. `revert` reverts one change.
 */
export function askEditedFiles(later: number, changes: AiFileChange[], revert: Ask["revert"]): Promise<boolean> {
  return new Promise((resolve) => {
    useAsk.getState().ask?.resolve(false);
    useAsk.setState({ ask: { later, changes, revert, resolve } });
  });
}

function close(ask: Ask, send: boolean) {
  if (useAsk.getState().ask === ask) useAsk.setState({ ask: null });
  ask.resolve(send);
}

/** The dialog of [`askEditedFiles`] (mounted once by the panel). The lock cancels it. */
export function EditFilesHost() {
  const ask = useAsk((st) => st.ask);
  const locked = useOverlaysLocked();
  useEffect(() => {
    if (locked && ask) close(ask, false);
  }, [locked, ask]);
  if (!ask || locked) return null;
  return <EditFilesDialog key={ask.changes.map(keyOf).join()} ask={ask} />;
}

const keyOf = (c: AiFileChange) => `${c.response_id}:${c.tool_call_id}`;

type Result = { state: "reverting" } | { state: "reverted" } | { state: "failed"; reason: string };

/**
 * Lists each change newest call first, the order a revert takes, with its server and whether reverting
 * deletes the file. Neither way of sending is the default. Changes that could not be reverted keep their
 * reason, and the user sends anyway or cancels; the reverts done stay done either way.
 */
function EditFilesDialog({ ask }: { ask: Ask }) {
  const t = useT();
  const [phase, setPhase] = useState<"choose" | "reverting" | "failed">("choose");
  const [results, setResults] = useState<Record<string, Result>>({});
  const newestFirst = [...ask.changes].reverse();

  const revertAll = async () => {
    setPhase("reverting");
    let failed = false;
    for (const change of newestFirst) {
      if (useAsk.getState().ask !== ask) return; // the lock closed the dialog
      const key = keyOf(change);
      setResults((r) => ({ ...r, [key]: { state: "reverting" } }));
      const outcome = await ask.revert(change);
      let result: Result;
      if (outcome.status === "reverted") result = { state: "reverted" };
      else if (outcome.status === "changed") result = { state: "failed", reason: t(outcome.current === null ? "ai.editFiles.gone" : "ai.editFiles.changed") };
      else result = { state: "failed", reason: revertFailure(t, outcome.reason, outcome.detail) };
      failed ||= result.state === "failed";
      setResults((r) => ({ ...r, [key]: result }));
    }
    if (useAsk.getState().ask !== ask) return;
    if (failed) setPhase("failed");
    else close(ask, true);
  };

  const reverting = phase === "reverting";
  return (
    <Dialog
      title={t("ai.edit.confirmTitle")}
      icon="warning-circle"
      iconColor="var(--red)"
      body={
        <>
          {t("ai.edit.confirmBody", { n: ask.later })} {t("ai.editFiles.body", { n: ask.changes.length })}
        </>
      }
      onClose={reverting ? undefined : () => close(ask, false)}
      actions={
        <>
          <Button disabled={reverting} onClick={() => close(ask, false)}>
            {t("btn.cancel")}
          </Button>
          {phase === "failed" ? (
            <Button variant="danger" onClick={() => close(ask, true)}>
              {t("ai.editFiles.sendAnyway")}
            </Button>
          ) : (
            <>
              <Button disabled={reverting} onClick={() => close(ask, true)}>
                {t("ai.editFiles.keep")}
              </Button>
              <Button icon="arrow-counter-clockwise" busy={reverting} onClick={() => void revertAll()}>
                {t("ai.editFiles.revert")}
              </Button>
            </>
          )}
        </>
      }
    >
      <ul className={s.files} aria-label={t("ai.editFiles.list")}>
        {newestFirst.map((change) => {
          const result = results[keyOf(change)];
          return (
            <li key={keyOf(change)} className={s.file}>
              <Icon name={change.created ? "file-plus" : "note-pencil"} size={14} className={s.fileIcon} />
              <div className={s.fileText}>
                <div className={cx(s.path, "selectable")}>{change.path}</div>
                <div className={s.server}>
                  {change.server}
                  {change.created && ` · ${t("ai.editFiles.created")}`}
                </div>
                {result?.state === "failed" && (
                  <div className={s.reason} role="alert">
                    {result.reason}
                  </div>
                )}
              </div>
              {result?.state === "reverting" && (
                <span className={s.status}>
                  <Spinner size={13} />
                </span>
              )}
              {result?.state === "reverted" && (
                <span className={s.status} role="img" aria-label={t("ai.revert.reverted")}>
                  <Icon name="check-circle" size={15} fill color="var(--green)" />
                </span>
              )}
              {result?.state === "failed" && (
                <span className={s.status}>
                  <Icon name="warning-circle" size={15} fill color="var(--red)" />
                </span>
              )}
            </li>
          );
        })}
      </ul>
      <div className={cx(s.note, phase === "failed" && s.noteFailed)} role={phase === "failed" ? "alert" : undefined}>
        {t(phase === "failed" ? "ai.editFiles.failed" : "ai.editFiles.hint")}
      </div>
    </Dialog>
  );
}
