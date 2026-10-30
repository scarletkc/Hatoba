import { useEffect, useId, useState } from "react";
import { Chip, DisclosureCaret, HintText, Icon, IconButton, controlStyles } from "@hatoba/ui";
import { useT } from "@/i18n";
import type { AiEntryView, AiFileChange, AiToolCall, McpToolInfo } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { answersText } from "../rules/autoText";
import { callSummary, exitStatusOf, MODE_ICON, prettyArgs, shownCall, toolKind, toolLabel } from "../tools";
import type { AutoAsk, CallState } from "../turn";
import { ApprovalCard } from "./ApprovalCard";
import { AutoVerdict } from "./AutoVerdict";
import { CallChange } from "./CallChange";
import type { HostInfo } from "./hostInfo";
import { RevertChange, useRevert } from "./RevertChange";
import { TOOL_ICON } from "./toolIcons";
import s from "../messages/Messages.module.css";

export type ToolEntry = Extract<AiEntryView, { role: "tool" }>;

/** Where a call stands, from its stored result or the turn (§9: tool running, waiting for approval, …). */
export type CallView =
  | { kind: "result"; entry: ToolEntry }
  | { kind: CallState; mcp?: McpToolInfo | null; auto?: AutoAsk | null }
  | { kind: "queued" }
  | { kind: "streaming" }
  | { kind: "none" };

function StatusChip({ view, name }: { view: CallView; name: string }) {
  const t = useT();
  switch (view.kind) {
    case "result": {
      const { status, content } = view.entry;
      if (status === "ok") {
        const code = toolKind(name) === "run_command" ? exitStatusOf(content) : null;
        if (code !== null && code !== 0) return <Chip tone="error">{t("ai.call.exit", { code })}</Chip>;
        return (
          <Chip tone="success" icon="check">
            {code === 0 ? t("ai.call.exit", { code }) : t("ai.call.done")}
          </Chip>
        );
      }
      if (status === "error") return <Chip tone="error">{t("ai.call.error")}</Chip>;
      if (status === "rejected") return <Chip tone="warning">{t("ai.call.rejected")}</Chip>;
      return <Chip>{t("ai.call.cancelled")}</Chip>;
    }
    case "running":
      return <Chip busy>{t("ai.call.running")}</Chip>;
    case "judging":
      // AI-47: Auto asks its judge whether the call can run without the card.
      return (
        <Chip busy title={t("ai.call.judgingHint")}>
          {t("ai.call.judging")}
        </Chip>
      );
    case "approval":
      return <Chip tone="warning">{t("ai.call.approval")}</Chip>;
    case "queued":
    case "streaming":
      return <Chip>{t("ai.call.queued")}</Chip>;
    case "none":
      return <Chip>{t("ai.call.notRun")}</Chip>;
  }
}

/**
 * Each call is a collapsible block with its input, output and status (§9); approval cards appear in place. An
 * `edit_file` or `write_file` call shows its change as a diff. A change of a file tool call this device keeps
 * can be reverted from the block's header, on hover or focus, or from its body (AI-41).
 */
export function ToolBlock({
  slotId,
  call,
  view,
  change,
  busy,
  host,
  reveal,
  flash,
}: {
  slotId: string;
  call: AiToolCall;
  view: CallView;
  change?: AiFileChange;
  busy: boolean;
  host: HostInfo | null;
  /** The key a search hit or a link reveals the block by (AI-24). */
  reveal?: string;
  flash?: boolean;
}) {
  const t = useT();
  const [open, setOpen] = useState(false);
  // AI-45: a `call_mcp_tool` call shows as the MCP tool it names.
  const shown = shownCall(call);
  const kind = toolKind(shown.name);
  const summary = callSummary(shown.name, shown.arguments);
  const revertible = change?.state === "written" || change?.state === "unfinished";
  const revert = useRevert(slotId, change);
  const statusId = useId();
  useEffect(() => {
    if (flash) setOpen(true);
  }, [flash]);

  if (view.kind === "approval")
    return <ApprovalCard slotId={slotId} call={call} host={host} mcp={view.mcp ?? null} auto={view.auto ?? null} />;
  // AI-47: the judge's verdict, when Auto decided the call.
  const auto = view.kind === "result" ? view.entry.auto : null;

  return (
    <div className={cx(s.tool, flash && s.flash)} data-reveal={reveal}>
      <div className={s.toolHeadRow}>
        <button
          type="button"
          className={s.toolHead}
          aria-expanded={open}
          aria-describedby={statusId}
          onClick={() => setOpen((v) => !v)}
        >
          <Icon name={TOOL_ICON[kind]} size={14} className={s.toolIcon} />
          <span className={s.toolName}>{toolLabel(t, shown.name, shown.arguments)}</span>
          <span className={s.toolSummary}>{summary}</span>
          {auto?.decision === "run" && (
            <Chip tone="accent" icon={MODE_ICON.auto} title={answersText(t, auto.answers) || t("ai.auto.ran")}>
              {t("ai.call.auto")}
            </Chip>
          )}
        </button>
        {revertible && (
          <IconButton
            icon="arrow-counter-clockwise"
            size={14}
            label={t("ai.revert.action")}
            title={busy ? t("ai.revert.busy") : t("ai.revert.action")}
            className={cx(s.toolHeadAction, revert.running && s.toolHeadActionOn)}
            disabled={busy || !!revert.running}
            onClick={() => {
              setOpen(true);
              void revert.run(false);
            }}
          />
        )}
        <span id={statusId} className={s.toolStatus}>
          {change?.state === "reverted" ? (
            <Chip icon="arrow-counter-clockwise">{t("ai.revert.reverted")}</Chip>
          ) : (
            <StatusChip view={view} name={call.name} />
          )}
        </span>
        <DisclosureCaret open={open} size={12} className={s.toolCaret} />
      </div>
      {open && (
        <div className={cx(s.toolBody, controlStyles.reveal)}>
          {auto && <AutoVerdict verdict={auto} />}
          {kind === "edit_file" || kind === "write_file" ? (
            <CallChange slotId={slotId} call={call} change={change} />
          ) : (
            <>
              <div className={s.label}>{t("ai.call.input")}</div>
              <pre className={cx(s.pre, "selectable")}>{prettyArgs(shown.arguments) || "{}"}</pre>
            </>
          )}
          {view.kind === "result" && (
            <>
              <div className={s.label}>{t("ai.call.output")}</div>
              <pre className={cx(s.pre, s.output, "selectable")}>{view.entry.content || t("ai.call.noOutput")}</pre>
            </>
          )}
          {revertible && <RevertChange slotId={slotId} change={change} busy={busy} revert={revert} />}
          {change?.state === "dropped" && (
            <div className={s.revert}>
              <HintText>{t("ai.revert.dropped")}</HintText>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
