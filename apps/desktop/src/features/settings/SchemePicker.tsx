import type { KeyboardEvent } from "react";
import { DEFAULT_SCHEME, schemeFor, schemesFor, type TermMode, type TermScheme } from "@/features/terminal/schemes";
import { cx } from "@/lib/cx";
import s from "./SchemePicker.module.css";

/** The built-in colour schemes of one mode, as miniature terminals (TERM-06). */
export function SchemePicker({
  mode,
  value,
  label,
  caption,
  onChange,
}: {
  mode: TermMode;
  /** The stored id; an unset or unknown one shows the mode's default as picked. */
  value: string | null;
  label: string;
  /** Shown above the grid, and names it, when both modes are listed. */
  caption?: string;
  onChange: (id: string) => void;
}) {
  const picked = schemeFor(mode, value).id;
  const schemes = schemesFor(mode);
  /** One tab stop for the group; the arrow keys pick the previous or next scheme, as radio groups do. */
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const at = schemes.findIndex((x) => x.id === picked);
    const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[e.key];
    const next = e.key === "Home" ? 0 : e.key === "End" ? schemes.length - 1 : step ? (at + step + schemes.length) % schemes.length : -1;
    if (next < 0) return;
    e.preventDefault();
    e.currentTarget.querySelectorAll<HTMLElement>("[role=radio]")[next]?.focus();
    if (next !== at) onChange(schemes[next].id);
  };
  return (
    <div className={s.section}>
      {caption && <div className={s.caption}>{caption}</div>}
      <div className={s.grid} role="radiogroup" aria-label={caption ?? label} onKeyDown={onKeyDown}>
        {schemes.map((scheme) => (
          <Card key={scheme.id} scheme={scheme} selected={scheme.id === picked} onSelect={() => onChange(scheme.id)} />
        ))}
      </div>
    </div>
  );
}

const SWATCHES = ["red", "yellow", "green", "cyan", "blue", "magenta"] as const;

function Card({ scheme, selected, onSelect }: { scheme: TermScheme; selected: boolean; onSelect: () => void }) {
  const c = scheme.colors;
  // Hatoba Dark sits on the app's terminal background, which differs between light and dark.
  const background = scheme.id === DEFAULT_SCHEME.dark ? "var(--term)" : c.background;
  return (
    <button type="button" role="radio" aria-checked={selected} tabIndex={selected ? 0 : -1} className={s.card} onClick={onSelect}>
      <span className={cx(s.term, s[scheme.mode], selected && s.termOn)} style={{ background, color: c.foreground }} aria-hidden>
        <span className={s.line}>
          <span style={{ color: c.green }}>❯</span> <span style={{ color: c.blue }}>~/hatoba</span>{" "}
          <span style={{ color: c.magenta }}>main</span>{" "}
          <span className={s.cursor} style={{ background: c.cursor }} />
        </span>
        <span className={s.line}>
          <span style={{ color: c.cyan }}>src</span> README <span style={{ color: c.yellow }}>run.sh</span>
        </span>
        <span className={s.swatches}>
          {SWATCHES.map((name) => (
            <i key={name} style={{ background: c[name] }} />
          ))}
        </span>
      </span>
      <span className={cx(s.name, selected && s.nameOn)}>{scheme.name}</span>
    </button>
  );
}
