import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ClipboardEvent, type KeyboardEvent, type MutableRefObject, type RefObject } from "react";
import { useApp } from "@/app/store";
import { Icon, LinkButton } from "@/components/controls";
import { confirm, Menu, toast, useMenu, type MenuEntry } from "@/components/overlay";
import { readClipboard } from "@/features/terminal/clipboard";
import { useT, type MessageKey } from "@/i18n";
import type { AiEffort, AiModelRef, AiPermissionMode } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { isImeEvent } from "@/lib/ime";
import { shortcutLabel } from "@/lib/platform";
import { AddButton, McpPopover } from "./AddMenu";
import {
  addPaste,
  askSideQuestion,
  attachFiles,
  compactConversation,
  discardQueued,
  editQueued,
  newConversation,
  pendingAttachments,
  queueMessage,
  removeExtra,
  removeQueuedAttachment,
  sendMessage,
  setSlotEffort,
  setSlotMode,
  setSlotModel,
  stopTurn,
} from "./actions";
import { attachCommand } from "./commandActions";
import { useCommandMenu } from "./CommandMenu";
import { commandDraft, findBuiltin, leadingCommand, type BuiltinCommand } from "./commands";
import { ComposerAttachments, QueuedAttachments, usePendingAttachments, useTerminalSelection } from "./AttachmentChips";
import { composeMessage, isLongPaste } from "./attachments";
import { chosenEffort, effectiveEffort, effortKey, modelEfforts } from "./effort";
import { AUTO_COMPACT_RATIO, contextUsage, estimateTokens, formatTokens, messageFit, type MeterState } from "./meter";
import { failedServers, loadMcp, toolsMenuRows, useMcp } from "./mcp";
import { chats, findModel, judgeChosen, sameModel } from "./models";
import { messageAttachments, messageText, takenBeside } from "./queue";
import { canAskSide } from "./side";
import { getSlot, patchSlot, slotGeneration, useAi, type QueuedMessage, type Slot } from "./store";
import { MODE_ICON } from "./tools";
import s from "./Composer.module.css";

/** Ctrl+Shift+V, or ⌘⇧V on macOS: always pastes plain text into the box (AI-35). */
export function isPlainPasteKey(e: KeyboardEvent): boolean {
  return e.key.toLowerCase() === "v" && e.shiftKey && !e.altKey && (e.ctrlKey || e.metaKey);
}

/** Puts `text` into a text box at its caret, replacing the selection, and returns the new value. */
export function insertAtCaret(el: HTMLTextAreaElement | null, value: string, text: string): { value: string; caret: number } {
  const start = el?.selectionStart ?? value.length;
  const end = el?.selectionEnd ?? value.length;
  return { value: value.slice(0, start) + text + value.slice(end), caret: start + text.length };
}

/** Alt+Enter, or ⌥↩ on macOS: asks what the box holds as a side question (AI-42). */
function isSideQuestionKey(e: KeyboardEvent): boolean {
  return e.key === "Enter" && e.altKey && !e.shiftKey && !e.ctrlKey && !e.metaKey;
}

/** Ctrl+Enter, or ⌘↩ on macOS: sends at once, stopping a running turn (§13.1). */
function isSendNowKey(e: KeyboardEvent): boolean {
  return e.key === "Enter" && (e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey;
}

const noModifier = (e: KeyboardEvent) => !e.shiftKey && !e.altKey && !e.ctrlKey && !e.metaKey;

/** How the box's text goes: Enter (sent, or queued while a turn runs), Ctrl+Enter (sent at once), Alt+Enter (a side question). */
type Submit = "enter" | "now" | "side";

/**
 * The input area (§9): the queued message (§13.1), the attachment chips, a multi-line box with its `/` and `@` menu
 * (AI-50, AI-51), and under it the + menu and the permission mode (AI-16) on the left, the context meter, the model
 * selector, Ask a Side Question (AI-42) and Send / Stop on the right. Above the box, a line names the MCP servers
 * the conversation uses that failed, which `/mcp` shows (AI-32).
 */
export function Composer({ slotId, slot, model, tools }: { slotId: string; slot: Slot; model: AiModelRef | null; tools: boolean }) {
  const t = useT();
  const ref = useRef<HTMLTextAreaElement>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const openModels = useRef<(() => void) | null>(null);
  const [mcpAnchor, setMcpAnchor] = useState<DOMRect | null>(null);
  const [resolving, setResolving] = useState(false);
  const focusTick = useAi((st) => st.focusTick);
  const providers = useAi((st) => st.catalog.providers);
  const platform = useApp((st) => st.info.platform);
  const busy = !!slot.turn || slot.remoteRunning;
  const queued = slot.queued;
  const sideKey = shortcutLabel(platform, "Alt+Enter", "⌥↩");
  const sendKey = shortcutLabel(platform, "Ctrl+Enter", "⌘↩");
  useTerminalSelection(slotId);

  // Focus requests (New conversation, Ask AI, …) put the caret after the draft, or select a
  // suggested question so typing replaces it.
  useEffect(() => {
    const el = ref.current;
    if (focusTick === 0 || !el) return;
    el.focus();
    if (useAi.getState().focusSelectAll) el.select();
    else el.setSelectionRange(el.value.length, el.value.length);
    el.scrollTop = el.scrollHeight;
  }, [focusTick]);

  // Grow with the text up to a few lines.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 168)}px`;
  }, [slot.draft]);

  // AI-20 and AI-35: the context in use, and whether the next message with its attachments fits.
  const ctxWindow = findModel(providers, model)?.model.context_window ?? null;
  const live = slot.turn?.live ? slot.turn.live.text + slot.turn.live.reasoning : "";
  const meter: MeterState = useMemo(
    () => contextUsage({ entries: slot.entries, contextStart: slot.conversation?.context_start ?? null, model, contextWindow: ctxWindow, pending: live }),
    [slot.entries, slot.conversation?.context_start, model, ctxWindow, live],
  );
  const pending = usePendingAttachments(slotId);
  // The queued message goes with what the box holds, which leaves a chip of a kind it has, or alone while
  // the box is empty, which leaves every attachment for a later message (§13.1).
  const fit = useMemo(() => {
    const taken = takenBeside(pending, queued);
    const typed =
      queued && !slot.draft.trim() ? 0 : taken.length > 0 ? estimateTokens(composeMessage(slot.draft.trim(), taken)) : estimateTokens(slot.draft);
    return messageFit(meter, typed + (queued ? estimateTokens(messageText(queued)) : 0));
  }, [meter, pending, slot.draft, queued]);
  const blocked = fit.level === "over";

  // §13.1: Enter with no turn running, or Ctrl+Enter: the queued message and the input area go now.
  const canSend = (!!slot.draft.trim() || !!queued) && !!model && !blocked;
  // AI-42: the turn, if any, goes on; the answer shows at the end of the conversation. While a turn
  // runs, Rust asks with the turn's model, not the one picked here that the fit is measured
  // against, so the provider's own limit decides.
  const canSide = !!model && (busy || !blocked) && canAskSide(slot, slot.draft);

  // AI-32: the MCP servers the conversation uses that failed, which the line above the box names.
  const mcpServers = useMcp((st) => st.servers);
  const mcpStatus = useMcp((st) => st.status);
  const failed = useMemo(() => (mcpServers ? failedServers(toolsMenuRows(mcpServers, mcpStatus, slot.mcpOff)) : []), [mcpServers, mcpStatus, slot.mcpOff]);
  useEffect(() => {
    if (useMcp.getState().servers === null) void loadMcp();
  }, []);
  useEffect(() => {
    setMcpAnchor(null);
    setResolving(false);
  }, [slotId]);

  /** Puts text into the box at the caret, as typing would. */
  const insertText = (text: string) => {
    const el = ref.current;
    const next = insertAtCaret(el, getSlot(slotId).draft, text);
    setDraft(next.value, next.caret, true);
  };

  /** Replaces the box's text and, when `focus`, focuses the box with the caret at `caret`. */
  const setDraft = (text: string, caret: number, focus: boolean) => {
    const el = ref.current;
    patchSlot(slotId, { draft: text });
    if (!focus) return;
    requestAnimationFrame(() => {
      el?.focus();
      el?.setSelectionRange(caret, caret);
      menu.onCaret();
    });
  };

  /** `/mcp`, or the line that names a failed server: the MCP servers above the input box (AI-50). */
  const showMcp = () => {
    const r = boxRef.current?.getBoundingClientRect();
    if (r) setMcpAnchor(r);
  };

  /**
   * AI-50: runs a built-in command, which takes the box's text. One that takes nothing after its name leaves what
   * follows it in the box.
   */
  const runBuiltin = (command: BuiltinCommand, args: string, raw: string) => {
    const s = getSlot(slotId);
    const rest = command.args ? "" : args;
    switch (command.name) {
      case "btw":
        if (!args) return;
        if (!s.conversationId) {
          toast(t("ai.cmd.btw.noConversation"), "error");
          return;
        }
        // As Alt+Enter: with no turn running, a question that does not fit the context waits (AI-42).
        if (!s.turn && !s.remoteRunning && blocked) {
          toast(t("ai.fit.blocked"), "error");
          return;
        }
        askSideQuestion(slotId, args, true, raw);
        return;
      case "clear":
        patchSlot(slotId, { draft: rest });
        newConversation(slotId);
        return;
      case "compact":
        if (!s.conversationId || s.entries.length === 0) {
          toast(t("ai.cmd.compact.nothing"), "error");
          return;
        }
        if (s.turn || s.remoteRunning || s.compacting) {
          toast(t("ai.cmd.compact.busy"), "error");
          return;
        }
        patchSlot(slotId, { draft: "" });
        // What was typed comes back when no summary was stored (refused or failed), unless the box holds something new.
        void compactConversation(slotId, args || null).then((done) => {
          if (!done && !getSlot(slotId).draft) patchSlot(slotId, { draft: raw });
        });
        return;
      case "mcp":
        patchSlot(slotId, { draft: rest });
        showMcp();
        return;
      case "memory":
        patchSlot(slotId, { draft: rest });
        useApp.getState().openSettings(true, "ai", "memory");
        return;
      case "model":
        patchSlot(slotId, { draft: rest });
        openModels.current?.();
        return;
    }
  };

  /**
   * Enter, Ctrl+Enter, Alt+Enter and the buttons (§13.1, AI-42). A built-in command runs (AI-50); a skill or MCP
   * prompt the text starts with is attached first, and the text then goes as typed, its command included.
   */
  const submit = async (how: Submit, raw: string = getSlot(slotId).draft) => {
    // An `@` pick still attaching (AI-51): Send would leave it behind for the next message.
    if (menu.busy) return;
    const command = leadingCommand(raw);
    if (command) {
      const builtin = findBuiltin(command.name);
      if (builtin) {
        runBuiltin(builtin, command.args, raw);
        return;
      }
      if (resolving) return;
      if (model) {
        setResolving(true);
        const generation = slotGeneration(slotId);
        const outcome = await attachCommand(slotId, command.name, command.args).finally(() => setResolving(false));
        // Not when it failed, or when the box changed or the tab opened another conversation meanwhile (New and
        // History keep the box).
        if (outcome === "failed" || getSlot(slotId).draft !== raw || slotGeneration(slotId) !== generation) return;
        // AI-35: what the command attached counts toward whether the message fits.
        if (outcome === "attached" && messageFit(meter, estimateTokens(composeMessage(raw.trim(), pendingAttachments(slotId)))).level === "over") {
          toast(t("ai.fit.blocked"), "error");
          return;
        }
      }
    }
    const now = getSlot(slotId);
    const running = !!now.turn || now.remoteRunning;
    if (how === "side") {
      if (model && (running || !blocked) && canAskSide(now, raw)) askSideQuestion(slotId, raw);
      return;
    }
    if (how === "enter" && running) {
      if (raw.trim() && model && !blocked) queueMessage(slotId, raw);
      return;
    }
    if ((raw.trim() || now.queued) && model && !blocked) void sendMessage(slotId, raw);
  };

  const menu = useCommandMenu({
    slotId,
    slot,
    input: ref,
    box: boxRef,
    setDraft,
    run: (text) => {
      patchSlot(slotId, { draft: text });
      void submit("enter", text);
    },
    onLocalFiles: () => fileRef.current?.click(),
  });

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (menu.onKeyDown(e)) return;
    if (isPlainPasteKey(e)) {
      e.preventDefault();
      readClipboard().then(
        (text) => text && insertText(text.replace(/\r\n?/g, "\n")),
        () => toast(t("terminal.clipboardDenied"), "error"),
      );
      return;
    }
    // Enter sends, or queues while a turn runs, and Ctrl+Enter sends at once (§13.1); Alt+Enter asks a
    // side question (AI-42), Shift+Enter adds a line, and nothing is sent while an IME is composing.
    if (isSideQuestionKey(e)) {
      if (isImeEvent(e)) return;
      e.preventDefault();
      void submit("side");
      return;
    }
    if (isSendNowKey(e)) {
      if (isImeEvent(e)) return;
      e.preventDefault();
      void submit("now");
      return;
    }
    // ↑ in the empty box takes the queued message back to edit it.
    if (e.key === "ArrowUp" && queued && !slot.draft && noModifier(e)) {
      if (isImeEvent(e)) return;
      e.preventDefault();
      editQueued(slotId);
      return;
    }
    if (e.key !== "Enter" || !noModifier(e)) return;
    if (isImeEvent(e)) return;
    e.preventDefault();
    void submit("enter");
  };

  // AI-35: a long paste becomes an attachment, and pasted files are attached like picked ones.
  const onPaste = (e: ClipboardEvent<HTMLTextAreaElement>) => {
    const text = e.clipboardData.getData("text/plain");
    if (!text && e.clipboardData.files.length > 0) {
      e.preventDefault();
      void attachFiles(slotId, Array.from(e.clipboardData.files));
      return;
    }
    if (!isLongPaste(text)) return;
    e.preventDefault();
    addPaste(slotId, text);
  };

  return (
    <div className={s.composer}>
      {queued && <QueuedCard slotId={slotId} queued={queued} />}
      {failed.length > 0 && (
        <div className={s.mcpFailed} role="status">
          <Icon name="warning-circle" size={13} className={s.fitIcon} />
          <span className={s.mcpFailedText}>
            {failed.length === 1 ? t("ai.mcp.failed", { name: failed[0].server.name }) : t("ai.mcp.failedMany", { n: failed.length })}
          </span>
          <LinkButton className={s.mcpFailedLink} onClick={showMcp}>
            /mcp
          </LinkButton>
        </div>
      )}
      <div ref={boxRef} className={s.box}>
        <ComposerAttachments
          slotId={slotId}
          onPasteAsText={(id, text) => {
            removeExtra(slotId, id);
            insertText(text);
          }}
        />
        {fit.level !== "ok" && (
          <div className={cx(s.fit, fit.level === "over" && s.fitOver)} role="status">
            <Icon name="warning" size={13} className={s.fitIcon} />
            <span>
              {fit.level === "over"
                ? t("ai.fit.over", { used: formatTokens(fit.tokens), total: formatTokens(fit.window) })
                : t("ai.fit.warn", { percent: Math.round(fit.ratio * 100) })}
            </span>
          </div>
        )}
        <textarea
          ref={ref}
          className={s.input}
          rows={1}
          value={slot.draft}
          placeholder={
            busy && slot.conversationId
              ? t("ai.input.placeholderBusy", { send: sendKey, side: sideKey })
              : tools
                ? t("ai.input.placeholder")
                : t("ai.input.placeholderChat")
          }
          aria-label={t("ai.input.label")}
          spellCheck={false}
          {...menu.inputProps}
          onChange={(e) => {
            patchSlot(slotId, { draft: e.target.value });
            menu.onCaret();
          }}
          onSelect={menu.onCaret}
          onFocus={menu.onCaret}
          onBlur={menu.onBlur}
          onKeyDown={onKeyDown}
          onPaste={onPaste}
        />
        {menu.element}
        <div className={s.bar}>
          <AddButton
            slotId={slotId}
            slot={slot}
            onAddFiles={() => fileRef.current?.click()}
            onCommand={(skill) => {
              const next = commandDraft(getSlot(slotId).draft, skill);
              setDraft(next.text, next.caret, true);
            }}
          />
          <ModeButton slotId={slotId} mode={slot.mode} />
          <input
            ref={fileRef}
            type="file"
            multiple
            hidden
            onChange={(e) => {
              const files = Array.from(e.target.files ?? []);
              e.target.value = "";
              if (files.length > 0) void attachFiles(slotId, files);
            }}
          />
          <div className={s.grow} data-grow />
          <ContextMeter slotId={slotId} slot={slot} m={meter} />
          <ModelPicker slotId={slotId} slot={slot} value={model} openRef={openModels} />
          {slot.conversationId && (
            <button
              type="button"
              className={s.side}
              title={t("ai.side.askHint", { key: sideKey })}
              aria-label={t("ai.side.ask")}
              disabled={!canSide || resolving || menu.busy}
              onClick={() => void submit("side")}
            >
              <Icon name="chat-teardrop-dots" size={15} />
            </button>
          )}
          {busy ? (
            <button
              type="button"
              className={cx(s.send, s.stop)}
              title={queued ? t("ai.stopAndSend") : t("ai.stopHint")}
              aria-label={t("ai.stop")}
              onClick={() => void stopTurn(slotId)}
            >
              <Icon name="stop" fill size={12} />
            </button>
          ) : (
            <button
              type="button"
              className={s.send}
              title={blocked ? t("ai.fit.blocked") : t("ai.send")}
              aria-label={t("ai.send")}
              disabled={!canSend || resolving || menu.busy}
              onClick={() => void submit("enter")}
            >
              <Icon name="arrow-up" size={14} />
            </button>
          )}
        </div>
      </div>
      {mcpAnchor && (
        <McpPopover
          slotId={slotId}
          slot={slot}
          anchor={mcpAnchor}
          onClose={(restoreFocus) => {
            setMcpAnchor(null);
            if (restoreFocus) ref.current?.focus();
          }}
        />
      )}
    </div>
  );
}

/** §13.1: the message waiting for the turn to end, above the input box: its text and attachments, Edit and ×. */
function QueuedCard({ slotId, queued }: { slotId: string; queued: QueuedMessage }) {
  const t = useT();
  return (
    <div className={s.queued} role="group" aria-label={t("ai.queued")}>
      <div className={s.queuedHead}>
        <Icon name="clock" size={13} className={s.queuedIcon} />
        <span className={s.queuedTitle}>{t("ai.queued")}</span>
        <LinkButton className={s.queuedEdit} title={t("ai.queued.editHint")} onClick={() => editQueued(slotId)}>
          {t("ai.queued.edit")}
        </LinkButton>
        <button type="button" className={s.queuedRemove} title={t("ai.queued.remove")} aria-label={t("ai.queued.remove")} onClick={() => discardQueued(slotId)}>
          <Icon name="x" size={12} />
        </button>
      </div>
      <QueuedAttachments attachments={messageAttachments(queued)} onRemove={(a) => removeQueuedAttachment(slotId, a)} />
      <div className={s.queuedText}>{queued.typed}</div>
    </div>
  );
}

/**
 * AI-05: the models of every provider that chats (not System One ones, AI-47), grouped by provider, and
 * under the current model the thinking levels it offers. The selector shows the level the next message is
 * sent at.
 */
function ModelPicker({ slotId, slot, value, openRef }: { slotId: string; slot: Slot; value: AiModelRef | null; openRef: MutableRefObject<(() => void) | null> }) {
  const t = useT();
  const providers = useAi((st) => st.catalog.providers);
  const settings = useAi((st) => st.catalog.settings);
  const menu = useMenu();
  const ref = useRef<HTMLButtonElement>(null);
  const fullRef = useRef<HTMLSpanElement>(null);
  const nameRef = useRef<HTMLSpanElement>(null);
  const current = findModel(providers, value);
  const offered = modelEfforts(current?.model);
  const effort = current ? effectiveEffort(chosenEffort(slot.effort, slot.conversationId, slot.conversation, settings), offered) : null;

  const entries = useMemo(() => {
    const list: MenuEntry[] = [];
    for (const p of providers) {
      if (!chats(p) || p.models.length === 0) continue;
      if (list.length > 0) list.push({ kind: "separator" });
      list.push({ kind: "header", label: p.name });
      for (const m of p.models) {
        const ref: AiModelRef = { provider_id: p.id, model_id: m.id };
        const checked = sameModel(ref, value);
        list.push({
          label: m.name || m.id,
          hint: m.context_window ? formatTokens(m.context_window) : undefined,
          checked,
          onSelect: () => setSlotModel(slotId, ref),
        });
        if (checked)
          list.push({
            kind: "choice",
            label: t("ai.effort"),
            options: [null, ...offered].map((e) => ({ value: e ?? "default", label: t(effortKey(e)) })),
            value: effort ?? "default",
            onChange: (v) => setSlotEffort(slotId, v as AiEffort | "default"),
          });
      }
    }
    return list;
  }, [providers, value, slotId, offered, effort, t]);

  // `/model` opens the selector (AI-50).
  useEffect(() => {
    openRef.current = () => {
      if (ref.current && entries.length > 0) menu.openBelow(ref.current, true);
    };
  }, [openRef, entries, menu]);

  const modelName = current ? current.model.name || current.model.id : null;
  const name = modelName ?? value?.model_id ?? t("ai.model.none");
  const level = current ? t(effortKey(effort)) : null;
  const title = current ? [current.provider.name, modelName, `${t("ai.effort")}: ${level}`].join(" · ") : t("ai.model");
  const fit = useModelFit(ref, fullRef, nameRef, name, level);
  return (
    <>
      <button
        ref={ref}
        type="button"
        className={cx(s.model, fit === "icon" && s.modelIconOnly)}
        aria-haspopup="menu"
        aria-expanded={!!menu.anchor}
        aria-label={current ? `${t("ai.model")}: ${title}` : t("ai.model")}
        title={title}
        disabled={entries.length === 0}
        onClick={() => ref.current && menu.openBelow(ref.current, true)}
      >
        <Icon name="cube" size={13} className={s.modelIcon} />
        {fit !== "icon" && <span className={s.modelName}>{name}</span>}
        {fit === "full" && level && <span className={s.modelEffort}>{level}</span>}
        {fit !== "icon" && <Icon name="caret-down" size={10} className={s.modelIcon} />}
        {/* Hidden copies at their full width, which tell how much of the button fits. */}
        <ModelLabel ref={fullRef} name={name} level={level} />
        <ModelLabel ref={nameRef} name={name} level={null} />
      </button>
      {menu.anchor && <Menu anchor={menu.anchor} entries={entries} onClose={menu.close} minWidth={220} />}
    </>
  );
}

/**
 * AI-16: the conversation's permission mode, beside the + menu. Manual approval shows only its
 * shield; Auto (AI-47) and bypass also their name, which a narrow panel leaves out. The first switch to
 * bypass on a device asks first; Auto needs no confirmation, but a judge chosen in Settings → AI.
 */
function ModeButton({ slotId, mode }: { slotId: string; mode: AiPermissionMode }) {
  const t = useT();
  const menu = useMenu();
  const ref = useRef<HTMLButtonElement>(null);
  const judge = useAi((st) => judgeChosen(st.catalog.providers, st.catalog.settings));

  const choose = async (next: AiPermissionMode) => {
    if (next === mode) return;
    if (next === "auto" && !judge) return;
    if (next === "bypass" && !useApp.getState().prefs.ai_bypass_confirmed) {
      const ok = await confirm({
        title: t("ai.bypass.title"),
        body: t("ai.bypass.body"),
        confirmLabel: t("ai.bypass.confirm"),
        danger: true,
        icon: "lightning",
        iconColor: "var(--orange)",
      });
      if (!ok) return;
      const app = useApp.getState();
      void app.setPrefs({ ...app.prefs, ai_bypass_confirmed: true }).catch(() => {});
    }
    setSlotMode(slotId, next);
  };

  const label = t(MODE_LABEL[mode]);
  const short = mode === "manual" ? null : t(mode === "auto" ? "ai.mode.autoShort" : "ai.mode.bypassShort");
  const entries: MenuEntry[] = [
    { kind: "header", label: t("ai.mode.menuTitle") },
    ...(["manual", "auto", "bypass"] as const).map(
      (m): MenuEntry => ({
        label: t(MODE_LABEL[m]),
        icon: MODE_ICON[m],
        checked: mode === m,
        // AI-47: Auto needs a judge; the conversation's own Auto stays checked once the judge is gone.
        disabled: m === "auto" && !judge && mode !== "auto",
        hint: m === "auto" && !judge ? t("ai.mode.autoNeedsJudge") : undefined,
        onSelect: () => void choose(m),
      }),
    ),
  ];
  if (!judge)
    entries.push(
      { kind: "separator" },
      { label: t("ai.mode.chooseJudge"), icon: "gear-six", onSelect: () => useApp.getState().openSettings(true, "ai") },
    );
  return (
    <>
      <button
        ref={ref}
        type="button"
        className={cx(s.mode, mode === "auto" && s.modeAuto, mode === "bypass" && s.modeBypass)}
        aria-haspopup="menu"
        aria-expanded={!!menu.anchor}
        aria-label={`${t("ai.mode")}: ${label}`}
        title={`${t("ai.mode")}: ${label}`}
        onClick={() => ref.current && menu.openBelow(ref.current)}
      >
        <Icon name={MODE_ICON[mode]} size={14} />
        {short && <span>{short}</span>}
      </button>
      {menu.anchor && <Menu anchor={menu.anchor} onClose={menu.close} minWidth={230} entries={entries} />}
    </>
  );
}

const MODE_LABEL: Record<AiPermissionMode, MessageKey> = { manual: "ai.mode.manual", auto: "ai.mode.auto", bypass: "ai.mode.bypass" };

/** What the model button shows: everything, the name without the thinking level, or its icon alone. */
type ModelFit = "full" | "name" | "icon";

/** The spacer's `min-width` in the bar (`.grow`). */
const GROW_MIN = 4;

/**
 * Rather than cut the model's name short, the button drops the thinking level when the bar has no
 * room for it, then the name, leaving its icon; the tooltip names both. The room is the button's
 * own width plus what the spacer beside it takes beyond its minimum.
 */
function useModelFit(
  button: RefObject<HTMLButtonElement | null>,
  full: RefObject<HTMLSpanElement | null>,
  nameOnly: RefObject<HTMLSpanElement | null>,
  name: string,
  level: string | null,
): ModelFit {
  const [fit, setFit] = useState<ModelFit>("full");
  useLayoutEffect(() => {
    const btn = button.current;
    const grow = btn?.parentElement?.querySelector<HTMLElement>("[data-grow]");
    if (!btn || !grow) return;
    const update = () => {
      const room = btn.offsetWidth + grow.offsetWidth - GROW_MIN + 0.5;
      setFit((full.current?.offsetWidth ?? 0) <= room ? "full" : (nameOnly.current?.offsetWidth ?? 0) <= room ? "name" : "icon");
    };
    update();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(update);
    observer.observe(btn);
    observer.observe(grow);
    return () => observer.disconnect();
  }, [button, full, nameOnly, name, level]);
  return fit;
}

/** The model button's content at its full width, hidden, for `useModelFit` to measure. */
function ModelLabel({ ref, name, level }: { ref: RefObject<HTMLSpanElement | null>; name: string; level: string | null }) {
  return (
    <span ref={ref} className={cx(s.model, s.modelMeasure)} aria-hidden>
      <Icon name="cube" size={13} className={s.modelIcon} />
      <span className={s.modelName}>{name}</span>
      {level && <span className={s.modelEffort}>{level}</span>}
      <Icon name="caret-down" size={10} className={s.modelIcon} />
    </span>
  );
}

/**
 * AI-20: a ring whose color tells how full the context is: orange from 80%, where Compact is
 * offered, red from 90%, where the next message compacts first. The share and the numbers are on
 * hover and in its menu. Without a context window it shows the token count instead.
 */
function ContextMeter({ slotId, slot, m }: { slotId: string; slot: Slot; m: MeterState }) {
  const t = useT();
  const menu = useMenu();
  const ref = useRef<HTMLButtonElement>(null);
  const approx = m.estimated ? "≈" : "";
  const percent = m.ratio !== null ? Math.min(999, Math.round(m.ratio * 100)) : null;
  const share = percent === null ? null : percent === 0 && m.tokens > 0 ? `${approx}<1%` : `${approx}${percent}%`;
  const full = m.ratio !== null && m.ratio >= AUTO_COMPACT_RATIO;
  const nf = new Intl.NumberFormat(t.locale);
  const detail =
    m.window !== null
      ? t("ai.meter.detail", { used: nf.format(m.tokens), total: nf.format(m.window) })
      : t("ai.meter.detailUnknown", { used: nf.format(m.tokens) });
  const summary = share ? `${share} · ${detail}` : detail;
  const title = m.estimated ? `${summary}\n${t("ai.meter.estimated")}` : summary;
  const canCompact = !!slot.conversationId && !slot.turn && !slot.compacting && slot.entries.length > 0;

  return (
    <>
      {m.warn && canCompact && (
        <LinkButton className={s.compact} onClick={() => void compactConversation(slotId)}>
          {t("ai.compact")}
        </LinkButton>
      )}
      <button
        ref={ref}
        type="button"
        className={cx(s.meter, m.warn && s.meterWarn, full && s.meterFull)}
        title={title}
        aria-haspopup="menu"
        aria-expanded={!!menu.anchor}
        aria-label={`${t("ai.meter")}: ${summary}`}
        onClick={() => ref.current && menu.openBelow(ref.current, true)}
      >
        {m.ratio !== null ? <Ring ratio={m.ratio} /> : <span>{`${approx}${formatTokens(m.tokens)}`}</span>}
      </button>
      {menu.anchor && (
        <Menu
          anchor={menu.anchor}
          onClose={menu.close}
          minWidth={220}
          entries={[
            { kind: "header", label: summary },
            ...(m.estimated ? [{ kind: "header" as const, label: t("ai.meter.estimated") }] : []),
            { kind: "separator" },
            { label: t("ai.compact.menu"), icon: "arrows-in-line-vertical", disabled: !canCompact, onSelect: () => void compactConversation(slotId) },
          ]}
        />
      )}
    </>
  );
}

function Ring({ ratio }: { ratio: number }) {
  const r = 6;
  const c = 2 * Math.PI * r;
  const shown = Math.min(1, Math.max(0, ratio));
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden className={s.ring}>
      <circle cx="8" cy="8" r={r} className={s.ringTrack} />
      <circle cx="8" cy="8" r={r} className={s.ringValue} strokeDasharray={`${shown * c} ${c}`} transform="rotate(-90 8 8)" />
    </svg>
  );
}
