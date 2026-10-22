import type { HTMLAttributes, ReactNode } from "react";
import { cx } from "@/lib/cx";
import { Icon, Spinner } from "./controls";
import p from "./patterns.module.css";

export { p as patternStyles };

/**
 * A list row: an optional leading icon, the body (its title, then a status line), and trailing controls.
 * `size="sm"` is the dense row of popovers and menus. A row that responds to the pointer is `interactive`.
 */
export function ListRow({
  leading,
  status,
  trailing,
  size = "md",
  interactive,
  selected,
  className,
  bodyClassName,
  children,
  ...rest
}: HTMLAttributes<HTMLDivElement> & {
  leading?: ReactNode;
  status?: ReactNode;
  trailing?: ReactNode;
  size?: "md" | "sm";
  interactive?: boolean;
  selected?: boolean;
  bodyClassName?: string;
}) {
  return (
    <div
      className={cx(
        p.row,
        size === "sm" && p.rowSmall,
        (interactive || selected) && p.rowInteractive,
        selected && p.rowSelected,
        className,
      )}
      {...rest}
    >
      {leading && <span className={p.rowLeading}>{leading}</span>}
      <div className={cx(p.rowBody, bodyClassName)}>
        {children}
        {status}
      </div>
      {trailing && <div className={p.rowTrailing}>{trailing}</div>}
    </div>
  );
}

export type StatusTone = "success" | "error" | "warning" | "progress" | "neutral";

const STATUS_ICON: Record<StatusTone, string | null> = {
  success: "check-circle",
  error: "warning-circle",
  warning: "warning",
  progress: null,
  neutral: null,
};

/**
 * One line of state under a title: done (a green check), failed (red, with the message wrapping), or in
 * progress (a spinner). `icon` replaces the tone's own icon; progress always shows the spinner.
 */
export function StatusLine({
  tone = "neutral",
  icon,
  className,
  children,
}: {
  tone?: StatusTone;
  icon?: string;
  className?: string;
  children: ReactNode;
}) {
  const name = icon ?? STATUS_ICON[tone];
  return (
    <span
      role={tone === "progress" ? "status" : undefined}
      className={cx(
        p.status,
        tone === "success" && p.statusSuccess,
        tone === "warning" && p.statusWarning,
        tone === "error" && p.statusError,
        className,
      )}
    >
      {tone === "progress" ? <Spinner size={14} /> : name && <Icon name={name} fill={tone !== "neutral"} />}
      <span className={p.statusText}>{children}</span>
    </span>
  );
}

/** The grey line under a field that says what it takes. */
export function HintText({ className, children }: { className?: string; children: ReactNode }) {
  return <div className={cx(p.hint, className)}>{children}</div>;
}

/** The red line under a field that says what is wrong with it. */
export function ErrorText({ className, children }: { className?: string; children: ReactNode }) {
  return (
    <div className={cx(p.error, className)} role="alert">
      {children}
    </div>
  );
}

export type ChipTone = "neutral" | "success" | "error" | "warning" | "accent";

/** A short label or state in a line, e.g. a tool call's "Done" or "Needs approval". */
export function Chip({
  tone = "neutral",
  icon,
  busy,
  mono,
  className,
  children,
  ...rest
}: HTMLAttributes<HTMLSpanElement> & {
  tone?: ChipTone;
  icon?: string;
  /** Shows a spinner in place of the icon. */
  busy?: boolean;
  mono?: boolean;
}) {
  return (
    <span
      className={cx(
        p.chip,
        tone === "success" && p.chipSuccess,
        tone === "error" && p.chipError,
        tone === "warning" && p.chipWarning,
        tone === "accent" && p.chipAccent,
        mono && p.chipMono,
        className,
      )}
      {...rest}
    >
      {busy ? <Spinner size={12} /> : icon && <Icon name={icon} />}
      {children}
    </span>
  );
}

/** A raised panel that stands on its own. An `interactive` card lightens under the pointer. */
export function Card({
  interactive,
  className,
  children,
  ...rest
}: HTMLAttributes<HTMLDivElement> & { interactive?: boolean }) {
  return (
    <div className={cx(p.card, interactive && p.cardInteractive, className)} {...rest}>
      {children}
    </div>
  );
}
