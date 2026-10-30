import { memo, useEffect, useState } from "react";
import { IconButton, toast } from "@hatoba/ui";
import { formatDateTime, formatMessageTime, useT, useToday } from "@/i18n";
import { writeClipboard } from "@/lib/clipboard";
import s from "./Messages.module.css";

/** How long Copy shows its check after copying. */
const COPIED_MS = 1500;

/** When a message was sent or a reply finished, with the full date and time on hover. */
export const MessageTime = memo(function MessageTime({ at }: { at: number }) {
  const t = useT();
  const today = useToday();
  return (
    <time className={s.time} dateTime={new Date(at).toISOString()} title={formatDateTime(t.locale, at)}>
      {formatMessageTime(t.locale, at, today)}
    </time>
  );
});

/** Copies a message's text and shows a check for a moment. */
export function CopyButton({ text }: { text: string }) {
  const t = useT();
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), COPIED_MS);
    return () => window.clearTimeout(timer);
  }, [copied]);
  return (
    <IconButton
      icon={copied ? "check" : "copy"}
      label={t(copied ? "btn.copied" : "btn.copy")}
      size={14}
      className={s.messageAction}
      onClick={() =>
        void writeClipboard(text).then(
          () => setCopied(true),
          () => toast(t("terminal.clipboardDenied"), "error"),
        )
      }
    />
  );
}

/** AI-43: copies the conversation up to this message into a new one, which opens in its place. */
export function ForkButton({ onFork }: { onFork: () => void }) {
  const t = useT();
  return <IconButton icon="git-fork" label={t("ai.fork")} size={14} className={s.messageAction} onClick={onFork} />;
}
