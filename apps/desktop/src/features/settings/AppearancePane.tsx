import { useRef } from "react";
import { Icon, Segmented, controlStyles } from "@/components/controls";
import { Group } from "@/components/layout";
import { Menu, useMenu, type MenuEntry } from "@/components/overlay";
import { useT } from "@/i18n";
import type { Language, LocalPrefs } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { Pane, PaneHint, SettingRow, usePrefs, type PaneProps } from "./shared";
import s from "./AppearancePane.module.css";

type Appearance = LocalPrefs["appearance"];

/** Settings → Appearance (design §07): theme thumbnails, terminal colors, density, language. */
export function AppearancePane({ settings, updateTerminal }: PaneProps) {
  const t = useT();
  const [prefs, setPrefs] = usePrefs();
  const langMenu = useMenu();
  const langButton = useRef<HTMLButtonElement>(null);

  const languages: { value: Language; label: string }[] = [
    { value: "system", label: t("settings.language.system") },
    { value: "zh-CN", label: t("settings.language.zh") },
    { value: "en", label: t("settings.language.en") },
    { value: "ja", label: t("settings.language.ja") },
  ];
  const langEntries: MenuEntry[] = languages.flatMap((l, i): MenuEntry[] => [
    { label: l.label, checked: prefs.language === l.value, onSelect: () => setPrefs({ language: l.value }) },
    ...(i === 0 ? [{ kind: "separator" } as const] : []),
  ]);

  return (
    <Pane>
      <Group>
        <SettingRow top label={t("settings.appearance")}>
          <div className={s.thumbs} role="radiogroup" aria-label={t("settings.appearance")}>
            <Thumb kind="light" label={t("settings.appearance.light")} selected={prefs.appearance === "light"} onSelect={() => setPrefs({ appearance: "light" })} />
            <Thumb kind="dark" label={t("settings.appearance.dark")} selected={prefs.appearance === "dark"} onSelect={() => setPrefs({ appearance: "dark" })} />
            <Thumb kind="system" label={t("settings.appearance.auto")} selected={prefs.appearance === "system"} onSelect={() => setPrefs({ appearance: "system" })} />
          </div>
        </SettingRow>
        <SettingRow label={t("settings.termColors")}>
          <Segmented
            ariaLabel={t("settings.termColors")}
            value={settings?.terminal.theme === "dark" ? "dark" : "system"}
            options={[
              { value: "dark", label: t("settings.termColors.dark") },
              { value: "system", label: t("settings.termColors.follow") },
            ]}
            onChange={(theme) => updateTerminal({ theme })}
          />
        </SettingRow>
        <SettingRow label={t("settings.density")}>
          <Segmented
            ariaLabel={t("settings.density")}
            value={prefs.density}
            options={[
              { value: "regular", label: t("settings.density.regular") },
              { value: "compact", label: t("settings.density.compact") },
            ]}
            onChange={(density) => setPrefs({ density })}
          />
        </SettingRow>
      </Group>

      <Group>
        <SettingRow label={t("settings.language")}>
          <button
            ref={langButton}
            type="button"
            aria-haspopup="menu"
            aria-expanded={!!langMenu.anchor}
            aria-label={t("settings.language")}
            className={cx(controlStyles.popup, s.langButton)}
            onClick={() => langButton.current && langMenu.openBelow(langButton.current, true)}
          >
            <Icon name="translate" className={controlStyles.popupIcon} style={{ fontSize: 14 }} />
            <span className={controlStyles.popupLabel}>{languages.find((l) => l.value === prefs.language)?.label}</span>
            <Icon name="caret-up-down" className={controlStyles.popupCaret} />
          </button>
        </SettingRow>
      </Group>
      <PaneHint>{t("settings.language.hint")}</PaneHint>

      {langMenu.anchor && <Menu anchor={langMenu.anchor} onClose={langMenu.close} minWidth={200} entries={langEntries} />}
    </Pane>
  );
}

/** Miniature window used as the appearance picker (colors are fixed illustrations, not theme tokens). */
function Thumb({ kind, label, selected, onSelect }: { kind: Appearance; label: string; selected: boolean; onSelect: () => void }) {
  return (
    <button type="button" role="radio" aria-checked={selected} className={s.thumb} onClick={onSelect}>
      <span className={cx(s.preview, s[kind], selected && s.previewOn)}>
        {kind === "system" ? (
          <>
            <span className={cx(s.half, s.light)}>
              <span className={s.side} />
              <span className={s.lines}>
                <i style={{ width: "90%" }} />
                <i />
              </span>
            </span>
            <span className={cx(s.half, s.dark)}>
              <span className={s.lines}>
                <i style={{ width: "60%" }} />
                <i style={{ width: "80%" }} />
              </span>
            </span>
          </>
        ) : (
          <>
            <span className={s.side} />
            <span className={s.lines}>
              <i style={{ width: "70%" }} />
              <i style={{ width: "90%" }} />
              <i style={{ width: "60%" }} />
            </span>
          </>
        )}
      </span>
      <span className={cx(s.thumbLabel, selected && s.thumbLabelOn)}>{label}</span>
    </button>
  );
}
