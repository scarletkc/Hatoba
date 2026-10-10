import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useApp } from "@/app/store";
import { Button, Icon, IconButton, LinkButton, Spinner } from "@/components/controls";
import { PopupSelect, toast } from "@/components/overlay";
import { readClipboard } from "@/features/terminal/clipboard";
import { useT, type MessageKey } from "@/i18n";
import type { AiEntryView, AiToolCall, HostView, McpToolAnnotations, McpToolInfo } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { isImeEvent } from "@/lib/ime";
import { decideCall, editAndResend, retryTurn, stopTurn, tooMuchMessage, type Decision } from "./actions";
import { AttachmentCardFor, AttachmentChipFor } from "./AttachmentChips";
import { FileChange } from "./FileChange";
import { useFilePreview } from "./filePreview";
import { composeMessage, fitsMessage, isLongPaste, makePaste, parseMessage, type MessageParts, type Note } from "./attachments";
import { insertAtCaret, isPlainPasteKey } from "./Composer";
import { Markdown } from "./Markdown";
import { noteLabel, noteTitle } from "./notes";
import { patchSlot, useAi, type Slot } from "./store";
import { callSummary, exitStatusOf, parseArgs, prettyArgs, SEND_KEYS, shownText, toolKind, toolLabel, type SendKey, type ToolKind } from "./tools";
import type { CallState, LiveResponse } from "./turn";
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
    const key = entry.role === "tool" ? `call:${entry.tool_call_id}` : `entry:${entry.entry_id}`;
    const el = list.querySelector(`[data-reveal="${CSS.escape(key)}"]`);
    patchSlot(slotId, { reveal: null });
    if (!el) return;
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

  // AI-26: the user's stored messages can be edited and sent again while nothing runs.
  const canEdit = !!slot.conversationId && !turn && !slot.remoteRunning && !slot.compacting && !slot.loading;

  const isEmpty = entries.length === 0 && !turn && !slot.remoteRunning && !outcome;

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
                pending={pending}
                onResend={canEdit && !pending ? (text) => editAndResend(slotId, entry.entry_id, text) : undefined}
              />
            );
            break;
          }
          case "assistant":
            body = (
              <AssistantMessage
                slotId={slotId}
                text={entry.text}
                reasoning={entry.reasoning}
                calls={entry.tool_calls}
                view={(call) => callView(call, entry.entry_id === lastAssistant)}
                host={host}
                finish={entry.finish}
                flash={flash}
              />
            );
            break;
          case "summary":
            body = <SummaryBlock text={entry.text} flash={flashed} />;
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
    </div>
  );
}

// ───────────────────────── entries ─────────────────────────

/**
 * The user's message. With `onResend`, Edit replaces it and sends it again (AI-26). The notes Hatoba
 * stored before it (AI-05, AI-09) are dividers above it, not part of the bubble; an edit shows them
 * while it is sent, and Rust works them out again for the new message.
 */
function UserMessage({ text, pending, onResend }: { text: string; pending: boolean; onResend?: (text: string) => Promise<boolean> }) {
  const t = useT();
  const [editing, setEditing] = useState(false);
  // AI-10, AI-35: what was attached to the message is in blocks at its start, after Hatoba's notes.
  const parts = useMemo(() => parseMessage(text), [text]);
  const notes = parts.notes.map((note, i) => <NoteDivider key={i} note={note} />);
  if (editing && onResend)
    return (
      <>
        {notes}
        <EditMessage parts={parts} onCancel={() => setEditing(false)} onSend={onResend} onSent={() => setEditing(false)} />
      </>
    );
  return (
    <>
      {notes}
      <div className={s.userRow}>
        {onResend && <IconButton icon="pencil-simple" label={t("ai.edit")} size={13} className={s.editButton} onClick={() => setEditing(true)} />}
        <div className={s.userStack}>
          {parts.attachments.map((a, i) => (
            <AttachmentCardFor key={i} attachment={a} />
          ))}
          <div className={cx(s.user, pending && s.userPending, "selectable")}>{parts.typed}</div>
        </div>
      </div>
    </>
  );
}

const NOTE_ICONS = { host_change: "arrows-left-right", model_change: "swap" } as const;

/**
 * A note Hatoba stored before a message: the conversation moved to another host, a terminal became
 * attached or went away, or it went to another model.
 */
function NoteDivider({ note }: { note: Note }) {
  const t = useT();
  const icon = note.kind === "terminal_change" ? (note.to === "attached" ? "plugs-connected" : "plugs") : NOTE_ICONS[note.kind];
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
 * paste becomes another attachment, as in the input box (AI-35).
 */
function EditMessage({
  parts,
  onCancel,
  onSend,
  onSent,
}: {
  parts: MessageParts;
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
  text,
  reasoning,
  calls,
  view,
  host,
  streaming,
  finish,
  flash,
}: {
  slotId: string;
  text: string;
  reasoning: string | null;
  calls: AiToolCall[];
  view: (call: AiToolCall) => CallView;
  host: HostInfo | null;
  streaming?: boolean;
  finish?: string;
  /** The highlighted search hit (AI-24). */
  flash?: string | null;
}) {
  const thinking = !!streaming && !text && calls.length === 0;
  return (
    <div className={s.assistant}>
      {reasoning && <Reasoning text={reasoning} active={thinking} />}
      {text && <Markdown text={text} streaming={streaming && calls.length === 0} />}
      {!text && !reasoning && calls.length === 0 && !streaming && finish !== "tool_calls" && <EmptyReply />}
      {calls.map((call) => (
        <ToolBlock key={call.id} slotId={slotId} call={call} view={view(call)} host={host} flash={flash === `call:${call.id}`} />
      ))}
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
    case "limit":
      return <span className={cx(s.chip, s.chipWarn)}>{t("ai.call.paused")}</span>;
    case "queued":
    case "streaming":
      return <span className={s.chip}>{t("ai.call.queued")}</span>;
    case "none":
      return <span className={s.chip}>{t("ai.call.notRun")}</span>;
  }
}

/** Each call is a collapsible block with its input, output and status (§9); approval cards appear in place. */
function ToolBlock({ slotId, call, view, host, flash }: { slotId: string; call: AiToolCall; view: CallView; host: HostInfo | null; flash?: boolean }) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const kind = toolKind(call.name);
  const summary = callSummary(call.name, call.arguments);
  useEffect(() => {
    if (flash) setOpen(true);
  }, [flash]);

  if (view.kind === "approval") return <ApprovalCard slotId={slotId} call={call} host={host} mcp={view.mcp ?? null} />;

  return (
    <div className={cx(s.tool, view.kind === "limit" && s.toolAttention, flash && s.flash)} data-reveal={`call:${call.id}`}>
      <button type="button" className={s.toolHead} aria-expanded={open} onClick={() => setOpen((v) => !v)}>
        <Icon name={TOOL_ICON[kind]} size={14} className={s.toolIcon} />
        <span className={s.toolName}>{toolLabel(t, call.name)}</span>
        <span className={s.toolSummary}>{summary}</span>
        <StatusChip view={view} name={call.name} />
        <Icon name={open ? "caret-down" : "caret-right"} size={11} className={s.toolCaret} />
      </button>
      {open && (
        <div className={s.toolBody}>
          <div className={s.label}>{t("ai.call.input")}</div>
          <pre className={cx(s.pre, "selectable")}>{prettyArgs(call.arguments) || "{}"}</pre>
          {view.kind === "result" && (
            <>
              <div className={s.label}>{t("ai.call.output")}</div>
              <pre className={cx(s.pre, s.output, "selectable")}>{view.entry.content || t("ai.call.noOutput")}</pre>
            </>
          )}
        </div>
      )}
      {view.kind === "limit" && <LimitCard slotId={slotId} />}
    </div>
  );
}

/** AI-18: the turn paused at the tool call limit. */
function LimitCard({ slotId }: { slotId: string }) {
  const t = useT();
  const limit = useApp((st) => st.prefs.ai_tool_call_limit);
  return (
    <div className={s.cardFoot}>
      <span className={s.cardText}>{t("ai.limit.body", { n: limit })}</span>
      <div className={s.actions}>
        <Button size="sm" onClick={() => void stopTurn(slotId)}>
          {t("ai.stop")}
        </Button>
        <Button size="sm" variant="primary" icon="play" onClick={() => decideCall(slotId, { kind: "continue" })}>
          {t("btn.continue")}
        </Button>
      </div>
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
    case "read_file":
      return { main: str(args.path), timeout: "", key: "", wait: "" };
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
    case "read_file":
      return { json: JSON.stringify({ ...base, path: d.main.trim() }) };
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

/**
 * AI-17: the tool, the tab's host, and the full input, with Run, Edit and Reject. An MCP call (AI-31)
 * shows its server, the tool's description and hints in place of the host, and offers Always allow.
 * Allow for this conversation (AI-19) is on every card.
 */
function ApprovalCard({ slotId, call, host, mcp }: { slotId: string; call: AiToolCall; host: HostInfo | null; mcp: McpToolInfo | null }) {
  const t = useT();
  const kind = toolKind(call.name);
  const args = useMemo(() => parseArgs(call.arguments) ?? {}, [call.arguments]);
  const [mode, setMode] = useState<"view" | "edit" | "reject">("view");
  const [draft, setDraft] = useState<Draft>(() => draftFrom(kind, args, call.arguments));
  const [reason, setReason] = useState("");
  const [invalid, setInvalid] = useState(false);
  const label = mcp ? t("ai.tool.mcp", { server: mcp.server_name, tool: mcp.tool.tool }) : toolLabel(t, call.name);
  const bypass = useAi((st) => st.slots[slotId]?.mode === "bypass");
  // AI-39, AI-40: an edit or write shows its diff, which follows the text changed in Edit.
  const fileChange = kind === "edit_file" || kind === "write_file";
  const editedFile = useMemo(() => {
    if (!fileChange || mode !== "edit") return null;
    const next = argsFrom(kind, args, draft);
    return "json" in next && next.json !== JSON.stringify(args) ? next.json : null;
  }, [fileChange, mode, kind, args, draft]);
  const file = useFilePreview(slotId, call.id, editedFile, fileChange);

  const run = (allow?: Extract<Decision, { kind: "run" }>["allow"]) => {
    if (mode !== "edit") return decideCall(slotId, { kind: "run", edited: null, allow });
    const next = argsFrom(kind, args, draft);
    if ("error" in next) return setInvalid(true);
    const same = next.json === JSON.stringify(args);
    decideCall(slotId, { kind: "run", edited: same ? null : next.json, allow });
  };
  const reject = () => decideCall(slotId, { kind: "reject", reason: reason.trim() });
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
          {kind === "fetch_url" || kind === "read_file" ? (
            <input
              className={s.input}
              value={draft.main}
              aria-label={kind === "read_file" ? t("ai.approval.path") : t("ai.approval.url")}
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
              <input className={cx(s.input, s.number)} type="number" min={1} max={600} placeholder="30" value={draft.timeout} onChange={(e) => setDraft({ ...draft, timeout: e.target.value })} />
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
              <div className={s.meta}>{t("ai.approval.timeoutValue", { s: str(args.timeout_seconds) || 30 })}</div>
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
          {kind === "read_file" && (
            <>
              <ShownInput text={str(args.path)} command />
              <div className={s.meta}>
                {[t("ai.approval.fromLine", { n: str(args.offset) || 1 }), args.limit !== undefined && args.limit !== null ? t("ai.approval.maxLines", { n: str(args.limit) }) : ""]
                  .filter(Boolean)
                  .join(" · ")}
              </div>
            </>
          )}
          {fileChange && <FileChange state={file} replaceAll={args.replace_all === true} />}
          {kind !== "run_command" && kind !== "send_input" && kind !== "fetch_url" && kind !== "read_file" && !fileChange && (
            <>
              {mcp && <div className={s.label}>{t("ai.approval.arguments")}</div>}
              <ShownInput text={prettyArgs(call.arguments) || "{}"} />
            </>
          )}
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
          <LinkButton tone="danger" className={s.rejectLink} onClick={() => setMode("reject")}>
            {t("ai.approval.reject")}
          </LinkButton>
          <span className={s.gap} />
          {mode === "edit" ? (
            <Button size="sm" onClick={() => setMode("view")}>
              {t("btn.cancel")}
            </Button>
          ) : (
            <Button size="sm" icon="pencil-simple" onClick={() => setMode("edit")}>
              {t("btn.edit")}
            </Button>
          )}
          {/* An edit or write runs once its diff has been read, so the user approves what they saw; so does Allow below. */}
          <Button size="sm" variant="primary" icon="play" disabled={fileChange && file.loading} onClick={() => run()}>
            {t("ai.approval.run")}
          </Button>
        </div>
      )}
      {/* An MCP call whose tool is unknown cannot be allowed ahead: nothing names what it would reach. */}
      {mode !== "reject" && (kind !== "mcp" || mcp) && (
        <div className={s.allowRow}>
          <LinkButton
            tone="muted"
            icon="chat-circle-dots"
            title={t("ai.approval.allowHereHint", { tool: label })}
            disabled={fileChange && file.loading}
            onClick={() => run("conversation")}
          >
            {t("ai.approval.allowHere")}
          </LinkButton>
          {/* Always allow covers manual mode; a server set to Always ask still asks in bypass mode. */}
          {mcp && !(bypass && mcp.always_ask) && (
            <LinkButton tone="muted" icon="check-circle" title={t("ai.approval.alwaysAllowHint", { tool: label })} onClick={() => run("always")}>
              {t("ai.approval.alwaysAllow")}
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
    const key: MessageKey = turn.phase === "tools" ? "ai.status.working" : "ai.status.waiting";
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
    case "error":
      return (
        <Banner
          icon="warning-circle"
          tone="error"
          text={outcome.status ? t("ai.outcome.errorStatus", { status: outcome.status }) : t("ai.outcome.error")}
          detail={outcome.message}
          action={
            slot.conversationId ? (
              <Button size="sm" icon="arrows-clockwise" onClick={() => void retryTurn(slotId)}>
                {t("btn.retry")}
              </Button>
            ) : null
          }
        />
      );
  }
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
