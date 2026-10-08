import { useEffect, useLayoutEffect, useMemo, useRef, type ClipboardEvent, type KeyboardEvent } from "react";
import { Icon, LinkButton } from "@/components/controls";
import { Menu, toast, useMenu, type MenuEntry } from "@/components/overlay";
import { readClipboard } from "@/features/terminal/clipboard";
import { useT } from "@/i18n";
import type { AiModelRef } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { addPaste, attachFiles, compactConversation, removeExtra, sendMessage, setSlotModel, stopTurn } from "./actions";
import { ComposerAttachments, usePendingAttachments, useTerminalSelection } from "./AttachmentChips";
import { composeMessage, isLongPaste } from "./attachments";
import { contextUsage, estimateTokens, formatTokens, messageFit, type MeterState } from "./meter";
import { findModel, sameModel } from "./models";
import { getSlot, patchSlot, useAi, type Slot } from "./store";
import { ToolsButton } from "./ToolsMenu";
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

/**
 * The input area (§9): the attachment chips, a multi-line box, the model selector, the tools menu,
 * the paperclip, the context meter and Send / Stop.
 */
export function Composer({ slotId, slot, model, tools }: { slotId: string; slot: Slot; model: AiModelRef | null; tools: boolean }) {
  const t = useT();
  const ref = useRef<HTMLTextAreaElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const focusTick = useAi((st) => st.focusTick);
  const providers = useAi((st) => st.catalog.providers);
  const busy = !!slot.turn || slot.remoteRunning;
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
  const fit = useMemo(() => messageFit(meter, pending.length > 0 ? estimateTokens(composeMessage(slot.draft.trim(), pending)) : estimateTokens(slot.draft)), [meter, pending, slot.draft]);
  const blocked = fit.level === "over";

  const send = () => {
    if (!slot.draft.trim() || !model || blocked) return;
    void sendMessage(slotId, slot.draft);
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
    // Enter sends, Shift+Enter adds a line, and nothing is sent while an IME is composing.
    if (e.key !== "Enter" || e.shiftKey || e.altKey || e.ctrlKey || e.metaKey) return;
    if (e.nativeEvent.isComposing || e.keyCode === 229) return;
    e.preventDefault();
    send();
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
          placeholder={tools ? t("ai.input.placeholder") : t("ai.input.placeholderChat")}
          aria-label={t("ai.input.label")}
          spellCheck={false}
          onChange={(e) => patchSlot(slotId, { draft: e.target.value })}
          onKeyDown={onKeyDown}
          onPaste={onPaste}
        />
        <div className={s.bar}>
          <ModelPicker slotId={slotId} value={model} />
          <ToolsButton slotId={slotId} off={slot.mcpOff} />
          <button
            type="button"
            className={s.attach}
            title={t("ai.attach")}
            aria-label={t("ai.attach")}
            onClick={() => fileRef.current?.click()}
          >
            <Icon name="paperclip" size={14} />
          </button>
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
          <div className={s.grow} />
          <ContextMeter slotId={slotId} slot={slot} m={meter} />
          {busy ? (
            <button type="button" className={cx(s.send, s.stop)} title={t("ai.stopHint")} aria-label={t("ai.stop")} onClick={() => void stopTurn(slotId)}>
              <Icon name="stop" fill size={12} />
            </button>
          ) : (
            <button
              type="button"
              className={s.send}
              title={blocked ? t("ai.fit.blocked") : t("ai.send")}
              aria-label={t("ai.send")}
              disabled={!slot.draft.trim() || !model || blocked}
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


/** AI-05: every provider's models, grouped by provider. */
function ModelPicker({ slotId, value }: { slotId: string; value: AiModelRef | null }) {
  const t = useT();
  const providers = useAi((st) => st.catalog.providers);
  const menu = useMenu();
  const ref = useRef<HTMLButtonElement>(null);
  const current = findModel(providers, value);

  const entries = useMemo(() => {
    const list: MenuEntry[] = [];
    for (const p of providers) {
      if (p.models.length === 0) continue;
      if (list.length > 0) list.push({ kind: "separator" });
      list.push({ kind: "header", label: p.name });
      for (const m of p.models) {
        const ref: AiModelRef = { provider_id: p.id, model_id: m.id };
        list.push({
          label: m.name || m.id,
          hint: m.context_window ? formatTokens(m.context_window) : undefined,
          checked: sameModel(ref, value),
          onSelect: () => setSlotModel(slotId, ref),
        });
      }
    }
    return list;
  }, [providers, value, slotId]);

  return (
    <>
      <button
        ref={ref}
        type="button"
        className={s.model}
        aria-haspopup="menu"
        aria-label={t("ai.model")}
        title={current ? `${current.provider.name} · ${current.model.name || current.model.id}` : t("ai.model")}
        disabled={entries.length === 0}
        onClick={() => ref.current && menu.openBelow(ref.current)}
      >
        <Icon name="cube" size={13} className={s.modelIcon} />
        <span className={s.modelName}>{current ? current.model.name || current.model.id : (value?.model_id ?? t("ai.model.none"))}</span>
        <Icon name="caret-up-down" size={11} className={s.modelIcon} />
      </button>
      {menu.anchor && <Menu anchor={menu.anchor} entries={entries} onClose={menu.close} minWidth={220} />}
    </>
  );
}

/** AI-20: a ring and a percentage (or only a count without a context window), numbers on hover. */
function ContextMeter({ slotId, slot, m }: { slotId: string; slot: Slot; m: MeterState }) {
  const t = useT();
  const menu = useMenu();
  const ref = useRef<HTMLButtonElement>(null);
  const approx = m.estimated ? "≈" : "";
  const percent = m.ratio !== null ? Math.min(999, Math.round(m.ratio * 100)) : null;
  const label =
    percent === null ? `${approx}${formatTokens(m.tokens)}` : percent === 0 && m.tokens > 0 ? `${approx}<1%` : `${approx}${percent}%`;
  const nf = new Intl.NumberFormat(t.locale);
  const detail =
    m.window !== null
      ? t("ai.meter.detail", { used: nf.format(m.tokens), total: nf.format(m.window) })
      : t("ai.meter.detailUnknown", { used: nf.format(m.tokens) });
  const title = m.estimated ? `${detail}\n${t("ai.meter.estimated")}` : detail;
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
        className={cx(s.meter, m.warn && s.meterWarn)}
        title={title}
        aria-label={`${t("ai.meter")}: ${detail}`}
        onClick={() => ref.current && menu.openBelow(ref.current, true)}
      >
        {m.ratio !== null && <Ring ratio={m.ratio} />}
        <span>{label}</span>
      </button>
      {menu.anchor && (
        <Menu
          anchor={menu.anchor}
          onClose={menu.close}
          minWidth={220}
          entries={[
            { kind: "header", label: detail },
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
