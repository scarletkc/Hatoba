import { useEffect, useMemo, useState } from "react";
import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { Button, Checkbox, Icon, Spinner } from "@/components/controls";
import { FooterSpacer, Sheet, SheetHeader, toast } from "@/components/overlay";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { SshConfigCandidate } from "@/ipc/types";
import { cx } from "@/lib/cx";
import s from "./ImportSshDialog.module.css";

/** The last segment of a path, for `/` and `\` alike. */
const fileName = (path: string) => path.split(/[\\/]/).pop() || path;

/**
 * SSH-11: preview `~/.ssh/config`, let the user pick hosts, import them. Their private keys are imported only when the
 * user ticks the separate option, which lists the files it reads.
 */
export function ImportSshDialog({ onClose }: { onClose: () => void }) {
  const t = useT();
  const platform = useApp((st) => st.info.platform);
  const path = platform === "windows" ? "%USERPROFILE%\\.ssh\\config" : "~/.ssh/config";
  const [items, setItems] = useState<SshConfigCandidate[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [importKeys, setImportKeys] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    api
      .ssh_config_preview()
      .then((list) => {
        if (cancelled) return;
        setItems(list);
        // Hosts that already exist stay unchecked.
        setPicked(new Set(list.filter((c) => !c.exists).map((c) => c.alias)));
      })
      .catch((e) => !cancelled && setError(errorMessage(t, e)));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const toggle = (alias: string, on: boolean) =>
    setPicked((prev) => {
      const next = new Set(prev);
      if (on) next.add(alias);
      else next.delete(alias);
      return next;
    });

  const allPicked = useMemo(() => !!items && items.length > 0 && items.every((c) => picked.has(c.alias)), [items, picked]);
  // The key files that importing keys reads: those of the ticked hosts, once each.
  const keyFiles = useMemo(
    () => [...new Set((items ?? []).filter((c) => picked.has(c.alias) && c.identity_file_found).flatMap((c) => c.identity_file ?? []))],
    [items, picked],
  );

  const submit = async () => {
    setBusy(true);
    try {
      // Only the files listed here are read, even if the config changed since the preview.
      const result = await api.ssh_config_import([...picked], importKeys ? keyFiles : []);
      await useVaultData.getState().reload();
      toast(t("hosts.import.done", { n: result.hosts_created }), "success");
      if (result.warnings.length > 0) toast(t("hosts.import.warnings", { n: result.warnings.length }));
      onClose();
    } catch (e) {
      setError(errorMessage(t, e));
      setBusy(false);
    }
  };

  return (
    <Sheet
      onClose={busy ? undefined : onClose}
      footer={
        <>
          {items && items.length > 0 && (
            <Button size="sm" onClick={() => setPicked(allPicked ? new Set() : new Set(items.map((c) => c.alias)))}>
              {allPicked ? t("hosts.import.selectNone") : t("hosts.import.selectAll")}
            </Button>
          )}
          <FooterSpacer />
          <Button onClick={onClose} disabled={busy}>
            {t("btn.cancel")}
          </Button>
          <Button variant="primary" busy={busy} disabled={picked.size === 0} onClick={() => void submit()}>
            {t("hosts.import.submit", { n: picked.size })}
          </Button>
        </>
      }
    >
      <SheetHeader title={t("hosts.import.title")} subtitle={t("hosts.import.subtitle", { path })} />
      {error && (
        <div className={s.error} role="alert">
          <Icon name="warning-circle" size={15} />
          {error}
        </div>
      )}
      {!items && !error && (
        <div className={s.note}>
          <Spinner /> {t("hosts.import.loading")}
        </div>
      )}
      {items && items.length === 0 && <div className={s.note}>{t("hosts.import.none", { path })}</div>}
      {items && items.length > 0 && (
        <div className={s.list} role="group">
          {items.map((c) => (
            <div key={c.alias} className={s.item}>
              <Checkbox checked={picked.has(c.alias)} onChange={(on) => toggle(c.alias, on)}>
                <span className={s.alias}>{c.alias}</span>
              </Checkbox>
              <span className={s.target}>
                {c.username && <span className={s.dim}>{c.username}@</span>}
                {c.address}
                <span className={s.dim}>:{c.port}</span>
              </span>
              {c.proxy_jump && (
                <span className={s.via} title={t("hosts.viaTitle", { name: c.proxy_jump })}>
                  <Icon name="path" />
                  {c.proxy_jump}
                </span>
              )}
              {c.identity_file && (
                <span
                  className={cx(s.via, !c.identity_file_found && s.missing)}
                  title={c.identity_file_found ? c.identity_file : t("hosts.import.keyMissing", { path: c.identity_file })}
                >
                  <Icon name="key" />
                  {fileName(c.identity_file)}
                </span>
              )}
              {c.proxy_command && (
                <span className={s.skipped} title={`ProxyCommand ${c.proxy_command}`}>
                  <Icon name="warning" />
                  {t("hosts.import.proxyCommand")}
                </span>
              )}
              <span className={s.spacer} />
              {c.exists && <span className={s.exists}>{t("hosts.import.exists")}</span>}
            </div>
          ))}
        </div>
      )}
      {keyFiles.length > 0 && (
        <div className={s.keys}>
          <Checkbox checked={importKeys} onChange={setImportKeys}>
            {t("hosts.import.keys", { n: keyFiles.length })}
          </Checkbox>
          <p className={s.keysNote}>{t("hosts.import.keysNote")}</p>
          <ul className={s.keyFiles}>
            {keyFiles.map((f) => (
              <li key={f}>{f}</li>
            ))}
          </ul>
        </div>
      )}
    </Sheet>
  );
}
