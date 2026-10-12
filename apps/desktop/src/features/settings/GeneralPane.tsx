import { useEffect, useState } from "react";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { Button, Switch } from "@/components/controls";
import { Group } from "@/components/layout";
import { PopupSelect, toast } from "@/components/overlay";
import { useT } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { CliCommandView, TerminalSettings } from "@/ipc/types";
import { pickSavePath } from "@/lib/native";
import { defaultRightClick } from "@/lib/platform";
import { Pane, SettingRow, usePrefs, type PaneProps } from "./shared";
import s from "./shared.module.css";

/**
 * Settings → General: terminal mouse behaviour (WIN-05), reachability dots (HOST-10), the `hatoba`
 * command (HOST-13), backup (VAULT-07).
 */
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
        <CliCommandRow />
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

/** HOST-13: the `hatoba` command. It lives in the system, so the row asks the backend for its state. */
function CliCommandRow() {
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

  const toggle = async (enabled: boolean) => {
    if (!cli) return;
    setBusy(true);
    try {
      setCli(await api.cli_command_set(enabled));
    } catch (e) {
      const err = toAppError(e);
      // macOS: the administrator password prompt was closed.
      if (err.code === "cancelled") return;
      toast(err.field === "command" ? t("settings.cli.taken", { location: cli.location }) : errorMessage(t, e), "error");
    } finally {
      setBusy(false);
    }
  };

  const hint = (
    <>
      {t("settings.cli.hint")}
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
    <SettingRow label={t("settings.cli")} hint={hint}>
      <Switch label={t("settings.cli")} checked={cli?.enabled ?? false} disabled={!cli || busy} onChange={(on) => void toggle(on)} />
    </SettingRow>
  );
}
