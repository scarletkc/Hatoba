import {
  forwardRef,
  useLayoutEffect,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
  type TextareaHTMLAttributes,
} from "react";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import s from "./controls.module.css";

export { s as controlStyles };

/** Phosphor icon, e.g. `<Icon name="key" />` → `<i class="ph ph-key">`. */
export function Icon({
  name,
  fill,
  size,
  color,
  className,
  style,
}: {
  name: string;
  fill?: boolean;
  size?: number;
  color?: string;
  className?: string;
  style?: React.CSSProperties;
}) {
  return (
    <i
      aria-hidden
      className={cx(fill ? "ph-fill" : "ph", `ph-${name}`, className)}
      style={{ fontSize: size, color, ...style }}
    />
  );
}

/** A disclosure's caret, which turns to point down as it opens. */
export function DisclosureCaret({ open, size, className }: { open: boolean; size?: number; className?: string }) {
  return <Icon name="caret-right" size={size} className={cx(s.disclosureCaret, open && s.disclosureCaretOpen, className)} />;
}

/** The app icon (`src-tauri/app-icon.svg`). `className` sets its size. */
export function AppLogo({ className }: { className?: string }) {
  return <div aria-hidden className={cx(s.appLogo, className)} />;
}

type ButtonVariant = "default" | "primary" | "danger";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: "md" | "sm" | "xs";
  icon?: string;
  /** Icon after the label (e.g. arrow-up-right for external links). */
  trailingIcon?: string;
  busy?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "default", size = "md", icon, trailingIcon, busy, className, children, disabled, type = "button", ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type}
      disabled={disabled || busy}
      aria-busy={busy || undefined}
      className={cx(
        s.button,
        variant === "primary" && s.primary,
        variant === "danger" && s.danger,
        size === "sm" && s.small,
        size === "xs" && s.tiny,
        className,
      )}
      {...rest}
    >
      {icon && <Icon name={busy ? "circle-notch" : icon} className={cx(s.icon, busy && s.spinner)} />}
      {/* Without an icon to swap, the label stays (hidden) to hold the width
          and the spinner sits over it. */}
      <span className={cx(s.buttonLabel, busy && !icon && s.buttonLabelHidden)}>
        {children}
        {trailingIcon && <Icon name={trailingIcon} className={s.icon} style={{ fontSize: 12 }} />}
      </span>
      {busy && !icon && (
        <span className={s.buttonBusy}>
          <Icon name="circle-notch" className={cx(s.icon, s.spinner)} />
        </span>
      )}
    </button>
  );
});

export function LinkButton({
  icon,
  tone,
  className,
  children,
  type = "button",
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { icon?: string; tone?: "accent" | "danger" | "muted" }) {
  return (
    <button
      type={type}
      className={cx(s.link, tone === "danger" && s.linkDanger, tone === "muted" && s.linkMuted, className)}
      {...rest}
    >
      {icon && <Icon name={icon} size={13} />}
      {children}
    </button>
  );
}

export function IconButton({
  icon,
  label,
  active,
  size,
  className,
  type = "button",
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { icon: string; label: string; active?: boolean; size?: number }) {
  return (
    <button
      type={type}
      aria-label={label}
      title={label}
      className={cx(s.iconButton, active && s.iconButtonActive, className)}
      {...rest}
    >
      <Icon name={icon} size={size} />
    </button>
  );
}

export interface TextFieldProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "size"> {
  mono?: boolean;
  large?: boolean;
  invalid?: boolean;
  /** Password-style input with a reveal toggle. */
  secret?: boolean;
  leading?: ReactNode;
  trailing?: ReactNode;
  fieldClassName?: string;
}

export const TextField = forwardRef<HTMLInputElement, TextFieldProps>(function TextField(
  { mono, large, invalid, secret, leading, trailing, fieldClassName, className, type, ...rest },
  ref,
) {
  const t = useT();
  const [revealed, setRevealed] = useState(false);
  return (
    <div
      className={cx(
        s.field,
        large && s.fieldLarge,
        mono && s.fieldMono,
        invalid && s.fieldInvalid,
        secret && !revealed && s.fieldSecret,
        fieldClassName,
      )}
    >
      {leading && <span className={s.fieldAdornment}>{leading}</span>}
      <input
        ref={ref}
        type={secret ? (revealed ? "text" : "password") : (type ?? "text")}
        spellCheck={false}
        autoComplete="off"
        autoCorrect="off"
        autoCapitalize="off"
        aria-invalid={invalid || undefined}
        className={className}
        {...rest}
      />
      {secret && (
        <button
          type="button"
          className={s.fieldAdornmentButton}
          aria-label={revealed ? t("field.hide") : t("field.show")}
          title={revealed ? t("field.hide") : t("field.show")}
          onClick={() => setRevealed((v) => !v)}
        >
          <Icon name={revealed ? "eye-slash" : "eye"} />
        </button>
      )}
      {trailing && <span className={s.fieldAdornment}>{trailing}</span>}
    </div>
  );
});

export const TextArea = forwardRef<HTMLTextAreaElement, TextareaHTMLAttributes<HTMLTextAreaElement>>(
  function TextArea({ className, ...rest }, ref) {
    return <textarea ref={ref} spellCheck={false} className={cx(s.textarea, className)} {...rest} />;
  },
);

export interface SegmentOption<V extends string> {
  value: V;
  label: string;
  icon?: string;
  /** Names the segment, also to assistive technology, where a narrow page shows only its icon. */
  title?: string;
  /** Not offered now; the selected option stays selected. */
  disabled?: boolean;
}

export function Segmented<V extends string>({
  value,
  options,
  onChange,
  ariaLabel,
  className,
}: {
  value: V;
  options: SegmentOption<V>[];
  onChange: (v: V) => void;
  ariaLabel?: string;
  className?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pill, setPill] = useState<{ x: number; y: number; w: number; h: number } | null>(null);
  // Placed once before it slides: a pill that slid in from the corner on first paint would look broken.
  const [placed, setPlaced] = useState(false);
  // Callers pass a new options array on every render; the labels are what can move the segments.
  const labels = options.map((o) => o.label).join("\n");
  useLayoutEffect(() => {
    const root = ref.current;
    if (!root) return;
    const place = () => {
      const on = root.querySelector<HTMLElement>('[aria-checked="true"]');
      const next = on && { x: on.offsetLeft, y: on.offsetTop, w: on.offsetWidth, h: on.offsetHeight };
      setPill((cur) =>
        cur && next && cur.x === next.x && cur.y === next.y && cur.w === next.w && cur.h === next.h ? cur : next,
      );
    };
    place();
    const frame = requestAnimationFrame(() => setPlaced(true));
    if (typeof ResizeObserver === "undefined") return () => cancelAnimationFrame(frame);
    const observer = new ResizeObserver(place);
    observer.observe(root);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
    };
  }, [value, labels]);
  return (
    <div ref={ref} className={cx(s.segmented, className)} role="radiogroup" aria-label={ariaLabel}>
      {pill && (
        <span
          className={s.segmentPill}
          data-placed={placed || undefined}
          style={{ width: pill.w, height: pill.h, transform: `translate(${pill.x}px, ${pill.y}px)` }}
        />
      )}
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          disabled={o.disabled}
          className={cx(s.segment, o.value === value && s.segmentOn)}
          title={o.title}
          aria-label={o.title && o.label}
          onClick={() => onChange(o.value)}
        >
          {o.icon && <Icon name={o.icon} />}
          <span data-label={o.label}>{o.label}</span>
        </button>
      ))}
    </div>
  );
}

export function Switch({
  checked,
  onChange,
  label,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      className={cx(s.switch, checked && s.switchOn)}
      onClick={() => onChange(!checked)}
    >
      <span className={s.switchKnob} />
    </button>
  );
}

export function Checkbox({
  checked,
  onChange,
  children,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      className={cx(s.checkbox, checked && s.checkboxOn)}
      onClick={() => onChange(!checked)}
    >
      <span className={s.checkboxBox}>
        <Icon name="check" />
      </span>
      <span>{children}</span>
    </button>
  );
}

export function StatusDot({ color, size = 7 }: { color: string; size?: number }) {
  return <span className={s.dot} style={{ background: color, width: size, height: size }} />;
}

export function Spinner({ size = 14 }: { size?: number }) {
  return <Icon name="circle-notch" className={s.spinner} size={size} />;
}

export function Badge({ children }: { children: ReactNode }) {
  return <span className={s.badge}>{children}</span>;
}
