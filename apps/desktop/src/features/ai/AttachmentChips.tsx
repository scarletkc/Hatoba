import { useEffect, useMemo, useRef, useState } from "react";
import { useVaultData } from "@/app/data";
import { useApp } from "@/app/store";
import { useTabs } from "@/app/tabs";
import { Icon } from "@/components/controls";
import { getSession } from "@/features/terminal/session";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import { shortcutLabel } from "@/lib/platform";
import { hideSelection, noteSelection, removeDiagnostics, removeExtra } from "./actions";
import { attachmentTokens, chipShown, makeAttachment, type Attachment, type SelectionAttachment } from "./attachments";
import { formatTokens } from "./meter";
import { HOME_SLOT, NO_SELECTION, useAi } from "./store";
import s from "./AttachmentChips.module.css";

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

/** The tab's selection as an attachment while its chip shows. */
function useSelectionAttachment(slotId: string): SelectionAttachment | null {
  const sel = useAi((st) => st.selections[slotId]) ?? NO_SELECTION;
  const host = useTabHostName(slotId);
  return useMemo(() => (slotId !== HOME_SLOT && chipShown(sel, sel.hidden) ? makeAttachment(host, sel.text) : null), [slotId, sel, host]);
}

/** Everything that goes with the slot's next message, in the order it is stored (AI-10, AI-35). */
export function usePendingAttachments(slotId: string): Attachment[] {
  const diagnostics = useAi((st) => (slotId === HOME_SLOT ? undefined : st.diagnostics[slotId]));
  const extras = useAi((st) => st.slots[slotId]?.extras);
  const selection = useSelectionAttachment(slotId);
  return useMemo(
    () => [...(diagnostics ? [diagnostics] : []), ...(selection ? [selection] : []), ...(extras ?? []).map((x) => x.attachment)],
    [diagnostics, selection, extras],
  );
}

/**
 * The input area's chips for what goes with the tab's next message: connection diagnostics attached
 * from the terminal's error card, the terminal selection (AI-10), then long pastes and text files
 * in the order they were added (AI-35).
 */
export function ComposerAttachments({ slotId, onPasteAsText }: { slotId: string; onPasteAsText: (id: string, text: string) => void }) {
  const diagnostics = useAi((st) => (slotId === HOME_SLOT ? undefined : st.diagnostics[slotId]));
  const extras = useAi((st) => st.slots[slotId]?.extras);
  const selection = useSelectionAttachment(slotId);
  if (!selection && !diagnostics && !extras?.length) return null;
  return (
    <div className={s.attachments}>
      {diagnostics && <AttachmentChipFor attachment={diagnostics} onRemove={() => removeDiagnostics(slotId)} />}
      {selection && <AttachmentChipFor attachment={selection} onRemove={() => hideSelection(slotId)} />}
      {extras?.map((x) => (
        <AttachmentChipFor
          key={x.id}
          attachment={x.attachment}
          onRemove={() => removeExtra(slotId, x.id)}
          onPasteAsText={x.attachment.kind === "paste" ? () => onPasteAsText(x.id, x.attachment.text) : undefined}
        />
      ))}
    </div>
  );
}

interface ChipAction {
  label: string;
  title?: string;
  onClick: () => void;
}

/** An attachment as a chip: a label, a preview on hover or click, and × to remove it. */
function AttachmentChip({
  icon,
  label,
  badge,
  size,
  previewTitle,
  removeLabel,
  head,
  body,
  more,
  wrap,
  action,
  onRemove,
}: {
  icon: string;
  label: string;
  /** A warning next to the label, such as "truncated". */
  badge?: string;
  /** The attachment's size, such as "≈2.4k tokens". */
  size?: string;
  previewTitle: string;
  removeLabel: string;
  head: string;
  body: string;
  more?: string;
  /** Wrap long lines in the preview instead of cutting them. */
  wrap?: boolean;
  /** A button at the foot of the preview. */
  action?: ChipAction;
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
          {size && <span className={s.size}>{size}</span>}
          {badge && <span className={s.truncated}>{badge}</span>}
        </button>
        <button type="button" className={s.chipRemove} title={removeLabel} aria-label={removeLabel} onClick={onRemove}>
          <Icon name="x" size={11} />
        </button>
      </div>
      {(hover || pinned) && (
        <div className={s.preview} role={action ? "group" : "tooltip"} aria-label={action ? previewTitle : undefined}>
          {head && <div className={s.previewHead}>{head}</div>}
          <pre className={cx(s.previewBody, wrap && s.previewWrap)}>{body}</pre>
          {(more || action) && (
            <div className={s.previewFoot}>
              {more && <span className={s.previewMore}>{more}</span>}
              {action && (
                <button
                  type="button"
                  className={s.previewAction}
                  title={action.title}
                  onClick={() => {
                    setPinned(false);
                    setHover(false);
                    action.onClick();
                  }}
                >
                  {action.label}
                </button>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

/** The first lines of a text for a chip's preview, and how many more there are. */
function previewOf(text: string): { body: string; more: number } {
  const lines = text.split("\n");
  return { body: lines.slice(0, PREVIEW_LINES).join("\n"), more: Math.max(0, lines.length - PREVIEW_LINES) };
}

/**
 * The chip of any attachment kind. `onPasteAsText` adds Paste as Text to a long paste's preview,
 * which moves the text into the input box.
 */
export function AttachmentChipFor({ attachment: a, onRemove, onPasteAsText }: { attachment: Attachment; onRemove: () => void; onPasteAsText?: () => void }) {
  const t = useT();
  const platform = useApp((st) => st.info.platform);
  const { body, more } = previewOf(a.text);
  const moreText = more > 0 ? t("ai.selection.more", { n: more }) : undefined;
  const size = t("ai.attach.tokens", { n: formatTokens(attachmentTokens(a)) });
  switch (a.kind) {
    case "diagnostics":
      // The preview shows all of what will be sent.
      return (
        <AttachmentChip
          icon="plugs"
          label={t("ai.diagnostics", { host: a.host })}
          previewTitle={t("ai.diagnostics.preview")}
          removeLabel={t("ai.diagnostics.remove")}
          head={t("ai.diagnostics.sent")}
          body={a.text}
          wrap
          onRemove={onRemove}
        />
      );
    case "selection":
      return (
        <AttachmentChip
          icon="selection"
          label={t("ai.selection", { n: a.lines })}
          badge={a.truncated ? t("ai.selection.truncated") : undefined}
          previewTitle={t("ai.selection.preview")}
          removeLabel={t("ai.selection.remove")}
          head={a.host ? t("ai.selection.from", { host: a.host }) : ""}
          body={body}
          more={moreText}
          onRemove={onRemove}
        />
      );
    case "paste":
      return (
        <AttachmentChip
          icon="clipboard-text"
          label={t("ai.paste", { n: a.lines })}
          size={size}
          previewTitle={t("ai.paste.preview")}
          removeLabel={t("ai.paste.remove")}
          head=""
          body={body}
          more={moreText}
          action={
            onPasteAsText
              ? { label: t("ai.paste.asText"), title: t("ai.paste.shortcut", { key: shortcutLabel(platform, "Ctrl+Shift+V", "⌘⇧V") }), onClick: onPasteAsText }
              : undefined
          }
          onRemove={onRemove}
        />
      );
    case "file":
      return (
        <AttachmentChip
          icon="file-text"
          label={t("ai.file", { name: a.name, n: a.lines })}
          size={size}
          previewTitle={t("ai.file.preview")}
          removeLabel={t("ai.file.remove", { name: a.name })}
          head=""
          body={body}
          more={moreText}
          onRemove={onRemove}
        />
      );
  }
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

export function AttachmentCardFor({ attachment: a }: { attachment: Attachment }) {
  const t = useT();
  switch (a.kind) {
    case "diagnostics":
      return <AttachmentCard icon="plugs" title={t("ai.diagnostics", { host: a.host })} body={a.text} />;
    case "selection":
      return (
        <AttachmentCard
          icon="selection"
          title={t("ai.selection", { n: a.lines })}
          host={a.host}
          badge={a.truncated ? t("ai.selection.truncated") : undefined}
          body={a.text}
        />
      );
    case "paste":
      return <AttachmentCard icon="clipboard-text" title={t("ai.paste", { n: a.lines })} body={a.text} />;
    case "file":
      return <AttachmentCard icon="file-text" title={t("ai.file", { name: a.name, n: a.lines })} body={a.text} />;
  }
}
