import type { ReactNode } from "react";
import { Button, Icon, StatusLine } from "@hatoba/ui";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import { retryTurn } from "../actions/turns";
import type { Slot } from "../store";
import s from "./Messages.module.css";

/** What the turn is doing between messages, and how the last one ended (§9 states). */
export function TurnStatus({ slotId, slot }: { slotId: string; slot: Slot }) {
  const t = useT();
  const { turn, outcome } = slot;

  if (slot.remoteRunning) return <StatusLine tone="progress">{t("ai.status.remote")}</StatusLine>;
  // A running turn shows itself: its reply, what that reply waits for (`usePendingLine`), and its calls.
  if (turn) return null;
  if (slot.compacting) return <StatusLine tone="progress">{t("ai.status.compacting")}</StatusLine>;
  if (!outcome) return null;
  const retry = slot.conversationId ? (
    <Button size="sm" icon="arrows-clockwise" onClick={() => void retryTurn(slotId)}>
      {t("btn.retry")}
    </Button>
  ) : null;
  switch (outcome.reason) {
    case "stopped":
      return <StatusLine icon="stop-circle">{t("ai.outcome.stopped")}</StatusLine>;
    case "length":
      return <Banner icon="scissors" tone="warn" text={t("ai.outcome.length")} />;
    case "refused":
      return <Banner icon="prohibit" tone="warn" text={t("ai.outcome.refused")} />;
    case "empty":
      return <Banner icon="chat-circle-slash" tone="warn" text={t("ai.outcome.empty")} action={retry} />;
    case "error":
      return (
        <Banner
          icon="warning-circle"
          tone="error"
          text={outcome.status ? t("ai.outcome.errorStatus", { status: outcome.status }) : t("ai.outcome.error")}
          detail={outcome.message}
          action={retry}
        />
      );
  }
}

export function Banner({
  icon,
  tone,
  text,
  detail,
  action,
}: {
  icon: string;
  tone: "warn" | "error";
  text: string;
  detail?: string;
  action?: ReactNode;
}) {
  return (
    <div className={cx(s.banner, tone === "error" ? s.bannerError : s.bannerWarn)} role="status">
      <Icon name={icon} size={16} className={s.bannerIcon} />
      <div className={s.bannerMain}>
        <div>{text}</div>
        {detail && <div className={cx(s.bannerDetail, "selectable")}>{detail}</div>}
      </div>
      {action}
    </div>
  );
}
