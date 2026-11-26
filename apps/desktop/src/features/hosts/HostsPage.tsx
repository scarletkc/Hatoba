import {
  useCallback,
  useDeferredValue,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent,
} from "react";
import { Button, EmptyState, LinkButton, Menu, PageHeader, confirm, layoutStyles, toast, useMenu } from "@hatoba/ui";
import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { useTabs, type TabStatus } from "@/app/tabs";
import { HostDetail } from "@/features/dashboard/HostDetail";
import { DASHBOARD_COLUMNS, leftOutBy, shownOnDashboard, sortForDashboard } from "@/features/dashboard/logic";
import { useDashboardWatch, useReadoutMap } from "@/features/dashboard/readings";
import {
  HostCard,
  StatsHead,
  StatsRow,
  fitStatsColumns,
  useWidth,
  type Alerts,
  type ListState,
  type View,
} from "@/features/dashboard/views";
import { durationLabel } from "@/features/settings/durations";
import { usePrefs } from "@/features/settings/shared";
import { formatRelative, useT } from "@/i18n";
import { api } from "@/ipc/api";
import { isTelnetHost, type AnyHostView, type GroupView, type ImportSource } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { copyText } from "@/lib/native";
import { isMac } from "@/lib/platform";
import { connectHost, connectTarget, connectTelnetTarget } from "@/session/connect";
import {
  formatTarget,
  parseQuickConnect,
  parseTelnet,
  sameTarget,
  savedHostFor,
  savedTelnetHostFor,
  type TypedTarget,
} from "@/session/quickConnect";
import { useRecentTargets } from "@/session/recent";
import { GroupDialog, deleteGroup } from "./GroupDialog";
import {
  GroupBar,
  HostsSearchBox,
  HostsTitle,
  HostsToolbar,
  ImportHostsButton,
  NewHostSplitButton,
} from "./HostsChrome";
import { HostRow, QuickRowsList, TELNET_ROW, TYPED_ROW, type QuickRowItem } from "./HostsRows";
import { ImportDialog } from "./ImportDialog";
import { hostApi } from "./hostApi";
import { useHostPointer } from "./hostPointer";
import {
  buildColumnsMenuEntries,
  buildGroupMenuEntries,
  buildHostMenuEntries,
  buildMultiHostMenuEntries,
  buildSortMenuEntries,
  buildTagsMenuEntries,
} from "./menus";
import { applyFilter, moveHosts, orderHosts } from "./order";
import { buildHostIndex, searchHosts } from "./search";
import { ALONE, pickHost, type PickKeys } from "./selection";
import { isListSort, useHostsUi, type HostSort, type ListSort } from "./ui";
import { useHostProbe } from "./useHostProbe";
import { useHostsKeyboard } from "./useHostsKeyboard";
import s from "./HostsPage.module.css";

let handledSearchTick = 0;

/** A card's list state while resource usage shows, which then has no use for it. */
const NO_LIST: ListState = { dot: "var(--fg3)", last: "", failed: false };

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
  const telnetHosts = useVaultData((st) => st.telnetHosts);
  // TELNET-01: Telnet hosts are listed with the SSH hosts.
  const allHosts = useMemo<AnyHostView[]>(() => [...hosts, ...telnetHosts], [hosts, telnetHosts]);
  const groups = useVaultData((st) => st.groups);
  const tagList = useVaultData((st) => st.tags);
  const tabs = useTabs((st) => st.tabs);
  const recents = useRecentTargets((st) => st.targets);
  const { selection, favorites, tags: pickedTags, select, setFavorites, setTags, clearFilters } = useHostsUi();
  const view = prefs.hosts_view;
  const sort = prefs.hosts_sort;
  const setSort = (hosts_sort: HostSort) => savePrefs({ hosts_sort });
  const stats = prefs.hosts_stats;
  const readouts = useReadoutMap();

  const [query, setQuery] = useState("");
  const deferredQuery = useDeferredValue(query);
  const [importing, setImporting] = useState<ImportSource | null>(null);
  /** The hosts the row menu acts on: the one right-clicked, or every selected host it is one of. */
  const [ctxIds, setCtxIds] = useState<string[]>([]);
  const [ctxGroup, setCtxGroup] = useState<GroupView | null>(null);
  const [editingGroup, setEditingGroup] = useState<GroupView | "new" | null>(null);
  const [detail, setDetail] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const [searchFocused, setSearchFocused] = useState(false);
  const [suggestDismissed, setSuggestDismissed] = useState(false);
  const [suggestAt, setSuggestAt] = useState(-1);
  /** The query at which the user last picked a row; until then the typed target stays selected. */
  const [pickedFor, setPickedFor] = useState<string | null>(null);
  /** A custom order just made (HOST-16), shown until the hosts it was made from reload with their new places. */
  const [arranged, setArranged] = useState<{ hosts: AnyHostView[]; order: string[] } | null>(null);
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

  const groupMap = useMemo(() => new Map(groups.map((g) => [g.id, g])), [groups]);
  const scope = requested ? groupMap.get(requested) : undefined;
  // A group that sync or another window deleted leaves every host.
  const group = scope ? requested : null;
  const title = scope?.name ?? t("hosts.title");
  // Tags no host has any more drop out of the filter.
  const activeTags = useMemo(
    () => pickedTags.filter((tag) => tagList.some((x) => x.name === tag)),
    [pickedTags, tagList],
  );
  const filtering = favorites || activeTags.length > 0;

  // Filter + sort only when inputs change; typing in the search box touches neither.
  const filtered = useMemo(
    () => applyFilter(allHosts, { group, favorites, tags: activeTags }),
    [allHosts, group, favorites, activeTags],
  );
  const listSort: ListSort = isListSort(sort) ? sort : "recent";
  const pending = arranged?.hosts === allHosts ? arranged.order : null;
  const base = useMemo(
    () => orderHosts(filtered, listSort, t.locale, pending),
    [filtered, listSort, t.locale, pending],
  );
  const probe = useHostProbe(base, hostProbe);
  const index = useMemo(() => buildHostIndex(base), [base]);
  // HOST-12: `user@host` and `ssh …` input is also a target to connect to, and TELNET-05:
  // `telnet host [port]`. The list is then filtered by its destination, without the syntax around it.
  const telnetTyped = useMemo(() => parseTelnet(deferredQuery), [deferredQuery]);
  const parsed = useMemo(() => (telnetTyped ? null : parseQuickConnect(deferredQuery)), [telnetTyped, deferredQuery]);
  const searchText = telnetTyped ? telnetTyped.search : parsed ? parsed.search : deferredQuery;
  const searching = deferredQuery.trim() !== "";
  const found = useMemo(() => searchHosts(index, searchText), [index, searchText]);
  // HOST-14: a resource order sorts again with each reading; a search keeps its ranking.
  const statsSort = stats && !isListSort(sort) && !searching ? sort : null;
  const ordered = useMemo(
    () => (statsSort ? sortForDashboard(found, statsSort, readouts, t.locale) : found),
    [found, statsSort, readouts, t.locale],
  );
  // A typed target that is a saved host connects as that host, with its key and jump host.
  const exact = useMemo<AnyHostView | undefined>(
    () =>
      telnetTyped?.ok
        ? savedTelnetHostFor(telnetHosts, telnetTyped.target)
        : parsed?.ok
          ? savedHostFor(hosts, parsed.target)
          : undefined,
    [telnetTyped, telnetHosts, parsed, hosts],
  );
  const visible = useMemo(() => (exact && !ordered.includes(exact) ? [exact, ...ordered] : ordered), [exact, ordered]);
  // Recent targets that have been saved as hosts since show as those hosts.
  const unsavedRecents = useMemo(() => recents.filter((r) => !savedHostFor(hosts, r)), [recents, hosts]);
  const quickRows = useMemo(() => {
    const rows: QuickRowItem[] = [];
    if (telnetTyped?.ok && !exact) rows.push({ id: TELNET_ROW, telnet: telnetTyped.target, recent: null });
    if (parsed?.ok && !exact)
      rows.push({
        id: TYPED_ROW,
        target: parsed.target,
        recent: unsavedRecents.find((r) => sameTarget(r, parsed.target)) ?? null,
      });
    const q = searchText.trim().toLowerCase();
    if (q)
      for (const r of unsavedRecents)
        if (!(parsed?.ok && sameTarget(r, parsed.target)) && formatTarget(r).toLowerCase().includes(q))
          rows.push({ id: `quick:recent:${formatTarget(r)}`, target: r, recent: r });
    return rows;
  }, [telnetTyped, parsed, exact, unsavedRecents, searchText]);
  const typedInput = telnetTyped ?? parsed;
  const problem = typedInput && !typedInput.ok ? typedInput.problem : null;
  const notice = parsed?.ok ? parsed.notice : undefined;

  const hostById = useMemo(() => new Map(allHosts.map((h) => [h.id, h])), [allHosts]);
  // Each group button's count, worked out once per reload rather than on every batch of readings.
  const groupCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const h of allHosts) if (h.group_id) counts.set(h.group_id, (counts.get(h.group_id) ?? 0) + 1);
    return counts;
  }, [allHosts]);

  // The latest tab per host tells us whether the last attempt failed (design: red "连接失败").
  const tabStatus = useMemo(() => {
    const m = new Map<string, TabStatus>();
    for (const tab of tabs) {
      const id = tab.hostId ?? tab.telnet?.hostId;
      if (id) m.set(id, tab.status);
    }
    return m;
  }, [tabs]);

  // HOST-14: the listed hosts are read while resource usage shows; the search does not change which.
  const interval = prefs.dashboard_interval_secs;
  const watched = useMemo(
    () => filtered.filter((h) => !isTelnetHost(h) && shownOnDashboard(h, groupMap)).map((h) => h.id),
    [filtered, groupMap],
  );
  useDashboardWatch(watched, interval, stats);
  const { dashboard_cpu_alert: cpuAlert, dashboard_mem_alert: memAlert, dashboard_disk_alert: diskAlert } = prefs;
  const alerts: Alerts = useMemo(
    () => ({ cpu: cpuAlert, mem: memAlert, disk: diskAlert }),
    [cpuAlert, memAlert, diskAlert],
  );
  const hiddenColumns = prefs.dashboard_hidden_columns;
  const [measureTable, tableWidth] = useWidth();
  const columns = useMemo(
    () =>
      fitStatsColumns(
        DASHBOARD_COLUMNS.filter((c) => !hiddenColumns.includes(c)),
        tableWidth,
      ),
    [hiddenColumns, tableWidth],
  );

  const hostIds = useMemo(() => visible.map((h) => h.id), [visible]);
  const rowIds = useMemo(() => [...quickRows.map((r) => r.id), ...hostIds], [quickRows, hostIds]);
  // The typed target, or the saved host it is, is selected until the user picks rows; otherwise a search selects
  // its best saved host, even with recent targets listed above it. Picked rows the list does not show are not
  // selected, so nothing acts on a host out of sight.
  const typedRow = quickRows[0]?.id === TYPED_ROW || quickRows[0]?.id === TELNET_ROW ? quickRows[0].id : null;
  const preferred = exact?.id ?? typedRow;
  const selected = useMemo(() => {
    const listed = new Set(rowIds);
    const shown = selection.filter((id) => listed.has(id));
    if (pickedFor === deferredQuery || (shown.length > 0 && !preferred && !problem)) return shown;
    // An ssh command quick connect cannot run selects nothing, so Enter does not connect a saved
    // host with other settings than the ones typed.
    if (problem || !searching) return [];
    const best = preferred ?? visible[0]?.id ?? quickRows[0]?.id;
    return best ? [best] : [];
  }, [rowIds, selection, preferred, problem, pickedFor, deferredQuery, searching, visible, quickRows]);
  const selectedSet = useMemo(() => new Set(selected), [selected]);
  /** The row the arrow keys move on from, and Space shows. */
  const focusId = selected.length > 0 ? selected[selected.length - 1] : null;
  const several = selected.length > 1;

  // The row callbacks read these through a ref, so they stay stable and memoized rows skip re-rendering.
  const latest = useRef({ deferredQuery, quickRows, hostIds, selected });
  useLayoutEffect(() => {
    latest.current = { deferredQuery, quickRows, hostIds, selected };
  });
  /** The host Shift picks from: the one last picked without it. */
  const anchor = useRef<string | null>(null);

  const pickRows = useCallback(
    (ids: string[]) => {
      setPickedFor(latest.current.deferredQuery);
      select(ids);
    },
    [select],
  );

  /** Picks a saved host with Shift and Ctrl (⌘ on macOS) as `keys` say, or any row alone. */
  const pickWith = useCallback(
    (id: string, keys: PickKeys) => {
      const { hostIds, selected } = latest.current;
      const from = anchor.current ?? selected[selected.length - 1] ?? id;
      anchor.current = keys.range ? from : id;
      pickRows(pickHost(hostIds, selected, from, id, keys));
    },
    [pickRows],
  );

  const pick = useCallback((id: string) => pickWith(id, ALONE), [pickWith]);

  const clickHost = useCallback(
    (e: MouseEvent, id: string) => pickWith(id, { range: e.shiftKey, add: isMac(platform) ? e.metaKey : e.ctrlKey }),
    [pickWith, platform],
  );

  const selectNone = () => {
    anchor.current = null;
    pickRows([]);
  };

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

  const newHost = useCallback(
    () => navigate({ kind: "host-edit", hostId: null, groupId: group, back: group }),
    [navigate, group],
  );
  const editHost = useCallback((host: AnyHostView) => navigate(hostApi(host).editRoute(group)), [navigate, group]);

  const connect = useCallback(
    (id: string) => {
      const quick = latest.current.quickRows.find((r) => r.id === id);
      if (quick) {
        if (quick.telnet) connectTelnetTarget(quick.telnet.address, quick.telnet.port);
        else void connectTarget(quick.target);
        return;
      }
      anchor.current = id;
      select([id]);
      const { hosts: sshList, telnetHosts: telnetList } = useVaultData.getState();
      const target = telnetList.find((h) => h.id === id) ?? sshList.find((h) => h.id === id);
      if (target) void hostApi(target).connect();
      else void connectHost(id);
    },
    [select],
  );

  /**
   * A tab on each host, one host after another, so the passwords asked before connecting come one at a time; the
   * host key and sign-in questions asked while connecting wait in line on their own.
   */
  const connectAll = async (list: AnyHostView[]) => {
    for (const host of list) await hostApi(host).connect();
  };

  const onContext = useCallback(
    (e: MouseEvent, host: AnyHostView) => {
      e.preventDefault();
      const { hostIds, selected } = latest.current;
      // On one of several selected hosts, the menu acts on all of them; on another host, it selects that one.
      if (selected.length > 1 && selected.includes(host.id)) {
        const picked = new Set(selected);
        setCtxIds(hostIds.filter((id) => picked.has(id)));
      } else {
        pick(host.id);
        setCtxIds([host.id]);
      }
      openRowMenu({ x: e.clientX, y: e.clientY });
    },
    [pick, openRowMenu],
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
        dot:
          status === "connected" ? "var(--green)" : failed ? "var(--red)" : probe[h.id] ? "var(--green)" : "var(--fg3)",
        last: failed
          ? t("hosts.failed")
          : h.last_connected_at
            ? formatRelative(t.locale, h.last_connected_at, now)
            : t("hosts.neverConnected"),
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
      onSelect: clickHost,
      onConnect: connect,
      onContext,
    }),
    [t, now, interval, alerts, groupMap, stats, tagRow, clickHost, connect, onContext],
  );

  const openGroupMenu = (e: MouseEvent, g: GroupView) => {
    e.preventDefault();
    setCtxGroup(g);
    groupMenu.openAt({ x: e.clientX, y: e.clientY });
  };

  const fail = (e: unknown) => toast(errorMessage(t, e), "error");

  /**
   * HOST-16: moves `moving` before the host `before`, or after the last listed host, and keeps the order this makes
   * as the custom order, which the page then shows. Every host has its place in it, the ones this page does not list
   * too: from any other order, the custom order starts from the one the page shows.
   */
  const moveTo = (moving: string[], before: string | null) => {
    const all = orderHosts(allHosts, listSort, t.locale, pending);
    const shown = statsSort ? sortForDashboard(all, statsSort, readouts, t.locale) : all;
    const ids = shown.map((h) => h.id);
    const order = moveHosts(ids, hostIds, moving, before);
    if (order.every((id, i) => id === ids[i])) return;
    setArranged({ hosts: allHosts, order });
    if (sort !== "custom") setSort("custom");
    api
      .hosts_arrange(order)
      .then(() => useVaultData.getState().reload())
      .catch((e: unknown) => {
        setArranged(null);
        fail(e);
      });
  };

  // The pointer reads the page at each press and move.
  const live = useRef({ view, searching, moveTo });
  useLayoutEffect(() => {
    live.current = { view, searching, moveTo };
  });
  const pointer = useHostPointer({
    list: () => listRef.current,
    layout: () => (live.current.view === "cards" ? "grid" : "rows"),
    hostIds: () => latest.current.hostIds,
    selected: () => latest.current.selected,
    canMove: () => !live.current.searching,
    mac: () => isMac(platform),
    boxSelect: (ids) => {
      anchor.current = ids[ids.length - 1] ?? null;
      pickRows(ids);
    },
    pick,
    drop: (moving, before) => live.current.moveTo(moving, before),
    classes: { box: s.marquee, line: s.dropLine, ghost: s.dragGhost, count: s.dragCount },
  });

  const markFavorite = async (list: AnyHostView[], favorite: boolean) => {
    try {
      for (const host of list) if (host.favorite !== favorite) await hostApi(host).setFavorite(favorite);
    } catch (e) {
      fail(e);
    }
    // Also after a failure, as the hosts before it changed.
    await useVaultData.getState().reload().catch(fail);
  };

  const duplicate = async (host: AnyHostView) => {
    try {
      const copy = await hostApi(host).duplicate();
      await useVaultData.getState().reload();
      anchor.current = copy.id;
      select([copy.id]);
      toast(t("hosts.duplicated", { name: copy.name }), "success");
    } catch (e) {
      fail(e);
    }
  };

  const remove = async (list: AnyHostView[]) => {
    if (list.length === 0) return;
    const one = list.length === 1 ? list[0] : null;
    const ok = await confirm({
      title: one ? t("hosts.deleteTitle", { name: one.name }) : t("hosts.deleteManyTitle", { n: list.length }),
      body: t("hosts.deleteBody"),
      confirmLabel: t("btn.delete"),
      danger: true,
    });
    if (!ok) return;
    try {
      for (const host of list) await hostApi(host).remove();
      toast(one ? t("hosts.deleted", { name: one.name }) : t("hosts.deletedMany", { n: list.length }));
    } catch (e) {
      fail(e);
    }
    // Also after a failure, as the hosts before it are gone.
    await useVaultData.getState().reload().catch(fail);
  };

  const copyAddress = async (host: AnyHostView) => {
    try {
      await copyText(hostApi(host).addressText());
      toast(t("hosts.copied"), "success");
    } catch (e) {
      fail(e);
    }
  };

  // SEC-08: the backend copies the saved password and clears the clipboard after this device's delay.
  const copyPassword = async (host: AnyHostView) => {
    try {
      await hostApi(host).copyPassword();
      const secs = useApp.getState().prefs.clipboard_clear_secs;
      toast(
        secs ? t("hosts.passwordCopied", { time: durationLabel(t, secs) }) : t("hosts.passwordCopiedKept"),
        "success",
      );
    } catch (e) {
      fail(e);
    }
  };

  const ctxHosts = ctxIds.flatMap((id) => hostById.get(id) ?? []);

  const onKeyDown = useHostsKeyboard({
    searchRef,
    listRef,
    cardsRef,
    anchorRef: anchor,
    platform,
    view,
    stats,
    searching,
    query,
    hosts,
    telnetHosts,
    visible,
    hostIds,
    rowIds,
    quickRows,
    selected,
    selectedSet,
    focusId,
    several,
    suggestions,
    suggestOpen,
    suggestActive,
    setSuggestAt,
    setSuggestDismissed,
    connectQuick,
    pickRows,
    pickWith,
    selectNone,
    moveTo,
    setDetail,
    remove: (list) => void remove(list),
    connect,
    connectAll: (list) => void connectAll(list),
  });

  const newControls = <NewHostSplitButton onNewHost={newHost} onNewGroup={() => setEditingGroup("new")} />;
  const importControls = <ImportHostsButton onImport={setImporting} />;
  const dialogs = (
    <>
      {importing && <ImportDialog source={importing} onClose={() => setImporting(null)} />}
      {editingGroup && (
        <GroupDialog
          group={editingGroup === "new" ? null : editingGroup}
          count={groups.length}
          onClose={() => setEditingGroup(null)}
        />
      )}
      {groupMenu.anchor && ctxGroup && (
        <Menu
          anchor={groupMenu.anchor}
          onClose={groupMenu.close}
          entries={buildGroupMenuEntries(ctxGroup, {
            t,
            stats,
            onRename: (g) => setEditingGroup(g),
            onDelete: (g) =>
              void deleteGroup(t, g).then((deleted) => {
                if (deleted && group === g.id) navigate({ kind: "hosts", group: null });
              }),
          })}
        />
      )}
    </>
  );

  const searchField = (
    <HostsSearchBox
      inputRef={searchRef}
      query={query}
      platform={platform}
      suggestions={suggestions}
      suggestOpen={suggestOpen}
      suggestActive={suggestActive}
      onChange={onQueryChange}
      onFocus={onSearchFocus}
      onBlur={() => setSearchFocused(false)}
      onHoverSuggestion={setSuggestAt}
      onConnectSuggestion={connectQuick}
      onRemoveSuggestion={(r) => void removeRecent(r)}
    />
  );

  const titleNode = (
    <HostsTitle
      scope={scope}
      title={title}
      onAllHosts={() => navigate({ kind: "hosts", group: null })}
      onGroupMenu={openGroupMenu}
    />
  );

  const groupBar = group === null && (
    <GroupBar
      groups={groups}
      counts={groupCounts}
      onOpenGroup={(id) => navigate({ kind: "hosts", group: id })}
      onGroupMenu={openGroupMenu}
    />
  );

  // No hosts at all, though groups made meanwhile stay reachable. Quick connect still works from the search field.
  if (allHosts.length === 0 && !query.trim()) {
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

  const shownSort = stats ? sort : listSort;
  const hasQuick = quickRows.length > 0 || problem !== null;
  const emptyBody = (() => {
    if (base.length > 0 || allHosts.length === 0 || hasQuick || exact) return null;
    return filtering ? ("filter" as const) : ("group" as const);
  })();

  const quickList = (
    <QuickRowsList
      problem={problem}
      notice={Boolean(notice)}
      quickRows={quickRows}
      selectedSet={selectedSet}
      onSelect={pick}
      onConnect={connect}
      onRemoveRecent={(target) => void removeRecent(target)}
    />
  );

  let body;
  if (emptyBody === "filter") {
    body = (
      <EmptyState
        icon="funnel"
        title={t("hosts.emptyFilter.title")}
        body={t("hosts.emptyFilter.body")}
        actions={<Button onClick={clearFilters}>{t("hosts.emptyFilter.clear")}</Button>}
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
      <div
        ref={listRef}
        className={s.scroll}
        tabIndex={0}
        role="listbox"
        aria-label={title}
        aria-multiselectable
        data-several={several || undefined}
        onPointerDown={pointer.down}
        onMouseDown={(e) => {
          // A press picks, moves or boxes hosts (HOST-16) rather than selecting their text.
          if (e.button !== 0 || (e.target as Element).closest("button, a, input, textarea, select")) return;
          e.preventDefault();
          listRef.current?.focus({ preventScroll: true });
        }}
        onClick={(e) => {
          // A click beside the rows selects none.
          if (!(e.target as Element).closest('[role="option"], button')) selectNone();
        }}
      >
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
                  selected={selectedSet.has(h.id)}
                  readout={readouts[h.id]}
                  leftOut={isTelnetHost(h) ? null : leftOutBy(h, groupMap)}
                  columns={columns}
                  view={itemView}
                />
              ) : (
                <HostRow
                  key={h.id}
                  host={h}
                  selected={selectedSet.has(h.id)}
                  jumpName={!isTelnetHost(h) && h.jump_host_id ? (hostById.get(h.jump_host_id)?.name ?? null) : null}
                  status={tabStatus.get(h.id)}
                  online={probe[h.id]}
                  now={now}
                  locale={t.locale}
                  connectLabel={t("hosts.connect")}
                  failedLabel={t("hosts.failed")}
                  viaTitle={
                    !isTelnetHost(h) && h.jump_host_id
                      ? t("hosts.viaTitle", { name: hostById.get(h.jump_host_id)?.name ?? "" })
                      : ""
                  }
                  onSelect={clickHost}
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
                  selected={selectedSet.has(h.id)}
                  readout={stats ? readouts[h.id] : undefined}
                  leftOut={stats && !isTelnetHost(h) ? leftOutBy(h, groupMap) : null}
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

  return (
    <div className={s.page} onKeyDown={onKeyDown}>
      <PageHeader title={titleNode} count={t("hosts.count", { n: visible.length })} search={searchField}>
        {importControls}
        {newControls}
      </PageHeader>
      <HostsToolbar
        favorites={favorites}
        onToggleFavorites={() => setFavorites(!favorites)}
        activeTags={activeTags}
        onRemoveTag={(tag) => setTags(activeTags.filter((x) => x !== tag))}
        tagsButtonRef={tagsButton}
        tagsOpen={!!tagsMenu.anchor}
        onOpenTags={() => tagsButton.current && tagsMenu.openBelow(tagsButton.current)}
        sortButtonRef={sortButton}
        sortOpen={!!sortMenu.anchor}
        shownSort={shownSort}
        onOpenSort={() => sortButton.current && sortMenu.openBelow(sortButton.current, true)}
        stats={stats}
        onChangeStats={(hosts_stats) => savePrefs({ hosts_stats })}
        view={view}
        onChangeView={(hosts_view) => savePrefs({ hosts_view })}
      />
      {groupBar}
      {body}

      {sortMenu.anchor && (
        <Menu
          anchor={sortMenu.anchor}
          onClose={sortMenu.close}
          minWidth={150}
          entries={buildSortMenuEntries(t, { stats, sort, shownSort, onSelect: setSort })}
        />
      )}
      {tagsMenu.anchor && (
        <Menu
          anchor={tagsMenu.anchor}
          onClose={tagsMenu.close}
          minWidth={180}
          entries={buildTagsMenuEntries(t, { tagList, activeTags, onChange: setTags })}
        />
      )}
      {columnsMenu.anchor && (
        <Menu
          anchor={columnsMenu.anchor}
          onClose={columnsMenu.close}
          minWidth={150}
          entries={buildColumnsMenuEntries(t, {
            hiddenColumns,
            onChange: (dashboard_hidden_columns) => savePrefs({ dashboard_hidden_columns }),
          })}
        />
      )}
      {rowMenu.anchor && ctxHosts.length > 0 && (
        <Menu
          anchor={rowMenu.anchor}
          onClose={rowMenu.close}
          entries={
            ctxHosts.length === 1
              ? buildHostMenuEntries(ctxHosts[0], {
                  t,
                  stats,
                  readouts,
                  groupMap,
                  onConnect: connect,
                  onEdit: editHost,
                  onDuplicate: (host) => void duplicate(host),
                  onFavorite: (list, fav) => void markFavorite(list, fav),
                  onCopyAddress: (host) => void copyAddress(host),
                  onCopyPassword: (host) => void copyPassword(host),
                  onDetails: setDetail,
                  onRemove: (list) => void remove(list),
                })
              : buildMultiHostMenuEntries(ctxHosts, {
                  t,
                  onConnectAll: (list) => void connectAll(list),
                  onFavorite: (list, fav) => void markFavorite(list, fav),
                  onRemove: (list) => void remove(list),
                })
          }
        />
      )}
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
