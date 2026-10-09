import {
  forwardRef,
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
      {busy ? (
        <Icon name="circle-notch" className={cx(s.icon, s.spinner)} />
      ) : icon ? (
        <Icon name={icon} className={s.icon} />
      ) : null}
      {children}
      {trailingIcon && <Icon name={trailingIcon} className={s.icon} style={{ fontSize: 12 }} />}
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
}

export function Segmented<V extends string>({
  value,
  options,
  onChange,
  ariaLabel,
}: {
  value: V;
  options: SegmentOption<V>[];
  onChange: (v: V) => void;
  ariaLabel?: string;
}) {
  return (
    <div className={s.segmented} role="radiogroup" aria-label={ariaLabel}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          className={cx(s.segment, o.value === value && s.segmentOn)}
          onClick={() => onChange(o.value)}
        >
          {o.icon && <Icon name={o.icon} />}
          {o.label}
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
      <span className={s.checkboxBox}>{checked && <Icon name="check" />}</span>
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
