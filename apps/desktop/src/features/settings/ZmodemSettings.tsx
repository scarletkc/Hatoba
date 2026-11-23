import { useEffect, useState } from "react";
import { Button, toast } from "@hatoba/ui";
import { errorMessage } from "@/app/errors";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import { pickOpenPath } from "@/lib/native";
import { Section, SettingRow } from "./shared";

export function ZmodemSettings() {
  const t = useT();
  const [directory, setDirectory] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let live = true;
    void api
      .zmodem_directory(null)
      .then((path) => {
        if (live) setDirectory(path);
      })
      .catch((e: unknown) => toast(errorMessage(t, e), "error"));
    return () => {
      live = false;
    };
  }, [t]);
  const change = async (reset: boolean) => {
    setBusy(true);
    try {
      const path = reset ? "" : await pickOpenPath({ directory: true, title: t("terminal.zmodem.directory") });
      if (path !== null) setDirectory(await api.zmodem_directory(path));
    } catch (e) {
      toast(errorMessage(t, e), "error");
    } finally {
      setBusy(false);
    }
  };
  return (
    <Section title="ZMODEM (sz/rz)">
      <SettingRow label={t("terminal.zmodem.directory")} hint={directory}>
        <Button disabled={busy} onClick={() => void change(false)}>
          {t("terminal.zmodem.browse")}
        </Button>
        <Button disabled={busy} onClick={() => void change(true)}>
          {t("terminal.zmodem.default")}
        </Button>
      </SettingRow>
      <SettingRow label={t("terminal.zmodem.directoryHint")} />
    </Section>
  );
}
