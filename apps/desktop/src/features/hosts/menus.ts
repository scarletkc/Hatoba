import type { MenuEntry } from "@hatoba/ui";
import { dashboardSwitch, retryHost, setShownOnDashboard } from "@/features/dashboard/actions";
import { DASHBOARD_COLUMNS, shownOnDashboard, type DashboardColumn } from "@/features/dashboard/logic";
import type { HostReadout } from "@/features/dashboard/readings";
import { canRetry } from "@/features/dashboard/views";
import type { MessageKey, t as translateFn } from "@/i18n";
import { isTelnetHost, type AnyHostView, type GroupView, type ImportSource, type TagCount } from "@/ipc/types";
import { IMPORT_APPS } from "./ImportDialog";
import { LIST_SORTS, STATS_SORTS, isListSort, type HostSort } from "./ui";

type Translator = typeof translateFn;

export function hostSortLabel(t: Translator, key: HostSort): string {
  return t(isListSort(key) ? (`hosts.sort.${key}` as MessageKey) : (`dashboard.sort.${key}` as MessageKey));
}

export interface HostMenuContext {
  t: Translator;
  stats: boolean;
  readouts: Record<string, HostReadout>;
  groupMap: Map<string, GroupView>;
  onConnect: (id: string) => void;
  onEdit: (host: AnyHostView) => void;
  onDuplicate: (host: AnyHostView) => void;
  onFavorite: (list: AnyHostView[], favorite: boolean) => void;
  onCopyAddress: (host: AnyHostView) => void;
  onCopyPassword: (host: AnyHostView) => void;
  onDetails: (id: string) => void;
  onRemove: (list: AnyHostView[]) => void;
}

/** Resource-usage menu entries for a single SSH host while stats mode is on (HOST-14). */
export function buildHostStatsEntries(host: AnyHostView, ctx: HostMenuContext): MenuEntry[] {
  if (!ctx.stats || isTelnetHost(host)) return [];
  const r = ctx.readouts[host.id];
  const included = shownOnDashboard(host, ctx.groupMap);
  const toggle = dashboardSwitch(host, ctx.groupMap);
  return [
    { kind: "separator" },
    ...(included
      ? [
          {
            label: ctx.t("hosts.menu.details"),
            icon: "chart-line",
            hint: ctx.t("hosts.menu.detailsKey"),
            onSelect: () => ctx.onDetails(host.id),
          },
        ]
      : []),
    ...((canRetry(r) || r?.phase === "offline") && included
      ? [{ label: ctx.t("btn.retry"), icon: "arrow-clockwise", onSelect: () => retryHost(host.id) }]
      : []),
    { label: ctx.t(toggle.key), icon: toggle.icon, onSelect: toggle.run },
  ];
}

/** Context menu entries for a single host. */
export function buildHostMenuEntries(host: AnyHostView, ctx: HostMenuContext): MenuEntry[] {
  const { t } = ctx;
  return [
    { label: t("hosts.menu.connect"), icon: "plugs-connected", onSelect: () => ctx.onConnect(host.id) },
    { label: t("hosts.menu.edit"), icon: "pencil-simple", onSelect: () => ctx.onEdit(host) },
    { label: t("hosts.menu.duplicate"), icon: "copy", onSelect: () => ctx.onDuplicate(host) },
    {
      label: host.favorite ? t("hosts.menu.unfavorite") : t("hosts.menu.favorite"),
      icon: "star",
      onSelect: () => ctx.onFavorite([host], !host.favorite),
    },
    { label: t("hosts.menu.copyAddress"), icon: "clipboard-text", onSelect: () => ctx.onCopyAddress(host) },
    ...(host.has_password
      ? [{ label: t("hosts.menu.copyPassword"), icon: "password", onSelect: () => ctx.onCopyPassword(host) }]
      : []),
    ...buildHostStatsEntries(host, ctx),
    { kind: "separator" },
    { label: t("hosts.menu.delete"), icon: "trash", danger: true, onSelect: () => ctx.onRemove([host]) },
  ];
}

export interface MultiHostMenuActions {
  t: Translator;
  onConnectAll: (list: AnyHostView[]) => void;
  onFavorite: (list: AnyHostView[], favorite: boolean) => void;
  onRemove: (list: AnyHostView[]) => void;
}

/** Context menu entries for several selected hosts: what applies to all of them at once. */
export function buildMultiHostMenuEntries(list: AnyHostView[], actions: MultiHostMenuActions): MenuEntry[] {
  const { t } = actions;
  const favorite = list.every((h) => h.favorite);
  return [
    {
      label: t("hosts.menu.connect"),
      icon: "plugs-connected",
      onSelect: () => actions.onConnectAll(list),
    },
    {
      label: favorite ? t("hosts.menu.unfavorite") : t("hosts.menu.favorite"),
      icon: "star",
      onSelect: () => actions.onFavorite(list, !favorite),
    },
    { kind: "separator" },
    { label: t("hosts.menu.delete"), icon: "trash", danger: true, onSelect: () => actions.onRemove(list) },
  ];
}

export interface GroupMenuActions {
  t: Translator;
  stats: boolean;
  onRename: (group: GroupView) => void;
  onDelete: (group: GroupView) => void;
}

/** Context menu entries for a host group. */
export function buildGroupMenuEntries(group: GroupView, actions: GroupMenuActions): MenuEntry[] {
  const { t, stats } = actions;
  return [
    { label: t("sidebar.renameGroup"), icon: "pencil-simple", onSelect: () => actions.onRename(group) },
    ...(stats
      ? [
          group.dashboard
            ? {
                label: t("dashboard.leaveOutGroup"),
                icon: "eye-slash",
                onSelect: () => void setShownOnDashboard(group.id, false),
              }
            : {
                label: t("dashboard.includeGroup"),
                icon: "eye",
                onSelect: () => void setShownOnDashboard(group.id, true),
              },
        ]
      : []),
    { kind: "separator" },
    {
      label: t("sidebar.deleteGroup"),
      icon: "trash",
      danger: true,
      onSelect: () => actions.onDelete(group),
    },
  ];
}

export function buildNewMenuEntries(t: Translator, onNewHost: () => void, onNewGroup: () => void): MenuEntry[] {
  return [
    { label: t("hosts.newHost"), icon: "hard-drives", onSelect: onNewHost },
    { label: t("sidebar.newGroup"), icon: "folder-simple-plus", onSelect: onNewGroup },
  ];
}

export function buildImportMenuEntries(t: Translator, onImport: (source: ImportSource) => void): MenuEntry[] {
  return [
    { label: t("hosts.import.sshConfig"), hint: "~/.ssh/config", onSelect: () => onImport("ssh_config") },
    { kind: "separator" },
    ...Object.entries(IMPORT_APPS).map(([source, app]) => ({
      label: app,
      onSelect: () => onImport(source as ImportSource),
    })),
  ];
}

export function buildSortMenuEntries(
  t: Translator,
  opts: { stats: boolean; sort: HostSort; shownSort: HostSort; onSelect: (sort: HostSort) => void },
): MenuEntry[] {
  return [
    ...LIST_SORTS.map((k) => ({
      label: hostSortLabel(t, k),
      checked: opts.shownSort === k,
      onSelect: () => opts.onSelect(k),
    })),
    ...(opts.stats
      ? [
          { kind: "separator" } as const,
          ...STATS_SORTS.map((k) => ({
            label: hostSortLabel(t, k),
            checked: opts.sort === k,
            onSelect: () => opts.onSelect(k),
          })),
        ]
      : []),
  ];
}

export function buildTagsMenuEntries(
  t: Translator,
  opts: { tagList: readonly TagCount[]; activeTags: readonly string[]; onChange: (tags: string[]) => void },
): MenuEntry[] {
  const { tagList, activeTags, onChange } = opts;
  if (tagList.length === 0) return [{ label: t("hosts.tags.none"), disabled: true, onSelect: () => {} }];
  return [
    ...tagList.map((tag) => ({
      label: tag.name,
      hint: String(tag.count),
      checked: activeTags.includes(tag.name),
      keepOpen: true,
      onSelect: () =>
        onChange(activeTags.includes(tag.name) ? activeTags.filter((x) => x !== tag.name) : [...activeTags, tag.name]),
    })),
    ...(activeTags.length > 0
      ? [{ kind: "separator" } as const, { label: t("hosts.tags.clear"), icon: "x", onSelect: () => onChange([]) }]
      : []),
  ];
}

export function buildColumnsMenuEntries(
  t: Translator,
  opts: { hiddenColumns: readonly string[]; onChange: (hidden: string[]) => void },
): MenuEntry[] {
  const { hiddenColumns, onChange } = opts;
  return DASHBOARD_COLUMNS.map((c: DashboardColumn) => ({
    label: t(`dashboard.col.${c}` as MessageKey),
    checked: !hiddenColumns.includes(c),
    keepOpen: true,
    onSelect: () => onChange(hiddenColumns.includes(c) ? hiddenColumns.filter((x) => x !== c) : [...hiddenColumns, c]),
  }));
}
