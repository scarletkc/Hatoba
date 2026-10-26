import { useMemo } from "react";
import { Button, Chip, Icon, IconButton, StatusLine } from "@hatoba/ui";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import { dismissSideQuestion, retrySideQuestion, stopSideQuestion } from "../actions/sideQuestions";
import { parseMessage } from "../attachments";
import { AttachmentCardFor } from "../composer/AttachmentChips";
import { sideRunning, type SideQuestion } from "../side";
import { EmptyReply, PendingLine } from "./AssistantMessage";
import { Markdown } from "./Markdown";
import { CopyButton } from "./MessageActions";
import { Banner } from "./TurnStatus";
import s from "./Messages.module.css";

/**
 * AI-42: a side question and its answer at the end of the conversation, marked as not saved. Stop (or Esc)
 * stops only it; × discards it.
 */
export function SideBubble({ slotId, q }: { slotId: string; q: SideQuestion }) {
  const t = useT();
  const running = sideRunning(q);
  const parts = useMemo(() => parseMessage(q.sent), [q.sent]);
  return (
    <div className={s.side} role="region" aria-label={t("ai.side.title")}>
      <div className={s.sideHead}>
        <Icon name="chat-teardrop-dots" size={14} className={s.sideIcon} />
        <span className={s.sideTitle}>{t("ai.side.title")}</span>
        <Chip title={t("ai.side.notSavedHint")}>{t("ai.side.notSaved")}</Chip>
        <span className={s.gap} />
        {running ? (
          <IconButton
            icon="stop"
            label={t("ai.side.stop")}
            size={14}
            className={s.messageAction}
            onClick={() => stopSideQuestion(slotId)}
          />
        ) : (
          q.phase === "done" && q.answer.trim() && <CopyButton text={q.answer} />
        )}
        <IconButton
          icon="x"
          label={t("btn.close")}
          size={14}
          className={s.messageAction}
          onClick={() => dismissSideQuestion(slotId)}
        />
      </div>
      {parts.attachments.map((a, i) => (
        <AttachmentCardFor key={i} attachment={a} />
      ))}
      <div className={cx(s.sideQuestion, "selectable")}>{q.question}</div>
      {q.answer && <Markdown text={q.answer} streaming={q.phase === "streaming"} />}
      {(q.phase === "waiting" || q.phase === "thinking") && (
        <PendingLine label={q.phase === "thinking" ? "ai.reasoning.active" : "ai.status.waiting"} />
      )}
      {q.phase === "stopped" && <StatusLine icon="stop-circle">{t("ai.outcome.stopped")}</StatusLine>}
      {q.phase === "done" && q.finish === "length" && (
        <Banner icon="scissors" tone="warn" text={t("ai.outcome.length")} />
      )}
      {q.phase === "done" && q.finish === "refused" && (
        <Banner icon="prohibit" tone="warn" text={t("ai.outcome.refused")} />
      )}
      {q.phase === "done" && !q.answer.trim() && q.finish !== "refused" && <EmptyReply />}
      {q.phase === "error" && (
        <Banner
          icon="warning-circle"
          tone="error"
          text={t("ai.outcome.error")}
          detail={q.error ?? undefined}
          action={
            <Button size="sm" icon="arrows-clockwise" onClick={() => retrySideQuestion(slotId)}>
              {t("btn.retry")}
            </Button>
          }
        />
      )}
    </div>
  );
}
