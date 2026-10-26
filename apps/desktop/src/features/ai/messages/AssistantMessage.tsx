import { useState } from "react";
import { DisclosureCaret, Icon, controlStyles } from "@hatoba/ui";
import { useT, type MessageKey } from "@/i18n";
import type { AiFileChange, AiToolCall } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { ToolBlock, type CallView } from "../calls/ToolBlock";
import type { HostInfo } from "../calls/hostInfo";
import { callRevealKey, type LiveResponse } from "../turn";
import { Markdown } from "./Markdown";
import { CopyButton, ForkButton, MessageTime } from "./MessageActions";
import type { ReplyEnd } from "./replies";
import s from "./Messages.module.css";

/** The reply of the turn: what streams of it, or until then what it waits for, in the place its text will take. */
export function LiveMessage({
  slotId,
  live,
  pending,
  host,
}: {
  slotId: string;
  live: LiveResponse | null;
  pending: MessageKey | null;
  host: HostInfo | null;
}) {
  return (
    <div className={cx(s.item, s.enter)}>
      <AssistantMessage
        slotId={slotId}
        text={live?.text ?? ""}
        reasoning={live?.reasoning || null}
        calls={live?.toolCalls ?? []}
        view={() => ({ kind: "streaming" })}
        host={host}
        streaming
        pending={pending}
      />
    </div>
  );
}

/**
 * The first line of a reply with nothing to show yet. It has the reasoning header's place and size, so the header
 * (the same "Thinking" with its caret) or the reply's text takes over where it stands.
 */
export function PendingLine({ label }: { label: MessageKey }) {
  const t = useT();
  return (
    <div className={s.pending} role="status">
      {label === "ai.reasoning.active" ? (
        <Icon name="brain" size={14} />
      ) : (
        <span className={s.pendingDot} aria-hidden="true" />
      )}
      <span className={s.shimmer}>{t(label)}</span>
    </div>
  );
}

export function AssistantMessage({
  slotId,
  entryId,
  text,
  reasoning,
  calls,
  view,
  change,
  busy,
  host,
  streaming,
  noAnswerShown,
  flash,
  reply,
  onFork,
  pending,
}: {
  slotId: string;
  /** The stored entry; a reply still streaming has none. */
  entryId?: string;
  text: string;
  reasoning: string | null;
  calls: AiToolCall[];
  view: (call: AiToolCall) => CallView;
  /** AI-41: the change a call made that this device keeps, to revert. */
  change?: (call: AiToolCall) => AiFileChange | undefined;
  /** The conversation is not idle, so nothing is reverted now. */
  busy?: boolean;
  host: HostInfo | null;
  streaming?: boolean;
  /** The turn's outcome below says that this reply has no answer (§13.1), so the reply does not say it too. */
  noAnswerShown?: boolean;
  /** The highlighted search hit (AI-24). */
  flash?: string | null;
  /** The finished reply this entry ends: its text for Copy, and when it ended. */
  reply?: ReplyEnd;
  /** AI-43: Fork from Here under the finished reply. */
  onFork?: () => void;
  /** What a streaming reply with nothing to show waits for. */
  pending?: MessageKey | null;
}) {
  const thinking = !!streaming && !text && calls.length === 0;
  return (
    <div className={s.assistant}>
      {pending && <PendingLine label={pending} />}
      {reasoning && <Reasoning text={reasoning} active={thinking} />}
      {text && <Markdown text={text} streaming={streaming && calls.length === 0} />}
      {/* No answer: nothing at all, or only reasoning. */}
      {!text.trim() && calls.length === 0 && !streaming && !noAnswerShown && <EmptyReply />}
      {calls.map((call) => {
        const reveal = entryId ? callRevealKey(entryId, call.id) : undefined;
        return (
          <ToolBlock
            key={call.id}
            slotId={slotId}
            call={call}
            view={view(call)}
            change={change?.(call)}
            busy={!!busy}
            host={host}
            reveal={reveal}
            flash={!!reveal && flash === reveal}
          />
        );
      })}
      {reply && (
        <div className={s.replyFoot}>
          {reply.text && <CopyButton text={reply.text} />}
          {onFork && <ForkButton onFork={onFork} />}
          <MessageTime at={reply.at} />
        </div>
      )}
    </div>
  );
}

export function EmptyReply() {
  const t = useT();
  return <div className={s.muted}>{t("ai.emptyReply")}</div>;
}

/** AI-06: reasoning above the answer, collapsed by default. */
function Reasoning({ text, active }: { text: string; active: boolean }) {
  const t = useT();
  const [open, setOpen] = useState(false);
  return (
    <div className={s.reasoning}>
      <button type="button" className={s.reasoningHead} aria-expanded={open} onClick={() => setOpen((v) => !v)}>
        <Icon name="brain" size={14} />
        <span className={active ? s.shimmer : undefined}>{t(active ? "ai.reasoning.active" : "ai.reasoning")}</span>
        <DisclosureCaret open={open} size={12} />
      </button>
      {open && <div className={cx(s.reasoningBody, controlStyles.reveal, "selectable")}>{text}</div>}
    </div>
  );
}
