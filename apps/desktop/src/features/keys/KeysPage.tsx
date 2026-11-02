import { useId, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent } from "react";
import {
  Button,
  Chip,
  EmptyState,
  Icon,
  Menu,
  PageHeader,
  SearchField,
  confirm,
  toast,
  useMenu,
  type MenuEntry,
} from "@hatoba/ui";
import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { formatDate, useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { KeyView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { isMac } from "@/lib/platform";
import { copyText } from "./clipboard";
import { DeployKeyDialog, GenerateKeyDialog, ImportKeyDialog, RenameKeyDialog } from "./KeyDialogs";
import { KeyDetail } from "./KeyDetail";
import { keyTypeLabel, shortFingerprint } from "./keyUtils";
import s from "./KeysPage.module.css";

type Dialog =
  { kind: "import" } | { kind: "generate" } | { kind: "rename"; key: KeyView } | { kind: "deploy"; key: KeyView };

/** Key vault (KEY-01..07): list + detail panel. Private keys never reach the WebView. */
export function KeysPage() {
  const t = useT();
  const platform = useApp((st) => st.info.platform);
  const keys = useVaultData((st) => st.keys);
  const hosts = useVaultData((st) => st.hosts);
  const [query, setQuery] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [ctxId, setCtxId] = useState<string | null>(null);
  const rowMenu = useMenu();
  const searchRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const optionId = `${useId()}-key-`;

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

  const copyPublic = async (key: KeyView) => {
    try {
      await copyText(key.public_key);
      toast(t("btn.copied"), "success");
    } catch {
      toast(t("err.internal"), "error");
    }
  };

  const move = (delta: number) => {
    const index = visible.findIndex((k) => k.id === selected?.id);
    const next = visible[Math.max(0, Math.min(visible.length - 1, index + delta))];
    if (!next) return;
    setSelectedId(next.id);
    requestAnimationFrame(() =>
      listRef.current?.querySelector(`[data-key-id="${CSS.escape(next.id)}"]`)?.scrollIntoView({ block: "nearest" }),
    );
  };

  // ↑/↓ move the selection, from the list and from the search field. On the list, Delete deletes and typing searches.
  const onKeyDown = (e: KeyboardEvent) => {
    const inSearch = e.target === searchRef.current;
    const onList = e.target === listRef.current;
    if (!inSearch && !onList) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      move(e.key === "ArrowDown" ? 1 : -1);
    } else if (onList && selected && (e.key === "Delete" || (isMac(platform) && e.metaKey && e.key === "Backspace"))) {
      e.preventDefault();
      void remove(selected);
    } else if (onList && e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey && e.key !== " ") {
      searchRef.current?.focus();
    }
  };

  const onContext = (e: MouseEvent, key: KeyView) => {
    e.preventDefault();
    setSelectedId(key.id);
    setCtxId(key.id);
    rowMenu.openAt({ x: e.clientX, y: e.clientY });
  };
  const ctxKey = keys.find((k) => k.id === ctxId);

  const menuEntries = (key: KeyView): MenuEntry[] => [
    { label: t("btn.rename"), icon: "pencil-simple", onSelect: () => setDialog({ kind: "rename", key }) },
    { label: t("keys.action.deploy"), icon: "upload-simple", onSelect: () => setDialog({ kind: "deploy", key }) },
    { label: t("keys.detail.copyPublic"), icon: "clipboard-text", onSelect: () => void copyPublic(key) },
    { kind: "separator" },
    { label: t("keys.action.delete"), icon: "trash", danger: true, onSelect: () => void remove(key) },
  ];

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
      {dialog?.kind === "deploy" && (
        <DeployKeyDialog keyView={dialog.key} hosts={hosts} onClose={() => setDialog(null)} />
      )}
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
    <div className={s.page} onKeyDown={onKeyDown}>
      <PageHeader
        title={t("sidebar.keys")}
        count={t("keys.count", { n: keys.length })}
        search={
          <SearchField ref={searchRef} value={query} onChange={setQuery} placeholder={t("keys.search")} width={200} />
        }
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
          {/* The focus stays on the list; the active option tells a screen reader which key is selected. */}
          <div
            ref={listRef}
            className={s.list}
            tabIndex={0}
            role="listbox"
            aria-label={t("keys.list")}
            aria-activedescendant={selected ? optionId + selected.id : undefined}
          >
            {visible.length === 0 && <div className={s.noMatch}>{t("keys.noMatch")}</div>}
            {visible.map((k) => {
              const users = k.used_by.map(hostName).filter(Boolean);
              const on = k.id === selected?.id;
              return (
                <div
                  key={k.id}
                  id={optionId + k.id}
                  role="option"
                  aria-selected={on}
                  data-key-id={k.id}
                  className={cx(s.row, on && s.rowOn)}
                  onClick={() => setSelectedId(k.id)}
                  onContextMenu={(e) => onContext(e, k)}
                >
                  <span className={s.cellName}>
                    <Icon name="key" className={s.keyIcon} />
                    <span className={s.nameText}>{k.name}</span>
                  </span>
                  <Chip mono className={s.typeBadge}>
                    {keyTypeLabel(k)}
                  </Chip>
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
            onCopyPublic={() => void copyPublic(selected)}
            onRename={() => setDialog({ kind: "rename", key: selected })}
            onDeploy={() => setDialog({ kind: "deploy", key: selected })}
            onDelete={() => void remove(selected)}
          />
        )}
      </div>
      {rowMenu.anchor && ctxKey && (
        <Menu anchor={rowMenu.anchor} onClose={rowMenu.close} entries={menuEntries(ctxKey)} />
      )}
      {dialogs}
    </div>
  );
}
