import { Fragment, useRef, useState } from "react";
import { Icon, isImeEvent } from "@hatoba/ui";
import { useT, type MessageKey } from "@/i18n";
import { cx } from "@/lib/cx";
import { parseSendKey } from "../terminal/sendKeys";
import { SEND_KEYS } from "../tools";
import s from "../messages/Messages.module.css";

/** AI-13: the names of the keys `send_input` presses, as keyboards print them. */
export const KEY_LABEL: Record<string, MessageKey> = {
  enter: "ai.key.enter",
  tab: "ai.key.tab",
  esc: "ai.key.esc",
  up: "ai.key.up",
  down: "ai.key.down",
  left: "ai.key.left",
  right: "ai.key.right",
  home: "ai.key.home",
  end: "ai.key.end",
  page_up: "ai.key.page_up",
  page_down: "ai.key.page_down",
  insert: "ai.key.insert",
  delete: "ai.key.delete",
  backspace: "ai.key.backspace",
  space: "ai.key.space",
  " ": "ai.key.space",
};

const MODIFIER_LABEL: Record<string, string> = { ctrl: "Ctrl", alt: "Alt", shift: "Shift" };

/**
 * A key as the card names it: `ctrl+x` as Ctrl+X, `page_up` as Page Up, a space as Space. A letter keeps its
 * case after Alt alone, where Alt+F and Alt+f send different bytes; a key Hatoba can't press shows as given.
 */
function keyLabel(t: ReturnType<typeof useT>, key: string): string {
  const parsed = parseSendKey(key);
  if (parsed === null) return key || '""';
  const [, prefix, base] = /^((?:ctrl\+|alt\+|shift\+)*)(.+)$/.exec(parsed)!;
  const modifiers = prefix.split("+").filter(Boolean);
  const name = Object.hasOwn(KEY_LABEL, base)
    ? t(KEY_LABEL[base])
    : /^f\d+$/.test(base) || modifiers.includes("ctrl") || modifiers.includes("shift")
      ? base.toUpperCase()
      : base;
  return [...modifiers.map((m) => MODIFIER_LABEL[m]), name].join("+");
}

/** AI-13: "Then press" with the keys a `send_input` call presses after its text, each in a chip of its own. */
export function PressedKeys({ keys }: { keys: string[] }) {
  const t = useT();
  const [before, after] = t("ai.approval.thenKey", { key: "\0" }).split("\0");
  return (
    <>
      {before}
      {keys.map((key, i) => (
        <Fragment key={i}>
          {i > 0 && " "}
          <kbd className={cx(s.chip, s.key, parseSendKey(key) === null && s.chipBad)}>{keyLabel(t, key)}</kbd>
        </Fragment>
      ))}
      {after}
    </>
  );
}

/** Whether a chip's box comes before the point (x, y) in reading order, the rows of the field first. */
function before(box: DOMRect, x: number, y: number): boolean {
  if (box.bottom <= y) return true;
  if (box.top > y) return false;
  return box.left + box.width / 2 < x;
}

/**
 * AI-13: the keys of a `send_input` call as chips in one field that wraps, like a host's tags (TagInput).
 * What is typed goes in at the caret, which a click puts between two chips and ← and → move; Enter or
 * leaving the field adds it. Backspace and Delete remove the key before or after the caret, × any key.
 */
export function KeysField({ id, keys, onChange }: { id: string; keys: string[]; onChange: (keys: string[]) => void }) {
  const t = useT();
  const field = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const [caret, setCaret] = useState(keys.length);
  const [text, setText] = useState("");
  const at = Math.min(caret, keys.length);

  /** The keys with what was typed added at the caret; a lone space is the Space key, not nothing. */
  const typed = () => {
    const raw = text.trim() || text;
    return raw ? [...keys.slice(0, at), parseSendKey(raw) ?? raw, ...keys.slice(at)] : keys;
  };
  const apply = (next: string[], to: number) => {
    if (next !== keys) onChange(next);
    setCaret(to);
    setText("");
  };
  const remove = (list: string[], i: number) => apply([...list.slice(0, i), ...list.slice(i + 1)], i);

  return (
    <div
      ref={field}
      className={s.keysField}
      onMouseDown={(e) => {
        // A click anywhere in the field puts the caret in the gap nearest to it, as in text.
        if (e.target === input.current || e.button !== 0) return;
        e.preventDefault();
        const next = typed();
        // What was typed becomes a chip where the input is, before the click or after it.
        let to = next !== keys && before(input.current!.getBoundingClientRect(), e.clientX, e.clientY) ? 1 : 0;
        for (const chip of field.current!.querySelectorAll("[data-key]"))
          if (before(chip.getBoundingClientRect(), e.clientX, e.clientY)) to++;
        apply(next, to);
        input.current?.focus();
      }}
    >
      {keys.map((key, i) => (
        // The input stays last in the DOM and moves by `order`, so moving the caret never takes its focus.
        <span
          key={i}
          data-key
          className={cx(s.keyChip, parseSendKey(key) === null && s.keyChipBad)}
          style={{ order: i < at ? 0 : 2 }}
        >
          {keyLabel(t, key)}
          <button
            type="button"
            tabIndex={-1}
            aria-label={t("ai.approval.removeKey", { key: keyLabel(t, key) })}
            onMouseDown={(e) => {
              e.preventDefault();
              e.stopPropagation();
            }}
            onClick={() => {
              const next = typed();
              remove(next, next !== keys && i >= at ? i + 1 : i);
              input.current?.focus();
            }}
          >
            <Icon name="x" />
          </button>
        </span>
      ))}
      <input
        ref={input}
        id={id}
        className={cx(s.keysInput, at < keys.length && s.keysInputInline)}
        style={{ order: 1 }}
        value={text}
        size={Math.max(1, Array.from(text).length)}
        list={`${id}-keys`}
        placeholder={keys.length ? "" : t("ai.approval.addKey")}
        spellCheck={false}
        autoComplete="off"
        onChange={(e) => setText(e.target.value)}
        onBlur={() => {
          const next = typed();
          apply(next, next.length);
        }}
        onKeyDown={(e) => {
          if (isImeEvent(e)) return;
          if (e.key === "Enter") {
            e.preventDefault();
            const next = typed();
            if (next !== keys) apply(next, at + 1);
          } else if (e.key === "Escape" && text) {
            // Esc drops what was typed; with nothing typed it leaves Edit, as in the other fields.
            e.preventDefault();
            e.stopPropagation();
            setText("");
          } else if (text) {
            return;
          } else if (e.key === "Backspace" && at > 0) {
            remove(keys, at - 1);
          } else if (e.key === "Delete" && at < keys.length) {
            remove(keys, at);
          } else if (e.key === "ArrowLeft" && at > 0) {
            setCaret(at - 1);
          } else if (e.key === "ArrowRight" && at < keys.length) {
            setCaret(at + 1);
          } else if (e.key === "Home" || e.key === "End") {
            setCaret(e.key === "Home" ? 0 : keys.length);
          }
        }}
      />
      <datalist id={`${id}-keys`}>
        {SEND_KEYS.map((key) => (
          <option key={key} value={key} />
        ))}
      </datalist>
    </div>
  );
}
