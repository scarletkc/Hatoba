import { useEffect, useState, type MouseEvent } from "react";
import { Button, Icon, TextField } from "@/components/controls";
import { Dialog, Menu, confirm, toast, useMenu } from "@/components/overlay";
import { openExternal } from "@/features/sync/external";
import { formatRelative, useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { GroupView, SyncStatus } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { REPO_URL, bugReportUrl } from "@/lib/github";
import { tagColor } from "@/lib/tags";
import { useVaultData } from "./data";
import { errorMessage } from "./errors";
import { starPromptDue, useStarPrompt } from "./star";
import { useApp, type HostFilter, type Page } from "./store";
import { useTabs } from "./tabs";
import { TitlebarDrag } from "./TitleBar";
import { selectUpdateAvailable, useUpdate } from "./update";
import st from "./Sidebar.module.css";

type Key = "all" | "favorites" | "recent" | `group:${string}` | `tag:${string}` | "keys" | "sync";

function pageKey(page: Page): Key | null {
  switch (page.kind) {
    case "hosts":
    case "host-edit": {
      const f: HostFilter = page.kind === "hosts" ? page.filter : page.back;
      if (f.kind === "group") return `group:${f.id}`;
      if (f.kind === "tag") return `tag:${f.name}`;
      return f.kind;
    }
    case "keys":
      return "keys";
    case "sync":
      return "sync";
  }
}

export function Sidebar() {
  const t = useT();
  const { page, navigate, info, sync, lock, openSettings } = useApp();
  const updateAvailable = useUpdate(selectUpdateAvailable);
  const settingsLabel = updateAvailable ? t("sidebar.settingsUpdate") : t("sidebar.settings");
  const activateHome = () => useTabs.getState().activate("home");
  const { hosts, groups, tags, keys } = useVaultData();
  const active = pageKey(page);
  const groupMenu = useMenu();
  const [menuGroup, setMenuGroup] = useState<GroupView | null>(null);
  const [editing, setEditing] = useState<GroupView | "new" | null>(null);

  const go = (p: Page) => {
    navigate(p);
    activateHome();
  };
  const hostsWith = (f: HostFilter) => go({ kind: "hosts", filter: f });
  const favorites = hosts.filter((h) => h.favorite).length;

  const item = (key: Key, icon: string, label: string, count: number | null, onClick: () => void, onContext?: (e: MouseEvent) => void) => (
    <button
      key={key}
      type="button"
      className={cx(st.item, active === key && st.itemActive)}
      aria-current={active === key ? "page" : undefined}
      onClick={onClick}
      onContextMenu={onContext}
    >
      <Icon name={icon} className={st.itemIcon} />
      <span className={st.itemLabel}>{label}</span>
      {count !== null && <span className={st.itemCount}>{count}</span>}
    </button>
  );

  const deleteGroup = async (g: GroupView) => {
    if (!(await confirm({ title: t("sidebar.deleteGroup"), body: t("sidebar.deleteGroupConfirm", { name: g.name }), confirmLabel: t("btn.delete"), danger: true })))
      return;
    try {
      await api.group_delete(g.id);
      if (active === `group:${g.id}`) hostsWith({ kind: "all" });
      await useVaultData.getState().reload();
    } catch (e) {
      toast(errorMessage(t, e), "error");
    }
  };

  return (
    <nav className={st.sidebar} aria-label="Hatoba">
      <TitlebarDrag className={st.header}>
        {info.platform === "macos" && <span className={st.traffic} />}
        <span style={{ flex: 1 }} data-tauri-drag-region />
        <button
          type="button"
          className={cx(st.footerButton, st.settingsButton)}
          title={settingsLabel}
          aria-label={settingsLabel}
          onClick={() => openSettings(true, updateAvailable ? "about" : undefined)}
        >
          <Icon name="gear-six" size={16} />
          {updateAvailable && <span className={st.updateDot} />}
        </button>
        <button type="button" className={st.footerButton} title={t("window.toggleSidebar")} aria-label={t("window.toggleSidebar")} onClick={() => useApp.getState().toggleSidebar()}>
          <Icon name="sidebar-simple" size={16} />
        </button>
      </TitlebarDrag>

      <div className={st.list}>
        {item("all", "hard-drives", t("sidebar.all"), hosts.length, () => hostsWith({ kind: "all" }))}
        {item("favorites", "star", t("sidebar.favorites"), favorites, () => hostsWith({ kind: "favorites" }))}
        {item("recent", "clock-counter-clockwise", t("sidebar.recent"), null, () => hostsWith({ kind: "recent" }))}

        <div className={st.heading}>
          <span>{t("sidebar.groups")}</span>
          <button type="button" className={st.headingButton} title={t("sidebar.newGroup")} aria-label={t("sidebar.newGroup")} onClick={() => setEditing("new")}>
            <Icon name="plus" />
          </button>
        </div>
        {groups.length === 0 && <div className={st.note}>{t("sidebar.noGroups")}</div>}
        {groups.map((g) =>
          item(
            `group:${g.id}`,
            "folder-simple",
            g.name,
            hosts.filter((h) => h.group_id === g.id).length,
            () => hostsWith({ kind: "group", id: g.id }),
            (e) => {
              e.preventDefault();
              setMenuGroup(g);
              groupMenu.openAt({ x: e.clientX, y: e.clientY });
            },
          ),
        )}

        {tags.length > 0 && <div className={st.heading}><span>{t("sidebar.tags")}</span></div>}
        {tags.map((tag) => (
          <button
            key={tag.name}
            type="button"
            className={cx(st.item, active === `tag:${tag.name}` && st.itemActive)}
            onClick={() => hostsWith({ kind: "tag", name: tag.name })}
          >
            <span className={st.itemDotWrap}>
              <span className={st.itemDot} style={{ background: tagColor(tag.name) }} />
            </span>
            <span className={st.itemLabel}>{tag.name}</span>
            <span className={st.itemCount}>{tag.count}</span>
          </button>
        ))}

        <div className={st.heading}><span>{t("sidebar.vault")}</span></div>
        {item("keys", "key", t("sidebar.keys"), keys.length, () => go({ kind: "keys" }))}
        {item("sync", "cloud", t("sidebar.sync"), null, () => go({ kind: "sync" }))}
      </div>

      <StarPrompt />
      <div className={st.footer}>
        <SyncFooter status={sync} onClick={() => go({ kind: "sync" })} />
        <button type="button" className={st.footerButton} title={t("sidebar.lock")} aria-label={t("sidebar.lock")} onClick={() => void lock()}>
          <Icon name="lock-simple" />
        </button>
      </div>

      {groupMenu.anchor && menuGroup && (
        <Menu
          anchor={groupMenu.anchor}
          onClose={groupMenu.close}
          entries={[
            { label: t("sidebar.renameGroup"), icon: "pencil-simple", onSelect: () => setEditing(menuGroup) },
            { kind: "separator" },
            { label: t("sidebar.deleteGroup"), icon: "trash", danger: true, onSelect: () => void deleteGroup(menuGroup) },
          ]}
        />
      )}
      {editing && <GroupDialog group={editing === "new" ? null : editing} count={groups.length} onClose={() => setEditing(null)} />}
    </nav>
  );
}

function GroupDialog({ group, count, onClose }: { group: GroupView | null; count: number; onClose: () => void }) {
  const t = useT();
  const [name, setName] = useState(group?.name ?? "");
  const [busy, setBusy] = useState(false);
  const save = async () => {
    if (!name.trim()) return;
    setBusy(true);
    try {
      await api.group_save({ id: group?.id ?? null, name: name.trim(), parent_id: group?.parent_id ?? null, sort: group?.sort ?? count });
      await useVaultData.getState().reload();
      onClose();
    } catch (e) {
      toast(errorMessage(t, e), "error");
      setBusy(false);
    }
  };
  return (
    <Dialog
      title={group ? t("sidebar.renameGroup") : t("sidebar.newGroup")}
      icon="folder-simple"
      onClose={onClose}
      actions={
        <>
          <Button onClick={onClose}>{t("btn.cancel")}</Button>
          <Button variant="primary" busy={busy} disabled={!name.trim()} onClick={() => void save()}>
            {t("btn.save")}
          </Button>
        </>
      }
    >
      <form
        style={{ marginTop: 14 }}
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <TextField large autoFocus value={name} placeholder={t("sidebar.groupName")} aria-label={t("sidebar.groupName")} maxLength={64} onChange={(e) => setName(e.target.value)} />
      </form>
    </Dialog>
  );
}

/** Asks for a GitHub star once it is due, on the home tab, and not while the update dot shows (spec §9). */
function StarPrompt() {
  const t = useT();
  const info = useApp((st) => st.info);
  const prompt = useStarPrompt((s) => s.prompt);
  const connectedOnce = useTabs((s) => s.connectedOnce);
  const home = useTabs((s) => s.active === "home");
  const updateAvailable = useUpdate(selectUpdateAvailable);
  useEffect(() => {
    void useStarPrompt.getState().load();
  }, []);

  if (!home || updateAvailable || !starPromptDue(prompt, connectedOnce, Date.now())) return null;
  const { finish } = useStarPrompt.getState();
  const open = (url: string) => {
    finish();
    void openExternal(url);
  };
  return (
    <section className={st.star} aria-label={t("sidebar.star.title")}>
      <div className={st.starHead}>
        <Icon name="star" fill className={st.starIcon} />
        <span className={st.starTitle}>{t("sidebar.star.title")}</span>
        <button type="button" className={st.headingButton} title={t("btn.close")} aria-label={t("btn.close")} onClick={finish}>
          <Icon name="x" />
        </button>
      </div>
      <p className={st.starBody}>{t("sidebar.star.body")}</p>
      <div className={st.starActions}>
        <Button size="sm" variant="primary" onClick={() => open(REPO_URL)}>
          {t("sidebar.star.star")}
        </Button>
        <Button size="sm" onClick={() => open(bugReportUrl(info))}>
          {t("sidebar.star.report")}
        </Button>
      </div>
    </section>
  );
}

function SyncFooter({ status, onClick }: { status: SyncStatus | null; onClick: () => void }) {
  const t = useT();
  const view = syncFooterView(t, status);
  return (
    <button type="button" className={st.syncButton} onClick={onClick}>
      <Icon name={view.icon} className={cx(st.syncIcon, view.spin && st.spin)} color={view.color} />
      <span className={st.syncText}>
        <div className={st.syncTitle}>{view.title}</div>
        <div className={st.syncSub}>{view.sub}</div>
      </span>
    </button>
  );
}

export function syncFooterView(t: ReturnType<typeof useT>, s: SyncStatus | null) {
  if (!s || s.kind === "none")
    return { icon: "cloud", color: "var(--fg2)", title: t("sync.footer.none"), sub: t("sync.footer.none_sub"), spin: false };
  switch (s.state) {
    case "syncing":
      return {
        icon: "arrows-clockwise",
        color: "var(--accent)",
        title: t("sync.footer.syncing"),
        sub: s.pending > 0 ? t("sync.footer.syncing_sub", { n: s.pending }) : t("sync.footer.syncing_sub_idle"),
        spin: true,
      };
    case "offline":
      return {
        icon: "cloud-slash",
        color: "var(--fg2)",
        title: t("sync.footer.offline"),
        sub: s.pending > 0 ? t("sync.footer.offline_sub", { n: s.pending }) : t("sync.footer.offline_sub_none"),
        spin: false,
      };
    case "auth_failed":
      return { icon: "cloud-warning", color: "var(--orange)", title: t("sync.footer.auth"), sub: t("sync.footer.auth_sub"), spin: false };
    case "error":
      return { icon: "cloud-warning", color: "var(--red)", title: t("sync.footer.error"), sub: t("sync.footer.error_sub"), spin: false };
    case "paused":
      return {
        icon: "cloud-warning",
        color: "var(--orange)",
        title: t(s.worker_update?.kind === "app_required" ? "sync.footer.appUpdate" : "sync.footer.workerUpdate"),
        sub: s.pending > 0 ? t("sync.footer.paused_sub", { n: s.pending }) : t("sync.footer.paused_sub_none"),
        spin: false,
      };
    case "off":
      return { icon: "cloud", color: "var(--fg2)", title: t("sync.footer.none"), sub: t("sync.footer.none_sub"), spin: false };
    default:
      if (s.conflicts > 0)
        return { icon: "warning-circle", color: "var(--orange)", title: t("sync.footer.conflict", { n: s.conflicts }), sub: t("sync.footer.conflict_sub"), spin: false };
      return {
        icon: "cloud-check",
        color: "var(--green)",
        title: t("sync.footer.synced"),
        sub: t("sync.footer.synced_sub", { time: s.last_synced_at ? formatRelative(t.locale, s.last_synced_at) : t("time.never") }),
        spin: false,
      };
  }
}
