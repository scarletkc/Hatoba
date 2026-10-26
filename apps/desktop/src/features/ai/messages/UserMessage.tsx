import { useMemo, useState } from "react";
import { Icon, IconButton } from "@hatoba/ui";
import { useApp } from "@/app/store";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import { parseMessage, type Note } from "../attachments";
import { AttachmentCardFor, ScreenCard } from "../composer/AttachmentChips";
import { MODE_ICON } from "../tools";
import { EditMessage } from "./EditMessage";
import { CopyButton, ForkButton, MessageTime } from "./MessageActions";
import { noteLabel, noteTitle } from "./notes";
import s from "./Messages.module.css";

/**
 * The user's message. Copy takes what was typed, without the attachments. With `onResend`, Edit
 * replaces it and sends it again (AI-26), and with `onFork`, Fork from Here copies the conversation up
 * to it into a new one (AI-43). The notes Hatoba stored before it (AI-05, AI-09) are dividers above
 * it, not part of the bubble; an edit shows them while it is sent, and Rust works them out again for
 * the new message. The conversation's `first` message shows no permission mode divider: its note
 * only tells the model the mode the conversation started in, which is no switch (AI-16).
 * `untracked` counts the file changes of the calls an edit deletes that this device cannot revert (AI-41).
 */
export function UserMessage({
  text,
  at,
  pending,
  first,
  onResend,
  onFork,
  untracked,
}: {
  text: string;
  at: number;
  pending: boolean;
  first: boolean;
  onResend?: (text: string) => Promise<boolean>;
  onFork?: () => void;
  untracked: () => number;
}) {
  const t = useT();
  const [editing, setEditing] = useState(false);
  // AI-10, AI-35: what was attached to the message is in blocks at its start, after Hatoba's notes.
  const parts = useMemo(() => parseMessage(text), [text]);
  // AI-53: the memories a conversation starts with are the assistant's context, not a change to show.
  const notes = parts.notes
    .filter(
      (note) => note.kind !== "terminal_state" && note.kind !== "memory" && !(first && note.kind === "permission_mode"),
    )
    .map((note, i) => <NoteDivider key={i} note={note} />);
  // AI-46: the screen the message showed is a collapsed block above its attachments.
  const screen = parts.notes.find((note) => note.kind === "terminal_state");
  if (editing && onResend)
    return (
      <>
        {notes}
        <EditMessage
          parts={parts}
          untracked={untracked() > 0}
          onCancel={() => setEditing(false)}
          onSend={onResend}
          onSent={() => setEditing(false)}
        />
      </>
    );
  return (
    <>
      {notes}
      <div className={s.userRow}>
        <div className={s.userActions}>
          {parts.typed && <CopyButton text={parts.typed} />}
          {onResend && (
            <IconButton
              icon="pencil-simple"
              label={t("ai.edit")}
              size={14}
              className={s.messageAction}
              onClick={() => setEditing(true)}
            />
          )}
          {onFork && <ForkButton onFork={onFork} />}
        </div>
        <div className={s.userStack}>
          {screen?.kind === "terminal_state" && <ScreenCard note={screen} />}
          {parts.attachments.map((a, i) => (
            <AttachmentCardFor key={i} attachment={a} />
          ))}
          <div className={cx(s.user, pending && s.userPending, "selectable")}>{parts.typed}</div>
          <MessageTime at={at} />
        </div>
      </div>
    </>
  );
}

const NOTE_ICONS = {
  interrupted: "stop-circle",
  host_change: "arrows-left-right",
  model_change: "swap",
  file_revert: "arrow-counter-clockwise",
  terminal_state: "terminal-window",
  mcp_tools: "plug",
  mcp_change: "plug",
  memory: "brain",
} as const;

/**
 * A note Hatoba stored before a message: the user stopped the reply before it, the conversation moved
 * to another host, a terminal became attached or went away, it went to another model, the user
 * reverted a file tool's change, it listed the deferred MCP tools (AI-45), or MCP servers' tools came
 * or went (AI-30).
 */
function NoteDivider({ note }: { note: Note }) {
  const t = useT();
  const device = useApp.getState().info.device_name;
  const icon =
    note.kind === "terminal_change"
      ? note.to === "attached"
        ? "plugs-connected"
        : "plugs"
      : note.kind === "permission_mode"
        ? MODE_ICON[note.mode]
        : NOTE_ICONS[note.kind];
  return (
    <div className={s.noteDivider} role="note" title={noteTitle(t, note, device)}>
      <span className={s.noteText}>
        <Icon name={icon} size={12} />
        {noteLabel(t, note, device)}
      </span>
    </div>
  );
}
