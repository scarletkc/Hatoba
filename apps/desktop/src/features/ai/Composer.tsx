import { useEffect, useLayoutEffect, useMemo, useRef, type KeyboardEvent } from "react";
import { Icon, LinkButton } from "@/components/controls";
import { Menu, useMenu, type MenuEntry } from "@/components/overlay";
import { useT } from "@/i18n";
import type { AiModelRef } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { compactConversation, sendMessage, setSlotModel, stopTurn } from "./actions";
import { contextUsage, formatTokens, type MeterState } from "./meter";
import { findModel, sameModel } from "./models";
import { ComposerAttachments, useTerminalSelection } from "./SelectionChip";
import { patchSlot, useAi, type Slot } from "./store";
import { ToolsButton } from "./ToolsMenu";
import s from "./Composer.module.css";

/** The input area (§9): the terminal selection chip, a multi-line box, the model selector, the tools menu, the context meter and Send / Stop. */
export function Composer({ slotId, slot, model, tools }: { slotId: string; slot: Slot; model: AiModelRef | null; tools: boolean }) {
  const t = useT();
  const ref = useRef<HTMLTextAreaElement>(null);
  const focusTick = useAi((st) => st.focusTick);
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

  const send = () => {
    if (!slot.draft.trim() || !model) return;
    void sendMessage(slotId, slot.draft);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    // Enter sends, Shift+Enter adds a line, and nothing is sent while an IME is composing.
    if (e.key !== "Enter" || e.shiftKey || e.altKey || e.ctrlKey || e.metaKey) return;
    if (e.nativeEvent.isComposing || e.keyCode === 229) return;
    e.preventDefault();
    send();
  };

  return (
    <div className={s.composer}>
      <div className={s.box}>
        <ComposerAttachments slotId={slotId} />
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
        />
        <div className={s.bar}>
          <ModelPicker slotId={slotId} value={model} />
          <ToolsButton slotId={slotId} off={slot.mcpOff} />
          <div className={s.grow} />
          <ContextMeter slotId={slotId} slot={slot} model={model} />
          {busy ? (
            <button type="button" className={cx(s.send, s.stop)} title={t("ai.stopHint")} aria-label={t("ai.stop")} onClick={() => void stopTurn(slotId)}>
              <Icon name="stop" fill size={12} />
            </button>
          ) : (
            <button type="button" className={s.send} title={t("ai.send")} aria-label={t("ai.send")} disabled={!slot.draft.trim() || !model} onClick={send}>
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
function ContextMeter({ slotId, slot, model }: { slotId: string; slot: Slot; model: AiModelRef | null }) {
  const t = useT();
  const providers = useAi((st) => st.catalog.providers);
  const menu = useMenu();
  const ref = useRef<HTMLButtonElement>(null);
  const ctxWindow = findModel(providers, model)?.model.context_window ?? null;
  const pending = slot.turn?.live ? slot.turn.live.text + slot.turn.live.reasoning : "";
  const m: MeterState = useMemo(
    () => contextUsage({ entries: slot.entries, contextStart: slot.conversation?.context_start ?? null, model, contextWindow: ctxWindow, pending }),
    [slot.entries, slot.conversation?.context_start, model, ctxWindow, pending],
  );
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
