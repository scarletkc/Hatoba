import type { ReactNode } from "react";
import { ErrorText, HintText } from "@/components/patterns";
import s from "./Field.module.css";

/** Label above a control with an optional hint / error line (the design's wizard + dialog fields). */
export function Field({
  label,
  htmlFor,
  hint,
  error,
  aside,
  dim,
  children,
}: {
  label: ReactNode;
  htmlFor?: string;
  hint?: ReactNode;
  error?: ReactNode;
  /** Right-aligned content on the label line (e.g. a link). */
  aside?: ReactNode;
  dim?: boolean;
  children: ReactNode;
}) {
  return (
    <div className={s.field} style={dim ? { opacity: 0.45 } : undefined}>
      <div className={s.labelLine}>
        <label className={s.label} htmlFor={htmlFor}>
          {label}
        </label>
        {aside}
      </div>
      {children}
      {error ? <ErrorText>{error}</ErrorText> : hint && <HintText>{hint}</HintText>}
    </div>
  );
}
