import { useMemo, useState, type KeyboardEvent } from "react";
import { Button, Icon } from "@/components/controls";
import { PageHeader, SearchField, EmptyState } from "@/components/layout";
import { confirm, toast } from "@/components/overlay";
import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { formatDate, useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { KeyView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { DeployKeyDialog, GenerateKeyDialog, ImportKeyDialog, RenameKeyDialog } from "./KeyDialogs";
import { KeyDetail } from "./KeyDetail";
import { keyTypeLabel, shortFingerprint } from "./keyUtils";
import s from "./KeysPage.module.css";

type Dialog =
  | { kind: "import" }
  | { kind: "generate" }
  | { kind: "rename"; key: KeyView }
  | { kind: "deploy"; key: KeyView };

/** Key vault (KEY-01..07, design §04): list + detail panel. Private keys never reach the WebView. */
export function KeysPage() {
  const t = useT();
  const keys = useVaultData((st) => st.keys);
  const hosts = useVaultData((st) => st.hosts);
  const [query, setQuery] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [dialog, setDialog] = useState<Dialog | null>(null);

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return keys;
    return keys.filter((k) =>
      [k.name, k.fingerprint, keyTypeLabel(k), k.comment].some((v) => v.toLowerCase().includes(q)),
    );
  }, [keys, query]);
  const selected = visible.find((k) => k.id === selectedId) ?? visible[0] ?? null;
  const hostName = (id: string) => hosts.find((h) => h.id === id)?.name ?? "";

  const reload = () => useVaultData.getState().reload();

  const afterAdd = async (key: KeyView, doneKey: "keys.import.done" | "keys.generate.done") => {
    setDialog(null);
    setQuery("");
    await reload();
    setSelectedId(key.id);
    toast(t(doneKey, { name: key.name }), "success");
  };

  const remove = async (key: KeyView) => {
    const users = key.used_by.map(hostName).filter(Boolean);
    const ok = await confirm({
      title: t("keys.delete.title", { name: key.name }),
      confirmLabel: t("keys.delete.confirm"),
      danger: true,
      body: (
        <div className={s.confirmBody}>
          <p>{t("keys.delete.body")}</p>
          {users.length > 0 && (
            <>
              <p>{t("keys.delete.used", { n: users.length })}</p>
              <ul className={s.confirmHosts}>
                {users.map((n, i) => (
                  <li key={i}>{n}</li>
                ))}
              </ul>
            </>
          )}
        </div>
      ),
    });
    if (!ok) return;
    const index = visible.findIndex((k) => k.id === key.id);
    const next = visible[index + 1] ?? visible[index - 1];
    try {
      await api.key_delete(key.id);
      setSelectedId(next?.id ?? null);
      await reload();
      toast(t("keys.delete.done", { name: key.name }), "success");
    } catch (e) {
      toast(errorMessage(t, e), "error");
    }
  };

  const onListKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const index = visible.findIndex((k) => k.id === selected?.id);
    const next = visible[Math.max(0, Math.min(visible.length - 1, index + (e.key === "ArrowDown" ? 1 : -1)))];
    if (!next) return;
    setSelectedId(next.id);
    e.currentTarget.querySelector<HTMLElement>(`[data-key-id="${CSS.escape(next.id)}"]`)?.focus();
  };

  const dialogs = (
    <>
      {dialog?.kind === "import" && (
        <ImportKeyDialog onClose={() => setDialog(null)} onDone={(k) => void afterAdd(k, "keys.import.done")} />
      )}
      {dialog?.kind === "generate" && (
        <GenerateKeyDialog onClose={() => setDialog(null)} onDone={(k) => void afterAdd(k, "keys.generate.done")} />
      )}
      {dialog?.kind === "rename" && (
        <RenameKeyDialog
          keyView={dialog.key}
          onClose={() => setDialog(null)}
          onDone={() => {
            setDialog(null);
            void reload();
          }}
        />
      )}
      {dialog?.kind === "deploy" && <DeployKeyDialog keyView={dialog.key} hosts={hosts} onClose={() => setDialog(null)} />}
    </>
  );

  const importButton = (
    <Button icon="download-simple" onClick={() => setDialog({ kind: "import" })}>
      {t("keys.import")}
    </Button>
  );
  const generateButton = (
    <Button variant="primary" icon="plus" onClick={() => setDialog({ kind: "generate" })}>
      {t("keys.generate")}
    </Button>
  );

  if (keys.length === 0) {
    return (
      <div className={s.page}>
        <PageHeader title={t("sidebar.keys")} count={t("keys.count", { n: 0 })} />
        <div className={s.emptyWrap}>
          <EmptyState
            icon="key"
            title={t("keys.empty.title")}
            body={t("keys.empty.body")}
            actions={
              <>
                {generateButton}
                {importButton}
              </>
            }
          />
        </div>
        {dialogs}
      </div>
    );
  }

  return (
    <div className={s.page}>
      <PageHeader
        title={t("sidebar.keys")}
        count={t("keys.count", { n: keys.length })}
        search={<SearchField value={query} onChange={setQuery} placeholder={t("keys.search")} width={200} />}
      >
        {importButton}
        {generateButton}
      </PageHeader>
      <div className={s.body}>
        <div className={s.listCol}>
          <div className={s.head}>
            <span className={s.headName}>{t("keys.col.name")}</span>
            <span>{t("keys.col.type")}</span>
            <span>{t("keys.col.fingerprint")}</span>
            <span>{t("keys.col.created")}</span>
            <span>{t("keys.col.usedBy")}</span>
          </div>
          <div className={s.list} role="listbox" aria-label={t("keys.list")} onKeyDown={onListKey}>
            {visible.length === 0 && <div className={s.noMatch}>{t("keys.noMatch")}</div>}
            {visible.map((k) => {
              const users = k.used_by.map(hostName).filter(Boolean);
              const on = k.id === selected?.id;
              return (
                <div
                  key={k.id}
                  role="option"
                  aria-selected={on}
                  tabIndex={on ? 0 : -1}
                  data-key-id={k.id}
                  className={cx(s.row, on && s.rowOn)}
                  onClick={() => setSelectedId(k.id)}
                >
                  <span className={s.cellName}>
                    <Icon name="key" className={s.keyIcon} />
                    <span className={s.nameText}>{k.name}</span>
                  </span>
                  <span className={s.typeBadge}>{keyTypeLabel(k)}</span>
                  <span className={s.cellFingerprint} title={k.fingerprint}>
                    {shortFingerprint(k.fingerprint)}
                  </span>
                  <span className={s.cellCreated}>{formatDate(k.created_at)}</span>
                  <span className={s.cellUsed}>
                    {users.length === 0 ? (
                      <span className={s.unused}>{t("keys.unused")}</span>
                    ) : (
                      <>
                        <span className={s.usedFirst}>{users[0]}</span>
                        {users.length > 1 && <span className={s.usedMore}>+{users.length - 1}</span>}
                      </>
                    )}
                  </span>
                </div>
              );
            })}
          </div>
        </div>
        {selected && (
          <KeyDetail
            key={selected.id}
            keyView={selected}
            onRename={() => setDialog({ kind: "rename", key: selected })}
            onDeploy={() => setDialog({ kind: "deploy", key: selected })}
            onDelete={() => void remove(selected)}
          />
        )}
      </div>
      {dialogs}
    </div>
  );
}
