import { TextArea } from "@/components/controls";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import { textSize } from "./instructions";
import s from "./InstructionsField.module.css";

/**
 * A text area for what the user tells the assistant (AI-36 custom instructions, AI-37 host notes),
 * with its size: the characters against the limit and the tokens it adds to every request. Text
 * over the limit is marked and says so; its owner does not save it. `error`: another problem to show.
 */
export function InstructionsField({
  id,
  label,
  value,
  max,
  placeholder,
  rows = 5,
  error,
  onChange,
  onBlur,
}: {
  id: string;
  label: string;
  value: string;
  max: number;
  placeholder?: string;
  rows?: number;
  error?: string | null;
  onChange: (value: string) => void;
  onBlur?: () => void;
}) {
  const t = useT();
  const size = textSize(value, max);
  const nf = new Intl.NumberFormat(t.locale);
  const sizeId = `${id}-size`;
  const problem = size.over ? t("ai.instructions.tooLong", { max: nf.format(max) }) : (error ?? null);
  return (
    <div className={s.field}>
      <TextArea
        id={id}
        className={s.input}
        rows={rows}
        value={value}
        aria-label={label}
        aria-invalid={!!problem || undefined}
        aria-describedby={sizeId}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
        onBlur={onBlur}
      />
      <div className={s.foot}>
        {problem && <span className={s.problem}>{problem}</span>}
        <span id={sizeId} className={cx(s.size, size.over && s.over)}>
          {t("ai.instructions.size", { n: nf.format(size.chars), max: nf.format(max), tokens: nf.format(size.tokens) })}
        </span>
      </div>
    </div>
  );
}
