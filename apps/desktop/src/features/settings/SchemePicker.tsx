import { Button } from "@/components/controls";
import { DEFAULT_SCHEME, schemeFor, schemesFor, type TermMode, type TermScheme } from "@/features/terminal/schemes";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import s from "./SchemePicker.module.css";

/**
 * The colour schemes of one mode, built in and the user's, as miniature terminals (TERM-06), with the picked
 * one's actions: duplicate it to edit the copy, edit it when it is the user's, or import a scheme file.
 */
export function SchemePicker({
  mode,
  value,
  custom,
  label,
  caption,
  onChange,
  onDuplicate,
  onEdit,
  onImport,
}: {
  mode: TermMode;
  /** The stored id; an unset or unknown one shows the mode's default as picked. */
  value: string | null;
  custom: readonly TermScheme[];
  label: string;
  /** Shown above the grid, and names it, when both modes are listed. */
  caption?: string;
  onChange: (id: string) => void;
  onDuplicate: (scheme: TermScheme) => void;
  onEdit: (scheme: TermScheme) => void;
  onImport: (button: HTMLElement) => void;
}) {
  const t = useT();
  const picked = schemeFor(mode, value, custom);
  return (
    <div className={s.section}>
      {caption && <div className={s.caption}>{caption}</div>}
      <div className={s.grid} role="radiogroup" aria-label={caption ?? label}>
        {schemesFor(mode, custom).map((scheme) => (
          <Card
            key={scheme.id}
            scheme={scheme}
            selected={scheme.id === picked.id}
            onSelect={() => onChange(scheme.id)}
            onEdit={scheme.custom ? () => onEdit(scheme) : undefined}
          />
        ))}
      </div>
      <div className={s.actions}>
        {picked.custom && (
          <Button size="sm" icon="pencil-simple" onClick={() => onEdit(picked)}>
            {t("settings.scheme.edit")}
          </Button>
        )}
        <Button size="sm" icon="copy" onClick={() => onDuplicate(picked)}>
          {t("settings.scheme.duplicate")}
        </Button>
        <Button size="sm" icon="file-arrow-down" onClick={(e) => onImport(e.currentTarget)}>
          {t("settings.scheme.import")}
        </Button>
      </div>
    </div>
  );
}

const SWATCHES = ["red", "yellow", "green", "cyan", "blue", "magenta"] as const;

function Card({ scheme, selected, onSelect, onEdit }: { scheme: TermScheme; selected: boolean; onSelect: () => void; onEdit?: () => void }) {
  const c = scheme.colors;
  // Hatoba Dark sits on the app's terminal background, which differs between light and dark.
  const background = scheme.id === DEFAULT_SCHEME.dark ? "var(--term)" : c.background;
  return (
    <button type="button" role="radio" aria-checked={selected} className={s.card} onClick={onSelect} onDoubleClick={onEdit}>
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
