import { useEffect, useState } from "react";
import { Button, Checkbox } from "@/components/controls";
import { Dialog } from "@/components/overlay";
import { translate, useT } from "@/i18n";
import { api } from "@/ipc/api";
import { useApp } from "./store";

/**
 * WIN-10: the close button's question on Windows and Linux, whether to minimize Hatoba to the
 * system tray or quit. The backend asks while the device pref says so and a tray shows; a
 * remembered choice goes into that pref, which Settings → General changes.
 */
export function ClosePrompt() {
  const t = useT();
  const [open, setOpen] = useState(false);
  const [remember, setRemember] = useState(false);

  useEffect(() => {
    const sub = api.listen("window://close-prompt", () => {
      setRemember(false);
      setOpen(true);
    });
    return () => void sub.then((un) => un());
  }, []);

  if (!open) return null;
  const choose = async (action: "tray" | "quit") => {
    setOpen(false);
    if (remember) {
      const { prefs, setPrefs } = useApp.getState();
      await setPrefs({ ...prefs, close_action: action }).catch(() => {});
    }
    await (action === "tray" ? api.window_hide_to_tray() : api.app_quit());
  };
  return (
    <Dialog
      title={t("close.title")}
      body={t("close.body")}
      whileLocked
      onClose={() => setOpen(false)}
      actions={
        <>
          <Button onClick={() => setOpen(false)}>{t("btn.cancel")}</Button>
          <Button onClick={() => void choose("quit")}>{t("close.quit")}</Button>
          <Button variant="primary" autoFocus onClick={() => void choose("tray")}>
            {t("close.tray")}
          </Button>
        </>
      }
    >
      <div style={{ marginTop: 14 }}>
        <Checkbox checked={remember} onChange={setRemember}>
          {t("close.remember")}
        </Checkbox>
      </div>
    </Dialog>
  );
}

/** WIN-10: the tray menu speaks the app's language, and **Lock** works while the vault is unlocked. */
export function useTrayMenu() {
  const locale = useT().locale;
  const canLock = useApp((st) => st.phase === "unlocked");
  useEffect(() => {
    const label = (key: "tray.open" | "tray.lock" | "tray.quit") => translate(locale, key);
    api.tray_set_menu({ open: label("tray.open"), lock: label("tray.lock"), quit: label("tray.quit"), can_lock: canLock }).catch(() => {});
  }, [locale, canLock]);
}
