import { useEffect, useRef, useState } from "react";
import { IconButton } from "@/components/controls";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import { isImeEvent } from "@/lib/ime";
import type { LiveSession } from "./session";
import s from "./FindBar.module.css";

/** Small inline find bar (TERM-07): Enter / Shift+Enter step through matches, Esc closes. */
export function FindBar({ session, onClose }: { session: LiveSession; onClose(): void }) {
  const t = useT();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<{ index: number; count: number } | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
    const sub = session.search.onDidChangeResults((e) => setResults({ index: e.resultIndex, count: e.resultCount }));
    return () => {
      sub.dispose();
      session.search.clearDecorations();
    };
  }, [session]);

  const step = (direction: "next" | "prev", incremental = false) => {
    if (!session.find(query, direction, incremental) && !query) setResults(null);
  };

  const none = query !== "" && results?.count === 0;
  const label = !query || !results ? "" : none ? t("terminal.find.none") : t("terminal.find.count", { i: results.index + 1, n: results.count });

  return (
    <div className={s.bar} role="search">
      <input
        ref={inputRef}
        className={cx(s.input, none && s.inputNone)}
        value={query}
        placeholder={t("terminal.find.placeholder")}
        aria-label={t("terminal.find")}
        spellCheck={false}
        autoComplete="off"
        onChange={(e) => {
          const v = e.target.value;
          setQuery(v);
          if (!session.find(v, "next", true) && !v) setResults(null);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            if (isImeEvent(e)) return;
            e.preventDefault();
            step(e.shiftKey ? "prev" : "next");
          } else if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            onClose();
          }
        }}
      />
      <span className={cx(s.count, none && s.countNone)} aria-live="polite">
        {label}
      </span>
      <IconButton icon="caret-up" size={13} label={t("terminal.find.prev")} className={s.button} onClick={() => step("prev")} />
      <IconButton icon="caret-down" size={13} label={t("terminal.find.next")} className={s.button} onClick={() => step("next")} />
      <IconButton icon="x" size={12} label={t("terminal.find.close")} className={s.button} onClick={onClose} />
    </div>
  );
}
