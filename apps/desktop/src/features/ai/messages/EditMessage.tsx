import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Button, WarningText, isImeEvent, toast } from "@hatoba/ui";
import { readClipboard } from "@/features/terminal/clipboard";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import { tooMuchMessage } from "../actions/attach";
import { composeMessage, fitsMessage, isLongPaste, makePaste, type MessageParts } from "../attachments";
import { AttachmentChipFor } from "../composer/AttachmentChips";
import { insertAtCaret, isPlainPasteKey } from "../composer/Composer";
import s from "./Messages.module.css";

/**
 * AI-26: edits the typed text; what was attached to it stays unless its chip is removed. A long
 * paste becomes another attachment, as in the input box (AI-35). With `untracked`, the calls the edit
 * deletes changed files this device cannot revert (AI-41), and the box says those changes stay.
 */
export function EditMessage({
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
    const ok = await onSend(
      composeMessage(
        value.trim(),
        items.map((x) => x.attachment),
      ),
    );
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
          if (
            !fitsMessage(
              items.map((x) => x.attachment),
              paste,
            )
          )
            return toast(tooMuchMessage(), "error");
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
        <div role="note">
          <WarningText>{t("ai.editFiles.untracked")}</WarningText>
        </div>
      )}
      <div className={s.actions}>
        <Button size="sm" onClick={onCancel}>
          {t("btn.cancel")}
        </Button>
        <Button
          size="sm"
          variant="primary"
          icon="arrow-up"
          busy={busy}
          disabled={!value.trim()}
          onClick={() => void send()}
        >
          {t("ai.edit.send")}
        </Button>
      </div>
    </div>
  );
}
