import { useApp } from "@/app/store";
import { useUpdate } from "@/app/update";
import { AppLogo, Button, Icon, LinkButton, Switch, controlStyles } from "@/components/controls";
import { Group } from "@/components/layout";
import { toast } from "@/components/overlay";
import { copyText } from "@/features/keys/clipboard";
import { openExternal } from "@/features/sync/external";
import { useT } from "@/i18n";
import { REPO_URL, bugReportUrl } from "@/lib/github";
import { Pane, SettingRow, usePrefs } from "./shared";
import s from "./AboutPane.module.css";

const LICENSE_URL = `${REPO_URL}/blob/main/LICENSE`;

/** Settings → About: the version, the update check (spec §11), and links to the source, license, and bug report form. */
export function AboutPane() {
  const t = useT();
  const version = useApp((st) => st.info.version);
  const platform = useApp((st) => st.info.platform);
  const [prefs, setPrefs] = usePrefs();
  const { checking, result, error, check } = useUpdate();
  const update = !checking && !error && result?.update_available ? result : null;

  const copyVersion = async () => {
    try {
      await copyText(version);
      toast(t("btn.copied"), "success");
    } catch {
      toast(t("err.internal"), "error");
    }
  };

  const status = checking
    ? { icon: "circle-notch", color: "var(--fg2)", text: t("settings.about.checking") }
    : error
      ? {
          icon: "warning-circle",
          color: "var(--red)",
          text: t(error.code === "sync_offline" ? "settings.about.offline" : "settings.about.failed"),
        }
      : update
        ? { icon: "arrow-circle-up", color: "var(--accent)", text: t("settings.about.available", { version: update.latest_version ?? "" }) }
        : result
          ? { icon: "check-circle", color: "var(--green)", text: t("settings.about.latest") }
          : null;

  return (
    <Pane>
      <div className={s.header}>
        <AppLogo className={s.logo} />
        <div className={s.name}>Hatoba</div>
        <button type="button" className={s.version} title={t("settings.about.copyVersion")} onClick={() => void copyVersion()}>
          {t("settings.about.version", { version })}
        </button>
      </div>

      <Group>
        <SettingRow
          label={t("settings.about.update")}
          hint={
            status ? (
              <span className={s.status} role="status">
                <Icon name={status.icon} color={status.color} className={checking ? controlStyles.spinner : undefined} />
                {status.text}
              </span>
            ) : (
              t("settings.about.update.hint")
            )
          }
        >
          {update?.release_url && (
            <Button variant="primary" trailingIcon="arrow-up-right" onClick={() => void openExternal(update.release_url!)}>
              {t("settings.about.download")}
            </Button>
          )}
          <Button disabled={checking} onClick={() => void check()}>
            {t("settings.about.check")}
          </Button>
        </SettingRow>
        <SettingRow label={t("settings.about.auto")} hint={t("settings.about.auto.hint")}>
          <Switch
            label={t("settings.about.auto")}
            checked={prefs.auto_update_check}
            onChange={(auto_update_check) => setPrefs({ auto_update_check })}
          />
        </SettingRow>
      </Group>

      <div className={s.links}>
        <LinkButton icon="github-logo" onClick={() => void openExternal(REPO_URL)}>
          {t("settings.about.source")}
        </LinkButton>
        <LinkButton icon="scales" onClick={() => void openExternal(LICENSE_URL)}>
          {t("settings.about.license")}
        </LinkButton>
        <LinkButton icon="bug" onClick={() => void openExternal(bugReportUrl({ version, platform }))}>
          {t("settings.about.report")}
        </LinkButton>
      </div>
    </Pane>
  );
}
