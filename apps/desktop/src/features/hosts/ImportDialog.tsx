import { useEffect, useMemo, useState } from "react";
import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { Button, Checkbox, Icon, Spinner } from "@/components/controls";
import { FooterSpacer, Sheet, SheetHeader, toast } from "@/components/overlay";
import { useT } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { ImportPreview, ImportSource } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { pickOpenPath } from "@/lib/native";
import s from "./ImportDialog.module.css";

/** The other SSH clients an import reads, in menu order; `~/.ssh/config` comes first. */
export const IMPORT_APPS = {
  xshell: "Xshell",
  finalshell: "FinalShell",
  mobaxterm: "MobaXterm",
  putty: "PuTTY",
  tabby: "Tabby",
} satisfies Record<Exclude<ImportSource, "ssh_config">, string>;

type Choice = { directory: true } | { extensions: string[] };

/** What the user can choose in place of where the client keeps its sessions: an export or a config file, or a folder. */
const CHOOSE: Record<ImportSource, Choice[]> = {
  ssh_config: [],
  xshell: [{ extensions: ["xts"] }, { directory: true }],
  finalshell: [{ directory: true }],
  mobaxterm: [{ extensions: ["mxtsessions", "ini"] }],
  putty: [{ extensions: ["reg"] }],
  tabby: [{ extensions: ["yaml", "yml"] }],
};

/** The last segment of a path, for `/` and `\` alike. */
const fileName = (path: string) => path.split(/[\\/]/).pop() || path;

/**
 * SSH-11, SSH-12, SSH-15: preview the hosts of `~/.ssh/config` or another SSH client, let the user pick hosts, import
 * them. Their private keys are imported only when the user ticks the separate option, which lists the files it reads.
 */
export function ImportDialog({ source, onClose }: { source: ImportSource; onClose: () => void }) {
  const t = useT();
  const platform = useApp((st) => st.info.platform);
  const app = source === "ssh_config" ? null : IMPORT_APPS[source];
  // A file or folder the user chose; `null` reads where the client keeps its sessions.
  const [path, setPath] = useState<string | null>(null);
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [importKeys, setImportKeys] = useState(false);
  const [busy, setBusy] = useState(false);

  const describe = (e: unknown) =>
    app && toAppError(e).code === "invalid_input" ? t("hosts.import.unreadable", { app }) : errorMessage(t, e);

  useEffect(() => {
    let cancelled = false;
    setPreview(null);
    setError(null);
    // Ids and key files belong to the sessions they were read from.
    setPicked(new Set());
    setImportKeys(false);
    api
      .hosts_import_preview(source, path)
      .then((found) => {
        if (cancelled) return;
        setPreview(found);
        // Hosts that already exist stay unchecked.
        setPicked(new Set(found.candidates.filter((c) => !c.exists).map((c) => c.id)));
      })
      .catch((e) => !cancelled && setError(describe(e)));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [source, path]);

  const items = preview?.candidates ?? null;
  // `~/.ssh/config` is named as on the user's system; another client's sessions where they were read.
  const location = app ? preview?.location : platform === "windows" ? "%USERPROFILE%\\.ssh\\config" : "~/.ssh/config";

  const toggle = (id: string, on: boolean) =>
    setPicked((prev) => {
      const next = new Set(prev);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });

  const allPicked = useMemo(() => !!items && items.length > 0 && items.every((c) => picked.has(c.id)), [items, picked]);
  // The key files that importing keys reads: those of the ticked hosts, once each.
  const keyFiles = useMemo(
    () => [...new Set((items ?? []).filter((c) => picked.has(c.id) && c.identity_file_found).flatMap((c) => c.identity_file ?? []))],
    [items, picked],
  );

  const chooseOther = async (choice: Choice) => {
    if (!app) return;
    const chosen = await pickOpenPath(
      "directory" in choice ? { directory: true } : { filter: { name: t("hosts.import.filter", { app }), extensions: choice.extensions } },
    ).catch(() => null);
    if (chosen) setPath(chosen);
  };

  const submit = async () => {
    setBusy(true);
    try {
      // Only the files listed here are read, even if the sessions changed since the preview.
      const result = await api.hosts_import(source, path, [...picked], importKeys ? keyFiles : []);
      // Closed first: once the empty host list fills, the page that opened it is replaced.
      onClose();
      await useVaultData.getState().reload();
      toast(t("hosts.import.done", { n: result.hosts_created }), "success");
      if (result.warnings.length > 0) toast(t("hosts.import.warnings", { n: result.warnings.length }));
    } catch (e) {
      setError(describe(e));
      setBusy(false);
    }
  };

  const none = () => {
    if (!app) return t("hosts.import.none", { path: location ?? "" });
    if (preview?.encrypted) return t("hosts.import.encrypted", { app });
    return location ? t("hosts.import.noneAt", { app, path: location }) : t("hosts.import.noneHere", { app });
  };

  return (
    <Sheet
      onClose={busy ? undefined : onClose}
      footer={
        <>
          {items && items.length > 0 && (
            <Button size="sm" onClick={() => setPicked(allPicked ? new Set() : new Set(items.map((c) => c.id)))}>
              {allPicked ? t("hosts.import.selectNone") : t("hosts.import.selectAll")}
            </Button>
          )}
          {CHOOSE[source].map((choice) => (
            <Button key={"directory" in choice ? "folder" : "file"} size="sm" disabled={busy} onClick={() => void chooseOther(choice)}>
              {"directory" in choice ? t("hosts.import.chooseFolder") : t("hosts.import.chooseFile")}
            </Button>
          ))}
          <FooterSpacer />
          <Button onClick={onClose} disabled={busy}>
            {t("btn.cancel")}
          </Button>
          <Button variant="primary" busy={busy} disabled={!items || picked.size === 0} onClick={() => void submit()}>
            {t("hosts.import.submit", { n: picked.size })}
          </Button>
        </>
      }
    >
      <SheetHeader
        title={app ? t("hosts.import.titleFrom", { app }) : t("hosts.import.title")}
        subtitle={location && (!app || (items && items.length > 0)) ? t("hosts.import.subtitle", { path: location }) : undefined}
      />
      {error && (
        <div className={s.error} role="alert">
          <Icon name="warning-circle" size={15} />
          {error}
        </div>
      )}
      {!items && !error && (
        <div className={s.note}>
          <Spinner /> {app ? t("hosts.import.loadingFrom", { app }) : t("hosts.import.loading")}
        </div>
      )}
      {items && items.length === 0 && <div className={s.note}>{none()}</div>}
      {items && items.length > 0 && (
        <div className={s.list} role="group">
          {items.map((c) => (
            <div key={c.id} className={s.item}>
              <Checkbox checked={picked.has(c.id)} onChange={(on) => toggle(c.id, on)}>
                <span className={s.alias}>{c.name}</span>
              </Checkbox>
              <span className={s.target}>
                {c.username && <span className={s.dim}>{c.username}@</span>}
                {c.address}
                <span className={s.dim}>:{c.port}</span>
              </span>
              {c.folder && (
                <span className={s.via} title={`${t("hosts.f.group")}: ${c.folder}`}>
                  <Icon name="folder-simple" />
                  <span className={s.chipText}>{c.folder}</span>
                </span>
              )}
              {c.jump && (
                <span className={s.via} title={t("hosts.viaTitle", { name: c.jump })}>
                  <Icon name="path" />
                  <span className={s.chipText}>{c.jump}</span>
                </span>
              )}
              {c.proxy && (
                <span className={s.via} title={`${t("hosts.f.proxy")}: ${c.proxy}`}>
                  <Icon name="globe" />
                  <span className={s.chipText}>{c.proxy}</span>
                </span>
              )}
              {c.identity_file && (
                <span
                  className={cx(s.via, !c.identity_file_found && s.missing)}
                  title={c.identity_file_found ? c.identity_file : t("hosts.import.keyMissing", { path: c.identity_file })}
                >
                  <Icon name="key" />
                  <span className={s.chipText}>{fileName(c.identity_file)}</span>
                </span>
              )}
              {app && c.kept_key !== null && !c.identity_file && (
                <span className={s.skipped} title={c.kept_key || undefined}>
                  <Icon name="key" />
                  {t("hosts.import.keyKept", { app })}
                </span>
              )}
              {c.skipped_proxy && (
                <span className={s.skipped} title={c.skipped_proxy}>
                  <Icon name="warning" />
                  {app ? t("hosts.import.proxySkipped") : t("hosts.import.proxyCommand")}
                </span>
              )}
              <span className={s.spacer} />
              {c.exists && <span className={s.exists}>{t("hosts.import.exists")}</span>}
            </div>
          ))}
        </div>
      )}
      {app && items && items.length > 0 && <p className={s.keysNote}>{t("hosts.import.passwordsNote")}</p>}
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
