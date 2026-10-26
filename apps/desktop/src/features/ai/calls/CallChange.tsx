import { useEffect, useState } from "react";
import { ErrorText, LinkButton } from "@hatoba/ui";
import { useT } from "@/i18n";
import type { AiFileChange, AiFilePreview, AiToolCall } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { fileChangeDiff } from "../actions/fileChanges";
import { callFileChange, prettyArgs } from "../tools";
import { FileChange } from "./FileChange";
import s from "../messages/Messages.module.css";

/**
 * AI-39, AI-40: the change an `edit_file` or `write_file` call made, as a diff: from its snapshot while this
 * device keeps one (AI-41), otherwise from the call's arguments. The arguments themselves are one click away.
 */
export function CallChange({ slotId, call, change }: { slotId: string; call: AiToolCall; change?: AiFileChange }) {
  const t = useT();
  const [input, setInput] = useState(false);
  const keeps = change?.state === "written" || change?.state === "unfinished";
  // undefined while it loads; "failed" when the snapshot could not be read, so nothing stands in for it.
  const [kept, setKept] = useState<AiFilePreview | null | undefined | "failed">(undefined);
  // Read once per change and state: a refresh of the slot's file changes gives the same change a new object.
  const [responseId, callId, state] = [change?.response_id, change?.tool_call_id, change?.state];
  useEffect(() => {
    if (!responseId || !callId || (state !== "written" && state !== "unfinished")) return;
    let live = true;
    fileChangeDiff(slotId, { response_id: responseId, tool_call_id: callId }).then(
      (preview) => live && setKept(preview),
      () => live && setKept("failed"),
    );
    return () => {
      live = false;
    };
  }, [slotId, responseId, callId, state]);
  const failed = keeps && kept === "failed";
  const preview = failed
    ? null
    : ((keeps && kept !== "failed" ? kept : null) ?? callFileChange(call.name, call.arguments));
  return (
    <>
      <div className={s.labelRow}>
        <span className={s.label}>{t("ai.call.change")}</span>
        <LinkButton tone="muted" aria-expanded={input} onClick={() => setInput((v) => !v)}>
          {t(input ? "ai.call.hideInput" : "ai.call.showInput")}
        </LinkButton>
      </div>
      {failed && <ErrorText>{t("ai.call.changeUnreadable")}</ErrorText>}
      {preview && <FileChange state={{ preview, loading: keeps && kept === undefined, failure: null }} />}
      {(input || !preview) && <pre className={cx(s.pre, "selectable")}>{prettyArgs(call.arguments) || "{}"}</pre>}
    </>
  );
}
