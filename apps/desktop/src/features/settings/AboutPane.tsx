import { useApp } from "@/app/store";
import { useTabs } from "@/app/tabs";
import { useUpdate } from "@/app/update";
import { AppLogo, Button, Icon, LinkButton, Switch, controlStyles } from "@/components/controls";
import { Group } from "@/components/layout";
import { confirm, toast } from "@/components/overlay";
import { Markdown } from "@/features/ai/Markdown";
import { copyText } from "@/features/keys/clipboard";
import { openExternal } from "@/features/sync/external";
import { formatBytes, formatDate, useT, type MessageKey, type Params } from "@/i18n";
import type { AppError, AvailableUpdate, UpdateProgress } from "@/ipc/types";
import { REPO_URL, bugReportUrl } from "@/lib/github";
import { Pane, SettingRow, usePrefs } from "./shared";
import s from "./AboutPane.module.css";

const LICENSE_URL = `${REPO_URL}/blob/main/LICENSE`;

type T = (key: MessageKey, params?: Params) => string;
type Install = UpdateProgress | { kind: "starting" };
interface Status {
  icon: string;
  color: string;
  text: string;
}

function installText(t: T, install: Install): string {
  if (install.kind === "installing") return t("settings.about.installing");
  if (install.kind === "starting") return t("settings.about.downloadStarting");
  const done = formatBytes(install.downloaded);
  return install.total
    ? t("settings.about.downloading", { done, total: formatBytes(install.total) })
    : t("settings.about.downloadingSize", { done });
}

function installErrorKey(error: AppError): MessageKey {
  if (error.code === "sync_offline") return "settings.about.offline";
  if (error.code === "update_signature") return "err.update_signature";
  return "settings.about.installFailed";
}

/** The update row's status line, or null before the first check. */
function updateStatus(
  t: T,
  { checking, error, install, installError, found, checked }: {
    checking: boolean;
    error: AppError | null;
    install: Install | null;
    installError: AppError | null;
    found: AvailableUpdate | null;
    checked: boolean;
  },
): Status | null {
  const busy = (text: string) => ({ icon: "circle-notch", color: "var(--fg2)", text });
  const failed = (text: string) => ({ icon: "warning-circle", color: "var(--red)", text });
  if (install) return busy(installText(t, install));
  if (checking) return busy(t("settings.about.checking"));
  if (error) return failed(t(error.code === "sync_offline" ? "settings.about.offline" : "settings.about.failed"));
  if (installError) return failed(t(installErrorKey(installError)));
  if (found) return { icon: "arrow-circle-up", color: "var(--accent)", text: t("settings.about.available", { version: found.version }) };
  if (checked) return { icon: "check-circle", color: "var(--green)", text: t("settings.about.latest") };
  return null;
}

/** Percent downloaded, or null while the size is unknown. */
function downloadPercent(install: Install): number | null {
  if (install.kind === "installing") return 100;
  if (install.kind !== "downloading" || !install.total) return null;
  return Math.min(100, Math.round((install.downloaded / install.total) * 100));
}

/**
 * Settings → About: the version, signed updates (spec §11), and links to the source, license, and
 * bug report form.
 */
export function AboutPane() {
  const t = useT();
  const version = useApp((st) => st.info.version);
  const platform = useApp((st) => st.info.platform);
  const [prefs, setPrefs] = usePrefs();
  const { checking, result, error, install, installError, check, installUpdate } = useUpdate();
  const update = !checking && !error ? (result?.update ?? null) : null;

  const copyVersion = async () => {
    try {
      await copyText(version);
      toast(t("btn.copied"), "success");
    } catch {
      toast(t("err.internal"), "error");
    }
  };

  const confirmInstall = async (target: AvailableUpdate) => {
    const n = useTabs.getState().tabs.filter((tab) => tab.status === "connected").length;
    const ok = await confirm({
      title: t("settings.about.install.title", { version: target.version }),
      body: n > 0 ? t("settings.about.install.bodySessions", { n }) : t("settings.about.install.body"),
      confirmLabel: t("settings.about.install.confirm"),
      icon: "arrow-circle-up",
    });
    if (ok) await installUpdate();
  };

  const spinning = checking || install !== null;
  const status = updateStatus(t, { checking, error, install, installError, found: update, checked: result !== null });
  const percent = install && downloadPercent(install);

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
              <span className={s.progress}>
                <span className={s.status} role="status">
                  <Icon name={status.icon} color={status.color} className={spinning ? controlStyles.spinner : undefined} />
                  {status.text}
                </span>
                {install && (
                  <span className={s.track} role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent ?? undefined}>
                    <span className={percent === null ? `${s.fill} ${s.fillUnknown}` : s.fill} style={percent === null ? undefined : { width: `${percent}%` }} />
                  </span>
                )}
              </span>
            ) : (
              t("settings.about.update.hint")
            )
          }
        >
          {update && !install && (
            <Button variant="primary" onClick={() => void confirmInstall(update)}>
              {t("settings.about.install")}
            </Button>
          )}
          <Button disabled={spinning} onClick={() => void check()}>
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

      {update && (
        <section className={s.notes} aria-label={t("settings.about.notes", { version: update.version })}>
          <div className={s.notesHead}>
            <span className={s.notesTitle}>{t("settings.about.notes", { version: update.version })}</span>
            {update.published_at !== null && (
              <span className={s.notesDate}>{t("settings.about.published", { date: formatDate(update.published_at) })}</span>
            )}
          </div>
          {update.notes && (
            <div className={s.notesBody}>
              <Markdown text={update.notes} />
            </div>
          )}
          <LinkButton icon="arrow-up-right" onClick={() => void openExternal(update.release_url)}>
            {t("settings.about.releasePage")}
          </LinkButton>
        </section>
      )}

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
