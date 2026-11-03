import { Fragment, forwardRef, useState, type CSSProperties, type ReactNode } from "react";
import { Icon, IconButton, controlStyles } from "./controls";
import { cx } from "./cx";
import { isImeEvent } from "./ime";
import l from "./layout.module.css";
import { useUiStrings } from "./strings";
import { tagColor } from "./tags";

export { l as layoutStyles };

export function PageHeader({
  title,
  count,
  search,
  children,
}: {
  title: ReactNode;
  count?: ReactNode;
  /** A SearchField before the buttons. */
  search?: ReactNode;
  /** The buttons, which stay together in one row. */
  children?: ReactNode;
}) {
  return (
    <div className={l.pageHeader}>
      <div className={l.pageTitle}>{title}</div>
      {count !== undefined && <div className={l.pageCount}>{count}</div>}
      {(search || children) && (
        <div className={l.pageActions}>
          {search}
          {children && <div className={l.pageButtons}>{children}</div>}
        </div>
      )}
    </div>
  );
}

export const SearchField = forwardRef<
  HTMLInputElement,
  {
    value: string;
    onChange: (v: string) => void;
    placeholder: string;
    shortcut?: string;
    width?: number;
    onFocus?: () => void;
    onBlur?: () => void;
    /** A list of suggestions under the field: its element id, whether it shows, and the active option's id. */
    popup?: { id: string; open: boolean; active?: string };
    /** Shows a clear button with this label while the field has text. */
    clearLabel?: string;
    /** Takes the whole width of its row, as in a side panel. */
    fill?: boolean;
    className?: string;
  }
>(function SearchField(
  { value, onChange, placeholder, shortcut, width, onFocus, onBlur, popup, clearLabel, fill, className },
  ref,
) {
  return (
    <label
      className={cx(l.search, fill && l.searchFill, className)}
      style={width ? ({ "--search-width": `${width}px` } as CSSProperties) : undefined}
    >
      <Icon name="magnifying-glass" />
      <input
        ref={ref}
        type="search"
        value={value}
        placeholder={placeholder}
        spellCheck={false}
        aria-label={placeholder}
        role={popup ? "combobox" : undefined}
        aria-autocomplete={popup ? "list" : undefined}
        aria-expanded={popup ? popup.open : undefined}
        aria-controls={popup?.open ? popup.id : undefined}
        aria-activedescendant={popup?.open ? popup.active : undefined}
        onFocus={onFocus}
        onBlur={onBlur}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape" && value) {
            e.preventDefault();
            e.stopPropagation();
            onChange("");
          }
        }}
      />
      {shortcut && !value && <span className={l.searchKbd}>{shortcut}</span>}
      {clearLabel && value && (
        <IconButton
          small
          fill
          icon="x-circle"
          size={14}
          label={clearLabel}
          className={l.searchClear}
          onClick={(e) => {
            onChange("");
            e.currentTarget.parentElement?.querySelector("input")?.focus();
          }}
        />
      )}
    </label>
  );
});

export function Section({ title, children, style }: { title?: ReactNode; children: ReactNode; style?: CSSProperties }) {
  return (
    <div className={l.section} style={style}>
      {title && <div className={l.sectionTitle}>{title}</div>}
      {children}
    </div>
  );
}

export function Group({ children, style }: { children: ReactNode; style?: CSSProperties }) {
  return (
    <div className={l.group} style={style}>
      {children}
    </div>
  );
}

/** A form row: 104px label column + control. In a narrow Group the label goes above the control. */
export function FormRow({
  label,
  children,
  columns,
  top,
  error,
  htmlFor,
}: {
  label: ReactNode;
  children: ReactNode;
  /** Override the grid columns of a wide row, e.g. "120px minmax(0,1fr)" for a longer label. */
  columns?: string;
  top?: boolean;
  error?: string | null;
  htmlFor?: string;
}) {
  return (
    <div
      className={cx(l.formRow, top && l.formRowTop)}
      style={columns ? ({ "--form-columns": columns } as CSSProperties) : undefined}
    >
      <label className={l.formLabel} htmlFor={htmlFor}>
        {label}
      </label>
      {error ? (
        <div>
          {children}
          <div className={l.formError} role="alert">
            {error}
          </div>
        </div>
      ) : (
        children
      )}
    </div>
  );
}

export function Row({ label, children, tall }: { label: ReactNode; children?: ReactNode; tall?: boolean }) {
  return (
    <div className={cx(l.row, tall && l.rowTall)}>
      <span className={l.rowLabel}>{label}</span>
      {children}
    </div>
  );
}

export function RowValue({ children, mono }: { children: ReactNode; mono?: boolean }) {
  return (
    <span className={l.rowValue} style={mono ? { fontFamily: "var(--font-mono)", fontSize: 12 } : undefined}>
      {children}
    </span>
  );
}

/** A script whose words have no spaces between them. */
const CJK = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}]/u;
const LETTER_START = /^[\p{L}\p{N}]/u;
const LETTER_END = /[\p{L}\p{N}ー]$/u;
/** Japanese particles and endings, a word's one Han character, and katakana that goes on: part of the word before. */
const JOINS_BEFORE = /^(?:[\p{Script=Hiragana}ー]+|\p{Script=Han})$/u;
const KATAKANA = /^[\p{Script=Katakana}ー]+$/u;
const segmenters = new Map<string, Intl.Segmenter>();

/**
 * The runs of `text` that a line must not break inside, for a `<wbr>` between each two under `word-break: keep-all`.
 * CJK has no spaces between words, so a browser would break between any two characters, also inside a word.
 * `Intl.Segmenter` finds the words, and a run ends between two words of which one is CJK. Its dictionary splits some
 * words (服务|器, イン|ポート), so a lone Han character, kana endings and katakana stay with the word before them.
 * Spaces and punctuation keep their own breaks under `keep-all`. Text without CJK is one run.
 */
export function wordRuns(text: string, locale?: string): string[] {
  if (!CJK.test(text) || typeof Intl.Segmenter !== "function") return [text];
  const key = locale ?? "";
  let segmenter = segmenters.get(key);
  if (!segmenter) {
    segmenter = new Intl.Segmenter(locale, { granularity: "word" });
    segmenters.set(key, segmenter);
  }
  const runs: string[] = [];
  let previous = "";
  for (const { segment } of segmenter.segment(text)) {
    const last = runs.at(-1);
    const breaks =
      last !== undefined &&
      LETTER_END.test(last) &&
      LETTER_START.test(segment) &&
      (CJK.test(previous) || CJK.test(segment)) &&
      !JOINS_BEFORE.test(segment) &&
      !(KATAKANA.test(previous) && KATAKANA.test(segment));
    if (last === undefined || breaks) runs.push(segment);
    else runs[runs.length - 1] = last + segment;
    previous = segment;
  }
  return runs;
}

/** Text that breaks a line only between words, also in Chinese and Japanese (`wordRuns`); other nodes as they are. */
function WordBreaks({ children }: { children: ReactNode }) {
  if (typeof children !== "string") return children;
  const lang = typeof document === "undefined" ? undefined : document.documentElement.lang || undefined;
  return wordRuns(children, lang).map((run, i) => (
    <Fragment key={i}>
      {i > 0 && <wbr />}
      {run}
    </Fragment>
  ));
}

export function EmptyState({
  icon,
  title,
  body,
  actions,
  foot,
  compact,
}: {
  icon: string;
  title: ReactNode;
  body?: ReactNode;
  actions?: ReactNode;
  foot?: ReactNode;
  /** Sized for a side panel or a popover rather than a page. */
  compact?: boolean;
}) {
  return (
    <div className={cx(l.empty, compact && l.emptyCompact)}>
      <div className={l.emptyIcon}>
        <Icon name={icon} />
      </div>
      <div className={l.emptyTitle}>
        <WordBreaks>{title}</WordBreaks>
      </div>
      {body && (
        <div className={l.emptyBody}>
          <WordBreaks>{body}</WordBreaks>
        </div>
      )}
      {actions && <div className={l.emptyActions}>{actions}</div>}
      {foot && <div className={l.emptyFoot}>{foot}</div>}
    </div>
  );
}

export function Tag({ name }: { name: string }) {
  return (
    <span className={controlStyles.tag}>
      <span className={controlStyles.tagDot} style={{ background: tagColor(name) }} />
      {name}
    </span>
  );
}

/** A tag with its remove button inside, in the tag field and the hosts tag filter. */
export function TagChip({ name, removeLabel, onRemove }: { name: string; removeLabel: string; onRemove: () => void }) {
  return (
    <span className={l.tagChip}>
      <span className={controlStyles.tagDot} style={{ background: tagColor(name) }} />
      {name}
      <button type="button" aria-label={removeLabel} title={removeLabel} onClick={onRemove}>
        <Icon name="x" />
      </button>
    </span>
  );
}

/** Chips + free text input. Enter / comma adds, Backspace on empty input removes the last tag. */
export function TagInput({
  value,
  onChange,
  placeholder,
  suggestions = [],
}: {
  value: string[];
  onChange: (tags: string[]) => void;
  placeholder: string;
  suggestions?: string[];
}) {
  const strings = useUiStrings();
  const [draft, setDraft] = useState("");
  const listId = "tag-suggestions";
  const add = (raw: string) => {
    const tag = raw.trim().replace(/\s+/g, "-");
    if (tag && !value.includes(tag)) onChange([...value, tag]);
    setDraft("");
  };
  return (
    <div className={l.tagInput}>
      {value.map((tag) => (
        <TagChip
          key={tag}
          name={tag}
          removeLabel={strings.removeTag(tag)}
          onRemove={() => onChange(value.filter((x) => x !== tag))}
        />
      ))}
      <input
        value={draft}
        list={listId}
        placeholder={value.length ? "" : placeholder}
        spellCheck={false}
        onChange={(e) => {
          const v = e.target.value;
          if (v.endsWith(",")) add(v.slice(0, -1));
          else setDraft(v);
        }}
        onBlur={() => draft && add(draft)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            if (isImeEvent(e)) return;
            e.preventDefault();
            add(draft);
          } else if (e.key === "Backspace" && !draft && value.length) {
            onChange(value.slice(0, -1));
          }
        }}
      />
      <datalist id={listId}>
        {suggestions
          .filter((s) => !value.includes(s))
          .map((s) => (
            <option key={s} value={s} />
          ))}
      </datalist>
    </div>
  );
}

export function Callout({
  icon,
  iconColor,
  title,
  children,
}: {
  icon: string;
  iconColor?: string;
  title?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className={l.callout}>
      <Icon name={icon} size={16} color={iconColor ?? "var(--fg2)"} style={{ marginTop: 1 }} />
      <div>
        {title && <div className={l.calloutTitle}>{title}</div>}
        {children && <div className={l.calloutBody}>{children}</div>}
      </div>
    </div>
  );
}

/** 44px tinted status tile, e.g. green cloud-check for "Synced". */
export function StatusTile({ icon, color }: { icon: string; color: string }) {
  return (
    <div className={l.tile} style={{ background: `color-mix(in srgb, ${color} 14%, transparent)`, color }}>
      <Icon name={icon} />
    </div>
  );
}
