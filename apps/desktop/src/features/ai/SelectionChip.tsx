import { useEffect, useMemo, useRef, useState } from "react";
import { useVaultData } from "@/app/data";
import { useTabs } from "@/app/tabs";
import { Icon } from "@/components/controls";
import { getSession } from "@/features/terminal/session";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import { hideSelection, noteSelection } from "./actions";
import { chipShown, makeAttachment, type SelectionAttachment } from "./selection";
import { HOME_SLOT, NO_SELECTION, useAi } from "./store";
import s from "./SelectionChip.module.css";

/** xterm reports every step of a drag; the chip follows once it settles. */
const SETTLE_MS = 120;
/** Lines the chip's preview shows. */
const PREVIEW_LINES = 12;

/** AI-10: follows the selection of the tab's terminal while the panel shows its conversation. */
export function useTerminalSelection(slotId: string) {
  // A tab's session can appear after the panel; its state changing is a cue to look again.
  const status = useTabs((st) => st.tabs.find((x) => x.id === slotId)?.status);
  useEffect(() => {
    if (slotId === HOME_SLOT) return;
    const term = getSession(slotId)?.term;
    if (!term) return;
    const text = () => (term.hasSelection() ? term.getSelection() : "");
    noteSelection(slotId, text());
    let timer = 0;
    // A new drag is a new selection even when it selects the same text, which xterm does not report.
    let renewed = false;
    let pressed = false;
    const settle = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        noteSelection(slotId, text(), renewed);
        renewed = false;
      }, SETTLE_MS);
    };
    const sub = term.onSelectionChange(() => {
      if (!term.hasSelection()) renewed = true;
      settle();
    });
    const onDown = (e: MouseEvent) => {
      if (e.button !== 0) return;
      pressed = true;
      renewed = true;
    };
    const onUp = () => {
      if (!pressed) return;
      pressed = false;
      settle();
    };
    const el = term.element;
    el?.addEventListener("mousedown", onDown, true);
    window.addEventListener("mouseup", onUp, true);
    return () => {
      window.clearTimeout(timer);
      sub.dispose();
      el?.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("mouseup", onUp, true);
    };
  }, [slotId, status]);
}

/** The tab host's display name, as the selection block carries it. */
function useTabHostName(slotId: string): string {
  const tab = useTabs((st) => st.tabs.find((x) => x.id === slotId));
  const name = useVaultData((st) => st.hosts.find((h) => h.id === tab?.hostId)?.name);
  return name ?? tab?.title ?? "";
}

/** The input area's chip for the tab's selection, which goes with the next message. */
export function ComposerSelection({ slotId }: { slotId: string }) {
  const sel = useAi((st) => st.selections[slotId]) ?? NO_SELECTION;
  const host = useTabHostName(slotId);
  const attachment = useMemo(() => (slotId !== HOME_SLOT && chipShown(sel, sel.hidden) ? makeAttachment(host, sel.text) : null), [slotId, sel, host]);
  if (!attachment) return null;
  return (
    <div className={s.attachments}>
      <SelectionChip attachment={attachment} onRemove={() => hideSelection(slotId)} />
    </div>
  );
}

/** A selection as a chip: its size, a preview on hover or click, and × to remove it. */
export function SelectionChip({ attachment, onRemove }: { attachment: SelectionAttachment; onRemove: () => void }) {
  const t = useT();
  const ref = useRef<HTMLDivElement>(null);
  const [hover, setHover] = useState(false);
  const [pinned, setPinned] = useState(false);

  useEffect(() => {
    if (!pinned) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setPinned(false);
    };
    window.addEventListener("mousedown", onDown, true);
    return () => window.removeEventListener("mousedown", onDown, true);
  }, [pinned]);

  const lines = attachment.text.split("\n");
  const more = lines.length - PREVIEW_LINES;
  return (
    <div ref={ref} className={s.chipWrap} onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)}>
      <div className={s.chip}>
        <button
          type="button"
          className={s.chipLabel}
          aria-expanded={pinned}
          title={t("ai.selection.preview")}
          onClick={() => setPinned((v) => !v)}
          onKeyDown={(e) => {
            if (e.key === "Escape" && pinned) {
              e.preventDefault();
              e.stopPropagation();
              setPinned(false);
            }
          }}
        >
          <Icon name="selection" size={13} className={s.chipIcon} />
          <span>{t("ai.selection", { n: attachment.lines })}</span>
          {attachment.truncated && <span className={s.truncated}>{t("ai.selection.truncated")}</span>}
        </button>
        <button type="button" className={s.chipRemove} title={t("ai.selection.remove")} aria-label={t("ai.selection.remove")} onClick={onRemove}>
          <Icon name="x" size={11} />
        </button>
      </div>
      {(hover || pinned) && (
        <div className={s.preview} role="tooltip">
          {attachment.host && <div className={s.previewHead}>{t("ai.selection.from", { host: attachment.host })}</div>}
          <pre className={s.previewBody}>{lines.slice(0, PREVIEW_LINES).join("\n")}</pre>
          {more > 0 && <div className={s.previewMore}>{t("ai.selection.more", { n: more })}</div>}
        </div>
      )}
    </div>
  );
}

/** A sent selection above the user's message: collapsed to its size, the text when opened. */
export function SelectionCard({ attachment }: { attachment: SelectionAttachment }) {
  const t = useT();
  const [open, setOpen] = useState(false);
  return (
    <div className={cx(s.card, open && s.cardOpen)} data-selection-open={open}>
      <button type="button" className={s.cardHead} aria-expanded={open} onClick={() => setOpen((v) => !v)}>
        <Icon name="selection" size={13} className={s.chipIcon} />
        <span className={s.cardTitle}>{t("ai.selection", { n: attachment.lines })}</span>
        {attachment.host && <span className={s.cardHost}>{attachment.host}</span>}
        {attachment.truncated && <span className={s.truncated}>{t("ai.selection.truncated")}</span>}
        <Icon name={open ? "caret-down" : "caret-right"} size={11} className={s.caret} />
      </button>
      {open && <pre className={cx(s.cardBody, "selectable")}>{attachment.text}</pre>}
    </div>
  );
}
