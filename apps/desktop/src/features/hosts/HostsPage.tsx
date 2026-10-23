import {
  memo,
  useCallback,
  useDeferredValue,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent,
} from "react";
import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { useTabs, type TabStatus } from "@/app/tabs";
import { Button, Icon, IconButton, LinkButton, Segmented, Switch } from "@/components/controls";
import { EmptyState, PageHeader, SearchField, Tag, layoutStyles } from "@/components/layout";
import { Menu, confirm, toast, useMenu, type MenuEntry } from "@/components/overlay";
import { dashboardSwitch, retryHost, setShownOnDashboard } from "@/features/dashboard/actions";
import { HostDetail } from "@/features/dashboard/HostDetail";
import { DASHBOARD_COLUMNS, leftOutBy, shownOnDashboard, sortForDashboard, type DashboardColumn } from "@/features/dashboard/logic";
import { useDashboardWatch, useReadoutMap } from "@/features/dashboard/readings";
import { HostCard, StatsHead, StatsRow, canRetry, fitStatsColumns, useWidth, type Alerts, type ListState, type View } from "@/features/dashboard/views";
import { durationLabel } from "@/features/settings/durations";
import { usePrefs } from "@/features/settings/shared";
import { connectHost, connectTarget } from "@/features/terminal/connect";
import { useDocumentVisible } from "@/features/terminal/stats";
import { formatRelative, useT, type Locale, type MessageKey } from "@/i18n";
import { api } from "@/ipc/api";
import type { GroupView, HostView, ImportSource, LocalPrefs, QuickTarget } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { isImeEvent } from "@/lib/ime";
import { copyText } from "@/lib/native";
import { shortcutLabel } from "@/lib/platform";
import { GroupDialog, deleteGroup } from "./GroupDialog";
import { IMPORT_APPS, ImportDialog } from "./ImportDialog";
import { OsIcon } from "./OsIcon";
import {
  bracketHost,
  formatTarget,
  parseQuickConnect,
  sameTarget,
  savedHostFor,
  type QuickProblem,
  type TypedTarget,
} from "./quickConnect";
import { applyFilter, sortHosts } from "./order";
import { useRecentTargets } from "./recent";
import { buildHostIndex, searchHosts } from "./search";
import { LIST_SORTS, STATS_SORTS, isListSort, useHostsUi, type HostSort, type ListSort } from "./ui";
import s from "./HostsPage.module.css";

const PROBE_INTERVAL_MS = 60_000;
/** Hosts probed per refresh (HOST-10 probes what is on screen; the list is ordered, so the top comes first). */
const PROBE_LIMIT = 200;

let handledSearchTick = 0;

/** Row id of the typed quick-connect target (HOST-12); saved hosts use their item ids. */
const TYPED_ROW = "quick:typed";
const SUGGESTIONS_ID = "recent-targets";
/** A card's list state while resource usage shows, which then has no use for it. */
const NO_LIST: ListState = { dot: "var(--fg3)", last: "", failed: false };

/** A Connect row for a target that is not a saved host: the one typed, or a recent one. */
interface QuickRowItem {
  id: string;
  target: TypedTarget;
  /** Set when the target is in the recent list, so the row can remove it. */
  recent: QuickTarget | null;
}

/**
 * Every saved host, or a group's (HOST-03), narrowed to favorites and tags, as a list or cards. With resource usage
 * on, the listed hosts are read in the background as the dashboard did (HOST-14).
 */
export function HostsPage({ group: requested }: { group: string | null }) {
  const t = useT();
  const platform = useApp((st) => st.info.platform);
  const hostProbe = useApp((st) => st.prefs.host_probe);
  const searchTick = useApp((st) => st.searchFocusTick);
  const navigate = useApp((st) => st.navigate);
  const [prefs, savePrefs] = usePrefs();
  const hosts = useVaultData((st) => st.hosts);
  const groups = useVaultData((st) => st.groups);
  const tagList = useVaultData((st) => st.tags);
  const tabs = useTabs((st) => st.tabs);
  const recents = useRecentTargets((st) => st.targets);
  const { selectedId, sort, favorites, tags: pickedTags, select, setSort, setFavorites, setTags } = useHostsUi();
  const view = prefs.hosts_view;
  const stats = prefs.hosts_stats;
  const readouts = useReadoutMap();
  const docVisible = useDocumentVisible();

  const [query, setQuery] = useState("");
  const deferredQuery = useDeferredValue(query);
  const [probe, setProbe] = useState<Record<string, boolean>>({});
  const [importing, setImporting] = useState<ImportSource | null>(null);
  const [ctxHost, setCtxHost] = useState<HostView | null>(null);
  const [ctxGroup, setCtxGroup] = useState<GroupView | null>(null);
  const [editingGroup, setEditingGroup] = useState<GroupView | "new" | null>(null);
  const [detail, setDetail] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const [searchFocused, setSearchFocused] = useState(false);
  const [suggestDismissed, setSuggestDismissed] = useState(false);
  const [suggestAt, setSuggestAt] = useState(-1);
  /** The query at which the user last picked a row; until then the typed target stays selected. */
  const [pickedFor, setPickedFor] = useState<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const cardsRef = useRef<HTMLDivElement>(null);
  const rowMenu = useMenu();
  const openRowMenu = rowMenu.openAt;
  const groupMenu = useMenu();
  const sortMenu = useMenu();
  const sortButton = useRef<HTMLButtonElement>(null);
  const tagsMenu = useMenu();
  const tagsButton = useRef<HTMLButtonElement>(null);
  const columnsMenu = useMenu();
  const importMenu = useMenu();
  const importButton = useRef<HTMLButtonElement>(null);
  const newMenu = useMenu();
  const newMenuButton = useRef<HTMLButtonElement>(null);

  const groupMap = useMemo(() => new Map(groups.map((g) => [g.id, g])), [groups]);
  const scope = requested ? groupMap.get(requested) : undefined;
  // A group that sync or another window deleted leaves every host.
  const group = scope ? requested : null;
  const title = scope?.name ?? t("hosts.title");
  // Tags no host has any more drop out of the filter.
  const activeTags = useMemo(() => pickedTags.filter((tag) => tagList.some((x) => x.name === tag)), [pickedTags, tagList]);
  const filtering = favorites || activeTags.length > 0;

  // Filter + sort only when inputs change; typing in the search box touches neither.
  const filtered = useMemo(() => applyFilter(hosts, { group, favorites, tags: activeTags }), [hosts, group, favorites, activeTags]);
  const listSort: ListSort = isListSort(sort) ? sort : "recent";
  const base = useMemo(() => sortHosts(filtered, listSort, t.locale), [filtered, listSort, t.locale]);
  const index = useMemo(() => buildHostIndex(base), [base]);
  // HOST-12: `user@host` and `ssh …` input is also a target to connect to. The list is then
  // filtered by its destination, without the ssh syntax around it.
  const parsed = useMemo(() => parseQuickConnect(deferredQuery), [deferredQuery]);
  const searchText = parsed ? parsed.search : deferredQuery;
  const searching = deferredQuery.trim() !== "";
  const found = useMemo(() => searchHosts(index, searchText), [index, searchText]);
  // HOST-14: a resource order sorts again with each reading; a search keeps its ranking.
  const statsSort = stats && !isListSort(sort) && !searching ? sort : null;
  const ordered = useMemo(() => (statsSort ? sortForDashboard(found, statsSort, readouts, t.locale) : found), [found, statsSort, readouts, t.locale]);
  // A typed target that is a saved host connects as that host, with its key and jump host.
  const exact = useMemo(() => (parsed?.ok ? savedHostFor(hosts, parsed.target) : undefined), [parsed, hosts]);
  const visible = useMemo(() => (exact && !ordered.includes(exact) ? [exact, ...ordered] : ordered), [exact, ordered]);
  // Recent targets that have been saved as hosts since show as those hosts.
  const unsavedRecents = useMemo(() => recents.filter((r) => !savedHostFor(hosts, r)), [recents, hosts]);
  const quickRows = useMemo(() => {
    const rows: QuickRowItem[] = [];
    if (parsed?.ok && !exact)
      rows.push({ id: TYPED_ROW, target: parsed.target, recent: unsavedRecents.find((r) => sameTarget(r, parsed.target)) ?? null });
    const q = searchText.trim().toLowerCase();
    if (q)
      for (const r of unsavedRecents)
        if (!(parsed?.ok && sameTarget(r, parsed.target)) && formatTarget(r).toLowerCase().includes(q))
          rows.push({ id: `quick:recent:${formatTarget(r)}`, target: r, recent: r });
    return rows;
  }, [parsed, exact, unsavedRecents, searchText]);
  const problem = parsed && !parsed.ok ? parsed.problem : null;
  const notice = parsed?.ok ? parsed.notice : undefined;

  const hostName = useMemo(() => new Map(hosts.map((h) => [h.id, h.name])), [hosts]);

  // The latest tab per host tells us whether the last attempt failed (design: red "连接失败").
  const tabStatus = useMemo(() => {
    const m = new Map<string, TabStatus>();
    for (const tab of tabs) if (tab.hostId) m.set(tab.hostId, tab.status);
    return m;
  }, [tabs]);

  // HOST-14: the listed hosts are read while resource usage shows; the search does not change which.
  const interval = prefs.dashboard_interval_secs;
  const watched = useMemo(() => filtered.filter((h) => shownOnDashboard(h, groupMap)).map((h) => h.id), [filtered, groupMap]);
  useDashboardWatch(watched, interval, stats && docVisible);
  const { dashboard_cpu_alert: cpuAlert, dashboard_mem_alert: memAlert, dashboard_disk_alert: diskAlert } = prefs;
  const alerts: Alerts = useMemo(() => ({ cpu: cpuAlert, mem: memAlert, disk: diskAlert }), [cpuAlert, memAlert, diskAlert]);
  const hiddenColumns = prefs.dashboard_hidden_columns;
  const [measureTable, tableWidth] = useWidth();
  const columns = useMemo(
    () => fitStatsColumns(DASHBOARD_COLUMNS.filter((c) => !hiddenColumns.includes(c)), tableWidth),
    [hiddenColumns, tableWidth],
  );

  const rowIds = useMemo(() => [...quickRows.map((r) => r.id), ...visible.map((h) => h.id)], [quickRows, visible]);
  // The typed target, or the saved host it is, is selected until the user picks another row; otherwise
  // the best saved host is, even with recent targets listed above it.
  const preferred = exact?.id ?? (quickRows[0]?.id === TYPED_ROW ? TYPED_ROW : null);
  const effectiveId = useMemo(() => {
    const picked = pickedFor === deferredQuery;
    if (selectedId && rowIds.includes(selectedId) && (picked || (!preferred && !problem))) return selectedId;
    // An ssh command quick connect cannot run selects nothing, so Enter does not connect a saved
    // host with other settings than the ones typed.
    if (problem) return null;
    return preferred ?? visible[0]?.id ?? quickRows[0]?.id ?? null;
  }, [rowIds, selectedId, preferred, problem, pickedFor, deferredQuery, visible, quickRows]);

  // The row callbacks read these through a ref, so they stay stable and memoized rows skip re-rendering.
  const latest = useRef({ deferredQuery, quickRows });
  useLayoutEffect(() => {
    latest.current = { deferredQuery, quickRows };
  });

  const pick = useCallback(
    (id: string) => {
      setPickedFor(latest.current.deferredQuery);
      select(id);
    },
    [select],
  );

  // Recent targets under the empty field (HOST-12).
  const suggestions = query.trim() ? [] : unsavedRecents;
  const suggestOpen = searchFocused && !suggestDismissed && suggestions.length > 0;
  const suggestActive = Math.min(suggestAt, suggestions.length - 1);
  const removeRecent = useRecentTargets((st) => st.remove);

  const onQueryChange = (value: string) => {
    setQuery(value);
    setSuggestDismissed(false);
    setSuggestAt(-1);
  };

  const onSearchFocus = () => {
    setSearchFocused(true);
    setSuggestDismissed(false);
    setSuggestAt(-1);
    void useRecentTargets.getState().load();
  };

  const connectQuick = (target: TypedTarget) => {
    setSuggestDismissed(true);
    void connectTarget(target);
  };

  // Relative times ("2 min ago") refresh once a minute; with readings, old ones show as old within seconds.
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), stats ? 5000 : 60_000);
    return () => window.clearInterval(timer);
  }, [stats]);

  // Ctrl+Shift+K focuses the search field (a page mounted by the shortcut handles the tick too).
  useEffect(() => {
    if (searchTick === handledSearchTick) return;
    handledSearchTick = searchTick;
    // A `hatoba …` command line that cannot connect (HOST-13).
    const text = useApp.getState().searchFocusText;
    if (text !== null) setQuery(text);
    searchRef.current?.focus();
    searchRef.current?.select();
  }, [searchTick]);

  useEffect(() => {
    if (searchTick === handledSearchTick) listRef.current?.focus({ preventScroll: true });
    // Only on mount: don't steal focus when the group changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // HOST-10: TCP reachability of the hosts in this view, now and every 60 s.
  const probeKey = useMemo(
    () =>
      base
        .slice(0, PROBE_LIMIT)
        .map((h) => h.id)
        .sort()
        .join(","),
    [base],
  );
  useEffect(() => {
    if (!hostProbe || !probeKey) return;
    const ids = probeKey.split(",");
    let cancelled = false;
    const run = async () => {
      try {
        const results = await api.hosts_probe(ids);
        if (cancelled) return;
        setProbe((prev) => {
          const next = { ...prev };
          for (const r of results) next[r.id] = r.online;
          return next;
        });
      } catch {
        /* probing is best effort */
      }
    };
    void run();
    const timer = window.setInterval(() => void run(), PROBE_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [probeKey, hostProbe]);

  const newHost = useCallback(() => navigate({ kind: "host-edit", hostId: null, groupId: group, back: group }), [navigate, group]);
  const editHost = useCallback((id: string) => navigate({ kind: "host-edit", hostId: id, groupId: null, back: group }), [navigate, group]);

  const connect = useCallback(
    (id: string) => {
      const quick = latest.current.quickRows.find((r) => r.id === id);
      if (quick) {
        void connectTarget(quick.target);
        return;
      }
      select(id);
      void connectHost(id);
    },
    [select],
  );

  const onContext = useCallback(
    (e: MouseEvent, host: HostView) => {
      e.preventDefault();
      select(host.id);
      setCtxHost(host);
      openRowMenu({ x: e.clientX, y: e.clientY });
    },
    [select, openRowMenu],
  );

  // Each card's last connection, made only without resource usage, so a batch of readings leaves every card's
  // props as they were and only the hosts that changed render again.
  const listStates = useMemo(() => {
    if (stats) return null;
    const states = new Map<string, ListState>();
    for (const h of visible) {
      const status = tabStatus.get(h.id);
      const failed = status === "failed";
      states.set(h.id, {
        dot: status === "connected" ? "var(--green)" : failed ? "var(--red)" : probe[h.id] ? "var(--green)" : "var(--fg3)",
        last: failed ? t("hosts.failed") : h.last_connected_at ? formatRelative(t.locale, h.last_connected_at, now) : t("hosts.neverConnected"),
        failed,
      });
    }
    return states;
  }, [stats, visible, tabStatus, probe, t, now]);

  const tagRow = useMemo(() => visible.some((h) => h.tags.length > 0), [visible]);
  const itemView: View = useMemo(
    () => ({
      t,
      now,
      interval,
      alerts,
      groups: groupMap,
      stats,
      tagRow,
      connectLabel: t("hosts.connect"),
      onSelect: pick,
      onConnect: connect,
      onContext,
    }),
    [t, now, interval, alerts, groupMap, stats, tagRow, pick, connect, onContext],
  );

  const openGroupMenu = (e: MouseEvent, g: GroupView) => {
    e.preventDefault();
    setCtxGroup(g);
    groupMenu.openAt({ x: e.clientX, y: e.clientY });
  };

  const fail = (e: unknown) => toast(errorMessage(t, e), "error");

  const toggleFavorite = async (host: HostView) => {
    try {
      await api.host_set_favorite(host.id, !host.favorite);
      await useVaultData.getState().reload();
    } catch (e) {
      fail(e);
    }
  };

  const duplicate = async (host: HostView) => {
    try {
      const copy = await api.host_duplicate(host.id);
      await useVaultData.getState().reload();
      select(copy.id);
      toast(t("hosts.duplicated", { name: copy.name }), "success");
    } catch (e) {
      fail(e);
    }
  };

  const remove = async (host: HostView) => {
    const ok = await confirm({
      title: t("hosts.deleteTitle", { name: host.name }),
      body: t("hosts.deleteBody"),
      confirmLabel: t("btn.delete"),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.host_delete(host.id);
      await useVaultData.getState().reload();
      toast(t("hosts.deleted", { name: host.name }));
    } catch (e) {
      fail(e);
    }
  };

  const copyAddress = async (host: HostView) => {
    const text = `${host.username ? `${host.username}@` : ""}${host.address}${host.port !== 22 ? ` -p ${host.port}` : ""}`;
    try {
      await copyText(text);
      toast(t("hosts.copied"), "success");
    } catch (e) {
      fail(e);
    }
  };

  // SEC-08: the backend copies the saved password and clears the clipboard after this device's delay.
  const copyPassword = async (host: HostView) => {
    try {
      await api.host_copy_password(host.id);
      const secs = useApp.getState().prefs.clipboard_clear_secs;
      toast(secs ? t("hosts.passwordCopied", { time: durationLabel(t, secs) }) : t("hosts.passwordCopiedKept"), "success");
    } catch (e) {
      fail(e);
    }
  };

  // HOST-14: with resource usage showing, the host's detail, Retry, and whether it is read.
  const statsEntries = (host: HostView): MenuEntry[] => {
    if (!stats) return [];
    const r = readouts[host.id];
    const included = shownOnDashboard(host, groupMap);
    const toggle = dashboardSwitch(host, groupMap);
    return [
      { kind: "separator" },
      ...(included ? [{ label: t("hosts.menu.details"), icon: "chart-line", hint: t("hosts.menu.detailsKey"), onSelect: () => setDetail(host.id) }] : []),
      ...((canRetry(r) || r?.phase === "offline") && included ? [{ label: t("btn.retry"), icon: "arrow-clockwise", onSelect: () => retryHost(host.id) }] : []),
      { label: t(toggle.key), icon: toggle.icon, onSelect: toggle.run },
    ];
  };

  const menuEntries = (host: HostView): MenuEntry[] => [
    { label: t("hosts.menu.connect"), icon: "plugs-connected", onSelect: () => connect(host.id) },
    { label: t("hosts.menu.edit"), icon: "pencil-simple", onSelect: () => editHost(host.id) },
    { label: t("hosts.menu.duplicate"), icon: "copy", onSelect: () => void duplicate(host) },
    {
      label: host.favorite ? t("hosts.menu.unfavorite") : t("hosts.menu.favorite"),
      icon: "star",
      onSelect: () => void toggleFavorite(host),
    },
    { label: t("hosts.menu.copyAddress"), icon: "clipboard-text", onSelect: () => void copyAddress(host) },
    ...(host.has_password
      ? [{ label: t("hosts.menu.copyPassword"), icon: "password", onSelect: () => void copyPassword(host) }]
      : []),
    ...statsEntries(host),
    { kind: "separator" },
    { label: t("hosts.menu.delete"), icon: "trash", danger: true, onSelect: () => void remove(host) },
  ];

  const groupEntries = (g: GroupView): MenuEntry[] => [
    { label: t("sidebar.renameGroup"), icon: "pencil-simple", onSelect: () => setEditingGroup(g) },
    ...(stats
      ? [
          g.dashboard
            ? { label: t("dashboard.leaveOutGroup"), icon: "eye-slash", onSelect: () => void setShownOnDashboard(g.id, false) }
            : { label: t("dashboard.includeGroup"), icon: "eye", onSelect: () => void setShownOnDashboard(g.id, true) },
        ]
      : []),
    { kind: "separator" },
    {
      label: t("sidebar.deleteGroup"),
      icon: "trash",
      danger: true,
      onSelect: () =>
        void deleteGroup(t, g).then((deleted) => {
          if (deleted && group === g.id) navigate({ kind: "hosts", group: null });
        }),
    },
  ];

  /** The number of cards in a row, for ↑/↓ in the cards view. */
  const cardsPerRow = () => {
    const el = cardsRef.current;
    if (!el) return 1;
    return Math.max(1, getComputedStyle(el).gridTemplateColumns.split(" ").filter(Boolean).length);
  };

  const move = (delta: number) => {
    if (rowIds.length === 0) return;
    const i = effectiveId ? rowIds.indexOf(effectiveId) : -1;
    const next = rowIds[Math.max(0, Math.min(rowIds.length - 1, i + delta))];
    pick(next);
    requestAnimationFrame(() =>
      listRef.current?.querySelector(`[data-id="${CSS.escape(next)}"]`)?.scrollIntoView({ block: "nearest" }),
    );
  };

  // ↑/↓ move the selection, Enter connects (HOST-07); among cards ←/→ too. Works from the list and from the search field.
  const onKeyDown = (e: KeyboardEvent) => {
    const target = e.target as HTMLElement;
    const inSearch = target === searchRef.current;
    const onList = target === listRef.current;
    if (!inSearch && !onList) return;
    // The Enter that commits an IME candidate must not connect.
    if (e.key === "Enter" && isImeEvent(e)) return;
    // While the recent targets show under the field, the arrows go through them first.
    if (inSearch && suggestOpen) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const n = suggestions.length;
        setSuggestAt(e.key === "ArrowDown" ? Math.min(n - 1, suggestActive + 1) : Math.max(-1, suggestActive - 1));
        return;
      }
      if (e.key === "Enter" && suggestActive >= 0) {
        e.preventDefault();
        connectQuick(suggestions[suggestActive]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setSuggestDismissed(true);
        return;
      }
    }
    const amongCards = view === "cards" && !quickRows.some((r) => r.id === effectiveId);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const step = amongCards ? cardsPerRow() : 1;
      move(e.key === "ArrowDown" ? step : -step);
    } else if (onList && amongCards && (e.key === "ArrowRight" || e.key === "ArrowLeft")) {
      e.preventDefault();
      move(e.key === "ArrowRight" ? 1 : -1);
    } else if (onList && e.key === " " && stats && effectiveId && hosts.some((h) => h.id === effectiveId)) {
      // Space shows the selected host's detail, as Quick Look does.
      e.preventDefault();
      setDetail(effectiveId);
    } else if (e.key === "Enter" && effectiveId === TYPED_ROW) {
      e.preventDefault();
      // The list may still show an earlier keystroke: connect what the field says now.
      const typed = parseQuickConnect(query);
      if (!typed?.ok) return;
      const saved = savedHostFor(hosts, typed.target);
      if (saved) connect(saved.id);
      else void connectTarget(typed.target);
    } else if (e.key === "Enter" && effectiveId) {
      e.preventDefault();
      connect(effectiveId);
    } else if (onList && e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey && e.key !== " ") {
      // Type to search.
      searchRef.current?.focus();
    }
  };

  // New Host, with New Group under the chevron beside it, as in Termius.
  const newControls = (
    <span className={s.split}>
      <Button variant="primary" icon="plus" onClick={newHost}>
        {t("hosts.newHost")}
      </Button>
      <Button
        ref={newMenuButton}
        variant="primary"
        icon="caret-down"
        className={s.splitCaret}
        aria-label={t("hosts.newMenu")}
        title={t("hosts.newMenu")}
        aria-haspopup="menu"
        aria-expanded={!!newMenu.anchor}
        onClick={() => newMenuButton.current && newMenu.openBelow(newMenuButton.current, true)}
      />
      {newMenu.anchor && (
        <Menu
          anchor={newMenu.anchor}
          onClose={newMenu.close}
          minWidth={160}
          entries={[
            { label: t("hosts.newHost"), icon: "hard-drives", onSelect: newHost },
            { label: t("sidebar.newGroup"), icon: "folder-simple-plus", onSelect: () => setEditingGroup("new") },
          ]}
        />
      )}
    </span>
  );

  // SSH-11, SSH-12, SSH-15: `~/.ssh/config`, then the other SSH clients.
  const importControls = (
    <>
      <Button
        ref={importButton}
        icon="file-arrow-down"
        trailingIcon="caret-down"
        aria-haspopup="menu"
        aria-expanded={!!importMenu.anchor}
        onClick={() => importButton.current && importMenu.openBelow(importButton.current, true)}
      >
        {t("hosts.import")}
      </Button>
      {importMenu.anchor && (
        <Menu
          anchor={importMenu.anchor}
          onClose={importMenu.close}
          minWidth={180}
          entries={[
            { label: t("hosts.import.sshConfig"), hint: "~/.ssh/config", onSelect: () => setImporting("ssh_config") },
            { kind: "separator" },
            ...Object.entries(IMPORT_APPS).map(([source, app]) => ({
              label: app,
              onSelect: () => setImporting(source as ImportSource),
            })),
          ]}
        />
      )}
    </>
  );
  const dialogs = (
    <>
      {importing && <ImportDialog source={importing} onClose={() => setImporting(null)} />}
      {editingGroup && <GroupDialog group={editingGroup === "new" ? null : editingGroup} count={groups.length} onClose={() => setEditingGroup(null)} />}
      {groupMenu.anchor && ctxGroup && <Menu anchor={groupMenu.anchor} onClose={groupMenu.close} entries={groupEntries(ctxGroup)} />}
    </>
  );

  const searchField = (
    <div className={s.searchWrap}>
      <SearchField
        ref={searchRef}
        value={query}
        onChange={onQueryChange}
        placeholder={t("hosts.search")}
        shortcut={shortcutLabel(platform, "Ctrl+Shift+K", "⌘K")}
        width={300}
        onFocus={onSearchFocus}
        onBlur={() => setSearchFocused(false)}
        popup={{
          id: SUGGESTIONS_ID,
          open: suggestOpen,
          active: suggestActive >= 0 ? `${SUGGESTIONS_ID}-${suggestActive}` : undefined,
        }}
      />
      {suggestOpen && (
        <RecentSuggestions
          targets={suggestions}
          active={suggestActive}
          title={t("hosts.quick.recentTitle")}
          removeLabel={t("hosts.quick.forget")}
          onHover={setSuggestAt}
          onConnect={connectQuick}
          onRemove={(r) => void removeRecent(r)}
        />
      )}
    </div>
  );

  // A group's page is a step down from every host, with the group's menu beside its name.
  const titleNode = scope ? (
    <span className={s.crumbs}>
      <button type="button" className={s.crumb} onClick={() => navigate({ kind: "hosts", group: null })}>
        {t("hosts.title")}
      </button>
      <Icon name="caret-right" className={s.crumbSep} />
      <span className={s.crumbHere}>{scope.name}</span>
      <IconButton icon="dots-three" label={t("dashboard.groupMenu")} onClick={(e) => openGroupMenu(e, scope)} />
    </span>
  ) : (
    title
  );

  // At the top level, the groups to step into, as in Termius; every host is listed below them.
  const groupBar = group === null && groups.length > 0 && (
    <div className={s.groupBar} role="list" aria-label={t("sidebar.groups")}>
      {groups.map((g) => (
        <button
          key={g.id}
          type="button"
          role="listitem"
          className={s.groupTile}
          onClick={() => navigate({ kind: "hosts", group: g.id })}
          onContextMenu={(e) => openGroupMenu(e, g)}
        >
          <Icon name="folder-simple" className={s.groupIcon} />
          <span className={s.groupName}>{g.name}</span>
          <span className={s.groupCount}>{hosts.filter((h) => h.group_id === g.id).length}</span>
        </button>
      ))}
    </div>
  );

  // No hosts at all (design §01b), though groups made meanwhile stay reachable. Quick connect still works from the search field.
  if (hosts.length === 0 && !query.trim()) {
    return (
      <div className={s.page} onKeyDown={onKeyDown}>
        <PageHeader title={titleNode} search={searchField}>
          {newControls}
        </PageHeader>
        {groupBar}
        <EmptyState
          icon="hard-drives"
          title={t("hosts.empty.title")}
          body={t("hosts.empty.body")}
          actions={
            <>
              <Button variant="primary" icon="plus" onClick={newHost}>
                {t("hosts.newHost")}
              </Button>
              {importControls}
            </>
          }
          foot={
            <>
              <span>{t("hosts.empty.sync")}</span>
              <LinkButton onClick={() => navigate({ kind: "sync" })}>{t("hosts.empty.syncLink")}</LinkButton>
            </>
          }
        />
        {dialogs}
      </div>
    );
  }

  const sortLabel = (k: HostSort) => t(isListSort(k) ? (`hosts.sort.${k}` as MessageKey) : (`dashboard.sort.${k}` as MessageKey));
  const shownSort = stats ? sort : listSort;

  const hasQuick = quickRows.length > 0 || problem !== null;
  const emptyBody = (() => {
    if (base.length > 0 || hosts.length === 0 || hasQuick || exact) return null;
    return filtering ? ("filter" as const) : ("group" as const);
  })();

  const toolbar = (
    <div className={s.toolbar}>
      <button type="button" className={cx(s.toggle, favorites && s.toggleOn)} aria-pressed={favorites} onClick={() => setFavorites(!favorites)}>
        <Icon name="star" fill={favorites} />
        {t("sidebar.favorites")}
      </button>
      <button
        ref={tagsButton}
        type="button"
        className={cx(s.toggle, activeTags.length > 0 && s.toggleOn)}
        aria-haspopup="menu"
        aria-expanded={!!tagsMenu.anchor}
        onClick={() => tagsButton.current && tagsMenu.openBelow(tagsButton.current)}
      >
        <Icon name="tag" />
        {t("sidebar.tags")}
        {activeTags.length > 0 && <span className={s.toggleCount}>{activeTags.length}</span>}
        <Icon name="caret-down" className={s.toggleCaret} />
      </button>
      {activeTags.map((tag) => (
        <button key={tag} type="button" className={s.tagChip} title={t("hosts.tags.remove", { tag })} onClick={() => setTags(activeTags.filter((x) => x !== tag))}>
          <Tag name={tag} />
          <Icon name="x" className={s.tagChipX} />
        </button>
      ))}
      <span className={s.toolbarGap} />
      <Button
        ref={sortButton}
        size="sm"
        trailingIcon="caret-down"
        aria-haspopup="menu"
        aria-expanded={!!sortMenu.anchor}
        aria-label={t("hosts.sort")}
        onClick={() => sortButton.current && sortMenu.openBelow(sortButton.current, true)}
      >
        {sortLabel(shownSort)}
      </Button>
      <label className={s.statsSwitch} title={t("hosts.stats.hint")}>
        <Switch checked={stats} label={t("hosts.stats")} onChange={(hosts_stats) => savePrefs({ hosts_stats })} />
        <span>{t("hosts.stats")}</span>
      </label>
      <Segmented<LocalPrefs["hosts_view"]>
        ariaLabel={t("hosts.view")}
        className={s.viewSwitch}
        value={view}
        options={[
          { value: "list", label: t("hosts.view.list"), title: t("hosts.view.list"), icon: "rows" },
          { value: "cards", label: t("dashboard.layout.cards"), title: t("dashboard.layout.cards"), icon: "squares-four" },
        ]}
        onChange={(hosts_view) => savePrefs({ hosts_view })}
      />
    </div>
  );

  const quickList = (
    <>
      {problem && <ProblemRow problem={problem} />}
      {notice && <NoticeRow />}
      {quickRows.map((r) => (
        <QuickRow
          key={r.id}
          item={r}
          selected={r.id === effectiveId}
          connectLabel={t("hosts.connect")}
          hint={r.recent ? t("hosts.quick.recent") : t("hosts.quick.notSaved")}
          removeLabel={t("hosts.quick.forget")}
          onSelect={pick}
          onConnect={connect}
          onRemove={(target) => void removeRecent(target)}
        />
      ))}
    </>
  );

  let body;
  if (emptyBody === "filter") {
    body = (
      <EmptyState
        icon="funnel"
        title={t("hosts.emptyFilter.title")}
        body={t("hosts.emptyFilter.body")}
        actions={
          <Button
            onClick={() => {
              setFavorites(false);
              setTags([]);
            }}
          >
            {t("hosts.emptyFilter.clear")}
          </Button>
        }
      />
    );
  } else if (emptyBody === "group") {
    body = (
      <EmptyState
        icon="folder-simple"
        title={t("hosts.emptyGroup.title")}
        body={t("hosts.emptyGroup.body")}
        actions={
          <Button variant="primary" icon="plus" onClick={newHost}>
            {t("hosts.newHost")}
          </Button>
        }
      />
    );
  } else if (visible.length === 0 && searching && !hasQuick) {
    body = (
      <EmptyState
        icon="magnifying-glass"
        title={t("hosts.noResults.title")}
        body={t("hosts.noResults.body", { q: deferredQuery.trim() })}
        actions={<Button onClick={() => setQuery("")}>{t("hosts.noResults.clear")}</Button>}
      />
    );
  } else {
    body = (
      <div ref={listRef} className={s.scroll} tabIndex={0} role="listbox" aria-label={title}>
        {view === "list" &&
          (stats ? (
            <StatsHead
              columns={columns}
              t={t}
              measure={measureTable}
              columnsOpen={!!columnsMenu.anchor}
              onColumns={(anchor) => columnsMenu.openBelow(anchor, true)}
            />
          ) : (
            <div className={cx(layoutStyles.tableHead, s.cols, s.head)}>
              <span className={s.headName}>{t("hosts.col.name")}</span>
              <span>{t("hosts.col.address")}</span>
              <span>{t("hosts.col.tags")}</span>
              <span>{t("hosts.col.last")}</span>
              <span />
            </div>
          ))}
        {view === "list" ? (
          <div className={s.rows}>
            {quickList}
            {visible.map((h) =>
              stats ? (
                <StatsRow
                  key={h.id}
                  host={h}
                  selected={h.id === effectiveId}
                  readout={readouts[h.id]}
                  leftOut={leftOutBy(h, groupMap)}
                  columns={columns}
                  view={itemView}
                />
              ) : (
                <HostRow
                  key={h.id}
                  host={h}
                  selected={h.id === effectiveId}
                  jumpName={h.jump_host_id ? (hostName.get(h.jump_host_id) ?? null) : null}
                  status={tabStatus.get(h.id)}
                  online={probe[h.id]}
                  now={now}
                  locale={t.locale}
                  connectLabel={t("hosts.connect")}
                  failedLabel={t("hosts.failed")}
                  viaTitle={h.jump_host_id ? t("hosts.viaTitle", { name: hostName.get(h.jump_host_id) ?? "" }) : ""}
                  onSelect={pick}
                  onConnect={connect}
                  onContext={onContext}
                />
              ),
            )}
          </div>
        ) : (
          <>
            {(hasQuick || notice) && <div className={s.rows}>{quickList}</div>}
            <div ref={cardsRef} className={s.cards}>
              {visible.map((h) => (
                <HostCard
                  key={h.id}
                  host={h}
                  selected={h.id === effectiveId}
                  readout={stats ? readouts[h.id] : undefined}
                  leftOut={stats ? leftOutBy(h, groupMap) : null}
                  list={listStates?.get(h.id) ?? NO_LIST}
                  view={itemView}
                />
              ))}
            </div>
          </>
        )}
      </div>
    );
  }

  const detailHost = detail ? hosts.find((h) => h.id === detail) : undefined;
  const sortEntries: MenuEntry[] = [
    ...LIST_SORTS.map((k) => ({ label: sortLabel(k), checked: shownSort === k, onSelect: () => setSort(k) })),
    ...(stats ? [{ kind: "separator" } as const, ...STATS_SORTS.map((k) => ({ label: sortLabel(k), checked: sort === k, onSelect: () => setSort(k) }))] : []),
  ];

  return (
    <div className={s.page} onKeyDown={onKeyDown}>
      <PageHeader title={titleNode} count={t("hosts.count", { n: visible.length })} search={searchField}>
        {importControls}
        {newControls}
      </PageHeader>
      {toolbar}
      {groupBar}
      {body}

      {sortMenu.anchor && <Menu anchor={sortMenu.anchor} onClose={sortMenu.close} minWidth={150} entries={sortEntries} />}
      {tagsMenu.anchor && (
        <Menu
          anchor={tagsMenu.anchor}
          onClose={tagsMenu.close}
          minWidth={180}
          entries={
            tagList.length === 0
              ? [{ label: t("hosts.tags.none"), disabled: true, onSelect: () => {} }]
              : [
                  ...tagList.map((tag) => ({
                    label: tag.name,
                    hint: String(tag.count),
                    checked: activeTags.includes(tag.name),
                    keepOpen: true,
                    onSelect: () => setTags(activeTags.includes(tag.name) ? activeTags.filter((x) => x !== tag.name) : [...activeTags, tag.name]),
                  })),
                  ...(activeTags.length > 0 ? [{ kind: "separator" } as const, { label: t("hosts.tags.clear"), icon: "x", onSelect: () => setTags([]) }] : []),
                ]
          }
        />
      )}
      {columnsMenu.anchor && (
        <Menu
          anchor={columnsMenu.anchor}
          onClose={columnsMenu.close}
          minWidth={150}
          entries={DASHBOARD_COLUMNS.map((c: DashboardColumn) => ({
            label: t(`dashboard.col.${c}` as MessageKey),
            checked: !hiddenColumns.includes(c),
            keepOpen: true,
            onSelect: () =>
              savePrefs({ dashboard_hidden_columns: hiddenColumns.includes(c) ? hiddenColumns.filter((x) => x !== c) : [...hiddenColumns, c] }),
          }))}
        />
      )}
      {rowMenu.anchor && ctxHost && <Menu anchor={rowMenu.anchor} onClose={rowMenu.close} entries={menuEntries(ctxHost)} />}
      {detailHost && (
        <HostDetail
          host={detailHost}
          readout={readouts[detailHost.id]}
          shown={shownOnDashboard(detailHost, groupMap)}
          groups={groupMap}
          interval={interval}
          alerts={alerts}
          onClose={() => setDetail(null)}
        />
      )}
      {dialogs}
    </div>
  );
}

interface RowProps {
  host: HostView;
  selected: boolean;
  jumpName: string | null;
  status: TabStatus | undefined;
  online: boolean | undefined;
  now: number;
  locale: Locale;
  connectLabel: string;
  failedLabel: string;
  viaTitle: string;
  onSelect: (id: string) => void;
  onConnect: (id: string) => void;
  onContext: (e: MouseEvent, host: HostView) => void;
}

const HostRow = memo(function HostRow({
  host,
  selected,
  jumpName,
  status,
  online,
  now,
  locale,
  connectLabel,
  failedLabel,
  viaTitle,
  onSelect,
  onConnect,
  onContext,
}: RowProps) {
  const failed = status === "failed";
  const dot = status === "connected" ? "var(--green)" : failed ? "var(--red)" : online ? "var(--green)" : "var(--fg3)";
  const last = failed
    ? failedLabel
    : host.last_connected_at
      ? formatRelative(locale, host.last_connected_at, now)
      : "—";
  return (
    <div
      role="option"
      aria-selected={selected}
      data-id={host.id}
      className={cx(s.row, s.cols, selected && s.rowSel)}
      onClick={() => onSelect(host.id)}
      onDoubleClick={() => onConnect(host.id)}
      onContextMenu={(e) => onContext(e, host)}
    >
      <div className={s.nameCell}>
        <OsIcon os={host.os} badge={dot} />
        <span className={s.name}>{host.name}</span>
        {host.favorite && <Icon name="star" fill className={s.fav} />}
      </div>
      <div className={s.addrCell}>
        <span className={s.addr}>
          {host.username && <span className={s.dim}>{host.username}@</span>}
          {host.address}
          <span className={s.dim}>:{host.port}</span>
        </span>
        {jumpName && (
          <span className={s.via} title={viaTitle}>
            <Icon name="path" />
            {jumpName}
          </span>
        )}
      </div>
      <div className={s.tags}>
        {host.tags.map((tag) => (
          <Tag key={tag} name={tag} />
        ))}
      </div>
      <div className={cx(s.last, failed && s.lastFailed)}>{last}</div>
      <div className={s.action}>
        <Button
          size="xs"
          variant="primary"
          tabIndex={-1}
          className={s.connect}
          onClick={(e) => {
            e.stopPropagation();
            onConnect(host.id);
          }}
        >
          {connectLabel}
        </Button>
      </div>
    </div>
  );
});

/** `user@host:port` in the address column's style. */
function TargetText({ target }: { target: TypedTarget }) {
  return (
    <span className={s.addr}>
      {target.username && <span className={s.dim}>{target.username}@</span>}
      {bracketHost(target.address)}
      <span className={s.dim}>:{target.port}</span>
    </span>
  );
}

interface QuickRowProps {
  item: QuickRowItem;
  selected: boolean;
  connectLabel: string;
  hint: string;
  removeLabel: string;
  onSelect: (id: string) => void;
  onConnect: (id: string) => void;
  onRemove: (target: QuickTarget) => void;
}

/** "Connect user@host:port" for a target that is not a saved host (HOST-12). */
function QuickRow({ item, selected, connectLabel, hint, removeLabel, onSelect, onConnect, onRemove }: QuickRowProps) {
  const recent = item.recent;
  return (
    <div
      role="option"
      aria-selected={selected}
      data-id={item.id}
      className={cx(s.row, s.cols, selected && s.rowSel)}
      onClick={() => onSelect(item.id)}
      onDoubleClick={() => onConnect(item.id)}
    >
      <div className={s.nameCell}>
        <Icon name={recent ? "clock-counter-clockwise" : "terminal-window"} className={s.quickIcon} />
        <span className={s.name}>{connectLabel}</span>
      </div>
      <div className={s.addrCell}>
        <TargetText target={item.target} />
      </div>
      <div className={s.tags} />
      <div className={cx(s.last, s.quickHint)}>
        <span>{hint}</span>
        {recent && (
          <IconButton
            small
            icon="x"
            tabIndex={-1}
            className={s.forget}
            label={removeLabel}
            onClick={(e) => {
              e.stopPropagation();
              onRemove(recent);

            }}
          />
        )}
      </div>
      <div className={s.action}>
        <Button
          size="xs"
          variant="primary"
          tabIndex={-1}
          className={s.connect}
          onClick={(e) => {
            e.stopPropagation();
            onConnect(item.id);
          }}
        >
          {connectLabel}
        </Button>
      </div>
    </div>
  );
}

/** An ssh command quick connect cannot run, with the reason. It is not selectable. */
function ProblemRow({ problem }: { problem: QuickProblem }) {
  const t = useT();
  const text =
    problem.kind === "option"
      ? t("hosts.quick.option", { option: problem.option })
      : problem.kind === "command"
        ? t("hosts.quick.command")
        : t("hosts.err.portInvalid");
  return (
    <div className={cx(s.row, s.problemRow)} role="option" aria-selected={false} aria-disabled>
      <Icon name="prohibit" className={s.quickIcon} />
      <span className={s.problemText}>{text}</span>
    </div>
  );
}

/** A note above the typed target that it connects without the forwarding it asked for. It is not selectable. */
function NoticeRow() {
  const t = useT();
  return (
    <div className={cx(s.row, s.problemRow)} role="option" aria-selected={false} aria-disabled>
      <Icon name="info" className={s.quickIcon} />
      <span className={s.problemText}>{t("hosts.quick.noForwarding")}</span>
    </div>
  );
}

interface SuggestionsProps {
  targets: QuickTarget[];
  active: number;
  title: string;
  removeLabel: string;
  onHover: (index: number) => void;
  onConnect: (target: QuickTarget) => void;
  onRemove: (target: QuickTarget) => void;
}

/** Recent quick-connect targets under the empty search field (HOST-12). */
function RecentSuggestions({ targets, active, title, removeLabel, onHover, onConnect, onRemove }: SuggestionsProps) {
  return (
    // Clicks inside keep the focus in the search field, so the list stays open.
    <div className={s.suggest} onMouseDown={(e) => e.preventDefault()}>
      <div className={s.suggestTitle}>{title}</div>
      <div id={SUGGESTIONS_ID} role="listbox" aria-label={title}>
        {targets.map((target, i) => (
          <div
            key={formatTarget(target)}
            id={`${SUGGESTIONS_ID}-${i}`}
            role="option"
            aria-selected={i === active}
            className={cx(s.suggestItem, i === active && s.suggestItemActive)}
            onMouseEnter={() => onHover(i)}
            onClick={() => onConnect(target)}
          >
            <Icon name="clock-counter-clockwise" className={s.quickIcon} />
            <span className={s.suggestLabel}>
              <TargetText target={target} />
            </span>
            <IconButton
              small
              icon="x"
              tabIndex={-1}
              className={s.forget}
              label={removeLabel}
              onClick={(e) => {
                e.stopPropagation();
                onRemove(target);

              }}
            />
          </div>
        ))}
      </div>
    </div>
  );
}
