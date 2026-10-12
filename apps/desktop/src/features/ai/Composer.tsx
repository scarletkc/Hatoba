import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ClipboardEvent, type KeyboardEvent, type RefObject } from "react";
import { useApp } from "@/app/store";
import { Icon, LinkButton } from "@/components/controls";
import { confirm, Menu, toast, useMenu, type MenuEntry } from "@/components/overlay";
import { readClipboard } from "@/features/terminal/clipboard";
import { useT } from "@/i18n";
import type { AiEffort, AiModelRef, AiPermissionMode } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { isImeEvent } from "@/lib/ime";
import { shortcutLabel } from "@/lib/platform";
import { AddButton } from "./AddMenu";
import {
  addPaste,
  askSideQuestion,
  attachFiles,
  compactConversation,
  discardQueued,
  editQueued,
  queueMessage,
  removeExtra,
  removeQueuedAttachment,
  sendMessage,
  setSlotEffort,
  setSlotMode,
  setSlotModel,
  stopTurn,
} from "./actions";
import { ComposerAttachments, QueuedAttachments, usePendingAttachments, useTerminalSelection } from "./AttachmentChips";
import { composeMessage, isLongPaste } from "./attachments";
import { chosenEffort, effectiveEffort, effortKey, modelEfforts } from "./effort";
import { AUTO_COMPACT_RATIO, contextUsage, estimateTokens, formatTokens, messageFit, type MeterState } from "./meter";
import { findModel, sameModel } from "./models";
import { messageAttachments, messageText, takenBeside } from "./queue";
import { canAskSide } from "./side";
import { getSlot, patchSlot, useAi, type QueuedMessage, type Slot } from "./store";
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

/**
 * The input area (§9): the queued message (§13.1), the attachment chips, a multi-line box, and under
 * it the + menu and the permission mode (AI-16) on the left, the context meter, the model selector,
 * Ask a Side Question (AI-42) and Send / Stop on the right.
 */
export function Composer({ slotId, slot, model, tools }: { slotId: string; slot: Slot; model: AiModelRef | null; tools: boolean }) {
  const t = useT();
  const ref = useRef<HTMLTextAreaElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);
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
  const send = () => {
    if (canSend) void sendMessage(slotId, slot.draft);
  };
  // Enter while a turn runs: the message waits for the turn to end.
  const queue = () => {
    if (slot.draft.trim() && model && !blocked) queueMessage(slotId, slot.draft);
  };

  // AI-42: the turn, if any, goes on; the answer shows at the end of the conversation. While a turn
  // runs, Rust asks with the turn's model, not the one picked here that the fit is measured
  // against, so the provider's own limit decides.
  const canSide = !!model && (busy || !blocked) && canAskSide(slot, slot.draft);
  const askSide = () => {
    if (canSide) askSideQuestion(slotId, slot.draft);
  };

  /** Puts text into the box at the caret, as typing would. */
  const insertText = (text: string) => {
    const el = ref.current;
    const next = insertAtCaret(el, getSlot(slotId).draft, text);
    patchSlot(slotId, { draft: next.value });
    requestAnimationFrame(() => {
      el?.focus();
      el?.setSelectionRange(next.caret, next.caret);
    });
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
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
      askSide();
      return;
    }
    if (isSendNowKey(e)) {
      if (isImeEvent(e)) return;
      e.preventDefault();
      send();
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
    if (busy) queue();
    else send();
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
      <div className={s.box}>
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
          onChange={(e) => patchSlot(slotId, { draft: e.target.value })}
          onKeyDown={onKeyDown}
          onPaste={onPaste}
        />
        <div className={s.bar}>
          <AddButton slotId={slotId} slot={slot} onAddFiles={() => fileRef.current?.click()} />
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
          <ModelPicker slotId={slotId} slot={slot} value={model} />
          {slot.conversationId && (
            <button
              type="button"
              className={s.side}
              title={t("ai.side.askHint", { key: sideKey })}
              aria-label={t("ai.side.ask")}
              disabled={!canSide}
              onClick={askSide}
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
              disabled={!canSend}
              onClick={send}
            >
              <Icon name="arrow-up" size={14} />
            </button>
          )}
        </div>
      </div>
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
 * AI-05: every provider's models, grouped by provider, and under the current model the thinking
 * levels it offers. The selector shows the level the next message is sent at.
 */
function ModelPicker({ slotId, slot, value }: { slotId: string; slot: Slot; value: AiModelRef | null }) {
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
      if (p.models.length === 0) continue;
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
 * shield; bypass also its name. The first switch to bypass on a device asks first.
 */
function ModeButton({ slotId, mode }: { slotId: string; mode: AiPermissionMode }) {
  const t = useT();
  const menu = useMenu();
  const ref = useRef<HTMLButtonElement>(null);

  const choose = async (next: AiPermissionMode) => {
    if (next === mode) return;
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

  const label = mode === "bypass" ? t("ai.mode.bypass") : t("ai.mode.manual");
  return (
    <>
      <button
        ref={ref}
        type="button"
        className={cx(s.mode, mode === "bypass" && s.modeBypass)}
        aria-haspopup="menu"
        aria-expanded={!!menu.anchor}
        aria-label={`${t("ai.mode")}: ${label}`}
        title={`${t("ai.mode")}: ${label}`}
        onClick={() => ref.current && menu.openBelow(ref.current)}
      >
        <Icon name={mode === "bypass" ? "lightning" : "shield-check"} size={14} />
        {mode === "bypass" && <span>{t("ai.mode.bypassShort")}</span>}
      </button>
      {menu.anchor && (
        <Menu
          anchor={menu.anchor}
          onClose={menu.close}
          minWidth={230}
          entries={[
            { kind: "header", label: t("ai.mode.menuTitle") },
            { label: t("ai.mode.manual"), icon: "shield-check", checked: mode === "manual", onSelect: () => void choose("manual") },
            { label: t("ai.mode.bypass"), icon: "lightning", checked: mode === "bypass", onSelect: () => void choose("bypass") },
          ]}
        />
      )}
    </>
  );
}

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
