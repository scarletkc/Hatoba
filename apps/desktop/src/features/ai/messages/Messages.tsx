import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { StatusLine } from "@hatoba/ui";
import { useT, type MessageKey } from "@/i18n";
import type { AiToolCall } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { forkConversation } from "../actions/conversations";
import { editAndResend } from "../actions/turns";
import { splitPermissionNote } from "../attachments";
import type { CallView, ToolEntry } from "../calls/ToolBlock";
import type { HostInfo } from "../calls/hostInfo";
import { forkPoints } from "../fork";
import { useMcp } from "../mcp";
import { patchSlot, useAi, type Slot } from "../store";
import { editedFiles, LOCAL_ENTRY, revealKey } from "../turn";
import { AssistantMessage, LiveMessage } from "./AssistantMessage";
import { useFollowBottom } from "./followBottom";
import { replies as findReplies } from "./replies";
import { SideBubble } from "./SideBubble";
import { SummaryBlock } from "./SummaryBlock";
import { TurnStatus } from "./TurnStatus";
import { UserMessage } from "./UserMessage";
import s from "./Messages.module.css";

/** How long a revealed entry stays highlighted (AI-24). */
const FLASH_MS = 2400;

export function MessageList({
  slotId,
  slot,
  host,
  empty,
}: {
  slotId: string;
  slot: Slot;
  host: HostInfo | null;
  empty: ReactNode;
}) {
  const t = useT();
  const ref = useRef<HTMLDivElement>(null);
  const { entries, turn, outcome } = slot;
  // The reply at the end may still grow while a turn runs: it shows its time and Copy once it is finished,
  // and until then the list follows what grows in it on its own.
  const running = !!turn || slot.remoteRunning;
  const bottom = useFollowBottom(ref, running);
  const pending = usePendingLine(slot);

  const results = useMemo(() => {
    const map = new Map<string, ToolEntry>();
    for (const e of entries) if (e.role === "tool") map.set(e.tool_call_id, e);
    return map;
  }, [entries]);

  const contextAt = slot.conversation?.context_start
    ? entries.findIndex((e) => e.entry_id === slot.conversation?.context_start)
    : -1;
  const lastAssistant = useMemo(() => {
    for (let i = entries.length - 1; i >= 0; i--) if (entries[i].role === "assistant") return entries[i].entry_id;
    return null;
  }, [entries]);
  const replies = useMemo(() => findReplies(entries), [entries]);
  const firstUser = entries.find((e) => e.role === "user")?.entry_id;
  // AI-41: the changes of file tool calls this device keeps, by response and call.
  const changes = useMemo(
    () => new Map(slot.fileChanges.map((c) => [`${c.response_id}:${c.tool_call_id}`, c])),
    [slot.fileChanges],
  );

  // The user's own message brings the list back to the bottom, to follow the reply.
  const sending = entries[entries.length - 1]?.entry_id === LOCAL_ENTRY;
  useLayoutEffect(() => {
    if (sending) bottom.stick();
  }, [sending, bottom]);

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
    bottom.release();
    el.scrollIntoView({ block: "center" });
    setFlash(key);
  }, [reveal, entries, slot.loading, slotId, bottom]);
  useEffect(() => {
    if (!flash) return;
    const timer = window.setTimeout(() => setFlash(null), FLASH_MS);
    return () => window.clearTimeout(timer);
  }, [flash]);

  const callView = (call: AiToolCall, newest: boolean): CallView => {
    const result = results.get(call.id);
    if (result) return { kind: "result", entry: result };
    const handled = turn?.calls.find((c) => c.id === call.id);
    if (handled) return { kind: handled.state, mcp: handled.mcp, auto: handled.auto };
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
    canEdit && !slot.reverting && !forking && conversationId && forkable.has(entryId)
      ? () => void forkConversation(slotId, conversationId, entryId)
      : undefined;

  const isEmpty = entries.length === 0 && !turn && !slot.remoteRunning && !outcome;
  // AI-41: a revert waits until the conversation is idle, so the next message tells the model before it acts again.
  const busy = running || slot.compacting || slot.loading || slot.reverting;

  return (
    <div
      ref={ref}
      className={s.list}
      data-text-scale
      onScroll={bottom.onScroll}
      onWheel={bottom.onWheel}
      onKeyDown={bottom.onKeyDown}
    >
      {isEmpty && !slot.loading && empty}
      {slot.loading && entries.length === 0 && <StatusLine tone="progress">{t("ai.loading")}</StatusLine>}
      {entries.map((entry, i) => {
        const outside = contextAt > 0 && i < contextAt;
        const divider =
          i === contextAt && contextAt > 0 ? <div className={s.contextDivider}>{t("ai.outsideContext")}</div> : null;
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
                first={entry.entry_id === firstUser}
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
            <div
              // A message just sent comes up into place; the stored copy that replaces it does not.
              className={cx(
                s.item,
                outside && s.outside,
                flashed && entry.role !== "summary" && s.flash,
                entry.entry_id.startsWith("~") && s.enter,
              )}
              data-reveal={key}
            >
              {body}
            </div>
          </Fragment>
        );
      })}
      {turn && (turn.live || pending) && <LiveMessage slotId={slotId} live={turn.live} pending={pending} host={host} />}
      <TurnStatus slotId={slotId} slot={slot} />
      {slot.effortIgnored && (
        <div role="status">
          <StatusLine icon="info">{t("ai.effortIgnored")}</StatusLine>
        </div>
      )}
      {slot.side && <SideBubble slotId={slotId} q={slot.side} />}
    </div>
  );
}

/**
 * What the turn waits for while its reply has nothing to show (§9 states), or null: once something streams, while
 * calls are handled, and while another device runs the conversation.
 */
function usePendingLine(slot: Slot): MessageKey | null {
  // AI-45: a message waits for the MCP servers its requests use before it is stored.
  const mcpStarting = useMcp((st) =>
    Object.values(st.status).some((x) => x.state === "starting" && !slot.mcpOff.includes(x.server_id)),
  );
  const { turn } = slot;
  if (!turn || slot.remoteRunning) return null;
  const live = turn.live;
  if (live && (live.text || live.reasoning || live.toolCalls.length > 0)) return null;
  if (turn.calls.length > 0) return null;
  if (turn.phase === "tools") return "ai.status.working";
  if (turn.phase === "starting" && mcpStarting) return "ai.status.mcpStarting";
  return live?.thinking ? "ai.reasoning.active" : "ai.status.waiting";
}
