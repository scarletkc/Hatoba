import { useEffect, useState } from "react";
import { Button, Icon } from "@/components/controls";
import { StatusTile } from "@/components/layout";
import { toast } from "@/components/overlay";
import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { formatRelative, useT, type MessageKey } from "@/i18n";
import { api } from "@/ipc/api";
import type { ConflictView } from "@/ipc/types";
import { refreshSyncStatus, type T } from "./syncUtils";
import s from "./Sync.module.css";

const TYPE_ICON: Record<ConflictView["item_type"], string> = {
  host: "hard-drives",
  key: "key",
  group: "folder-simple",
  known_host: "fingerprint",
  forward: "arrows-left-right",
  snippet: "terminal-window",
  proxy: "globe",
  settings: "gear",
  ai_provider: "sparkle",
  search_provider: "magnifying-glass",
  ai_conversation: "chat-circle-text",
  ai_message: "chat-text",
  skill: "book-open",
  skill_file: "file-text",
  mcp_server: "plugs",
};

/** Fields whose values read better in the monospace face (as in the design). */
const MONO_FIELDS = new Set(["name", "address", "port", "username", "key", "fingerprint", "base_url", "path"]);

/** Fields the backend sends as "true" or "false". */
const BOOL_FIELDS = new Set(["pinned", "enabled", "always_ask"]);

interface Row {
  field: string;
  local: string | null;
  remote: string | null;
}

/** Translates a field key such as "jump_host"; unknown keys fall back to the raw name. */
function fieldLabel(t: T, field: string): string {
  const key = `sync.field.${field}` as MessageKey;
  const label = t(key);
  return label === key ? field : label;
}

function describe(t: T, c: ConflictView): string {
  const type = t(`sync.type.${c.item_type}` as MessageKey);
  if (c.remote_deleted && !c.local_deleted) return t("sync.cf.desc.remoteDeleted", { type });
  if (c.local_deleted && !c.remote_deleted) return t("sync.cf.desc.localDeleted", { type });
  if (c.resolution === "kept_both") return t("sync.cf.desc.keptBoth", { type });
  return t("sync.cf.desc.both", { type });
}

/** Differing fields, plus a state row when one side deleted the item (design §05c). */
function rowsOf(c: ConflictView): Row[] {
  const rows: Row[] = [...c.fields];
  if ((c.local_deleted || c.remote_deleted) && !rows.some((r) => r.field === "state")) {
    rows.unshift({
      field: "state",
      local: c.local_deleted ? "deleted" : "modified",
      remote: c.remote_deleted ? "deleted" : "modified",
    });
  }
  return rows;
}

function ConflictCard({
  c,
  busy,
  onResolve,
}: {
  c: ConflictView;
  busy: "keep" | "restore" | null;
  onResolve: (action: "keep" | "restore") => void;
}) {
  const t = useT();
  const when = (ms: number | null) => (ms ? ` · ${formatRelative(t.locale, ms)}` : "");
  const value = (field: string, v: string | null, deleted: boolean) => {
    if (field === "state" && v) {
      const key = `sync.state.${v}` as MessageKey;
      const label = t(key);
      return { text: label === key ? v : label, dim: false };
    }
    if (BOOL_FIELDS.has(field) && (v === "true" || v === "false")) return { text: t(`sync.value.${v}`), dim: false };
    // A host's proxy (SSH-13) is a proxy's name, or one of these two.
    if (field === "proxy" && (v === "device_default" || v === "direct")) return { text: t(`sync.value.proxy.${v}`), dim: false };
    if (v === null || v === "") return { text: deleted ? "—" : t("sync.cf.none"), dim: true };
    return { text: v, dim: false };
  };

  return (
    <section className={s.conflict} aria-label={c.item_name}>
      <div className={s.conflictHead}>
        <Icon name={TYPE_ICON[c.item_type] ?? "circle"} className={s.conflictIcon} />
        <span className={s.conflictName}>{c.item_name}</span>
        <span className={s.conflictDesc}>{describe(t, c)}</span>
      </div>
      <div className={s.diff}>
        <span />
        <span className={s.diffHead}>
          {t("sync.cf.local")}
          {when(c.local_updated_at)}
        </span>
        <span className={s.diffHead}>
          {t("sync.cf.cloud")}
          {when(c.remote_updated_at)}
        </span>
        {rowsOf(c).map((r) => {
          const local = value(r.field, r.local, c.local_deleted);
          const remote = value(r.field, r.remote, c.remote_deleted);
          const mono = MONO_FIELDS.has(r.field);
          return (
            <div key={r.field} className={s.diffRow}>
              <span className={s.diffLabel}>
                <span className={s.diffDot} />
                {fieldLabel(t, r.field)}
              </span>
              <span className={`${s.diffValue} ${mono ? s.mono : ""} ${local.dim ? s.dim : ""} selectable`}>{local.text}</span>
              <span className={`${s.diffValue} ${mono ? s.mono : ""} ${remote.dim ? s.dim : ""} selectable`}>{remote.text}</span>
            </div>
          );
        })}
      </div>
      <div className={s.conflictFoot}>
        <span className={s.result}>
          <Icon name="check-circle" fill />
          {t(`sync.cf.result.${c.resolution}` as MessageKey)}
        </span>
        <Button size="sm" busy={busy === "keep"} disabled={busy !== null} onClick={() => onResolve("keep")}>
          {t("sync.cf.keep")}
        </Button>
        <Button size="sm" busy={busy === "restore"} disabled={busy !== null} onClick={() => onResolve("restore")}>
          {t("sync.cf.restore")}
        </Button>
      </div>
    </section>
  );
}

/**
 * Review of automatically resolved conflicts (spec §6.4, P1 "item-by-item review"): sync was not interrupted,
 * so each card offers to keep the result or restore the version that lost.
 */
export function SyncConflictsView({ onClose }: { onClose: () => void }) {
  const t = useT();
  const [list, setList] = useState<ConflictView[] | null>(null);
  const [busy, setBusy] = useState<{ id: number; action: "keep" | "restore" } | null>(null);

  useEffect(() => {
    let live = true;
    api
      .sync_conflicts()
      .then((l) => live && setList(l))
      .catch((e) => {
        toast(errorMessage(t, e), "error");
        if (live) setList([]);
      });
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const resolve = async (c: ConflictView, action: "keep" | "restore") => {
    setBusy({ id: c.id, action });
    try {
      await api.sync_conflict_resolve(c.id, action);
      const rest = (list ?? []).filter((x) => x.id !== c.id);
      setList(rest);
      await refreshSyncStatus();
      if (action === "restore") await useVaultData.getState().reload();
      if (rest.length === 0) onClose();
    } catch (e) {
      toast(errorMessage(t, e), "error");
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className={s.page}>
      <div className={s.scroll}>
        <div className={`${s.column} ${s.columnTight}`}>
          <div className={s.header}>
            <StatusTile icon="warning-circle" color="var(--orange)" />
            <div className={s.headerText}>
              <div className={s.title}>{t("sync.cf.title", { n: list?.length ?? 0 })}</div>
              <div className={s.sub}>{t("sync.cf.sub")}</div>
            </div>
            <Button onClick={onClose}>{t("btn.later")}</Button>
          </div>

          {list?.length === 0 && <div className={s.empty}>{t("sync.cf.empty")}</div>}
          {list?.map((c) => (
            <ConflictCard
              key={c.id}
              c={c}
              busy={busy?.id === c.id ? busy.action : null}
              onResolve={(action) => void resolve(c, action)}
            />
          ))}

          <div className={s.note}>
            <Icon name="shield-check" />
            {t("sync.cf.note")}
          </div>
        </div>
      </div>
    </div>
  );
}
