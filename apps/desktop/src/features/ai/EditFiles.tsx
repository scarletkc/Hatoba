import { useEffect, useMemo, useState } from "react";
import { create } from "zustand";
import { Button, Icon, Spinner } from "@/components/controls";
import { Dialog, useOverlaysLocked } from "@/components/overlay";
import { ErrorText, HintText, ListRow } from "@/components/patterns";
import { useT } from "@/i18n";
import type { AiFileChange, AiFilePreview, AiFileRevert } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { diffLines, diffStats } from "./diff";
import { revertFailure, serverLabel } from "./revertText";
import s from "./EditFiles.module.css";

/** An edit (AI-26) that deletes calls whose file changes this device can revert (AI-41). */
interface Ask {
  /** The messages after the edited one, which the edit deletes. */
  later: number;
  /** Oldest call first. */
  changes: AiFileChange[];
  /**
   * The server the slot's tab is connected to, as changes name theirs; null while it is not connected. Only
   * changes on it can be reverted from the tab.
   */
  server: string | null;
  /** Reverts one change. */
  revert: (change: AiFileChange) => Promise<AiFileRevert>;
  /** What a change did, for the lines a revert adds and removes; null when it cannot be read. */
  diff?: (change: AiFileChange) => Promise<AiFilePreview | null>;
  resolve: (send: boolean) => void;
}

const useAsk = create<{ ask: Ask | null }>(() => ({ ask: null }));

/**
 * Asks before an edit deletes the calls that made `changes`: revert them and send, keep the files as they
 * are and send, or cancel. Resolves true when the edit goes ahead.
 */
export function askEditedFiles(ask: Omit<Ask, "resolve">): Promise<boolean> {
  return new Promise((resolve) => {
    useAsk.getState().ask?.resolve(false);
    useAsk.setState({ ask: { ...ask, resolve } });
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

/** The changes to one file, newest call first. */
interface FileGroup {
  key: string;
  server: string;
  path: string;
  changes: AiFileChange[];
}

/** The changes grouped by file, the file whose newest change is newest first. */
export function groupChanges(newestFirst: AiFileChange[]): FileGroup[] {
  const groups = new Map<string, FileGroup>();
  for (const c of newestFirst) {
    const key = `${c.server}\n${c.path}`;
    const group = groups.get(key) ?? { key, server: c.server, path: c.path, changes: [] };
    group.changes.push(c);
    groups.set(key, group);
  }
  return [...groups.values()];
}

/**
 * Lists each file once, by its path and server, with how many of the deleted calls changed it, the lines a
 * revert adds and removes, and whether reverting deletes it. Reverts go newest call first, of the files on the
 * server the tab is connected to: without one, Revert and Send is off, so Cancel keeps every change revertible
 * once the tab is connected. Neither way of sending is the default. Changes that were not reverted keep their
 * reason, and the user sends anyway, which deletes their copies, or cancels; the reverts done stay done either
 * way.
 */
function EditFilesDialog({ ask }: { ask: Ask }) {
  const t = useT();
  const [phase, setPhase] = useState<"choose" | "reverting" | "failed">("choose");
  const [results, setResults] = useState<Record<string, Result>>({});
  const [stats, setStats] = useState<Record<string, { added: number; removed: number }>>({});
  const newestFirst = useMemo(() => [...ask.changes].reverse(), [ask.changes]);
  const groups = useMemo(() => groupChanges(newestFirst), [newestFirst]);
  const here = (c: { server: string }) => c.server === ask.server;
  const anyHere = newestFirst.some(here);
  // Why a file on another server is not reverted from this tab.
  const elsewhere = ask.server === null ? t("ai.revert.failed.disconnected") : t("ai.editFiles.otherServer", { server: serverLabel(t, ask.server) });

  // What reverting each change writes back: from what the call wrote to what the file held before.
  useEffect(() => {
    const diff = ask.diff;
    if (!diff) return;
    let live = true;
    void Promise.all(newestFirst.map((c) => diff(c).catch(() => null))).then((previews) => {
      if (!live) return;
      const next: Record<string, { added: number; removed: number }> = {};
      previews.forEach((p, i) => {
        if (p?.after != null) next[keyOf(newestFirst[i])] = diffStats(diffLines(p.after, p.before ?? ""));
      });
      setStats(next);
    });
    return () => {
      live = false;
    };
  }, [ask, newestFirst]);

  const revertAll = async () => {
    setPhase("reverting");
    let failed = false;
    for (const change of newestFirst) {
      if (useAsk.getState().ask !== ask) return; // the lock closed the dialog
      const key = keyOf(change);
      if (!here(change)) {
        failed = true;
        setResults((r) => ({ ...r, [key]: { state: "failed", reason: elsewhere } }));
        continue;
      }
      setResults((r) => ({ ...r, [key]: { state: "reverting" } }));
      let outcome: AiFileRevert;
      try {
        outcome = await ask.revert(change);
      } catch (e) {
        outcome = { status: "failed", reason: "failed", detail: e instanceof Error ? e.message : String(e) };
      }
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
          {t("ai.edit.confirmBody", { n: ask.later })} {t("ai.editFiles.body", { n: groups.length })}
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
              <Button icon="arrow-counter-clockwise" busy={reverting} disabled={!anyHere} onClick={() => void revertAll()}>
                {t("ai.editFiles.revert")}
              </Button>
            </>
          )}
        </>
      }
    >
      <ul className={s.files} aria-label={t("ai.editFiles.list")}>
        {groups.map((group) => {
          const states = group.changes.map((c) => results[keyOf(c)]);
          const reasons = [...new Set(states.flatMap((r) => (r?.state === "failed" ? [r.reason] : [])))];
          const state = states.some((r) => r?.state === "reverting")
            ? "reverting"
            : reasons.length > 0
              ? "failed"
              : states.every((r) => r?.state === "reverted")
                ? "reverted"
                : null;
          // Reverting every change of the file deletes it when its oldest call created it.
          const created = group.changes[group.changes.length - 1].created;
          const counted = group.changes.map((c) => stats[keyOf(c)]);
          const lines = counted.every(Boolean) ? counted.reduce((sum, x) => ({ added: sum.added + x!.added, removed: sum.removed + x!.removed }), { added: 0, removed: 0 }) : null;
          return (
            <li key={group.key} className={s.file}>
              <ListRow
                className={s.fileRow}
                leading={<Icon name={created ? "file-plus" : "note-pencil"} size={14} className={s.fileIcon} />}
                trailing={
                  <>
                    {state === "reverting" && (
                      <span className={s.status}>
                        <Spinner size={14} />
                      </span>
                    )}
                    {state === "reverted" && (
                      <span className={s.status} role="img" aria-label={t("ai.revert.reverted")}>
                        <Icon name="check-circle" size={16} fill color="var(--green)" />
                      </span>
                    )}
                    {state === "failed" && (
                      <span className={s.status}>
                        <Icon name="warning-circle" size={16} fill color="var(--red)" />
                      </span>
                    )}
                  </>
                }
              >
                <div className={cx(s.path, "selectable")}>{group.path}</div>
                <div className={s.server}>
                  {serverLabel(t, group.server)}
                  {group.changes.length > 1 && ` · ${t("ai.editFiles.changes", { n: group.changes.length })}`}
                  {created && ` · ${t("ai.editFiles.created")}`}
                  {lines && (lines.added > 0 || lines.removed > 0) && (
                    <span className={s.lines} aria-label={t("ai.approval.diffStats", lines)}>
                      {lines.added > 0 && <span className={s.added}>+{lines.added}</span>}
                      {lines.removed > 0 && <span className={s.removed}>−{lines.removed}</span>}
                    </span>
                  )}
                </div>
                {phase === "choose" && ask.server !== null && !here(group) && <HintText>{elsewhere}</HintText>}
                {reasons.map((reason) => (
                  <ErrorText key={reason}>{reason}</ErrorText>
                ))}
              </ListRow>
            </li>
          );
        })}
      </ul>
      {phase === "failed" ? (
        <ErrorText className={s.note}>{t("ai.editFiles.failed")}</ErrorText>
      ) : (
        <HintText className={s.note}>
          {anyHere
            ? t("ai.editFiles.hint")
            : ask.server === null
              ? t("ai.editFiles.disconnected")
              : t("ai.editFiles.noneHere", { server: serverLabel(t, ask.server) })}
        </HintText>
      )}
    </Dialog>
  );
}
