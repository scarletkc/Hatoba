import { useState } from "react";
import { Button, ErrorText, HintText, LinkButton, WarningText } from "@hatoba/ui";
import { useT } from "@/i18n";
import type { AiFileChange, AiFileRevert } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { revertFileChange } from "../actions/reverts";
import { patchSlot, useAi } from "../store";
import { FileChange } from "./FileChange";
import { revertFailure } from "./revertText";
import s from "../messages/Messages.module.css";

/** AI-41: the revert of a call's change, which the Revert in the block's header and the one in its body share. */
export function useRevert(slotId: string, change: AiFileChange | undefined) {
  const [running, setRunning] = useState<"revert" | "restore" | null>(null);
  const [outcome, setOutcome] = useState<{ result: AiFileRevert; restore: boolean } | null>(null);
  const run = async (restore: boolean) => {
    if (!change) return;
    try {
      const next = await revertFileChange(slotId, change, {
        restore,
        recreate: outcome?.result.status === "changed" && outcome.result.current === null,
        onStart: () => {
          setRunning(restore ? "restore" : "revert");
          if (!restore) setOutcome(null);
        },
      });
      if (next) setOutcome({ result: next, restore });
    } finally {
      setRunning(null);
    }
  };
  return { running, outcome, run };
}

export type RevertState = ReturnType<typeof useRevert>;

/**
 * AI-41: Revert on the block of a file tool call that wrote, or whose write failed after it may have begun; its
 * confirmation says what it restores and what it does not. When the file holds neither what the call wrote nor
 * what it held before, the block shows the diff from what it holds now back to that, links the later call that
 * wrote it too, and offers Restore anyway over exactly what the diff shows; or the failure.
 */
export function RevertChange({
  slotId,
  change,
  busy,
  revert,
}: {
  slotId: string;
  change: AiFileChange;
  busy: boolean;
  revert: RevertState;
}) {
  const t = useT();
  const { running, outcome, run } = revert;
  const result = outcome?.result;
  // The result of the newest later call that wrote the same file, which the diff can jump to.
  const laterEntry = useAi((st) => {
    const slot = st.slots[slotId];
    if (!slot) return null;
    const at = slot.fileChanges.findIndex(
      (c) => c.response_id === change.response_id && c.tool_call_id === change.tool_call_id,
    );
    const later = slot.fileChanges
      .slice(at + 1)
      .filter(
        (c) =>
          c.server === change.server && c.path === change.path && (c.state === "written" || c.state === "unfinished"),
      )
      .at(-1);
    if (!later) return null;
    return (
      slot.entries.find(
        (e) => e.role === "tool" && e.tool_call_id === later.tool_call_id && e.entry_id > later.response_id,
      )?.entry_id ?? null
    );
  });
  return (
    <div className={s.revert}>
      <div className={s.revertRow}>
        <Button
          size="sm"
          icon="arrow-counter-clockwise"
          busy={running === "revert"}
          disabled={busy || !!running}
          title={busy ? t("ai.revert.busy") : undefined}
          onClick={() => void run(false)}
        >
          {t("ai.revert.action")}
        </Button>
      </div>
      {change.state === "unfinished" && <WarningText>{t("ai.revert.unfinished")}</WarningText>}
      {result?.status === "changed" && (
        <>
          <ErrorText>
            {outcome?.restore && `${t("ai.revert.changedAgain")} `}
            {t(
              result.before === null
                ? "ai.revert.changedCreated"
                : result.current === null
                  ? "ai.revert.gone"
                  : "ai.revert.changed",
            )}
          </ErrorText>
          {result.later && (
            <HintText>
              {t("ai.revert.later")}
              {laterEntry && (
                <>
                  {" "}
                  <LinkButton className={s.inlineLink} onClick={() => patchSlot(slotId, { reveal: laterEntry })}>
                    {t("ai.revert.goLater")}
                  </LinkButton>
                </>
              )}
            </HintText>
          )}
          <FileChange
            state={{
              preview: {
                path: result.path,
                before: result.current,
                after: result.before ?? "",
                error: null,
                crlf: false,
              },
              loading: false,
              failure: null,
            }}
          />
          <div className={s.revertRow}>
            <Button
              size="sm"
              variant="danger"
              icon={result.before === null ? "trash" : "arrow-counter-clockwise"}
              busy={running === "restore"}
              disabled={busy || !!running}
              onClick={() => void run(true)}
            >
              {t(result.before === null ? "ai.revert.deleteAnyway" : "ai.revert.restore")}
            </Button>
          </div>
        </>
      )}
      {result?.status === "failed" && (
        <ErrorText>
          {revertFailure(t, result.reason, result.detail)}
          {result.detail && result.reason !== "other_server" && (
            <div className={cx(s.revertDetail, "selectable")}>{result.detail}</div>
          )}
        </ErrorText>
      )}
    </div>
  );
}
