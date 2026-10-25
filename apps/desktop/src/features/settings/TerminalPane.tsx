import { useEffect, useRef, useState, type CSSProperties } from "react";
import { PopupSelect, Segmented, Stepper, Switch, TextField, isImeEvent } from "@hatoba/ui";
import { useApp } from "@/app/store";
import { loadLocalShells, shellName } from "@/features/terminal/localShells";
import { FONT_SIZE_MAX, FONT_SIZE_MIN, resolveScheme, terminalTheme, useTermSettings } from "@/features/terminal/theme";
import { useT } from "@/i18n";
import type { LocalShellView, TerminalSettings } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { defaultTerminalFont } from "@/lib/platform";
import { SchemePicker } from "./SchemePicker";
import { Pane, Section, SettingRow, usePrefs, type PaneProps } from "./shared";
import s from "./TerminalPane.module.css";

const SCROLLBACK = [1000, 5000, 10000, 50000];

/**
 * Settings → Terminal (TERM-05 / TERM-06): font, cursor and colour schemes with a preview of them, the
 * mouse and paste behaviour (WIN-05), scrollback, and the local tabs' shell (LOCAL-02).
 */
export function TerminalPane({ settings, updateTerminal }: PaneProps) {
  const t = useT();
  const [prefs, setPrefs] = usePrefs();
  const [shells, setShells] = useState<LocalShellView[] | null>(null);
  useEffect(() => {
    let live = true;
    loadLocalShells()
      .then((list) => live && setShells(list))
      .catch(() => live && setShells([]));
    return () => {
      live = false;
    };
  }, []);
  const platform = useApp((a) => a.info.platform);
  const defaultFont = defaultTerminalFont(platform);
  // "Match Appearance" previews the scheme of the app's current theme.
  useTermSettings((st) => st.appearanceTick);
  const term = settings?.terminal;
  const fontFamily = term?.font_family;
  const [font, setFont] = useState(fontFamily ?? defaultFont);
  /** What the font field showed when the user started typing in it; null while they are not. */
  const typedFrom = useRef<string | null>(null);
  useEffect(() => {
    // A font synced from another device shows, unless the user is typing in the field.
    if (fontFamily !== undefined && typedFrom.current === null) setFont(fontFamily);
  }, [fontFamily]);

  if (!term) return <Pane>{null}</Pane>;

  const commitFont = () => {
    const from = typedFrom.current;
    typedFrom.current = null;
    const next = font.trim() || defaultFont;
    // Typed and changed back: nothing to save, and the field shows the stored font again.
    if (next === from) return setFont(term.font_family);
    setFont(next);
    if (next !== term.font_family) updateTerminal({ font_family: next });
  };

  const scrollbackValues = SCROLLBACK.includes(term.scrollback)
    ? SCROLLBACK
    : [...SCROLLBACK, term.scrollback].sort((a, b) => a - b);
  const lines = (n: number) => t("settings.term.lines", { n: n.toLocaleString(t.locale) });
  /** Following the app appearance: a scheme for each mode. */
  const both = term.theme === "system";
  const scheme = resolveScheme(term);
  const colors = terminalTheme(scheme);

  return (
    <Pane>
      <Section title={t("settings.section.text")}>
        <SettingRow label={t("settings.term.font")}>
          <div className={s.fontField}>
            <TextField
              mono
              aria-label={t("settings.term.font")}
              value={font}
              placeholder={defaultFont}
              onChange={(e) => {
                typedFrom.current ??= font;
                setFont(e.target.value);
              }}
              onBlur={commitFont}
              onKeyDown={(e) => e.key === "Enter" && !isImeEvent(e) && commitFont()}
            />
          </div>
        </SettingRow>
        <SettingRow label={t("settings.term.size")}>
          <Stepper
            value={term.font_size}
            min={FONT_SIZE_MIN}
            max={FONT_SIZE_MAX}
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
      </Section>

      <div
        className={cx(s.preview, s[scheme.mode])}
        style={
          {
            fontFamily: `"${font.split(",")[0].trim()}", var(--font-mono)`,
            fontSize: term.font_size,
            background: colors.background,
            color: colors.foreground,
            "--preview-dim": scheme.colors.brightBlack,
            "--preview-cursor": scheme.colors.cursor,
          } as CSSProperties
        }
        aria-hidden
      >
        <div>
          <span className={s.dim}>$ </span>ssh deploy@prod-api-tokyo
        </div>
        <div>
          <span className={s.dim}>deploy@prod-api-tokyo:~$ </span>
          <span className={`${s.cursor} ${s[term.cursor_style]}`}>&nbsp;</span>
        </div>
      </div>

      <Section title={t("settings.section.colors")}>
        <SettingRow label={t("settings.termColors.mode")}>
          <Segmented
            ariaLabel={t("settings.termColors.mode")}
            value={term.theme}
            options={[
              { value: "dark", label: t("settings.termColors.dark") },
              { value: "light", label: t("settings.termColors.light") },
              { value: "system", label: t("settings.termColors.follow") },
            ]}
            onChange={(theme) => updateTerminal({ theme })}
          />
        </SettingRow>
        {term.theme !== "light" && (
          <SchemePicker
            mode="dark"
            value={term.dark_scheme}
            label={t("settings.termColors")}
            caption={both ? t("settings.termColors.forDark") : undefined}
            onChange={(dark_scheme) => updateTerminal({ dark_scheme })}
          />
        )}
        {term.theme !== "dark" && (
          <SchemePicker
            mode="light"
            value={term.light_scheme}
            label={t("settings.termColors")}
            caption={both ? t("settings.termColors.forLight") : undefined}
            onChange={(light_scheme) => updateTerminal({ light_scheme })}
          />
        )}
      </Section>

      <Section title={t("settings.section.behavior")}>
        <SettingRow label={t("settings.rightClick")}>
          <PopupSelect<TerminalSettings["right_click"]>
            ariaLabel={t("settings.rightClick")}
            value={term.right_click}
            minWidth={230}
            options={[
              { value: "copy_paste", label: t("settings.rightClick.copyPaste") },
              { value: "menu", label: t("settings.rightClick.menu") },
            ]}
            onChange={(right_click) => updateTerminal({ right_click })}
          />
        </SettingRow>
        <SettingRow label={t("settings.confirmPaste")}>
          <Switch
            label={t("settings.confirmPaste")}
            checked={term.confirm_multiline_paste}
            onChange={(confirm_multiline_paste) => updateTerminal({ confirm_multiline_paste })}
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
        {shells && shells.length > 0 && (
          <LocalShellRow
            shells={shells}
            value={prefs.local_shell}
            onChange={(local_shell) => setPrefs({ local_shell })}
          />
        )}
      </Section>
    </Pane>
  );
}

/**
 * LOCAL-02: the shell new local tabs run on this device. A shell that is gone, such as an uninstalled
 * WSL distribution, shows as the system default, which new tabs then run.
 */
function LocalShellRow({
  shells,
  value,
  onChange,
}: {
  shells: LocalShellView[];
  value: string | null;
  onChange(id: string | null): void;
}) {
  const t = useT();
  const system = shells.find((s) => s.default) ?? shells[0];
  const current = shells.some((s) => s.id === value) ? (value ?? "") : "";
  const options = [
    { value: "", label: t("settings.term.localShellDefault", { name: shellName(system, t) }) },
    ...shells.map((shell) => ({ value: shell.id, label: shellName(shell, t) })),
  ];
  return (
    <SettingRow label={t("settings.term.localShell")}>
      <PopupSelect
        ariaLabel={t("settings.term.localShell")}
        value={current}
        minWidth={200}
        options={options}
        onChange={(id) => onChange(id === "" ? null : id)}
      />
    </SettingRow>
  );
}
