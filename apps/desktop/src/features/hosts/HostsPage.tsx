import {
  memo,
  useCallback,
  useDeferredValue,
  useEffect,
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
import { Button, Icon, LinkButton, StatusDot } from "@/components/controls";
import { EmptyState, PageHeader, SearchField, Tag, layoutStyles } from "@/components/layout";
import { Menu, confirm, toast, useMenu, type MenuEntry } from "@/components/overlay";
import { connectHost } from "@/features/terminal/connect";
import { formatRelative, useT, type Locale } from "@/i18n";
import { api } from "@/ipc/api";
import type { HostView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { copyText } from "@/lib/native";
import { shortcutLabel } from "@/lib/platform";
import { ImportSshDialog } from "./ImportSshDialog";
import { buildHostIndex, searchHosts } from "./search";
import { useHostsUi, type HostSort } from "./ui";
import s from "./HostsPage.module.css";

const PROBE_INTERVAL_MS = 60_000;
/** Hosts probed per refresh (HOST-10 probes what is on screen; the list is ordered, so the top comes first). */
const PROBE_LIMIT = 200;

let handledSearchTick = 0;

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
  const { selectedId, sort, select, setSort } = useHostsUi();

  const [query, setQuery] = useState("");
  const deferredQuery = useDeferredValue(query);
  const [probe, setProbe] = useState<Record<string, boolean>>({});
  const [importing, setImporting] = useState(false);
  const [ctxHost, setCtxHost] = useState<HostView | null>(null);
  const [now, setNow] = useState(() => Date.now());
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
  const visible = useMemo(() => searchHosts(index, deferredQuery), [index, deferredQuery]);

  const hostName = useMemo(() => new Map(hosts.map((h) => [h.id, h.name])), [hosts]);

  // The latest tab per host tells us whether the last attempt failed (design: red "连接失败").
  const tabStatus = useMemo(() => {
    const m = new Map<string, TabStatus>();
    for (const tab of tabs) m.set(tab.hostId, tab.status);
    return m;
  }, [tabs]);

  const effectiveId = useMemo(
    () => (visible.some((h) => h.id === selectedId) ? selectedId : (visible[0]?.id ?? null)),
    [visible, selectedId],
  );

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
    if (visible.length === 0) return;
    const i = visible.findIndex((h) => h.id === effectiveId);
    const next = visible[Math.max(0, Math.min(visible.length - 1, i + delta))];
    select(next.id);
    requestAnimationFrame(() =>
      listRef.current?.querySelector(`[data-id="${CSS.escape(next.id)}"]`)?.scrollIntoView({ block: "nearest" }),
    );
  };

  // ↑/↓ move the selection, Enter connects (HOST-07). Works from the list and from the search field.
  const onKeyDown = (e: KeyboardEvent) => {
    const target = e.target as HTMLElement;
    const inSearch = target === searchRef.current;
    const onList = target === listRef.current;
    if (!inSearch && !onList) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      move(e.key === "ArrowDown" ? 1 : -1);
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

  // The vault has no hosts at all: design §01b.
  if (hosts.length === 0) {
    return (
      <div className={s.page}>
        <PageHeader title={title}>{newButton}</PageHeader>
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
  const emptyBody = (() => {
    if (base.length === 0) {
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
        <SearchField
          ref={searchRef}
          value={query}
          onChange={setQuery}
          placeholder={t("hosts.search")}
          shortcut={shortcutLabel(platform, "Ctrl+Shift+K", "⌘K")}
        />
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
      ) : visible.length === 0 && searching ? (
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
                onSelect={select}
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
        <StatusDot color={dot} />
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
