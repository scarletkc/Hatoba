import { useState } from "react";
import { IconButton, LinkButton, TextField } from "@/components/controls";
import { Group, Section } from "@/components/layout";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import { ENV_MAX_VARS, ENV_NAME_MAX_CHARS, ENV_VALUE_MAX_CHARS, newEnvRow, type EnvRow, type EnvRowProblem } from "./envVars";
import s from "./EnvVarsSection.module.css";

/** Id of the field the host editor focuses when a variable keeps the form from saving. */
export const ENV_FOCUS_ID = "host-env";

/**
 * "Environment Variables" section of the host editor (SSH-14). The rows are part of the host form and saved with it.
 * An empty name is only a problem once the user tried to save (`showEmpty`); everything else shows as it is typed.
 */
export function EnvVarsSection({
  rows,
  problems,
  showEmpty,
  error,
  onChange,
}: {
  rows: EnvRow[];
  /** Index for index with `rows`. */
  problems: (EnvRowProblem | null)[];
  showEmpty: boolean;
  /** What the backend refused, if it did. */
  error?: string | null;
  onChange: (rows: EnvRow[]) => void;
}) {
  const t = useT();
  const [focusUid, setFocusUid] = useState<number | null>(null);
  const patchRow = (uid: number, p: Partial<EnvRow>) => onChange(rows.map((r) => (r.uid === uid ? { ...r, ...p } : r)));
  const full = rows.length >= ENV_MAX_VARS;
  const add = () => {
    const row = newEnvRow();
    onChange([...rows, row]);
    setFocusUid(row.uid);
  };

  const shown = problems.map((p): EnvRowProblem | null => {
    if (!p) return null;
    const visible = { ...p, name: p.name === "empty" && !showEmpty ? undefined : p.name };
    return visible.name || visible.value ? visible : null;
  });
  const firstBad = shown.findIndex(Boolean);

  const problemText = (p: EnvRowProblem): string => {
    const text: string[] = [];
    if (p.name === "empty") text.push(t("hosts.env.err.nameRequired"));
    if (p.name === "invalid") text.push(t("hosts.env.err.nameInvalid"));
    if (p.name === "tooLong") text.push(t("hosts.env.err.nameTooLong", { max: ENV_NAME_MAX_CHARS }));
    if (p.name === "duplicate") text.push(t("hosts.env.err.nameDuplicate"));
    if (p.value === "invalid") text.push(t("hosts.env.err.valueInvalid"));
    if (p.value === "tooLong") text.push(t("hosts.env.err.valueTooLong", { max: ENV_VALUE_MAX_CHARS.toLocaleString(t.locale) }));
    return text.join(" ");
  };

  return (
    <Section title={t("hosts.edit.sec.env")}>
      <Group>
        {rows.map((row, i) => {
          const problem = shown[i];
          const name = row.name.trim();
          return (
            <div key={row.uid} className={s.row}>
              <div className={s.fields}>
                <TextField
                  id={i === firstBad && problem?.name ? ENV_FOCUS_ID : undefined}
                  mono
                  autoFocus={focusUid === row.uid}
                  aria-label={t("hosts.env.nameLabel")}
                  placeholder={t("hosts.env.namePlaceholder")}
                  value={row.name}
                  invalid={!!problem?.name}
                  onChange={(e) => patchRow(row.uid, { name: e.target.value })}
                />
                <span className={s.eq} aria-hidden>
                  =
                </span>
                <TextField
                  id={i === firstBad && !problem?.name ? ENV_FOCUS_ID : undefined}
                  mono
                  aria-label={name ? t("hosts.env.valueLabelFor", { name }) : t("hosts.env.valueLabel")}
                  placeholder={t("hosts.env.valuePlaceholder")}
                  value={row.value}
                  invalid={!!problem?.value}
                  onChange={(e) => patchRow(row.uid, { value: e.target.value })}
                />
                <IconButton
                  icon="x"
                  size={13}
                  className={s.remove}
                  label={name ? t("hosts.env.remove", { name }) : t("hosts.env.removeBlank")}
                  onClick={() => onChange(rows.filter((r) => r.uid !== row.uid))}
                />
              </div>
              {problem && (
                <div className={s.problem} role="alert">
                  {problemText(problem)}
                </div>
              )}
            </div>
          );
        })}
        <div className={cx(s.footRow, rows.length === 0 && s.footSplit)}>
          {rows.length === 0 && <span className={s.muted}>{t("hosts.env.empty")}</span>}
          <LinkButton icon="plus" disabled={full} onClick={add}>
            {t("hosts.env.add")}
          </LinkButton>
          {full && <span className={s.muted}>{t("hosts.env.full", { max: ENV_MAX_VARS })}</span>}
        </div>
      </Group>
      {error && (
        <div className={s.error} role="alert">
          {error}
        </div>
      )}
      <div className={s.hint}>{t("hosts.env.hint")}</div>
    </Section>
  );
}
