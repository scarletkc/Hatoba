import { useEffect, useRef, useState } from "react";
import {
  Button,
  Group,
  Icon,
  Menu,
  PopupSelect,
  Switch,
  controlStyles,
  toast,
  useMenu,
  type MenuEntry,
} from "@hatoba/ui";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { formatDateTime, formatRelative, useT } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { CliActivity, CliCommandView, CloseAction, Language } from "@/ipc/types";
import { copyText } from "@/features/keys/clipboard";
import { cx } from "@/lib/cx";
import { pickSavePath } from "@/lib/native";
import { Pane, Section, SettingRow, usePrefs } from "./shared";
import s from "./shared.module.css";

/** Seconds between the dashboard's readings (HOST-14). */
const INTERVALS = [1, 2, 3, 5, 10, 30, 60];
/** The thresholds offered, in percent; 0 is off. */
const THRESHOLDS = [0, 70, 75, 80, 85, 90, 95];
const ALERTS = [
  ["dashboard_cpu_alert", "dashboard.settings.cpuAlert"],
  ["dashboard_mem_alert", "dashboard.settings.memAlert"],
  ["dashboard_disk_alert", "dashboard.settings.diskAlert"],
] as const;

/** The choices, with a stored value that is not one of them (set by another build) kept in its place. */
function withCurrent(choices: number[], current: number): number[] {
  return choices.includes(current) ? choices : [...choices, current].sort((a, b) => a - b);
}

/**
 * Settings → General: the language, reachability dots (HOST-10), the close button (WIN-10), AI
 * notifications (AI-07), the `hatoba` command (HOST-13) and other programs' use of the saved hosts
 * through it (HOST-15), the dashboard (HOST-14), backup (VAULT-07).
 */
export function GeneralPane() {
  const t = useT();
  const [prefs, setPrefs] = usePrefs();
  const platform = useApp((a) => a.info.platform);
  const [exporting, setExporting] = useState(false);

  const exportBackup = async () => {
    setExporting(true);
    try {
      const d = new Date();
      const p = (n: number) => String(n).padStart(2, "0");
      const name = `hatoba-backup-${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}.hatoba`;
      const path = await pickSavePath(name, { name: "Hatoba Backup", extension: "hatoba" });
      if (!path) return;
      await api.vault_export_backup(path);
      toast(t("settings.backup.done"), "success");
    } catch (e) {
      toast(errorMessage(t, e), "error");
    } finally {
      setExporting(false);
    }
  };

  return (
    <Pane>
      <Group>
        <LanguageRow />
      </Group>

      {/* macOS has no close-button row, which leaves the notifications. */}
      <Section title={t(platform === "macos" ? "settings.section.notifications" : "settings.section.window")}>
        {/* WIN-10: macOS's close button hides the window (MAC-06). */}
        {platform !== "macos" && (
          <SettingRow label={t("settings.closeAction")} hint={t("settings.closeAction.hint")}>
            <PopupSelect<CloseAction>
              ariaLabel={t("settings.closeAction")}
              value={prefs.close_action}
              minWidth={150}
              options={[
                { value: "ask", label: t("settings.closeAction.ask") },
                { value: "tray", label: t("settings.closeAction.tray") },
                { value: "quit", label: t("settings.closeAction.quit") },
              ]}
              onChange={(close_action) => setPrefs({ close_action })}
            />
          </SettingRow>
        )}
        <SettingRow label={t("settings.aiNotifications")} hint={t("settings.aiNotifications.hint")}>
          <Switch
            label={t("settings.aiNotifications")}
            checked={prefs.ai_notifications}
            onChange={(ai_notifications) => setPrefs({ ai_notifications })}
          />
        </SettingRow>
      </Section>

      <Section title={t("settings.section.cli")}>
        <CliCommandRows />
      </Section>

      <Section title={t("settings.section.monitoring")}>
        <SettingRow label={t("settings.probe")} hint={t("settings.probe.hint")}>
          <Switch
            label={t("settings.probe")}
            checked={prefs.host_probe}
            onChange={(host_probe) => setPrefs({ host_probe })}
          />
        </SettingRow>
        <SettingRow label={t("dashboard.settings.interval")} hint={t("dashboard.settings.interval.hint")}>
          <PopupSelect<number>
            ariaLabel={t("dashboard.settings.interval")}
            value={prefs.dashboard_interval_secs}
            minWidth={110}
            options={withCurrent(INTERVALS, prefs.dashboard_interval_secs).map((n) => ({
              value: n,
              label: t("dashboard.settings.seconds", { n }),
            }))}
            onChange={(dashboard_interval_secs) => setPrefs({ dashboard_interval_secs })}
          />
        </SettingRow>
        {ALERTS.map(([key, label]) => (
          <SettingRow
            key={key}
            label={t(label)}
            hint={key === "dashboard_disk_alert" ? t("dashboard.settings.alert.hint") : undefined}
          >
            <PopupSelect<number>
              ariaLabel={t(label)}
              value={prefs[key]}
              minWidth={110}
              options={withCurrent(THRESHOLDS, prefs[key]).map((n) => ({
                value: n,
                label: n === 0 ? t("dashboard.settings.off") : `${n}%`,
              }))}
              onChange={(value) => setPrefs({ [key]: value })}
            />
          </SettingRow>
        ))}
      </Section>

      <Group>
        <SettingRow label={t("settings.backup")} hint={t("settings.backup.hint")}>
          <Button icon="download-simple" busy={exporting} onClick={() => void exportBackup()}>
            {t("settings.backup.button")}
          </Button>
        </SettingRow>
      </Group>
    </Pane>
  );
}

/** The interface language, a device-local preference. */
function LanguageRow() {
  const t = useT();
  const [prefs, setPrefs] = usePrefs();
  const menu = useMenu();
  const button = useRef<HTMLButtonElement>(null);
  const languages: { value: Language; label: string }[] = [
    { value: "system", label: t("settings.language.system") },
    { value: "zh-CN", label: t("settings.language.zh") },
    { value: "zh-TW", label: t("settings.language.zhTW") },
    { value: "en", label: t("settings.language.en") },
    { value: "ja", label: t("settings.language.ja") },
  ];
  const entries: MenuEntry[] = languages.flatMap((l, i): MenuEntry[] => [
    { label: l.label, checked: prefs.language === l.value, onSelect: () => setPrefs({ language: l.value }) },
    ...(i === 0 ? [{ kind: "separator" } as const] : []),
  ]);
  return (
    <SettingRow label={t("settings.language")}>
      <button
        ref={button}
        type="button"
        aria-haspopup="menu"
        aria-expanded={!!menu.anchor}
        aria-label={t("settings.language")}
        className={cx(controlStyles.popup, s.langButton)}
        onClick={() => button.current && menu.openBelow(button.current, true)}
      >
        <Icon name="translate" className={controlStyles.popupIcon} style={{ fontSize: 14 }} />
        <span className={controlStyles.popupLabel}>{languages.find((l) => l.value === prefs.language)?.label}</span>
        <Icon name="caret-up-down" className={controlStyles.popupCaret} />
      </button>
      {menu.anchor && <Menu anchor={menu.anchor} onClose={menu.close} minWidth={200} entries={entries} />}
    </SettingRow>
  );
}

/** Installs the repository's `skills/hatoba-cli` for the user's coding agents (HOST-15). */
const SKILL_INSTALL = "npx skills add scarletkc/Hatoba --skill hatoba-cli -g";

/**
 * HOST-13: the `hatoba` command, and under it HOST-15's switch, the agent skill and what programs did
 * through it. The command lives in the system, so the rows ask the backend for its state.
 */
function CliCommandRows() {
  const t = useT();
  const platform = useApp((a) => a.info.platform);
  const [cli, setCli] = useState<CliCommandView | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let live = true;
    api
      .cli_command_get()
      .then((view) => live && setCli(view))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);

  const toggle = async (change: () => Promise<CliCommandView>) => {
    if (!cli) return;
    setBusy(true);
    try {
      setCli(await change());
    } catch (e) {
      const err = toAppError(e);
      // macOS: the administrator password prompt was closed.
      if (err.code === "cancelled") return;
      // `console`: on Windows, the console program is not beside the app.
      const message =
        err.field === "command"
          ? t("settings.cli.taken", { location: cli.location })
          : err.field === "console"
            ? t("settings.cli.missing", { path: err.detail })
            : errorMessage(t, e);
      toast(message, "error");
    } finally {
      setBusy(false);
    }
  };

  const copySkillInstall = async () => {
    try {
      await copyText(SKILL_INSTALL);
      toast(t("btn.copied"), "success");
    } catch {
      toast(t("err.internal"), "error");
    }
  };

  const hint = (
    <>
      {t("settings.cli.hint")}
      {/* HOST-15's switch shows only while the command is on, so say what it adds. */}
      {cli && !cli.enabled && (
        <>
          <br />
          {t("settings.cli.agents")}
        </>
      )}
      {cli?.enabled && (
        <>
          <br />
          {t(platform === "windows" ? "settings.cli.onPath" : "settings.cli.added", { location: cli.location })}
        </>
      )}
      {cli?.enabled && !cli.on_path && (
        <>
          <br />
          <span className={s.warn}>{t("settings.cli.offPath")}</span>
        </>
      )}
    </>
  );

  return (
    <>
      <SettingRow label={t("settings.cli")} hint={hint}>
        <Switch
          label={t("settings.cli")}
          checked={cli?.enabled ?? false}
          disabled={!cli || busy}
          onChange={(on) => void toggle(() => api.cli_command_set(on))}
        />
      </SettingRow>
      {cli?.enabled && (
        <SettingRow label={t("settings.cliAccess")} hint={t("settings.cliAccess.hint")}>
          <Switch
            label={t("settings.cliAccess")}
            checked={cli.access}
            disabled={busy}
            onChange={(on) => void toggle(() => api.cli_access_set(on))}
          />
        </SettingRow>
      )}
      {cli?.enabled && cli.access && (
        <SettingRow
          label={t("settings.cliAccess.skill")}
          hint={
            <>
              {t("settings.cliAccess.skill.hint")}
              <br />
              <span className={cx(s.command, "selectable")}>{SKILL_INSTALL}</span>
            </>
          }
        >
          <Button onClick={() => void copySkillInstall()}>{t("settings.about.copyCommand")}</Button>
        </SettingRow>
      )}
      {cli?.enabled && cli.access && <CliActivityList />}
    </>
  );
}

const ACTIVITY_ICON: Record<CliActivity["kind"], string> = {
  exec: "terminal-window",
  upload: "upload-simple",
  download: "download-simple",
};

/** HOST-15: what programs did through the command since the unlock, newest first, under **Details**. */
function CliActivityList() {
  const t = useT();
  const [entries, setEntries] = useState<CliActivity[] | null>(null);
  const [details, setDetails] = useState(false);

  useEffect(() => {
    let live = true;
    // Listening first, so no use is missed; an event is newer than the list.
    let pushed = false;
    const sub = api.listen("cli://activity", (e) => {
      pushed = true;
      setEntries(e.entries);
    });
    void sub
      .then(() => api.cli_activity_list())
      .then((list) => live && !pushed && setEntries(list))
      .catch(() => {});
    return () => {
      live = false;
      void sub.then((un) => un());
    };
  }, []);

  const outcome = (a: CliActivity) => {
    if (a.state === "running") return t("settings.cliAccess.running");
    if (a.state === "failed") return a.error ?? "";
    return a.kind === "exec" ? t("settings.cliAccess.exit", { status: a.status ?? 0 }) : t("settings.cliAccess.copied");
  };

  return (
    <>
      <SettingRow
        label={t("settings.cliAccess.activity")}
        hint={
          entries === null
            ? undefined
            : entries.length === 0
              ? t("settings.cliAccess.activity.empty")
              : t("settings.cliAccess.activity.hint", { n: entries.length })
        }
      >
        {!!entries?.length && (
          <Button
            size="xs"
            trailingIcon={details ? "caret-up" : "caret-down"}
            aria-expanded={details}
            onClick={() => setDetails((open) => !open)}
          >
            {t("settings.cliAccess.activity.details")}
          </Button>
        )}
      </SettingRow>
      {details &&
        entries?.map((a) => (
          <div key={a.id} className={s.activity}>
            <Icon name={ACTIVITY_ICON[a.kind]} className={s.activityIcon} />
            <span className={s.activityText}>
              <span className={s.activityWhat}>{a.detail}</span>
              <span className={s.activityMeta} title={formatDateTime(t.locale, a.started_at)}>
                {a.host_name} · {formatRelative(t.locale, a.started_at)} ·{" "}
                <span className={a.state === "failed" ? s.warn : undefined}>{outcome(a)}</span>
              </span>
            </span>
          </div>
        ))}
    </>
  );
}
