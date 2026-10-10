import { useEffect, useState } from "react";
import { useApp } from "@/app/store";
import { Icon, Segmented, TextField } from "@/components/controls";
import { Group } from "@/components/layout";
import { PopupSelect } from "@/components/overlay";
import { useT } from "@/i18n";
import type { TerminalSettings } from "@/ipc/types";
import { isImeEvent } from "@/lib/ime";
import { defaultTerminalFont } from "@/lib/platform";
import { Pane, SettingRow, type PaneProps } from "./shared";
import s from "./TerminalPane.module.css";

const SIZE_MIN = 10;
const SIZE_MAX = 24;
const SCROLLBACK = [1000, 5000, 10000, 50000];

/** Settings → Terminal (TERM-05 / TERM-06): font, cursor and scrollback. */
export function TerminalPane({ settings, updateTerminal }: PaneProps) {
  const t = useT();
  const defaultFont = defaultTerminalFont(useApp((a) => a.info.platform));
  const term = settings?.terminal;
  const fontFamily = term?.font_family;
  const [font, setFont] = useState(fontFamily ?? defaultFont);
  useEffect(() => {
    if (fontFamily !== undefined) setFont(fontFamily);
  }, [fontFamily]);

  if (!term) return <Pane>{null}</Pane>;

  const commitFont = () => {
    const next = font.trim() || defaultFont;
    setFont(next);
    if (next !== term.font_family) updateTerminal({ font_family: next });
  };

  const scrollbackValues = SCROLLBACK.includes(term.scrollback) ? SCROLLBACK : [...SCROLLBACK, term.scrollback].sort((a, b) => a - b);
  const lines = (n: number) => t("settings.term.lines", { n: n.toLocaleString(t.locale) });

  return (
    <Pane>
      <Group>
        <SettingRow label={t("settings.term.font")}>
          <div className={s.fontField}>
            <TextField
              mono
              aria-label={t("settings.term.font")}
              value={font}
              placeholder={defaultFont}
              onChange={(e) => setFont(e.target.value)}
              onBlur={commitFont}
              onKeyDown={(e) => e.key === "Enter" && !isImeEvent(e) && commitFont()}
            />
          </div>
        </SettingRow>
        <SettingRow label={t("settings.term.size")}>
          <Stepper
            value={term.font_size}
            min={SIZE_MIN}
            max={SIZE_MAX}
            downLabel={t("settings.term.sizeDown")}
            upLabel={t("settings.term.sizeUp")}
            onChange={(font_size) => updateTerminal({ font_size })}
          />
        </SettingRow>
        <SettingRow label={t("settings.term.cursor")}>
          <Segmented<TerminalSettings["cursor_style"]>
            ariaLabel={t("settings.term.cursor")}
            value={term.cursor_style}
            options={[
              { value: "block", label: t("settings.term.cursor.block") },
              { value: "bar", label: t("settings.term.cursor.bar") },
              { value: "underline", label: t("settings.term.cursor.underline") },
            ]}
            onChange={(cursor_style) => updateTerminal({ cursor_style })}
          />
        </SettingRow>
        <SettingRow label={t("settings.term.scrollback")}>
          <PopupSelect
            ariaLabel={t("settings.term.scrollback")}
            value={term.scrollback}
            minWidth={140}
            options={scrollbackValues.map((n) => ({ value: n, label: lines(n) }))}
            onChange={(scrollback) => updateTerminal({ scrollback })}
          />
        </SettingRow>
      </Group>

      <div className={s.preview} style={{ fontFamily: `"${font.split(",")[0].trim()}", var(--font-mono)`, fontSize: term.font_size }} aria-hidden>
        <div>
          <span className={s.dim}>$ </span>ssh deploy@prod-api-tokyo
        </div>
        <div>
          <span className={s.dim}>deploy@prod-api-tokyo:~$ </span>
          <span className={`${s.cursor} ${s[term.cursor_style]}`}>&nbsp;</span>
        </div>
      </div>
    </Pane>
  );
}

function Stepper({
  value,
  min,
  max,
  downLabel,
  upLabel,
  onChange,
}: {
  value: number;
  min: number;
  max: number;
  downLabel: string;
  upLabel: string;
  onChange: (v: number) => void;
}) {
  return (
    <div className={s.stepper}>
      <button type="button" aria-label={downLabel} title={downLabel} disabled={value <= min} onClick={() => onChange(value - 1)}>
        <Icon name="minus" />
      </button>
      <span className={s.stepValue} aria-live="polite">
        {value}
      </span>
      <button type="button" aria-label={upLabel} title={upLabel} disabled={value >= max} onClick={() => onChange(value + 1)}>
        <Icon name="plus" />
      </button>
    </div>
  );
}
