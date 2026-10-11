import { useState } from "react";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { Button, Switch } from "@/components/controls";
import { Group } from "@/components/layout";
import { PopupSelect, toast } from "@/components/overlay";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import { shortcutLabel } from "@/lib/platform";
import { ChangePasswordDialog, NewRecoveryDialog, VerifyPasswordDialog } from "./dialogs";
import { Pane, SettingRow, type PaneProps } from "./shared";
import s from "./SecurityPane.module.css";

const AUTO_LOCK_MINUTES = [1, 5, 15, 30, 60, 0];

type Dialog = null | "hello" | "recovery" | "password";

/** Settings → Security: auto-lock (SEC-02/03), Windows Hello (SEC-07), password (VAULT-05), recovery code, lock. */
export function SecurityPane({ settings, update }: PaneProps) {
  const t = useT();
  const platform = useApp((st) => st.info.platform);
  const vault = useApp((st) => st.vault);
  const [dialog, setDialog] = useState<Dialog>(null);
  const [newCode, setNewCode] = useState<string | null>(null);

  const minutesLabel = (m: number) => (m === 0 ? t("settings.sec.never") : m === 60 ? t("settings.sec.hour") : t("settings.sec.minutes", { n: m }));
  const autoLockValues = settings && !AUTO_LOCK_MINUTES.includes(settings.auto_lock_minutes) ? [...AUTO_LOCK_MINUTES, settings.auto_lock_minutes] : AUTO_LOCK_MINUTES;

  const mac = platform === "macos";
  const biometricLabel = mac ? t("settings.sec.touchId") : t("settings.sec.hello");

  const setBiometric = async (on: boolean) => {
    if (on) return setDialog("hello");
    try {
      await api.biometric_disable();
      await useApp.getState().refreshVault();
    } catch (e) {
      toast(errorMessage(t, e), "error");
    }
  };

  return (
    <Pane>
      {settings && (
        <Group>
          <SettingRow label={t("settings.sec.autoLock")} hint={t("settings.sec.autoLock.hint")}>
            <PopupSelect
              ariaLabel={t("settings.sec.autoLock")}
              value={settings.auto_lock_minutes}
              minWidth={140}
              options={autoLockValues.map((m) => ({ value: m, label: minutesLabel(m) }))}
              onChange={(auto_lock_minutes) => update({ auto_lock_minutes })}
            />
          </SettingRow>
          <SettingRow label={t("settings.sec.disconnect")} hint={t("settings.sec.disconnect.hint")}>
            <Switch
              label={t("settings.sec.disconnect")}
              checked={settings.lock_disconnects_sessions}
              onChange={(lock_disconnects_sessions) => update({ lock_disconnects_sessions })}
            />
          </SettingRow>
          {vault?.biometric_available && (
            <SettingRow label={biometricLabel} hint={t("settings.sec.biometric.hint")}>
              <Switch label={biometricLabel} checked={vault.biometric_enabled} onChange={(on) => void setBiometric(on)} />
            </SettingRow>
          )}
        </Group>
      )}

      <Group>
        <SettingRow label={t("settings.sec.password")}>
          <Button icon="lock-key" onClick={() => setDialog("password")}>
            {t("settings.sec.password.button")}
          </Button>
        </SettingRow>
        <SettingRow label={t("settings.sec.recovery")}>
          <Button icon="shield-check" onClick={() => setDialog("recovery")}>
            {t("settings.sec.recovery.button")}
          </Button>
        </SettingRow>
      </Group>

      <Group>
        <SettingRow label={t("settings.sec.lockNow")} hint={t("settings.sec.lockNow.hint")}>
          <Button icon="lock-simple" onClick={() => void useApp.getState().lock()}>
            {t("settings.sec.lockNow.button")}
            <span className={s.kbd}>{shortcutLabel(platform, "Ctrl+Shift+L", "⌘L")}</span>
          </Button>
        </SettingRow>
      </Group>

      {dialog === "hello" && (
        <VerifyPasswordDialog
          body={mac ? t("settings.verify.touchId") : t("settings.verify.hello")}
          onClose={() => setDialog(null)}
          onVerify={async (pw) => {
            await api.biometric_enable(pw);
            await useApp.getState().refreshVault();
          }}
        />
      )}
      {dialog === "recovery" && (
        <VerifyPasswordDialog
          body={t("settings.verify.recovery")}
          onClose={() => setDialog(null)}
          onVerify={async (pw) => setNewCode(await api.vault_rotate_recovery(pw))}
        />
      )}
      {dialog === "password" && <ChangePasswordDialog onClose={() => setDialog(null)} />}
      {newCode && <NewRecoveryDialog code={newCode} onClose={() => setNewCode(null)} />}
    </Pane>
  );
}
