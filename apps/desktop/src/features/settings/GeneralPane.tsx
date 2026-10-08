import { useState } from "react";
import { errorMessage } from "@/app/errors";
import { Button, Switch } from "@/components/controls";
import { Group } from "@/components/layout";
import { PopupSelect, toast } from "@/components/overlay";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { TerminalSettings } from "@/ipc/types";
import { pickSavePath } from "@/lib/native";
import { Pane, SettingRow, usePrefs, type PaneProps } from "./shared";

/** Settings → General: terminal mouse behaviour (WIN-05), reachability dots (HOST-10), backup (VAULT-07). */
export function GeneralPane({ settings, updateTerminal }: PaneProps) {
  const t = useT();
  const [prefs, setPrefs] = usePrefs();
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
            value={settings?.terminal.right_click ?? "copy_paste"}
            minWidth={230}
            options={[
              { value: "copy_paste", label: t("settings.rightClick.copyPaste") },
              { value: "menu", label: t("settings.rightClick.menu") },
            ]}
            onChange={(right_click) => updateTerminal({ right_click })}
          />
        </SettingRow>
        <SettingRow label={t("settings.confirmPaste")} hint={t("settings.confirmPaste.hint")}>
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
        <SettingRow label={t("settings.backup")} hint={t("settings.backup.hint")}>
          <Button icon="download-simple" busy={exporting} onClick={() => void exportBackup()}>
            {t("settings.backup.button")}
          </Button>
        </SettingRow>
      </Group>
    </Pane>
  );
}
