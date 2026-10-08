import { useEffect, useMemo, useRef, useState } from "react";
import { useVaultData } from "@/app/data";
import { useTabs } from "@/app/tabs";
import { Icon } from "@/components/controls";
import { getSession } from "@/features/terminal/session";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import { hideSelection, noteSelection, removeDiagnostics } from "./actions";
import { chipShown, makeAttachment, type DiagnosticsAttachment, type SelectionAttachment } from "./selection";
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

/**
 * The input area's chips for what goes with the tab's next message: connection diagnostics attached
 * from the terminal's error card, then the terminal selection (AI-10).
 */
export function ComposerAttachments({ slotId }: { slotId: string }) {
  const sel = useAi((st) => st.selections[slotId]) ?? NO_SELECTION;
  const diagnostics = useAi((st) => (slotId === HOME_SLOT ? undefined : st.diagnostics[slotId]));
  const host = useTabHostName(slotId);
  const attachment = useMemo(() => (slotId !== HOME_SLOT && chipShown(sel, sel.hidden) ? makeAttachment(host, sel.text) : null), [slotId, sel, host]);
  if (!attachment && !diagnostics) return null;
  return (
    <div className={s.attachments}>
      {diagnostics && <DiagnosticsChip attachment={diagnostics} onRemove={() => removeDiagnostics(slotId)} />}
      {attachment && <SelectionChip attachment={attachment} onRemove={() => hideSelection(slotId)} />}
    </div>
  );
}

/** An attachment as a chip: a label, a preview on hover or click, and × to remove it. */
function AttachmentChip({
  icon,
  label,
  badge,
  previewTitle,
  removeLabel,
  head,
  body,
  more,
  wrap,
  onRemove,
}: {
  icon: string;
  label: string;
  badge?: string;
  previewTitle: string;
  removeLabel: string;
  head: string;
  body: string;
  more?: string;
  /** Wrap long lines in the preview instead of cutting them. */
  wrap?: boolean;
  onRemove: () => void;
}) {
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

  return (
    <div ref={ref} className={s.chipWrap} onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)}>
      <div className={s.chip}>
        <button
          type="button"
          className={s.chipLabel}
          aria-expanded={pinned}
          title={previewTitle}
          onClick={() => setPinned((v) => !v)}
          onKeyDown={(e) => {
            if (e.key === "Escape" && pinned) {
              e.preventDefault();
              e.stopPropagation();
              setPinned(false);
            }
          }}
        >
          <Icon name={icon} size={13} className={s.chipIcon} />
          <span className={s.chipText}>{label}</span>
          {badge && <span className={s.truncated}>{badge}</span>}
        </button>
        <button type="button" className={s.chipRemove} title={removeLabel} aria-label={removeLabel} onClick={onRemove}>
          <Icon name="x" size={11} />
        </button>
      </div>
      {(hover || pinned) && (
        <div className={s.preview} role="tooltip">
          {head && <div className={s.previewHead}>{head}</div>}
          <pre className={cx(s.previewBody, wrap && s.previewWrap)}>{body}</pre>
          {more && <div className={s.previewMore}>{more}</div>}
        </div>
      )}
    </div>
  );
}

/** A selection as a chip: its size, its first lines on hover or click, and × to remove it. */
export function SelectionChip({ attachment, onRemove }: { attachment: SelectionAttachment; onRemove: () => void }) {
  const t = useT();
  const lines = attachment.text.split("\n");
  const more = lines.length - PREVIEW_LINES;
  return (
    <AttachmentChip
      icon="selection"
      label={t("ai.selection", { n: attachment.lines })}
      badge={attachment.truncated ? t("ai.selection.truncated") : undefined}
      previewTitle={t("ai.selection.preview")}
      removeLabel={t("ai.selection.remove")}
      head={attachment.host ? t("ai.selection.from", { host: attachment.host }) : ""}
      body={lines.slice(0, PREVIEW_LINES).join("\n")}
      more={more > 0 ? t("ai.selection.more", { n: more }) : undefined}
      onRemove={onRemove}
    />
  );
}

/** Connection diagnostics as a chip; the preview shows all of what will be sent. */
export function DiagnosticsChip({ attachment, onRemove }: { attachment: DiagnosticsAttachment; onRemove: () => void }) {
  const t = useT();
  return (
    <AttachmentChip
      icon="plugs"
      label={t("ai.diagnostics", { host: attachment.host })}
      previewTitle={t("ai.diagnostics.preview")}
      removeLabel={t("ai.diagnostics.remove")}
      head={t("ai.diagnostics.sent")}
      body={attachment.text}
      wrap
      onRemove={onRemove}
    />
  );
}

/** A sent attachment above the user's message: collapsed to its label, the text when opened. */
function AttachmentCard({ icon, title, host, badge, body }: { icon: string; title: string; host?: string; badge?: string; body: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className={cx(s.card, open && s.cardOpen)} data-selection-open={open}>
      <button type="button" className={s.cardHead} aria-expanded={open} onClick={() => setOpen((v) => !v)}>
        <Icon name={icon} size={13} className={s.chipIcon} />
        <span className={s.cardTitle}>{title}</span>
        {host && <span className={s.cardHost}>{host}</span>}
        {badge && <span className={s.truncated}>{badge}</span>}
        <Icon name={open ? "caret-down" : "caret-right"} size={11} className={s.caret} />
      </button>
      {open && <pre className={cx(s.cardBody, "selectable")}>{body}</pre>}
    </div>
  );
}

export function SelectionCard({ attachment }: { attachment: SelectionAttachment }) {
  const t = useT();
  return (
    <AttachmentCard
      icon="selection"
      title={t("ai.selection", { n: attachment.lines })}
      host={attachment.host}
      badge={attachment.truncated ? t("ai.selection.truncated") : undefined}
      body={attachment.text}
    />
  );
}

export function DiagnosticsCard({ attachment }: { attachment: DiagnosticsAttachment }) {
  const t = useT();
  return <AttachmentCard icon="plugs" title={t("ai.diagnostics", { host: attachment.host })} body={attachment.text} />;
}
