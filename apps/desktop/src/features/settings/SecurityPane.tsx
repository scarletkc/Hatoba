import { useState } from "react";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { Button, Switch } from "@/components/controls";
import { Group } from "@/components/layout";
import { PopupSelect, toast } from "@/components/overlay";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import { shortcutLabel } from "@/lib/platform";
import { ChangePasswordDialog, NewRecoveryDialog, SetPinDialog, VerifyPasswordDialog } from "./dialogs";
import { durationLabel } from "./durations";
import { Pane, SettingRow, usePrefs } from "./shared";
import s from "./SecurityPane.module.css";

const AUTO_LOCK_MINUTES = [1, 5, 15, 30, 60, 240, 480, 0];
const CLIPBOARD_CLEAR_SECS = [30, 60, 90, 120, 300, 0];

type Dialog = null | "hello" | "pin" | "recovery" | "password";

/** The choices, plus the stored value when it is none of them. */
const withValue = (choices: number[], value: number) => (choices.includes(value) ? choices : [...choices, value]);

/**
 * Settings → Security: auto-lock (SEC-02/03) and clipboard clearing (SEC-08), kept per device, Windows Hello
 * (SEC-07), PIN (SEC-11), password (VAULT-05), recovery code, lock.
 */
export function SecurityPane() {
  const t = useT();
  const platform = useApp((st) => st.info.platform);
  const vault = useApp((st) => st.vault);
  const [prefs, setPrefs] = usePrefs();
  const [dialog, setDialog] = useState<Dialog>(null);
  const [newCode, setNewCode] = useState<string | null>(null);

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

  const setPin = async (on: boolean) => {
    if (on) return setDialog("pin");
    try {
      await api.pin_disable();
      await useApp.getState().refreshVault();
    } catch (e) {
      toast(errorMessage(t, e), "error");
    }
  };

  const setPinPersist = async (persist: boolean) => {
    try {
      await api.pin_set_persist(persist);
      await useApp.getState().refreshVault();
    } catch (e) {
      toast(errorMessage(t, e), "error");
    }
  };

  return (
    <Pane>
      <Group>
        <SettingRow label={t("settings.sec.autoLock")}>
          <PopupSelect
            ariaLabel={t("settings.sec.autoLock")}
            value={prefs.auto_lock_minutes}
            minWidth={140}
            options={withValue(AUTO_LOCK_MINUTES, prefs.auto_lock_minutes).map((m) => ({ value: m, label: durationLabel(t, m * 60) }))}
            onChange={(auto_lock_minutes) => setPrefs({ auto_lock_minutes })}
          />
        </SettingRow>
        <SettingRow label={t("settings.sec.sleep")}>
          <Switch label={t("settings.sec.sleep")} checked={prefs.lock_on_sleep} onChange={(lock_on_sleep) => setPrefs({ lock_on_sleep })} />
        </SettingRow>
        <SettingRow label={t("settings.sec.disconnect")} hint={t("settings.sec.disconnect.hint")}>
          <Switch
            label={t("settings.sec.disconnect")}
            checked={prefs.lock_disconnects_sessions}
            onChange={(lock_disconnects_sessions) => setPrefs({ lock_disconnects_sessions })}
          />
        </SettingRow>
        <SettingRow label={t("settings.sec.clipboard")} hint={t("settings.sec.clipboard.hint")}>
          <PopupSelect
            ariaLabel={t("settings.sec.clipboard")}
            value={prefs.clipboard_clear_secs}
            minWidth={140}
            options={withValue(CLIPBOARD_CLEAR_SECS, prefs.clipboard_clear_secs).map((n) => ({ value: n, label: durationLabel(t, n) }))}
            onChange={(clipboard_clear_secs) => setPrefs({ clipboard_clear_secs })}
          />
        </SettingRow>
        {vault?.biometric_available && (
          <SettingRow label={biometricLabel}>
            <Switch label={biometricLabel} checked={vault.biometric_enabled} onChange={(on) => void setBiometric(on)} />
          </SettingRow>
        )}
        <SettingRow label={t("settings.sec.pin")}>
          <Switch label={t("settings.sec.pin")} checked={!!vault?.pin_enabled} onChange={(on) => void setPin(on)} />
        </SettingRow>
        {vault?.pin_enabled && (
          <SettingRow label={t("settings.sec.pinRestart")} hint={t("settings.sec.pinRestart.hint")}>
            <Switch
              label={t("settings.sec.pinRestart")}
              checked={!vault.pin_persist}
              onChange={(requirePassword) => void setPinPersist(!requirePassword)}
            />
          </SettingRow>
        )}
      </Group>

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
        <SettingRow label={t("settings.sec.lockNow")}>
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
      {dialog === "pin" && <SetPinDialog onClose={() => setDialog(null)} />}
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
