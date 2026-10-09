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
import { useApp, type HostFilter } from "@/app/store";
import { useTabs, type TabStatus } from "@/app/tabs";
import { Button, Icon, LinkButton } from "@/components/controls";
import { EmptyState, PageHeader, SearchField, Tag, layoutStyles } from "@/components/layout";
import { Menu, confirm, toast, useMenu, type MenuEntry } from "@/components/overlay";
import { connectHost, connectTarget } from "@/features/terminal/connect";
import { formatRelative, useT, type Locale } from "@/i18n";
import { api } from "@/ipc/api";
import type { HostView, QuickTarget } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { copyText } from "@/lib/native";
import { shortcutLabel } from "@/lib/platform";
import { ImportSshDialog } from "./ImportSshDialog";
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
import { useRecentTargets } from "./recent";
import { buildHostIndex, searchHosts } from "./search";
import { useHostsUi, type HostSort } from "./ui";
import s from "./HostsPage.module.css";

const PROBE_INTERVAL_MS = 60_000;
/** Hosts probed per refresh (HOST-10 probes what is on screen; the list is ordered, so the top comes first). */
const PROBE_LIMIT = 200;

let handledSearchTick = 0;

/** Row id of the typed quick-connect target (HOST-12); saved hosts use their item ids. */
const TYPED_ROW = "quick:typed";
const SUGGESTIONS_ID = "recent-targets";

/** A Connect row for a target that is not a saved host: the one typed, or a recent one. */
interface QuickRowItem {
  id: string;
  target: TypedTarget;
  /** Set when the target is in the recent list, so the row can remove it. */
  recent: QuickTarget | null;
}

function applyFilter(hosts: HostView[], filter: HostFilter): HostView[] {
  switch (filter.kind) {
    case "all":
      return hosts;
    case "favorites":
      return hosts.filter((h) => h.favorite);
    case "recent":
      return hosts.filter((h) => h.last_connected_at !== null);
    case "group":
      return hosts.filter((h) => h.group_id === filter.id);
    case "tag":
      return hosts.filter((h) => h.tags.includes(filter.name));
  }
}

function sortHosts(hosts: HostView[], sort: HostSort, locale: string): HostView[] {
  const collator = new Intl.Collator(locale, { numeric: true, sensitivity: "base" });
  const byName = (a: HostView, b: HostView) => collator.compare(a.name, b.name);
  const out = [...hosts];
  switch (sort) {
    case "recent":
      return out.sort((a, b) => (b.last_connected_at ?? -1) - (a.last_connected_at ?? -1) || byName(a, b));
    case "name":
      return out.sort(byName);
    case "address":
      return out.sort((a, b) => collator.compare(a.address, b.address) || byName(a, b));
  }
}

export function HostsPage({ filter }: { filter: HostFilter }) {
  const t = useT();
  const platform = useApp((st) => st.info.platform);
  const hostProbe = useApp((st) => st.prefs.host_probe);
  const searchTick = useApp((st) => st.searchFocusTick);
  const navigate = useApp((st) => st.navigate);
  const hosts = useVaultData((st) => st.hosts);
  const groups = useVaultData((st) => st.groups);
  const tabs = useTabs((st) => st.tabs);
  const recents = useRecentTargets((st) => st.targets);
  const { selectedId, sort, select, setSort } = useHostsUi();

  const [query, setQuery] = useState("");
  const deferredQuery = useDeferredValue(query);
  const [probe, setProbe] = useState<Record<string, boolean>>({});
  const [importing, setImporting] = useState(false);
  const [ctxHost, setCtxHost] = useState<HostView | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const [searchFocused, setSearchFocused] = useState(false);
  const [suggestDismissed, setSuggestDismissed] = useState(false);
  const [suggestAt, setSuggestAt] = useState(-1);
  /** The query at which the user last picked a row; until then the typed target stays selected. */
  const [pickedFor, setPickedFor] = useState<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const rowMenu = useMenu();
  const openRowMenu = rowMenu.openAt;
  const sortMenu = useMenu();
  const sortButton = useRef<HTMLButtonElement>(null);

  const title = useMemo(() => {
    switch (filter.kind) {
      case "all":
        return t("sidebar.all");
      case "favorites":
        return t("sidebar.favorites");
      case "recent":
        return t("sidebar.recent");
      case "group":
        return groups.find((g) => g.id === filter.id)?.name ?? t("sidebar.groups");
      case "tag":
        return filter.name;
    }
  }, [filter, groups, t]);

  // Filter + sort only when inputs change; typing in the search box touches neither.
  const base = useMemo(() => sortHosts(applyFilter(hosts, filter), sort, t.locale), [hosts, filter, sort, t.locale]);
  const index = useMemo(() => buildHostIndex(base), [base]);
  // HOST-12: `user@host` and `ssh …` input is also a target to connect to. The list is then
  // filtered by its destination, without the ssh syntax around it.
  const parsed = useMemo(() => parseQuickConnect(deferredQuery), [deferredQuery]);
  const searchText = parsed ? parsed.search : deferredQuery;
  const found = useMemo(() => searchHosts(index, searchText), [index, searchText]);
  // A typed target that is a saved host connects as that host, with its key and jump host.
  const exact = useMemo(() => (parsed?.ok ? savedHostFor(hosts, parsed.target) : undefined), [parsed, hosts]);
  const visible = useMemo(() => (exact && !found.includes(exact) ? [exact, ...found] : found), [exact, found]);
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

  const hostName = useMemo(() => new Map(hosts.map((h) => [h.id, h.name])), [hosts]);

  // The latest tab per host tells us whether the last attempt failed (design: red "连接失败").
  const tabStatus = useMemo(() => {
    const m = new Map<string, TabStatus>();
    for (const tab of tabs) if (tab.hostId) m.set(tab.hostId, tab.status);
    return m;
  }, [tabs]);

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

  // Relative times ("2 min ago") refresh once a minute.
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(timer);
  }, []);

  // Ctrl+Shift+K focuses the search field (a page mounted by the shortcut handles the tick too).
  useEffect(() => {
    if (searchTick === handledSearchTick) return;
    handledSearchTick = searchTick;
    searchRef.current?.focus();
    searchRef.current?.select();
  }, [searchTick]);

  useEffect(() => {
    if (searchTick === handledSearchTick) listRef.current?.focus({ preventScroll: true });
    // Only on mount: don't steal focus when the filter changes.
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

  const newHost = useCallback(
    () =>
      navigate({ kind: "host-edit", hostId: null, groupId: filter.kind === "group" ? filter.id : null, back: filter }),
    [navigate, filter],
  );
  const editHost = useCallback(
    (id: string) => navigate({ kind: "host-edit", hostId: id, groupId: null, back: filter }),
    [navigate, filter],
  );

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

  // SEC-08: the backend copies the saved password and clears the clipboard after 30 s.
  const copyPassword = async (host: HostView) => {
    try {
      await api.host_copy_password(host.id);
      toast(t("hosts.passwordCopied"), "success");
    } catch (e) {
      fail(e);
    }
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
    { kind: "separator" },
    { label: t("hosts.menu.delete"), icon: "trash", danger: true, onSelect: () => void remove(host) },
  ];

  const move = (delta: 1 | -1) => {
    if (rowIds.length === 0) return;
    const i = effectiveId ? rowIds.indexOf(effectiveId) : -1;
    const next = rowIds[Math.max(0, Math.min(rowIds.length - 1, i + delta))];
    pick(next);
    requestAnimationFrame(() =>
      listRef.current?.querySelector(`[data-id="${CSS.escape(next)}"]`)?.scrollIntoView({ block: "nearest" }),
    );
  };

  // ↑/↓ move the selection, Enter connects (HOST-07). Works from the list and from the search field.
  const onKeyDown = (e: KeyboardEvent) => {
    const target = e.target as HTMLElement;
    const inSearch = target === searchRef.current;
    const onList = target === listRef.current;
    if (!inSearch && !onList) return;
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
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      move(e.key === "ArrowDown" ? 1 : -1);
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

  const newButton = (
    <Button variant="primary" icon="plus" onClick={newHost}>
      {t("hosts.newHost")}
    </Button>
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

  // The vault has no hosts at all: design §01b. Quick connect still works from the search field.
  if (hosts.length === 0 && !query.trim()) {
    return (
      <div className={s.page} onKeyDown={onKeyDown}>
        <PageHeader title={title}>
          {searchField}
          {newButton}
        </PageHeader>
        <EmptyState
          icon="hard-drives"
          title={t("hosts.empty.title")}
          body={t("hosts.empty.body")}
          actions={
            <>
              {newButton}
              <Button icon="file-arrow-down" onClick={() => setImporting(true)}>
                {t("hosts.empty.import")}
              </Button>
            </>
          }
          foot={
            <>
              <span>{t("hosts.empty.sync")}</span>
              <LinkButton onClick={() => navigate({ kind: "sync" })}>{t("hosts.empty.syncLink")}</LinkButton>
            </>
          }
        />
        {importing && <ImportSshDialog onClose={() => setImporting(false)} />}
      </div>
    );
  }

  const sortLabels: Record<HostSort, string> = {
    recent: t("hosts.sort.recent"),
    name: t("hosts.sort.name"),
    address: t("hosts.sort.address"),
  };

  const searching = deferredQuery.trim() !== "";
  const hasQuick = quickRows.length > 0 || problem !== null;
  const emptyBody = (() => {
    if (base.length === 0 && hosts.length > 0 && !hasQuick && !exact) {
      switch (filter.kind) {
        case "favorites":
          return { icon: "star", key: "hosts.emptyFav" } as const;
        case "recent":
          return { icon: "clock-counter-clockwise", key: "hosts.emptyRecent" } as const;
        case "tag":
          return { icon: "tag", key: "hosts.emptyTag" } as const;
        default:
          return { icon: "folder-simple", key: "hosts.emptyGroup" } as const;
      }
    }
    return null;
  })();

  return (
    <div className={s.page} onKeyDown={onKeyDown}>
      <PageHeader title={title} count={t("hosts.count", { n: visible.length })}>
        {searchField}
        <Button
          ref={sortButton}
          trailingIcon="caret-down"
          aria-haspopup="menu"
          aria-expanded={!!sortMenu.anchor}
          aria-label={t("hosts.sort")}
          onClick={() => sortButton.current && sortMenu.openBelow(sortButton.current, true)}
        >
          {sortLabels[sort]}
        </Button>
        {newButton}
      </PageHeader>

      {emptyBody ? (
        <EmptyState
          icon={emptyBody.icon}
          title={t(`${emptyBody.key}.title` as "hosts.emptyFav.title")}
          body={t(`${emptyBody.key}.body` as "hosts.emptyFav.body")}
          actions={
            filter.kind === "group" ? (
              <Button variant="primary" icon="plus" onClick={newHost}>
                {t("hosts.newHost")}
              </Button>
            ) : undefined
          }
        />
      ) : visible.length === 0 && searching && !hasQuick ? (
        <EmptyState
          icon="magnifying-glass"
          title={t("hosts.noResults.title")}
          body={t("hosts.noResults.body", { q: deferredQuery.trim() })}
          actions={<Button onClick={() => setQuery("")}>{t("hosts.noResults.clear")}</Button>}
        />
      ) : (
        <div ref={listRef} className={s.scroll} tabIndex={0} role="listbox" aria-label={title}>
          <div className={cx(layoutStyles.tableHead, s.cols, s.head)}>
            <span className={s.headName}>{t("hosts.col.name")}</span>
            <span>{t("hosts.col.address")}</span>
            <span>{t("hosts.col.tags")}</span>
            <span>{t("hosts.col.last")}</span>
            <span />
          </div>
          <div className={s.rows}>
            {problem && <ProblemRow problem={problem} />}
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
            {visible.map((h) => (
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
            ))}
          </div>
        </div>
      )}

      {sortMenu.anchor && (
        <Menu
          anchor={sortMenu.anchor}
          onClose={sortMenu.close}
          minWidth={140}
          entries={(Object.keys(sortLabels) as HostSort[]).map((k) => ({
            label: sortLabels[k],
            checked: sort === k,
            onSelect: () => setSort(k),
          }))}
        />
      )}
      {rowMenu.anchor && ctxHost && <Menu anchor={rowMenu.anchor} onClose={rowMenu.close} entries={menuEntries(ctxHost)} />}
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
          <button
            type="button"
            tabIndex={-1}
            className={s.forget}
            aria-label={removeLabel}
            title={removeLabel}
            onClick={(e) => {
              e.stopPropagation();
              onRemove(recent);
            }}
          >
            <Icon name="x" />
          </button>
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
            <button
              type="button"
              tabIndex={-1}
              className={s.forget}
              aria-label={removeLabel}
              title={removeLabel}
              onClick={(e) => {
                e.stopPropagation();
                onRemove(target);
              }}
            >
              <Icon name="x" />
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}
