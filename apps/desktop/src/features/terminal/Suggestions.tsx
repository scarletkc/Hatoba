import { useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type RefObject } from "react";
import { createPortal } from "react-dom";
import { isImeEvent, overlayStyles, useExitAnimation, useOverlaysLocked, windowSurface } from "@hatoba/ui";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import type { LiveSession } from "@/session/session";
import { onHistoryChange } from "@/session/history";
import { listMatches } from "@/session/suggest";
import { terminalFontFamily, useTermSettings, useTerminalChrome } from "@/session/theme";
import s from "./Suggestions.module.css";

/** East Asian wide and fullwidth characters, and emoji, which take two cells. */
const WIDE = /[ᄀ-ᅟ⺀-〾ぁ-㏿㐀-䶿一-鿿ꀀ-꓏가-힣豈-﫿︰-﹏＀-｠￠-￦\u{1F300}-\u{1FAFF}\u{20000}-\u{3FFFD}]/u;

/**
 * TERM-14: the rest of a command, dimmed after the cursor. Each character takes its own cells, so the
 * text lines up with the terminal's grid, and what does not fit on the cursor's row is cut.
 */
export function SuggestionGhost({ session, area }: { session: LiveSession; area: RefObject<HTMLDivElement | null> }) {
  const suggestion = useSyncExternalStore(session.subscribeSuggestion, () => session.suggestion);
  const fontFamily = useTermSettings((st) => st.settings.font_family);
  const fontSize = useTermSettings((st) => st.settings.font_size);
  const chrome = useTerminalChrome();
  if (!suggestion || !area.current) return null;
  const box = session.cellBox(suggestion.x, suggestion.y);
  if (!box) return null;
  const origin = area.current.getBoundingClientRect();
  const room = Math.round((box.screenRight - box.left) / box.width);
  const cells: { ch: string; wide: boolean }[] = [];
  let used = 0;
  for (const ch of suggestion.rest) {
    const wide = WIDE.test(ch);
    if (used + (wide ? 2 : 1) > room) break;
    cells.push({ ch, wide });
    used += wide ? 2 : 1;
  }
  return (
    <div
      className={s.ghost}
      aria-hidden
      style={{
        left: box.left - origin.left,
        top: box.top - origin.top,
        height: box.height,
        fontFamily: terminalFontFamily(fontFamily),
        fontSize,
        color: chrome.foreground,
      }}
    >
      {cells.map((c, i) => (
        <span key={i} style={{ width: (c.wide ? 2 : 1) * box.width }}>
          {c.ch}
        </span>
      ))}
    </div>
  );
}

/** At most this many rows; typing more narrows them. */
const MAX_ROWS = 200;
const WIDTH = 480;
const MAX_HEIGHT = 300;

/**
 * TERM-14: the list of the tab's commands that Ctrl+Shift+Space (⌘⇧Space) opens at the cursor. Typing
 * filters it, ↑ and ↓ select, Enter, Tab or a click types the command in place of what was typed without
 * running it, Shift+Delete removes it from the history, and Esc or a click elsewhere closes the list.
 */
export function SuggestionList({ session, onClose }: { session: LiveSession; onClose: () => void }) {
  const t = useT();
  const locked = useOverlaysLocked();
  const [context] = useState(() => session.listContext());
  const [filter, setFilter] = useState(context.typed);
  const history = useSyncExternalStore(onHistoryChange, session.commandHistory);
  const [active, setActive] = useState(0);
  const items = useMemo(() => listMatches(filter, history).slice(0, MAX_ROWS), [filter, history]);
  const ref = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  useExitAnimation(ref, !locked);

  const [pos, setPos] = useState<{ left: number; top?: number; bottom?: number; maxHeight: number } | null>(null);
  useLayoutEffect(() => {
    const cursor = context.cursor;
    if (!cursor) return;
    const b = windowSurface();
    const below = b.bottom - (cursor.top + cursor.height) - 12;
    const above = cursor.top - b.top - 12;
    const left = Math.max(b.left + 8, Math.min(cursor.left, b.right - WIDTH - 8));
    setPos(
      below >= Math.min(MAX_HEIGHT, 160) || below >= above
        ? { left, top: cursor.top + cursor.height + 4, maxHeight: Math.min(MAX_HEIGHT, below) }
        : { left, bottom: window.innerHeight - cursor.top + 4, maxHeight: Math.min(MAX_HEIGHT, above) },
    );
  }, [context.cursor]);

  useEffect(() => {
    input.current?.focus();
  }, [pos]);

  // The selected row stays in view as ↑ and ↓ move it.
  useEffect(() => {
    ref.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const close = () => {
    onClose();
    session.focus();
  };
  const pick = (command: string | undefined) => {
    if (command === undefined) return;
    onClose();
    session.acceptListed(command);
  };

  if (locked || !pos) return null;
  const listId = `suggest-${session.tabId}`;

  return createPortal(
    <div
      ref={ref}
      className={cx(s.list, overlayStyles.popIn)}
      style={{ ...pos, width: WIDTH }}
      onMouseDown={(e) => {
        // The filter keeps the focus when a row, the padding or the scroll bar is clicked.
        if (e.target !== input.current) e.preventDefault();
      }}
    >
      <input
        ref={input}
        className={s.filter}
        value={filter}
        spellCheck={false}
        autoComplete="off"
        placeholder={t("terminal.suggest.filter")}
        role="combobox"
        aria-label={t("terminal.suggest.list")}
        aria-expanded
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={items.length > 0 ? `${listId}-${active}` : undefined}
        onChange={(e) => {
          setFilter(e.target.value);
          setActive(0);
        }}
        onBlur={(e) => {
          // A click on another field keeps the focus there; a click on nothing that takes it returns it to the terminal.
          onClose();
          if (!e.relatedTarget) session.focus();
        }}
        onKeyDown={(e) => {
          if (isImeEvent(e)) return;
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            const step = e.key === "ArrowDown" ? 1 : -1;
            if (items.length > 0) setActive((i) => (i + step + items.length) % items.length);
          } else if (e.key === "Enter" || (e.key === "Tab" && !e.shiftKey)) {
            e.preventDefault();
            pick(items[active]);
          } else if (e.key === "Escape") {
            e.preventDefault();
            close();
          } else if (e.key === "Delete" && e.shiftKey) {
            e.preventDefault();
            const command = items[active];
            if (command === undefined) return;
            session.removeListed(command);
            setActive((i) => Math.max(0, Math.min(i, items.length - 2)));
          }
        }}
      />
      <div id={listId} role="listbox" aria-label={t("terminal.suggest.list")} className={s.rows}>
        {items.length === 0 ? (
          <div className={s.empty}>{t("terminal.suggest.empty")}</div>
        ) : (
          items.map((command, i) => (
            <div
              key={command}
              id={`${listId}-${i}`}
              data-index={i}
              role="option"
              aria-selected={i === active}
              className={cx(s.row, i === active && s.active)}
              onMouseMove={() => setActive(i)}
              onClick={() => pick(command)}
            >
              {command}
            </div>
          ))
        )}
      </div>
    </div>,
    document.body,
  );
}
