import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { setAiTextSize } from "@/app/zoomKeys";
import { Segmented, Stepper } from "@/components/controls";
import { Group, layoutStyles } from "@/components/layout";
import { PopupSelect, toast } from "@/components/overlay";
import { useAi } from "@/features/ai/store";
import { AI_TEXT_SIZE_MAX, AI_TEXT_SIZE_MIN } from "@/features/ai/textSize";
import { useT } from "@/i18n";
import type { LocalPrefs } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { ZOOM_STEPS } from "@/lib/zoom";
import { Pane, SettingRow, usePrefs } from "./shared";
import s from "./AppearancePane.module.css";

type Appearance = LocalPrefs["appearance"];

/** Settings → Appearance (design §07): theme thumbnails, density and zoom, and the AI panel's text size (TERM-13). */
export function AppearancePane() {
  const t = useT();
  const [prefs, setPrefs] = usePrefs();
  const zoom = useApp((st) => st.zoom);
  const aiTextSize = useAi((st) => st.catalog.settings?.text_size);

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
        <SettingRow label={t("settings.zoom")} hint={zoom.max < ZOOM_STEPS[ZOOM_STEPS.length - 1] ? t("settings.zoom.max", { n: zoom.max }) : undefined}>
          <PopupSelect
            ariaLabel={t("settings.zoom")}
            value={zoom.zoom}
            minWidth={100}
            options={ZOOM_STEPS.filter((z) => z <= zoom.max).map((z) => ({ value: z, label: `${z}%` }))}
            onChange={(z) => void useApp.getState().setZoom(z).catch((e) => toast(errorMessage(t, e), "error"))}
          />
        </SettingRow>
      </Group>

      {aiTextSize !== undefined && (
        <section className={layoutStyles.section}>
          <div className={layoutStyles.sectionTitle}>{t("settings.aiPanel")}</div>
          <Group>
            <SettingRow label={t("settings.aiText")} hint={t("settings.aiText.hint")}>
              <Stepper
                value={aiTextSize}
                min={AI_TEXT_SIZE_MIN}
                max={AI_TEXT_SIZE_MAX}
                downLabel={t("settings.aiText.down")}
                upLabel={t("settings.aiText.up")}
                onChange={setAiTextSize}
              />
            </SettingRow>
          </Group>
        </section>
      )}
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
