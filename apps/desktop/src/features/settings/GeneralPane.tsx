import { useState } from "react";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { Button, Switch } from "@/components/controls";
import { Group } from "@/components/layout";
import { PopupSelect, toast } from "@/components/overlay";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { TerminalSettings } from "@/ipc/types";
import { pickSavePath } from "@/lib/native";
import { defaultRightClick } from "@/lib/platform";
import { Pane, SettingRow, usePrefs, type PaneProps } from "./shared";

/** Seconds between the dashboard's readings (HOST-13). */
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

/** Settings → General: terminal mouse behaviour (WIN-05), reachability dots (HOST-10), the dashboard (HOST-13), backup (VAULT-07). */
export function GeneralPane({ settings, updateTerminal }: PaneProps) {
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
        <SettingRow label={t("settings.rightClick")}>
          <PopupSelect<TerminalSettings["right_click"]>
            ariaLabel={t("settings.rightClick")}
            value={settings?.terminal.right_click ?? defaultRightClick(platform)}
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
            checked={settings?.terminal.confirm_multiline_paste ?? true}
            onChange={(confirm_multiline_paste) => updateTerminal({ confirm_multiline_paste })}
          />
        </SettingRow>
        <SettingRow label={t("settings.probe")} hint={t("settings.probe.hint")}>
          <Switch label={t("settings.probe")} checked={prefs.host_probe} onChange={(host_probe) => setPrefs({ host_probe })} />
        </SettingRow>
      </Group>

      <Group>
        <SettingRow label={t("dashboard.settings.interval")} hint={t("dashboard.settings.interval.hint")}>
          <PopupSelect<number>
            ariaLabel={t("dashboard.settings.interval")}
            value={prefs.dashboard_interval_secs}
            minWidth={110}
            options={withCurrent(INTERVALS, prefs.dashboard_interval_secs).map((n) => ({ value: n, label: t("dashboard.settings.seconds", { n }) }))}
            onChange={(dashboard_interval_secs) => setPrefs({ dashboard_interval_secs })}
          />
        </SettingRow>
        {ALERTS.map(([key, label]) => (
          <SettingRow key={key} label={t(label)} hint={key === "dashboard_disk_alert" ? t("dashboard.settings.alert.hint") : undefined}>
            <PopupSelect<number>
              ariaLabel={t(label)}
              value={prefs[key]}
              minWidth={110}
              options={withCurrent(THRESHOLDS, prefs[key]).map((n) => ({ value: n, label: n === 0 ? t("dashboard.settings.off") : `${n}%` }))}
              onChange={(value) => setPrefs({ [key]: value })}
            />
          </SettingRow>
        ))}
      </Group>

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
