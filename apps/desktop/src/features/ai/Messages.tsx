import { Fragment, memo, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Button, Icon, IconButton, LinkButton, Spinner } from "@/components/controls";
import { PopupSelect, toast } from "@/components/overlay";
import { readClipboard, writeClipboard } from "@/features/terminal/clipboard";
import { formatDateTime, formatMessageTime, useT, useToday, type MessageKey } from "@/i18n";
import type { AiEntryView, AiFileChange, AiFilePreview, AiFileRevert, AiToolCall, HostView, McpToolAnnotations, McpToolInfo } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { isImeEvent } from "@/lib/ime";
import {
  askSideQuestion,
  decideCall,
  dismissSideQuestion,
  editAndResend,
  fileChangeDiff,
  forkConversation,
  retrySideQuestion,
  retryTurn,
  revertFileChange,
  stopSideQuestion,
  tooMuchMessage,
  type Decision,
} from "./actions";
import { AttachmentCardFor, AttachmentChipFor, ScreenCard } from "./AttachmentChips";
import { FileChange } from "./FileChange";
import { useFilePreview } from "./filePreview";
import { composeMessage, fitsMessage, isLongPaste, makePaste, parseMessage, splitPermissionNote, type MessageParts, type Note } from "./attachments";
import { insertAtCaret, isPlainPasteKey } from "./Composer";
import { forkPoints } from "./fork";
import { Markdown } from "./Markdown";
import { useMcp } from "./mcp";
import { noteLabel, noteTitle } from "./notes";
import { replies as findReplies, type ReplyEnd } from "./replies";
import { revertFailure } from "./revertText";
import { sideRunning, type SideQuestion } from "./side";
import { patchSlot, useAi, type Slot } from "./store";
import {
  allowanceScope,
  callFileChange,
  callSummary,
  exitStatusOf,
  parseArgs,
  prettyArgs,
  proxyArguments,
  proxyCall,
  SEND_KEYS,
  shownCall,
  shownText,
  toolKind,
  toolLabel,
  type Scope,
  type SendKey,
  type ToolKind,
} from "./tools";
import { callRevealKey, editedFiles, revealKey, type CallState, type LiveResponse } from "./turn";
import s from "./Messages.module.css";

type ToolEntry = Extract<AiEntryView, { role: "tool" }>;

const TOOL_ICON: Record<ToolKind, string> = {
  read_terminal: "terminal-window",
  run_command: "terminal",
  send_input: "keyboard",
  read_file: "file-text",
  edit_file: "note-pencil",
  write_file: "file-plus",
  web_search: "magnifying-glass",
  fetch_url: "globe",
  read_skill: "book-open",
  search_tools: "list-magnifying-glass",
  mcp: "plug",
  unknown: "question",
};

export const KEY_LABEL: Record<SendKey, MessageKey> = {
  enter: "ai.key.enter",
  tab: "ai.key.tab",
  esc: "ai.key.esc",
  ctrl_c: "ai.key.ctrl_c",
  ctrl_d: "ai.key.ctrl_d",
  up: "ai.key.up",
  down: "ai.key.down",
  left: "ai.key.left",
  right: "ai.key.right",
};

/** Where a call stands, from its stored result or the turn (§9: tool running, waiting for approval, …). */
type CallView =
  | { kind: "result"; entry: ToolEntry }
  | { kind: CallState; mcp?: McpToolInfo | null }
  | { kind: "queued" }
  | { kind: "streaming" }
  | { kind: "none" };

/** How long a revealed entry stays highlighted (AI-24). */
const FLASH_MS = 2400;
/** How long Copy shows its check after copying. */
const COPIED_MS = 1500;

export interface HostInfo {
  name: string;
  target: string;
}

export function hostInfo(host: HostView | undefined): HostInfo | null {
  return host ? { name: host.name, target: `${host.username}@${host.address}:${host.port}` } : null;
}

// ───────────────────────── the list ─────────────────────────

export function MessageList({ slotId, slot, host, empty }: { slotId: string; slot: Slot; host: HostInfo | null; empty: ReactNode }) {
  const t = useT();
  const ref = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const { entries, turn, outcome } = slot;

  const results = useMemo(() => {
    const map = new Map<string, ToolEntry>();
    for (const e of entries) if (e.role === "tool") map.set(e.tool_call_id, e);
    return map;
  }, [entries]);

  const contextAt = slot.conversation?.context_start ? entries.findIndex((e) => e.entry_id === slot.conversation?.context_start) : -1;
  const lastAssistant = useMemo(() => {
    for (let i = entries.length - 1; i >= 0; i--) if (entries[i].role === "assistant") return entries[i].entry_id;
    return null;
  }, [entries]);
  const replies = useMemo(() => findReplies(entries), [entries]);
  // AI-41: the changes of file tool calls this device keeps, by response and call.
  const changes = useMemo(() => new Map(slot.fileChanges.map((c) => [`${c.response_id}:${c.tool_call_id}`, c])), [slot.fileChanges]);

  // Follow new content while the view is at the bottom.
  useLayoutEffect(() => {
    const el = ref.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  });

  // AI-24: a search hit scrolls to its entry and highlights it, or the block of the call a result belongs to.
  const [flash, setFlash] = useState<string | null>(null);
  const reveal = slot.reveal;
  useLayoutEffect(() => {
    const list = ref.current;
    if (!reveal || !list) return;
    const entry = entries.find((e) => e.entry_id === reveal);
    if (!entry) {
      if (!slot.loading && entries.length > 0) patchSlot(slotId, { reveal: null }); // not in this conversation (any more)
      return;
    }
    const key = revealKey(entries, entry);
    const el = key && list.querySelector(`[data-reveal="${CSS.escape(key)}"]`);
    patchSlot(slotId, { reveal: null });
    if (!key || !el) return;
    stick.current = false;
    el.scrollIntoView({ block: "center" });
    setFlash(key);
  }, [reveal, entries, slot.loading, slotId]);
  useEffect(() => {
    if (!flash) return;
    const timer = window.setTimeout(() => setFlash(null), FLASH_MS);
    return () => window.clearTimeout(timer);
  }, [flash]);

  const callView = (call: AiToolCall, newest: boolean): CallView => {
    const result = results.get(call.id);
    if (result) return { kind: "result", entry: result };
    if (turn?.call?.id === call.id) return { kind: turn.call.state, mcp: turn.call.mcp };
    return newest && turn ? { kind: "queued" } : { kind: "none" };
  };

  // AI-26: the user's stored messages can be edited and sent again while nothing runs; AI-43: the
  // conversation can be forked from them and from finished replies then too, unless a file change is being
  // reverted (AI-41) or a fork of it is being made.
  const canEdit = !!slot.conversationId && !turn && !slot.remoteRunning && !slot.compacting && !slot.loading;
  const forkable = useMemo(() => forkPoints(entries), [entries]);
  const conversationId = slot.conversationId;
  const forking = useAi((st) => !!conversationId && st.forking.includes(conversationId));
  const fork = (entryId: string) =>
    canEdit && !slot.reverting && !forking && conversationId && forkable.has(entryId) ? () => void forkConversation(slotId, conversationId, entryId) : undefined;

  const isEmpty = entries.length === 0 && !turn && !slot.remoteRunning && !outcome;
  // A reply shows its time and Copy once it is finished: the one at the end may still grow while a turn runs.
  const running = !!turn || slot.remoteRunning;
  // AI-41: a revert waits until the conversation is idle, so the next message tells the model before it acts again.
  const busy = running || slot.compacting || slot.loading || slot.reverting;

  return (
    <div
      ref={ref}
      className={s.list}
      onScroll={(e) => {
        const el = e.currentTarget;
        stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
      }}
    >
      {isEmpty && !slot.loading && empty}
      {slot.loading && entries.length === 0 && (
        <div className={s.status}>
          <Spinner size={13} />
          {t("ai.loading")}
        </div>
      )}
      {entries.map((entry, i) => {
        const outside = contextAt > 0 && i < contextAt;
        const divider = i === contextAt && contextAt > 0 ? <div className={s.contextDivider}>{t("ai.outsideContext")}</div> : null;
        const key = `entry:${entry.entry_id}`;
        const flashed = flash === key;
        let body: ReactNode = null;
        switch (entry.role) {
          case "user": {
            const pending = entry.entry_id.startsWith("~");
            body = (
              <UserMessage
                text={entry.text}
                at={entry.created_at}
                pending={pending}
                onResend={canEdit && !pending ? (text) => editAndResend(slotId, entry.entry_id, text) : undefined}
                onFork={fork(entry.entry_id)}
                untracked={() => editedFiles(entries, slot.fileChanges, entry.entry_id).untracked}
              />
            );
            break;
          }
          case "assistant": {
            const reply = running && entry.entry_id === replies.open ? undefined : replies.ends.get(entry.entry_id);
            body = (
              <AssistantMessage
                slotId={slotId}
                entryId={entry.entry_id}
                text={entry.text}
                reasoning={entry.reasoning}
                calls={entry.tool_calls}
                view={(call) => callView(call, entry.entry_id === lastAssistant)}
                change={(call) => changes.get(`${entry.entry_id}:${call.id}`)}
                busy={busy}
                host={host}
                noAnswerShown={outcome?.reason === "empty" && entry.entry_id === lastAssistant}
                flash={flash}
                reply={reply}
                onFork={reply ? fork(entry.entry_id) : undefined}
              />
            );
            break;
          }
          case "summary":
            body = <SummaryBlock text={splitPermissionNote(entry.text).summary} flash={flashed} />;
            break;
          case "tool":
            return null; // shown in its call's block
        }
        return (
          <Fragment key={entry.entry_id}>
            {divider}
            <div className={cx(s.item, outside && s.outside, flashed && entry.role !== "summary" && s.flash)} data-reveal={key}>
              {body}
            </div>
          </Fragment>
        );
      })}
      {turn?.live && <LiveMessage slotId={slotId} live={turn.live} host={host} />}
      <TurnStatus slotId={slotId} slot={slot} />
      {slot.effortIgnored && (
        <div className={s.status} role="status">
          <Icon name="info" size={13} />
          {t("ai.effortIgnored")}
        </div>
      )}
      {slot.side && <SideBubble slotId={slotId} q={slot.side} />}
    </div>
  );
}

// ───────────────────────── entries ─────────────────────────

/** When a message was sent or a reply finished, with the full date and time on hover. */
const MessageTime = memo(function MessageTime({ at }: { at: number }) {
  const t = useT();
  const today = useToday();
  return (
    <time className={s.time} dateTime={new Date(at).toISOString()} title={formatDateTime(t.locale, at)}>
      {formatMessageTime(t.locale, at, today)}
    </time>
  );
});

/** Copies a message's text and shows a check for a moment. */
function CopyButton({ text }: { text: string }) {
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
      size={13}
      className={s.messageAction}
      onClick={() => void writeClipboard(text).then(() => setCopied(true), () => toast(t("terminal.clipboardDenied"), "error"))}
    />
  );
}

/** AI-43: copies the conversation up to this message into a new one, which opens in its place. */
function ForkButton({ onFork }: { onFork: () => void }) {
  const t = useT();
  return <IconButton icon="git-fork" label={t("ai.fork")} size={13} className={s.messageAction} onClick={onFork} />;
}

/**
 * The user's message. Copy takes what was typed, without the attachments. With `onResend`, Edit
 * replaces it and sends it again (AI-26), and with `onFork`, Fork from Here copies the conversation up
 * to it into a new one (AI-43). The notes Hatoba stored before it (AI-05, AI-09) are dividers above
 * it, not part of the bubble; an edit shows them while it is sent, and Rust works them out again for
 * the new message. `untracked` counts the file changes of the calls an edit deletes that this device
 * cannot revert (AI-41).
 */
function UserMessage({
  text,
  at,
  pending,
  onResend,
  onFork,
  untracked,
}: {
  text: string;
  at: number;
  pending: boolean;
  onResend?: (text: string) => Promise<boolean>;
  onFork?: () => void;
  untracked: () => number;
}) {
  const t = useT();
  const [editing, setEditing] = useState(false);
  // AI-10, AI-35: what was attached to the message is in blocks at its start, after Hatoba's notes.
  const parts = useMemo(() => parseMessage(text), [text]);
  const notes = parts.notes.filter((note) => note.kind !== "terminal_state").map((note, i) => <NoteDivider key={i} note={note} />);
  // AI-46: the screen the message showed is a collapsed block above its attachments.
  const screen = parts.notes.find((note) => note.kind === "terminal_state");
  if (editing && onResend)
    return (
      <>
        {notes}
        <EditMessage parts={parts} untracked={untracked() > 0} onCancel={() => setEditing(false)} onSend={onResend} onSent={() => setEditing(false)} />
      </>
    );
  return (
    <>
      {notes}
      <div className={s.userRow}>
        <div className={s.userActions}>
          {parts.typed && <CopyButton text={parts.typed} />}
          {onResend && <IconButton icon="pencil-simple" label={t("ai.edit")} size={13} className={s.messageAction} onClick={() => setEditing(true)} />}
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
} as const;

/**
 * A note Hatoba stored before a message: the user stopped the reply before it, the conversation moved
 * to another host, a terminal became attached or went away, it went to another model, the user
 * reverted a file tool's change, or it listed the deferred MCP tools (AI-45).
 */
function NoteDivider({ note }: { note: Note }) {
  const t = useT();
  const icon =
    note.kind === "terminal_change"
      ? note.to === "attached"
        ? "plugs-connected"
        : "plugs"
      : note.kind === "permission_mode"
        ? note.mode === "bypass"
          ? "lightning"
          : "shield-check"
        : NOTE_ICONS[note.kind];
  return (
    <div className={s.noteDivider} role="note" title={noteTitle(t, note)}>
      <span className={s.noteText}>
        <Icon name={icon} size={12} />
        {noteLabel(t, note)}
      </span>
    </div>
  );
}

/**
 * AI-26: edits the typed text; what was attached to it stays unless its chip is removed. A long
 * paste becomes another attachment, as in the input box (AI-35). With `untracked`, the calls the edit
 * deletes changed files this device cannot revert (AI-41), and the box says those changes stay.
 */
function EditMessage({
  parts,
  untracked,
  onCancel,
  onSend,
  onSent,
}: {
  parts: MessageParts;
  untracked: boolean;
  onCancel: () => void;
  onSend: (text: string) => Promise<boolean>;
  onSent: () => void;
}) {
  const t = useT();
  const ref = useRef<HTMLTextAreaElement>(null);
  const [value, setValue] = useState(parts.typed);
  const [items, setItems] = useState(() => parts.attachments.map((attachment, i) => ({ id: `a${i}`, attachment })));
  const [busy, setBusy] = useState(false);
  const seq = useRef(parts.attachments.length);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 240)}px`;
  }, [value]);
  useEffect(() => {
    const el = ref.current;
    el?.focus();
    el?.setSelectionRange(el.value.length, el.value.length);
  }, []);

  const send = async () => {
    if (!value.trim() || busy) return;
    setBusy(true);
    const ok = await onSend(composeMessage(value.trim(), items.map((x) => x.attachment)));
    setBusy(false);
    if (ok) onSent();
  };

  const insertText = (text: string) => {
    const el = ref.current;
    const next = insertAtCaret(el, el?.value ?? value, text);
    setValue(next.value);
    requestAnimationFrame(() => {
      el?.focus();
      el?.setSelectionRange(next.caret, next.caret);
    });
  };

  return (
    <div className={s.editMessage}>
      {items.length > 0 && (
        <div className={s.editAttachment}>
          {items.map((x) => (
            <AttachmentChipFor
              key={x.id}
              attachment={x.attachment}
              onRemove={() => setItems((list) => list.filter((y) => y.id !== x.id))}
              onPasteAsText={
                x.attachment.kind === "paste"
                  ? () => {
                      setItems((list) => list.filter((y) => y.id !== x.id));
                      insertText(x.attachment.text);
                    }
                  : undefined
              }
            />
          ))}
        </div>
      )}
      <textarea
        ref={ref}
        className={cx(s.input, s.textarea, s.editInput)}
        value={value}
        aria-label={t("ai.edit.label")}
        spellCheck={false}
        rows={1}
        onChange={(e) => setValue(e.target.value)}
        onPaste={(e) => {
          const text = e.clipboardData.getData("text/plain");
          if (!isLongPaste(text)) return;
          e.preventDefault();
          const paste = makePaste(text);
          if (!paste) return;
          if (!fitsMessage(items.map((x) => x.attachment), paste)) return toast(tooMuchMessage(), "error");
          setItems((list) => [...list, { id: `a${++seq.current}`, attachment: paste }]);
        }}
        onKeyDown={(e) => {
          // As in the input box: Enter sends, Shift+Enter adds a line, Ctrl+Shift+V pastes plain text;
          // Esc leaves the editor, not the turn.
          if (isPlainPasteKey(e)) {
            e.preventDefault();
            readClipboard().then(
              (text) => text && insertText(text.replace(/\r\n?/g, "\n")),
              () => toast(t("terminal.clipboardDenied"), "error"),
            );
          } else if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            onCancel();
          } else if (e.key === "Enter" && !e.shiftKey && !e.altKey && !e.ctrlKey && !e.metaKey && !isImeEvent(e)) {
            e.preventDefault();
            void send();
          }
        }}
      />
      {untracked && (
        <div className={s.editWarning} role="note">
          <Icon name="warning" size={13} />
          <span>{t("ai.editFiles.untracked")}</span>
        </div>
      )}
      <div className={s.actions}>
        <Button size="sm" onClick={onCancel}>
          {t("btn.cancel")}
        </Button>
        <Button size="sm" variant="primary" icon="arrow-up" busy={busy} disabled={!value.trim()} onClick={() => void send()}>
          {t("ai.edit.send")}
        </Button>
      </div>
    </div>
  );
}

function LiveMessage({ slotId, live, host }: { slotId: string; live: LiveResponse; host: HostInfo | null }) {
  return (
    <div className={s.item}>
      <AssistantMessage
        slotId={slotId}
        text={live.text}
        reasoning={live.reasoning || null}
        calls={live.toolCalls}
        view={() => ({ kind: "streaming" })}
        host={host}
        streaming
      />
    </div>
  );
}

function AssistantMessage({
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
}) {
  const thinking = !!streaming && !text && calls.length === 0;
  return (
    <div className={s.assistant}>
      {reasoning && <Reasoning text={reasoning} active={thinking} />}
      {text && <Markdown text={text} streaming={streaming && calls.length === 0} />}
      {/* No answer: nothing at all, or only reasoning. */}
      {!text.trim() && calls.length === 0 && !streaming && !noAnswerShown && <EmptyReply />}
      {calls.map((call) => {
        const reveal = entryId ? callRevealKey(entryId, call.id) : undefined;
        return <ToolBlock key={call.id} slotId={slotId} call={call} view={view(call)} change={change?.(call)} busy={!!busy} host={host} reveal={reveal} flash={!!reveal && flash === reveal} />;
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

function EmptyReply() {
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
        <Icon name="brain" size={13} />
        <span className={active ? s.shimmer : undefined}>{t(active ? "ai.reasoning.active" : "ai.reasoning")}</span>
        <Icon name={open ? "caret-down" : "caret-right"} size={11} />
      </button>
      {open && <div className={cx(s.reasoningBody, "selectable")}>{text}</div>}
    </div>
  );
}

function SummaryBlock({ text, flash }: { text: string; flash?: boolean }) {
  const t = useT();
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (flash) setOpen(true);
  }, [flash]);
  return (
    <div className={cx(s.summary, flash && s.flash)}>
      <button type="button" className={s.summaryHead} aria-expanded={open} onClick={() => setOpen((v) => !v)}>
        <Icon name="arrows-in-line-vertical" size={13} />
        <span>{t("ai.summary")}</span>
        <Icon name={open ? "caret-down" : "caret-right"} size={11} />
      </button>
      {open && (
        <div className={s.summaryBody}>
          <Markdown text={text} />
        </div>
      )}
    </div>
  );
}

// ───────────────────────── tool calls ─────────────────────────

function StatusChip({ view, name }: { view: CallView; name: string }) {
  const t = useT();
  switch (view.kind) {
    case "result": {
      const { status, content } = view.entry;
      if (status === "ok") {
        const code = toolKind(name) === "run_command" ? exitStatusOf(content) : null;
        if (code !== null && code !== 0) return <span className={cx(s.chip, s.chipBad)}>{t("ai.call.exit", { code })}</span>;
        return (
          <span className={cx(s.chip, s.chipOk)}>
            <Icon name="check" size={11} />
            {code === 0 ? t("ai.call.exit", { code }) : t("ai.call.done")}
          </span>
        );
      }
      if (status === "error") return <span className={cx(s.chip, s.chipBad)}>{t("ai.call.error")}</span>;
      if (status === "rejected") return <span className={cx(s.chip, s.chipWarn)}>{t("ai.call.rejected")}</span>;
      return <span className={s.chip}>{t("ai.call.cancelled")}</span>;
    }
    case "running":
      return (
        <span className={s.chip}>
          <Spinner size={11} />
          {t("ai.call.running")}
        </span>
      );
    case "approval":
      return <span className={cx(s.chip, s.chipWarn)}>{t("ai.call.approval")}</span>;
    case "queued":
    case "streaming":
      return <span className={s.chip}>{t("ai.call.queued")}</span>;
    case "none":
      return <span className={s.chip}>{t("ai.call.notRun")}</span>;
  }
}

/** AI-41: the revert of a call's change, which the Revert in the block's header and the one in its body share. */
function useRevert(slotId: string, change: AiFileChange | undefined) {
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

type RevertState = ReturnType<typeof useRevert>;

/**
 * Each call is a collapsible block with its input, output and status (§9); approval cards appear in place. An
 * `edit_file` or `write_file` call shows its change as a diff. A change of a file tool call this device keeps
 * can be reverted from the block's header, on hover or focus, or from its body (AI-41).
 */
function ToolBlock({
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
  useEffect(() => {
    if (flash) setOpen(true);
  }, [flash]);

  if (view.kind === "approval") return <ApprovalCard slotId={slotId} call={call} host={host} mcp={view.mcp ?? null} />;

  return (
    <div className={cx(s.tool, flash && s.flash)} data-reveal={reveal}>
      <div className={s.toolHeadRow}>
        <button type="button" className={s.toolHead} aria-expanded={open} onClick={() => setOpen((v) => !v)}>
          <Icon name={TOOL_ICON[kind]} size={14} className={s.toolIcon} />
          <span className={s.toolName}>{toolLabel(t, shown.name)}</span>
          <span className={s.toolSummary}>{summary}</span>
          {change?.state === "reverted" ? (
            <span className={s.chip}>
              <Icon name="arrow-counter-clockwise" size={11} />
              {t("ai.revert.reverted")}
            </span>
          ) : (
            <StatusChip view={view} name={call.name} />
          )}
          <Icon name={open ? "caret-down" : "caret-right"} size={11} className={s.toolCaret} />
        </button>
        {revertible && (
          <IconButton
            icon="arrow-counter-clockwise"
            size={13}
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
      </div>
      {open && (
        <div className={s.toolBody}>
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
              <span className={s.meta}>{t("ai.revert.dropped")}</span>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

/**
 * AI-39, AI-40: the change an `edit_file` or `write_file` call made, as a diff: from its snapshot while this
 * device keeps one (AI-41), otherwise from the call's arguments. The arguments themselves are one click away.
 */
function CallChange({ slotId, call, change }: { slotId: string; call: AiToolCall; change?: AiFileChange }) {
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
  const preview = failed ? null : ((keeps && kept !== "failed" ? kept : null) ?? callFileChange(call.name, call.arguments));
  return (
    <>
      <div className={s.labelRow}>
        <span className={s.label}>{t("ai.call.change")}</span>
        <LinkButton tone="muted" className={s.labelLink} aria-expanded={input} onClick={() => setInput((v) => !v)}>
          {t(input ? "ai.call.hideInput" : "ai.call.showInput")}
        </LinkButton>
      </div>
      {failed && <div className={s.fieldError}>{t("ai.call.changeUnreadable")}</div>}
      {preview && <FileChange state={{ preview, loading: keeps && kept === undefined, failure: null }} />}
      {(input || !preview) && <pre className={cx(s.pre, "selectable")}>{prettyArgs(call.arguments) || "{}"}</pre>}
    </>
  );
}

/**
 * AI-41: Revert on the block of a file tool call that wrote, or whose write failed after it may have begun; its
 * confirmation says what it restores and what it does not. When the file holds neither what the call wrote nor
 * what it held before, the block shows the diff from what it holds now back to that, links the later call that
 * wrote it too, and offers Restore anyway over exactly what the diff shows; or the failure.
 */
function RevertChange({ slotId, change, busy, revert }: { slotId: string; change: AiFileChange; busy: boolean; revert: RevertState }) {
  const t = useT();
  const { running, outcome, run } = revert;
  const result = outcome?.result;
  // The result of the newest later call that wrote the same file, which the diff can jump to.
  const laterEntry = useAi((st) => {
    const slot = st.slots[slotId];
    if (!slot) return null;
    const at = slot.fileChanges.findIndex((c) => c.response_id === change.response_id && c.tool_call_id === change.tool_call_id);
    const later = slot.fileChanges
      .slice(at + 1)
      .filter((c) => c.server === change.server && c.path === change.path && (c.state === "written" || c.state === "unfinished"))
      .at(-1);
    if (!later) return null;
    return slot.entries.find((e) => e.role === "tool" && e.tool_call_id === later.tool_call_id && e.entry_id > later.response_id)?.entry_id ?? null;
  });
  return (
    <div className={s.revert}>
      <div className={s.revertRow}>
        <Button size="sm" icon="arrow-counter-clockwise" busy={running === "revert"} disabled={busy || !!running} title={busy ? t("ai.revert.busy") : undefined} onClick={() => void run(false)}>
          {t(running === "revert" ? "ai.revert.reverting" : "ai.revert.action")}
        </Button>
      </div>
      {change.state === "unfinished" && (
        <div className={s.revertWarn}>
          <Icon name="warning" size={13} />
          <span>{t("ai.revert.unfinished")}</span>
        </div>
      )}
      {result?.status === "changed" && (
        <>
          <div className={s.fieldError} role="alert">
            {outcome?.restore && `${t("ai.revert.changedAgain")} `}
            {t(result.before === null ? "ai.revert.changedCreated" : result.current === null ? "ai.revert.gone" : "ai.revert.changed")}
          </div>
          {result.later && (
            <div className={s.meta}>
              {t("ai.revert.later")}
              {laterEntry && (
                <>
                  {" "}
                  <LinkButton className={s.inlineLink} onClick={() => patchSlot(slotId, { reveal: laterEntry })}>
                    {t("ai.revert.goLater")}
                  </LinkButton>
                </>
              )}
            </div>
          )}
          <FileChange state={{ preview: { path: result.path, before: result.current, after: result.before ?? "", error: null, crlf: false }, loading: false, failure: null }} />
          <div className={s.revertRow}>
            <Button size="sm" variant="danger" icon={result.before === null ? "trash" : "arrow-counter-clockwise"} busy={running === "restore"} disabled={busy || !!running} onClick={() => void run(true)}>
              {t(result.before === null ? "ai.revert.deleteAnyway" : "ai.revert.restore")}
            </Button>
          </div>
        </>
      )}
      {result?.status === "failed" && (
        <div className={s.fieldError} role="alert">
          {revertFailure(t, result.reason, result.detail)}
          {result.detail && result.reason !== "other_server" && <div className={cx(s.revertDetail, "selectable")}>{result.detail}</div>}
        </div>
      )}
    </div>
  );
}

/** The fields the Edit form shows: the main input of each tool, or the JSON for others. */
interface Draft {
  main: string;
  timeout: string;
  key: SendKey | "";
  wait: string;
}

function draftFrom(kind: ToolKind, args: Record<string, unknown>, json: string): Draft {
  const str = (v: unknown) => (typeof v === "string" ? v : v === undefined || v === null ? "" : String(v));
  switch (kind) {
    case "run_command":
      return { main: str(args.command), timeout: str(args.timeout_seconds), key: "", wait: "" };
    case "send_input":
      return { main: str(args.text), timeout: "", key: (SEND_KEYS as readonly string[]).includes(str(args.key)) ? (str(args.key) as SendKey) : "", wait: str(args.wait_seconds) };
    case "fetch_url":
      return { main: str(args.url), timeout: "", key: "", wait: "" };
    // AI-39, AI-40: Edit changes the text the call writes; the diff follows.
    case "edit_file":
      return { main: str(args.new_string), timeout: "", key: "", wait: "" };
    case "write_file":
      return { main: str(args.content), timeout: "", key: "", wait: "" };
    default:
      return { main: prettyArgs(json), timeout: "", key: "", wait: "" };
  }
}

/** The arguments after Edit, or an error for invalid JSON. Numbers left empty are dropped. */
function argsFrom(kind: ToolKind, base: Record<string, unknown>, d: Draft): { json: string } | { error: true } {
  const num = (v: string) => (v.trim() === "" ? undefined : Number(v));
  const without = (o: Record<string, unknown>) => Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined));
  switch (kind) {
    case "run_command":
      return { json: JSON.stringify(without({ ...base, command: d.main, timeout_seconds: num(d.timeout) })) };
    case "send_input":
      return { json: JSON.stringify(without({ ...base, text: d.main, key: d.key || undefined, wait_seconds: num(d.wait) })) };
    case "fetch_url":
      return { json: JSON.stringify({ ...base, url: d.main.trim() }) };
    case "edit_file":
      return { json: JSON.stringify({ ...base, new_string: d.main }) };
    case "write_file":
      return { json: JSON.stringify({ ...base, content: d.main }) };
    default: {
      const parsed = parseArgs(d.main);
      return parsed ? { json: JSON.stringify(parsed) } : { error: true };
    }
  }
}

/** MCP annotations as chips (AI-31): shown, but they never change whether a call asks. */
const HINTS: { key: Exclude<keyof McpToolAnnotations, "title">; yes: MessageKey; no: MessageKey; warn?: boolean }[] = [
  { key: "read_only_hint", yes: "ai.annot.readOnly", no: "ai.annot.notReadOnly" },
  { key: "destructive_hint", yes: "ai.annot.destructive", no: "ai.annot.notDestructive", warn: true },
  { key: "idempotent_hint", yes: "ai.annot.idempotent", no: "ai.annot.notIdempotent" },
  { key: "open_world_hint", yes: "ai.annot.openWorld", no: "ai.annot.closedWorld" },
];

function McpDetails({ mcp }: { mcp: McpToolInfo }) {
  const t = useT();
  const { annotations: a, description } = mcp.tool;
  const hints = HINTS.filter((h) => a[h.key] !== null);
  return (
    <>
      {a.title && a.title !== mcp.tool.tool && <div className={s.mcpTitle}>{a.title}</div>}
      {description.trim() && (
        <>
          <div className={s.label}>{t("ai.approval.description")}</div>
          <div className={cx(s.description, "selectable")}>{description}</div>
        </>
      )}
      {hints.length > 0 && (
        <div className={s.hints} title={t("ai.approval.hints")} aria-label={t("ai.approval.hints")}>
          {hints.map((h) => (
            <span key={h.key} className={cx(s.chip, h.warn && a[h.key] && s.chipBad)}>
              {t(a[h.key] ? h.yes : h.no)}
            </span>
          ))}
        </div>
      )}
    </>
  );
}

/**
 * A call's input on the approval card (AI-17), all of it: no scroll fold, every line break marked (a final
 * one presses Enter), the line count when there are several, and hidden characters (controls, bidirectional
 * overrides) as escapes, with a note that there are some.
 */
function ShownInput({ text, command }: { text: string; command?: boolean }) {
  const t = useT();
  const shown = useMemo(() => shownText(text), [text]);
  const multiline = shown.lines.length > 1;
  return (
    <>
      {(multiline || shown.escaped) && (
        <div className={s.inputNotes}>
          {multiline && <span className={cx(s.chip, s.chipWarn)}>{t("ai.approval.lines", { n: shown.lines.length })}</span>}
          {shown.escaped && <span className={cx(s.chip, s.chipWarn)}>{t("ai.approval.escaped")}</span>}
        </div>
      )}
      <pre className={cx(s.pre, s.whole, command && s.command, "selectable")}>
        {shown.lines.map((line, i) => (
          <Fragment key={i}>
            {line}
            {i < shown.lines.length - 1 && (
              <>
                <span className={s.lineBreak} aria-hidden>
                  ↵
                </span>
                {"\n"}
              </>
            )}
          </Fragment>
        ))}
      </pre>
    </>
  );
}

interface AllowLabels {
  here: string;
  hereHint: string;
  /**
   * Null for an MCP server set to Always ask in bypass mode, which asks anyway, and for commands that
   * Always allow does not save (`savablePrefixes`).
   */
  always: string | null;
  alwaysHint: string;
}

/** The labels of a card's Allow for this conversation and Always allow (AI-19, AI-31), which name what they cover. */
function allowLabels(t: ReturnType<typeof useT>, scope: Scope, tool: string, mcp: McpToolInfo | null, bypass: boolean): AllowLabels {
  switch (scope.kind) {
    case "command": {
      const prefix = scope.prefixes.join(", ");
      return {
        here: t("ai.approval.allowCommandHere", { prefix }),
        hereHint: t("ai.approval.allowCommandHereHint", { prefix }),
        // Interpreters, deleting and the like are allowed for the conversation only (`savablePrefixes`).
        always: scope.savable ? t("ai.approval.alwaysAllowCommand", { prefix }) : null,
        alwaysHint: t("ai.approval.alwaysAllowCommandHint", { prefix }),
      };
    }
    case "domain": {
      const { domain } = scope;
      return {
        here: t("ai.approval.allowDomainHere", { domain }),
        hereHint: t("ai.approval.allowDomainHereHint", { domain }),
        always: t("ai.approval.alwaysAllowDomain", { domain }),
        alwaysHint: t("ai.approval.alwaysAllowDomainHint", { domain }),
      };
    }
    case "tool":
      return {
        here: t("ai.approval.allowHere"),
        hereHint: t("ai.approval.allowHereHint", { tool }),
        always: t("ai.approval.alwaysAllowHost"),
        alwaysHint: t("ai.approval.alwaysAllowHostHint", { tool }),
      };
    case "mcp":
      return {
        here: t("ai.approval.allowHere"),
        hereHint: t("ai.approval.allowHereHint", { tool }),
        always: mcp && !(bypass && mcp.always_ask) ? t("ai.approval.alwaysAllow") : null,
        alwaysHint: t("ai.approval.alwaysAllowHint", { tool }),
      };
  }
}

/**
 * AI-17: the tool, the tab's host, and the full input, with Run, Edit and Reject. An MCP call (AI-31)
 * shows its server, the tool's description and hints in place of the host. Allow for this conversation
 * and Always allow (AI-19) name what they cover: a command's prefixes, a page's domain, or the tool. A
 * `call_mcp_tool` call (AI-45) shows, and edits, the tool it names and that tool's own arguments.
 */
function ApprovalCard({ slotId, call, host, mcp }: { slotId: string; call: AiToolCall; host: HostInfo | null; mcp: McpToolInfo | null }) {
  const t = useT();
  const shown = useMemo(() => shownCall(call), [call]);
  const proxied = proxyCall(call.name, call.arguments) !== null;
  const kind = toolKind(shown.name);
  const args = useMemo(() => parseArgs(shown.arguments) ?? {}, [shown.arguments]);
  const [mode, setMode] = useState<"view" | "edit" | "reject">("view");
  const [draft, setDraft] = useState<Draft>(() => draftFrom(kind, args, shown.arguments));
  const [reason, setReason] = useState("");
  const [invalid, setInvalid] = useState(false);
  // AI-42: a side question about the call, which neither runs nor rejects it.
  const [question, setQuestion] = useState<string | null>(null);
  const label = mcp ? t("ai.tool.mcp", { server: mcp.server_name, tool: mcp.tool.tool }) : toolLabel(t, shown.name);
  const bypass = useAi((st) => st.slots[slotId]?.mode === "bypass");
  // AI-39, AI-40: an edit or write shows its diff, which follows the text changed in Edit.
  const fileChange = kind === "edit_file" || kind === "write_file";
  const editedFile = useMemo(() => {
    if (!fileChange || mode !== "edit") return null;
    const next = argsFrom(kind, args, draft);
    return "json" in next && next.json !== JSON.stringify(args) ? next.json : null;
  }, [fileChange, mode, kind, args, draft]);
  const file = useFilePreview(slotId, call.id, editedFile, fileChange);

  // AI-39, AI-40: an edit or write runs only once the card has shown its diff, which Rust then writes
  // over. When the card showed why the call cannot apply, Run answers the model with that instead, so a
  // file that comes back meanwhile is not written without a diff; without any answer, it waits.
  const holdRun = fileChange && (file.loading || !file.preview);
  const refusal = fileChange ? (file.preview?.error ?? undefined) : undefined;
  // AI-19: what Allow for this conversation and Always allow cover, for the input that would run; nothing
  // when the card shows why an edit or write cannot apply, since it showed no change.
  let allow: AllowLabels | null = null;
  if (refusal === undefined) {
    const next = mode === "edit" ? argsFrom(kind, args, draft) : null;
    const scope = allowanceScope(kind, next && "json" in next ? (parseArgs(next.json) ?? args) : args, mcp);
    allow = scope && allowLabels(t, scope, label, mcp, bypass);
  }
  const run = (allow?: Extract<Decision, { kind: "run" }>["allow"]) => {
    if (holdRun) return;
    if (mode !== "edit") return decideCall(slotId, { kind: "run", edited: null, allow, refusal });
    const next = argsFrom(kind, args, draft);
    if ("error" in next) return setInvalid(true);
    const same = next.json === JSON.stringify(args);
    const edited = proxied ? proxyArguments(call.arguments, next.json) : next.json;
    decideCall(slotId, { kind: "run", edited: same ? null : edited, allow, refusal });
  };
  const reject = () => decideCall(slotId, { kind: "reject", reason: reason.trim() });
  const ask = () => {
    if (question !== null && askSideQuestion(slotId, question, false)) setQuestion(null);
  };
  /** Edit and Reject close the box of a side question. */
  const switchMode = (next: "edit" | "reject") => {
    setMode(next);
    setQuestion(null);
  };
  const str = (v: unknown) => (typeof v === "string" ? v : v === undefined || v === null ? "" : String(v));

  return (
    <div className={cx(s.tool, s.approval)} role="group" aria-label={t("ai.approval.title", { tool: label })}>
      <div className={s.approvalHead}>
        <Icon name={TOOL_ICON[kind]} size={14} className={s.toolIcon} />
        <span className={s.toolName}>{label}</span>
        <span className={s.gap} />
        <span className={cx(s.chip, s.chipWarn)}>{t("ai.call.approval")}</span>
      </div>
      {kind === "mcp" ? (
        mcp && (
          <div className={s.approvalHost}>
            <Icon name="plug" size={12} />
            <span>{t("ai.approval.server")}</span>
            <span className={s.hostName}>{mcp.server_name}</span>
          </div>
        )
      ) : (
        host && (
          <div className={s.approvalHost}>
            <Icon name="hard-drives" size={12} />
            <span className={s.hostName}>{host.name}</span>
            <span className={s.hostTarget}>{host.target}</span>
          </div>
        )
      )}
      {mcp?.always_ask && bypass && <div className={s.meta}>{t("ai.approval.alwaysAsk")}</div>}
      {mcp && mode !== "edit" && <McpDetails mcp={mcp} />}

      {mode === "edit" ? (
        <div
          className={s.editForm}
          onKeyDown={(e) => {
            // Esc leaves the form instead of stopping the turn.
            if (e.key === "Escape" && !e.defaultPrevented) {
              e.stopPropagation();
              setMode("view");
            }
          }}
        >
          {kind === "fetch_url" ? (
            <input
              className={s.input}
              value={draft.main}
              aria-label={t("ai.approval.url")}
              spellCheck={false}
              onChange={(e) => setDraft({ ...draft, main: e.target.value })}
              autoFocus
            />
          ) : (
            <>
              {fileChange && <div className={s.label}>{kind === "edit_file" ? t("ai.approval.newString") : t("ai.approval.content")}</div>}
              <textarea
                className={cx(s.input, s.textarea)}
                value={draft.main}
                aria-label={
                  kind === "send_input"
                    ? t("ai.approval.text")
                    : kind === "run_command"
                      ? t("ai.approval.command")
                      : kind === "edit_file"
                        ? t("ai.approval.newString")
                        : kind === "write_file"
                          ? t("ai.approval.content")
                          : t("ai.call.input")
                }
                spellCheck={false}
                rows={Math.min(fileChange ? 14 : 8, Math.max(2, draft.main.split("\n").length))}
                onChange={(e) => {
                  setInvalid(false);
                  setDraft({ ...draft, main: e.target.value });
                }}
                autoFocus
              />
            </>
          )}
          {invalid && <div className={s.fieldError}>{t("ai.approval.invalidJson")}</div>}
          {kind === "run_command" && (
            <label className={s.inline}>
              {t("ai.approval.timeout")}
              <input className={cx(s.input, s.number)} type="number" min={1} max={600} placeholder="120" value={draft.timeout} onChange={(e) => setDraft({ ...draft, timeout: e.target.value })} />
              {t("ai.approval.seconds")}
            </label>
          )}
          {kind === "send_input" && (
            <div className={s.inline}>
              {t("ai.approval.key")}
              <PopupSelect<SendKey | "">
                value={draft.key}
                ariaLabel={t("ai.approval.key")}
                options={[{ value: "", label: t("ai.key.none") }, ...SEND_KEYS.map((k) => ({ value: k, label: t(KEY_LABEL[k]) }))]}
                onChange={(key) => setDraft({ ...draft, key })}
              />
              <span className={s.gap} />
              {t("ai.approval.wait")}
              <input className={cx(s.input, s.number)} type="number" min={1} max={120} placeholder="10" value={draft.wait} onChange={(e) => setDraft({ ...draft, wait: e.target.value })} />
              {t("ai.approval.seconds")}
            </div>
          )}
          {fileChange && <FileChange state={file} replaceAll={args.replace_all === true} />}
        </div>
      ) : (
        <div className={s.approvalInput}>
          {kind === "run_command" && (
            <>
              <ShownInput text={str(args.command)} command />
              <div className={s.meta}>{t("ai.approval.timeoutValue", { s: str(args.timeout_seconds) || 120 })}</div>
            </>
          )}
          {kind === "send_input" && (
            <>
              <ShownInput text={str(args.text) || " "} command />
              <div className={s.meta}>
                {args.key ? t("ai.approval.thenKey", { key: (SEND_KEYS as readonly string[]).includes(str(args.key)) ? t(KEY_LABEL[str(args.key) as SendKey]) : str(args.key) }) : t("ai.approval.noKey")}
                {" · "}
                {t("ai.approval.waitValue", { s: str(args.wait_seconds) || 10 })}
              </div>
            </>
          )}
          {kind === "fetch_url" && (
            <>
              <ShownInput text={str(args.url)} command />
              {args.offset !== undefined && <div className={s.meta}>{t("ai.approval.offset", { n: str(args.offset) })}</div>}
            </>
          )}
          {fileChange && <FileChange state={file} replaceAll={args.replace_all === true} />}
          {kind !== "run_command" && kind !== "send_input" && kind !== "fetch_url" && !fileChange && (
            <>
              {mcp && <div className={s.label}>{t("ai.approval.arguments")}</div>}
              <ShownInput text={prettyArgs(shown.arguments) || "{}"} />
            </>
          )}
        </div>
      )}

      {question !== null && mode === "view" && (
        <div className={s.rejectRow}>
          <input
            className={s.input}
            value={question}
            aria-label={t("ai.side.question")}
            onChange={(e) => setQuestion(e.target.value)}
            onFocus={(e) => e.currentTarget.select()}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !isImeEvent(e)) {
                e.preventDefault();
                ask();
              } else if (e.key === "Escape") {
                e.stopPropagation();
                setQuestion(null);
              }
            }}
            autoFocus
          />
          <div className={s.actions}>
            <Button size="sm" onClick={() => setQuestion(null)}>
              {t("btn.cancel")}
            </Button>
            <Button size="sm" variant="primary" icon="chat-teardrop-dots" disabled={!question.trim()} onClick={ask}>
              {t("ai.side.send")}
            </Button>
          </div>
        </div>
      )}

      {mode === "reject" ? (
        <div className={s.rejectRow}>
          <input
            className={s.input}
            value={reason}
            placeholder={t("ai.approval.reasonPlaceholder")}
            aria-label={t("ai.approval.reason")}
            onChange={(e) => setReason(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !isImeEvent(e)) {
                e.preventDefault();
                reject();
              } else if (e.key === "Escape") {
                e.stopPropagation();
                setMode("view");
              }
            }}
            autoFocus
          />
          <div className={s.actions}>
            <Button size="sm" onClick={() => setMode("view")}>
              {t("btn.cancel")}
            </Button>
            <Button size="sm" variant="danger" onClick={reject}>
              {t("ai.approval.reject")}
            </Button>
          </div>
        </div>
      ) : (
        <div className={s.actions}>
          <LinkButton tone="danger" className={s.rejectLink} onClick={() => switchMode("reject")}>
            {t("ai.approval.reject")}
          </LinkButton>
          {/* The model would see the call as stored, not as edited here, so not while editing. */}
          {question === null && mode === "view" && (
            <LinkButton tone="muted" icon="chat-teardrop-dots" className={s.rejectLink} title={t("ai.side.askCardHint")} onClick={() => setQuestion(t("ai.side.suggest"))}>
              {t("ai.side.askCard")}
            </LinkButton>
          )}
          <span className={s.gap} />
          {mode === "edit" ? (
            <Button size="sm" onClick={() => setMode("view")}>
              {t("btn.cancel")}
            </Button>
          ) : (
            <Button size="sm" icon="pencil-simple" onClick={() => switchMode("edit")}>
              {t("btn.edit")}
            </Button>
          )}
          {/* An edit or write runs once its card has an answer (a diff or why it cannot apply); so does Allow below. */}
          <Button size="sm" variant="primary" icon="play" disabled={holdRun} onClick={() => run()}>
            {t("ai.approval.run")}
          </Button>
        </div>
      )}
      {mode !== "reject" && allow && (
        <div className={s.allowRow}>
          <LinkButton tone="muted" icon="chat-circle-dots" title={allow.hereHint} disabled={holdRun} onClick={() => run("conversation")}>
            {allow.here}
          </LinkButton>
          {/* Always allow covers manual mode; an MCP server set to Always ask still asks in bypass mode. */}
          {allow.always && (
            <LinkButton tone="muted" icon="check-circle" title={allow.alwaysHint} disabled={holdRun} onClick={() => run("always")}>
              {allow.always}
            </LinkButton>
          )}
        </div>
      )}
    </div>
  );
}

// ───────────────────────── turn status ─────────────────────────

/** What the turn is doing between messages, and how the last one ended (§9 states). */
function TurnStatus({ slotId, slot }: { slotId: string; slot: Slot }) {
  const t = useT();
  const { turn, outcome } = slot;
  // AI-45: a message waits for the MCP servers its requests use before it is stored.
  const mcpStarting = useMcp((st) => Object.values(st.status).some((x) => x.state === "starting" && !slot.mcpOff.includes(x.server_id)));

  if (slot.remoteRunning)
    return (
      <div className={s.status}>
        <Spinner size={13} />
        {t("ai.status.remote")}
      </div>
    );
  if (turn) {
    const live = turn.live;
    // A streaming response shows itself (its reasoning header says it is thinking), and so does a call.
    if (live && (live.text || live.reasoning || live.toolCalls.length > 0)) return null;
    if (turn.call) return null;
    const key: MessageKey =
      turn.phase === "tools" ? "ai.status.working" : turn.phase === "starting" && mcpStarting ? "ai.status.mcpStarting" : "ai.status.waiting";
    return (
      <div className={s.status}>
        <Spinner size={13} />
        <span className={s.shimmer}>{t(key)}</span>
      </div>
    );
  }
  if (slot.compacting)
    return (
      <div className={s.status}>
        <Spinner size={13} />
        {t("ai.status.compacting")}
      </div>
    );
  if (!outcome) return null;
  const retry = slot.conversationId ? (
    <Button size="sm" icon="arrows-clockwise" onClick={() => void retryTurn(slotId)}>
      {t("btn.retry")}
    </Button>
  ) : null;
  switch (outcome.reason) {
    case "stopped":
      return (
        <div className={s.status}>
          <Icon name="stop-circle" size={13} />
          {t("ai.outcome.stopped")}
        </div>
      );
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

/**
 * AI-42: a side question and its answer at the end of the conversation, marked as not saved. Stop (or Esc)
 * stops only it; × discards it.
 */
function SideBubble({ slotId, q }: { slotId: string; q: SideQuestion }) {
  const t = useT();
  const running = sideRunning(q);
  const parts = useMemo(() => parseMessage(q.sent), [q.sent]);
  return (
    <div className={s.side} role="region" aria-label={t("ai.side.title")}>
      <div className={s.sideHead}>
        <Icon name="chat-teardrop-dots" size={14} className={s.sideIcon} />
        <span className={s.sideTitle}>{t("ai.side.title")}</span>
        <span className={s.sideNote} title={t("ai.side.notSavedHint")}>
          {t("ai.side.notSaved")}
        </span>
        <span className={s.gap} />
        {running ? (
          <IconButton icon="stop" label={t("ai.side.stop")} size={13} className={s.messageAction} onClick={() => stopSideQuestion(slotId)} />
        ) : (
          q.phase === "done" && q.answer.trim() && <CopyButton text={q.answer} />
        )}
        <IconButton icon="x" label={t("btn.close")} size={13} className={s.messageAction} onClick={() => dismissSideQuestion(slotId)} />
      </div>
      {parts.attachments.map((a, i) => (
        <AttachmentCardFor key={i} attachment={a} />
      ))}
      <div className={cx(s.sideQuestion, "selectable")}>{q.question}</div>
      {q.answer && <Markdown text={q.answer} streaming={q.phase === "streaming"} />}
      {(q.phase === "waiting" || q.phase === "thinking") && (
        <div className={s.status}>
          <Spinner size={13} />
          <span className={s.shimmer}>{t(q.phase === "thinking" ? "ai.reasoning.active" : "ai.status.waiting")}</span>
        </div>
      )}
      {q.phase === "stopped" && (
        <div className={s.status}>
          <Icon name="stop-circle" size={13} />
          {t("ai.outcome.stopped")}
        </div>
      )}
      {q.phase === "done" && q.finish === "length" && <Banner icon="scissors" tone="warn" text={t("ai.outcome.length")} />}
      {q.phase === "done" && q.finish === "refused" && <Banner icon="prohibit" tone="warn" text={t("ai.outcome.refused")} />}
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

function Banner({ icon, tone, text, detail, action }: { icon: string; tone: "warn" | "error"; text: string; detail?: string; action?: ReactNode }) {
  return (
    <div className={cx(s.banner, tone === "error" ? s.bannerError : s.bannerWarn)} role="status">
      <Icon name={icon} size={15} className={s.bannerIcon} />
      <div className={s.bannerMain}>
        <div>{text}</div>
        {detail && <div className={cx(s.bannerDetail, "selectable")}>{detail}</div>}
      </div>
      {action}
    </div>
  );
}
