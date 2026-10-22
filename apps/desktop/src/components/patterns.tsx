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

interface MessageProps {
  className?: string;
  /** An icon before the text, which stays by its first line. */
  icon?: string;
  children: ReactNode;
}

function Message({ className, icon, children, tone }: MessageProps & { tone: "hint" | "warning" | "error" }) {
  return (
    <div className={cx(p[tone], icon && p.messageIcon, className)} role={tone === "error" ? "alert" : undefined}>
      {icon && <Icon name={icon} />}
      {icon ? <span>{children}</span> : children}
    </div>
  );
}

/** The grey line under a field that says what it takes. */
export function HintText(props: MessageProps) {
  return <Message tone="hint" {...props} />;
}

/** The orange line under a field: allowed, but worth a second look. */
export function WarningText({ icon = "warning", ...props }: MessageProps) {
  return <Message tone="warning" icon={icon} {...props} />;
}

/** The red line under a field that says what is wrong with it. */
export function ErrorText(props: MessageProps) {
  return <Message tone="error" {...props} />;
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
