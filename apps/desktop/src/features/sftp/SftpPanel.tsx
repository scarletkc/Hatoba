import { useCallback, useEffect, useRef, useState, type KeyboardEvent, type MouseEvent } from "react";
import { errorMessage } from "@/app/errors";
import { Icon, IconButton, LinkButton, Spinner } from "@/components/controls";
import { confirm, Menu, toast, useMenu, type MenuEntry } from "@/components/overlay";
import { writeClipboard } from "@/features/terminal/clipboard";
import { formatBytes, formatDate, useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { AppError, FileEntry } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { isImeEvent } from "@/lib/ime";
import { pickSavePath, pickUploadFiles } from "./native";
import { joinPath, parentPath, splitLastSegment } from "./paths";
import { TransferList } from "./TransferList";
import { useTransfers } from "./transfers";
import { useFileDrop } from "./useFileDrop";
import s from "./SftpPanel.module.css";

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

/** Folders first, then names (dot-files sort before letters, like the design's list). */
function sortEntries(list: FileEntry[]): FileEntry[] {
  return [...list].sort((a, b) => Number(b.is_dir) - Number(a.is_dir) || collator.compare(a.name, b.name));
}

function shortDate(ms: number | null): string {
  if (ms == null) return "—";
  const d = new Date(ms);
  return `${String(d.getMonth() + 1).padStart(2, "0")}/${String(d.getDate()).padStart(2, "0")}`;
}

/** SFTP-04: permissions, exact size and modification time on hover. */
function tooltip(e: FileEntry): string {
  const lines = [e.name, [e.permissions, e.is_dir ? null : `${formatBytes(e.size)} (${e.size.toLocaleString()} B)`].filter(Boolean).join(" · ")];
  if (e.modified != null) {
    const d = new Date(e.modified);
    lines.push(`${formatDate(e.modified)} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`);
  }
  return lines.join("\n");
}

function fileIcon(e: FileEntry): { name: string; fill: boolean; color: string } {
  if (e.is_dir) return { name: "folder-simple", fill: true, color: "var(--accent)" };
  // Owner-only files (private keys, .env…) get the lock.
  if (/^-rw-------/.test(e.permissions)) return { name: "file-lock", fill: false, color: "var(--fg2)" };
  return { name: "file-text", fill: false, color: "var(--fg2)" };
}

interface Props {
  sessionId: string;
  hostName: string;
  /** Only the visible tab's panel reacts to native file drops. */
  active: boolean;
}

/** Remote file browser to the right of a terminal (design §03, SFTP-01..04). */
export function SftpPanel({ sessionId, hostName, active }: Props) {
  const t = useT();
  const rootRef = useRef<HTMLDivElement>(null);
  const requestSeq = useRef(0);
  const [cwd, setCwd] = useState<string | null>(null);
  const cwdRef = useRef<string | null>(null);
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<AppError | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const menu = useMenu();
  const [menuEntries, setMenuEntries] = useState<MenuEntry[]>([]);

  /** Load a directory; `null` means the home directory (SFTP-01). */
  const load = useCallback(
    async (path: string | null) => {
      const seq = ++requestSeq.current;
      setLoading(true);
      setError(null);
      try {
        const dir = path ?? (await api.sftp_home(sessionId));
        const list = await api.sftp_list(sessionId, dir);
        if (seq !== requestSeq.current) return;
        cwdRef.current = dir;
        setCwd(dir);
        setEntries(sortEntries(list));
        setSelected(null);
        setRenaming(null);
        setCreating(false);
      } catch (e) {
        if (seq !== requestSeq.current) return;
        setError(e as AppError);
      } finally {
        if (seq === requestSeq.current) setLoading(false);
      }
    },
    [sessionId],
  );

  useEffect(() => {
    void load(null);
  }, [load]);

  const refresh = useCallback(() => void load(cwdRef.current), [load]);

  // Refresh after an upload finishes.
  const uploadsDone = useTransfers((st) => st.uploadsDone[sessionId] ?? 0);
  const seenUploads = useRef(uploadsDone);
  useEffect(() => {
    if (uploadsDone !== seenUploads.current) {
      seenUploads.current = uploadsDone;
      refresh();
    }
  }, [uploadsDone, refresh]);

  const fail = (e: unknown) => toast(errorMessage(t, e), "error");

  const upload = async (paths: string[]) => {
    const dir = cwdRef.current;
    if (!dir || paths.length === 0) return;
    const results = await Promise.allSettled(paths.map((p) => api.sftp_upload(sessionId, p, dir)));
    const failed = results.find((r): r is PromiseRejectedResult => r.status === "rejected");
    if (failed) fail(failed.reason);
  };

  const download = async (entry: FileEntry) => {
    if (entry.is_dir) return;
    const local = await pickSavePath(entry.name);
    if (!local) return;
    try {
      await api.sftp_download(sessionId, entry.path, local);
    } catch (e) {
      fail(e);
    }
  };

  const open = (entry: FileEntry) => (entry.is_dir ? void load(entry.path) : void download(entry));

  const validName = (name: string) => {
    const ok = name.trim() !== "" && !name.includes("/");
    if (!ok) toast(t("sftp.invalidName"), "error");
    return ok;
  };

  const createFolder = async (name: string) => {
    setCreating(false);
    const dir = cwdRef.current;
    if (!dir || !validName(name)) return;
    try {
      await api.sftp_mkdir(sessionId, joinPath(dir, name.trim()));
    } catch (e) {
      fail(e);
    }
    refresh();
  };

  const rename = async (entry: FileEntry, name: string) => {
    setRenaming(null);
    if (name === entry.name || !validName(name)) return;
    try {
      await api.sftp_rename(sessionId, entry.path, joinPath(parentPath(entry.path), name.trim()));
    } catch (e) {
      fail(e);
    }
    refresh();
  };

  const remove = async (entry: FileEntry) => {
    const ok = await confirm({
      title: t("sftp.delete.title", { name: entry.name }),
      body: t(entry.is_dir ? "sftp.delete.dir" : "sftp.delete.file"),
      confirmLabel: t("btn.delete"),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.sftp_remove(sessionId, entry.path, entry.is_dir);
    } catch (e) {
      fail(e);
    }
    refresh();
  };

  const copyPath = async (path: string) => {
    try {
      await writeClipboard(path);
      toast(t("sftp.pathCopied"), "success");
    } catch {
      toast(t("terminal.clipboardDenied"), "error");
    }
  };

  const pickAndUpload = async () => upload(await pickUploadFiles());

  const { dragging, html5 } = useFileDrop(rootRef, active, (paths) => void upload(paths));

  const showMenu = (e: MouseEvent, entry: FileEntry | null) => {
    e.preventDefault();
    e.stopPropagation();
    setSelected(entry?.path ?? null);
    const common: MenuEntry[] = [
      { label: t("sftp.menu.newFolder"), icon: "folder-plus", onSelect: () => setCreating(true) },
      { label: t("sftp.menu.upload"), icon: "upload-simple", onSelect: () => void pickAndUpload() },
      { label: t("sftp.menu.refresh"), icon: "arrows-clockwise", onSelect: refresh },
    ];
    if (!entry) {
      setMenuEntries([...common, { kind: "separator" }, { label: t("sftp.menu.copyPath"), icon: "copy", disabled: !cwd, onSelect: () => cwd && void copyPath(cwd) }]);
    } else {
      setMenuEntries([
        entry.is_dir
          ? { label: t("sftp.menu.open"), icon: "folder-open", onSelect: () => open(entry) }
          : { label: t("sftp.menu.download"), icon: "download-simple", onSelect: () => void download(entry) },
        { label: t("sftp.menu.rename"), icon: "pencil-simple", onSelect: () => setRenaming(entry.path) },
        { label: t("sftp.menu.delete"), icon: "trash", danger: true, onSelect: () => void remove(entry) },
        { kind: "separator" },
        { label: t("sftp.menu.copyPath"), icon: "copy", onSelect: () => void copyPath(entry.path) },
        { kind: "separator" },
        ...common.slice(0, 1),
      ]);
    }
    menu.openAt({ x: e.clientX, y: e.clientY });
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if ((e.target as HTMLElement).tagName === "INPUT") return;
    const index = entries.findIndex((x) => x.path === selected);
    const current = index >= 0 ? entries[index] : undefined;
    switch (e.key) {
      case "ArrowDown":
      case "ArrowUp": {
        e.preventDefault();
        const next = Math.max(0, Math.min(entries.length - 1, index + (e.key === "ArrowDown" ? 1 : -1)));
        setSelected(entries[index < 0 ? 0 : next]?.path ?? null);
        break;
      }
      case "Enter":
        if (current) open(current);
        break;
      case "F2":
        if (current) setRenaming(current.path);
        break;
      case "Delete":
        if (current) void remove(current);
        break;
      case "Backspace":
        if (cwd && cwd !== "/") void load(parentPath(cwd));
        break;
    }
  };

  const [dirPrefix, dirLast] = cwd ? splitLastSegment(cwd) : ["", ""];

  return (
    <div ref={rootRef} className={s.panel} {...html5}>
      <div className={s.header}>
        <span className={s.title}>{t("sftp.title")}</span>
        <span className={s.host}>{hostName}</span>
        <div className={s.grow} />
        <IconButton icon="upload-simple" label={t("sftp.upload")} className={s.iconBtn} onClick={() => void pickAndUpload()} />
        <IconButton icon="folder-plus" label={t("sftp.newFolder")} className={s.iconBtn} disabled={!cwd} onClick={() => setCreating(true)} />
        <IconButton icon="arrows-clockwise" label={t("sftp.refresh")} className={s.iconBtn} onClick={refresh} />
      </div>

      <div className={s.pathBar}>
        <button
          type="button"
          className={s.back}
          aria-label={t("sftp.parent")}
          title={t("sftp.parent")}
          disabled={!cwd || cwd === "/"}
          onClick={() => cwd && void load(parentPath(cwd))}
        >
          <Icon name="caret-left" size={13} />
        </button>
        <span className={cx(s.path, "selectable")} title={cwd ?? undefined}>
          {dirPrefix}
          <span className={s.pathLast}>{dirLast}</span>
        </span>
        <div className={s.grow} />
        {cwd && !error && <span className={s.count}>{t("sftp.items", { n: entries.length })}</span>}
      </div>

      <div className={s.columns}>
        <span>{t("sftp.col.name")}</span>
        <span className={s.right}>{t("sftp.col.size")}</span>
        <span className={s.right}>{t("sftp.col.modified")}</span>
      </div>

      <div
        className={s.list}
        tabIndex={0}
        role="listbox"
        aria-label={t("sftp.title")}
        aria-busy={loading}
        onKeyDown={onKeyDown}
        onClick={(e) => e.target === e.currentTarget && setSelected(null)}
        onContextMenu={(e) => showMenu(e, null)}
      >
        {creating && (
          <div className={s.row}>
            <span className={s.nameCell}>
              <Icon name="folder-simple" fill size={14} color="var(--accent)" />
              <NameInput initial={t("sftp.newFolderName")} selectAll onCommit={(v) => void createFolder(v)} onCancel={() => setCreating(false)} />
            </span>
            <span />
            <span />
          </div>
        )}
        {entries.map((entry) => {
          const icon = fileIcon(entry);
          return (
            <div
              key={entry.path}
              role="option"
              aria-selected={selected === entry.path}
              className={cx(s.row, selected === entry.path && s.rowSelected)}
              title={renaming === entry.path ? undefined : tooltip(entry)}
              onClick={() => setSelected(entry.path)}
              onDoubleClick={() => renaming !== entry.path && open(entry)}
              onContextMenu={(e) => showMenu(e, entry)}
            >
              <span className={s.nameCell}>
                <Icon name={icon.name} fill={icon.fill} size={14} color={icon.color} />
                {renaming === entry.path ? (
                  <NameInput initial={entry.name} onCommit={(v) => void rename(entry, v)} onCancel={() => setRenaming(null)} />
                ) : (
                  <span className={s.name}>{entry.name}</span>
                )}
              </span>
              <span className={s.meta}>{entry.is_dir ? "—" : formatBytes(entry.size)}</span>
              <span className={s.meta}>{shortDate(entry.modified)}</span>
            </div>
          );
        })}
        {loading && entries.length === 0 && (
          <div className={s.message}>
            <Spinner size={13} />
            {t("sftp.loading")}
          </div>
        )}
        {error && (
          <div className={cx(s.message, s.messageError)}>
            <div>{t("sftp.loadFailed")}</div>
            <div className={s.errorDetail}>{errorMessage(t, error)}</div>
            <LinkButton icon="arrows-clockwise" onClick={refresh}>
              {t("btn.retry")}
            </LinkButton>
          </div>
        )}
        {!loading && !error && entries.length === 0 && !creating && <div className={s.message}>{t("sftp.empty")}</div>}
      </div>

      <TransferList sessionId={sessionId} />

      {dragging && (
        <div className={s.drop}>
          <Icon name="upload-simple" size={22} />
          <span>{t("sftp.dropHere", { path: cwd ?? "~" })}</span>
        </div>
      )}
      {menu.anchor && <Menu anchor={menu.anchor} entries={menuEntries} onClose={menu.close} minWidth={168} />}
    </div>
  );
}

/** Inline name editor for rename / new folder. Enter or blur commits, Esc cancels. */
function NameInput({
  initial,
  selectAll,
  onCommit,
  onCancel,
}: {
  initial: string;
  selectAll?: boolean;
  onCommit(value: string): void;
  onCancel(): void;
}) {
  const t = useT();
  const ref = useRef<HTMLInputElement>(null);
  const finished = useRef(false);
  const [value, setValue] = useState(initial);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.focus();
    const dot = initial.lastIndexOf(".");
    el.setSelectionRange(0, selectAll || dot <= 0 ? initial.length : dot);
  }, [initial, selectAll]);

  const finish = (commit: boolean) => {
    if (finished.current) return;
    finished.current = true;
    if (commit) onCommit(value);
    else onCancel();
  };

  return (
    <input
      ref={ref}
      className={s.nameInput}
      value={value}
      aria-label={t("sftp.nameLabel")}
      spellCheck={false}
      autoComplete="off"
      onChange={(e) => setValue(e.target.value)}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onBlur={() => finish(true)}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") {
          if (isImeEvent(e)) return;
          e.preventDefault();
          finish(true);
        } else if (e.key === "Escape") {
          e.preventDefault();
          finish(false);
        }
      }}
    />
  );
}
