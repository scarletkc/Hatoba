import { forwardRef, useState, type CSSProperties, type ReactNode } from "react";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import { tagColor } from "@/lib/tags";
import { Icon, controlStyles } from "./controls";
import l from "./layout.module.css";

export { l as layoutStyles };

export function PageHeader({
  title,
  count,
  children,
}: {
  title: ReactNode;
  count?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className={l.pageHeader}>
      <div className={l.pageTitle}>{title}</div>
      {count !== undefined && <div className={l.pageCount}>{count}</div>}
      <div className={l.spacer} />
      {children}
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
  }
>(function SearchField({ value, onChange, placeholder, shortcut, width, onFocus, onBlur, popup }, ref) {
  return (
    <label className={l.search} style={{ width }}>
      <Icon name="magnifying-glass" />
      <input
        ref={ref}
        type="search"
        value={value}
        placeholder={placeholder}
        spellCheck={false}
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
            e.stopPropagation();
            onChange("");
          }
        }}
      />
      {shortcut && !value && <span className={l.searchKbd}>{shortcut}</span>}
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

/** 104px label column + control, the design's form row. */
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
  /** Override grid columns, e.g. "104px minmax(0,1fr) auto 72px" for address + port. */
  columns?: string;
  top?: boolean;
  error?: string | null;
  htmlFor?: string;
}) {
  return (
    <div className={cx(l.formRow, top && l.formRowTop)} style={columns ? { gridTemplateColumns: columns } : undefined}>
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

export function EmptyState({
  icon,
  title,
  body,
  actions,
  foot,
}: {
  icon: string;
  title: ReactNode;
  body?: ReactNode;
  actions?: ReactNode;
  foot?: ReactNode;
}) {
  return (
    <div className={l.empty}>
      <div className={l.emptyIcon}>
        <Icon name={icon} />
      </div>
      <div className={l.emptyTitle}>{title}</div>
      {body && <div className={l.emptyBody}>{body}</div>}
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
  const t = useT();
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
        <span key={tag} className={l.tagChip}>
          <span className={controlStyles.tagDot} style={{ background: tagColor(tag) }} />
          {tag}
          <button
            type="button"
            aria-label={t("field.removeTag", { name: tag })}
            onClick={() => onChange(value.filter((x) => x !== tag))}
          >
            <Icon name="x" />
          </button>
        </span>
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
