import { useRef, type MouseEvent, type RefObject } from "react";
import { Button, Icon, IconButton, Menu, SearchField, Segmented, Switch, TagChip, useMenu } from "@hatoba/ui";
import { useT } from "@/i18n";
import type { GroupView, ImportSource, LocalPrefs, QuickTarget } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { shortcutLabel, type Platform } from "@/lib/platform";
import { RecentSuggestions, SUGGESTIONS_ID } from "./HostsRows";
import { buildImportMenuEntries, buildNewMenuEntries, hostSortLabel } from "./menus";
import type { HostSort } from "./ui";
import s from "./HostsPage.module.css";

/** Split primary button: New Host, with New Group under the chevron beside it. */
export function NewHostSplitButton({ onNewHost, onNewGroup }: { onNewHost: () => void; onNewGroup: () => void }) {
  const t = useT();
  const menu = useMenu();
  const buttonRef = useRef<HTMLButtonElement>(null);
  return (
    <span className={s.split}>
      <Button variant="primary" icon="plus" onClick={onNewHost}>
        {t("hosts.newHost")}
      </Button>
      <Button
        ref={buttonRef}
        variant="primary"
        icon="caret-down"
        className={s.splitCaret}
        aria-label={t("hosts.newMenu")}
        title={t("hosts.newMenu")}
        aria-haspopup="menu"
        aria-expanded={!!menu.anchor}
        onClick={() => buttonRef.current && menu.openBelow(buttonRef.current, true)}
      />
      {menu.anchor && (
        <Menu
          anchor={menu.anchor}
          onClose={menu.close}
          minWidth={160}
          entries={buildNewMenuEntries(t, onNewHost, onNewGroup)}
        />
      )}
    </span>
  );
}

/** Import dropdown: `~/.ssh/config`, then other SSH clients (SSH-11, SSH-12, SSH-15). */
export function ImportHostsButton({ onImport }: { onImport: (source: ImportSource) => void }) {
  const t = useT();
  const menu = useMenu();
  const buttonRef = useRef<HTMLButtonElement>(null);
  return (
    <>
      <Button
        ref={buttonRef}
        icon="download-simple"
        trailingIcon="caret-down"
        aria-haspopup="menu"
        aria-expanded={!!menu.anchor}
        onClick={() => buttonRef.current && menu.openBelow(buttonRef.current, true)}
      >
        {t("hosts.import")}
      </Button>
      {menu.anchor && (
        <Menu anchor={menu.anchor} onClose={menu.close} minWidth={180} entries={buildImportMenuEntries(t, onImport)} />
      )}
    </>
  );
}

export interface HostsSearchBoxProps {
  inputRef: RefObject<HTMLInputElement | null>;
  query: string;
  platform: Platform;
  suggestions: QuickTarget[];
  suggestOpen: boolean;
  suggestActive: number;
  onChange: (value: string) => void;
  onFocus: () => void;
  onBlur: () => void;
  onHoverSuggestion: (index: number) => void;
  onConnectSuggestion: (target: QuickTarget) => void;
  onRemoveSuggestion: (target: QuickTarget) => void;
}

export function HostsSearchBox({
  inputRef,
  query,
  platform,
  suggestions,
  suggestOpen,
  suggestActive,
  onChange,
  onFocus,
  onBlur,
  onHoverSuggestion,
  onConnectSuggestion,
  onRemoveSuggestion,
}: HostsSearchBoxProps) {
  const t = useT();
  return (
    <div className={s.searchWrap}>
      <SearchField
        ref={inputRef}
        value={query}
        onChange={onChange}
        placeholder={t("hosts.search")}
        shortcut={shortcutLabel(platform, "Ctrl+Shift+K", "⌘K")}
        width={300}
        onFocus={onFocus}
        onBlur={onBlur}
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
          onHover={onHoverSuggestion}
          onConnect={onConnectSuggestion}
          onRemove={onRemoveSuggestion}
        />
      )}
    </div>
  );
}

/** Page title, or breadcrumbs when scoped to a group. */
export function HostsTitle({
  scope,
  title,
  onAllHosts,
  onGroupMenu,
}: {
  scope: GroupView | undefined;
  title: string;
  onAllHosts: () => void;
  onGroupMenu: (e: MouseEvent, group: GroupView) => void;
}) {
  const t = useT();
  if (!scope) return <>{title}</>;
  return (
    <span className={s.crumbs}>
      <button type="button" className={s.crumb} onClick={onAllHosts}>
        {t("hosts.title")}
      </button>
      <Icon name="caret-right" className={s.crumbSep} />
      <span className={s.crumbHere}>{scope.name}</span>
      <IconButton icon="dots-three" label={t("dashboard.groupMenu")} onClick={(e) => onGroupMenu(e, scope)} />
    </span>
  );
}

/** Top-level row of host group tiles. */
export function GroupBar({
  groups,
  counts,
  onOpenGroup,
  onGroupMenu,
}: {
  groups: readonly GroupView[];
  counts: ReadonlyMap<string, number>;
  onOpenGroup: (id: string) => void;
  onGroupMenu: (e: MouseEvent, group: GroupView) => void;
}) {
  const t = useT();
  if (groups.length === 0) return null;
  return (
    <div className={s.groupBar} role="list" aria-label={t("sidebar.groups")}>
      {groups.map((g) => (
        <div key={g.id} role="listitem" className={s.groupItem}>
          <button
            type="button"
            className={s.groupTile}
            onClick={() => onOpenGroup(g.id)}
            onContextMenu={(e) => onGroupMenu(e, g)}
          >
            <Icon name="folder-simple" className={s.groupIcon} />
            <span className={s.groupName}>{g.name}</span>
            <span className={s.groupCount}>{counts.get(g.id) ?? 0}</span>
          </button>
        </div>
      ))}
    </div>
  );
}

export interface HostsToolbarProps {
  favorites: boolean;
  onToggleFavorites: () => void;
  activeTags: readonly string[];
  onRemoveTag: (tag: string) => void;
  tagsButtonRef: RefObject<HTMLButtonElement | null>;
  tagsOpen: boolean;
  onOpenTags: () => void;
  sortButtonRef: RefObject<HTMLButtonElement | null>;
  sortOpen: boolean;
  shownSort: HostSort;
  onOpenSort: () => void;
  stats: boolean;
  onChangeStats: (stats: boolean) => void;
  view: LocalPrefs["hosts_view"];
  onChangeView: (view: LocalPrefs["hosts_view"]) => void;
}

export function HostsToolbar({
  favorites,
  onToggleFavorites,
  activeTags,
  onRemoveTag,
  tagsButtonRef,
  tagsOpen,
  onOpenTags,
  sortButtonRef,
  sortOpen,
  shownSort,
  onOpenSort,
  stats,
  onChangeStats,
  view,
  onChangeView,
}: HostsToolbarProps) {
  const t = useT();
  return (
    <div className={s.toolbar}>
      <button
        type="button"
        className={cx(s.toggle, favorites && s.toggleOn)}
        aria-pressed={favorites}
        onClick={onToggleFavorites}
      >
        <Icon name="star" fill={favorites} />
        {t("sidebar.favorites")}
      </button>
      <button
        ref={tagsButtonRef}
        type="button"
        className={cx(s.toggle, activeTags.length > 0 && s.toggleOn)}
        aria-haspopup="menu"
        aria-expanded={tagsOpen}
        onClick={onOpenTags}
      >
        <Icon name="tag" />
        {t("sidebar.tags")}
        <Icon name="caret-down" className={s.toggleCaret} />
      </button>
      {activeTags.map((tag) => (
        <TagChip key={tag} name={tag} removeLabel={t("hosts.tags.remove", { tag })} onRemove={() => onRemoveTag(tag)} />
      ))}
      <span className={s.toolbarGap} />
      <Button
        ref={sortButtonRef}
        size="sm"
        trailingIcon="caret-down"
        aria-haspopup="menu"
        aria-expanded={sortOpen}
        aria-label={t("hosts.sort")}
        onClick={onOpenSort}
      >
        {hostSortLabel(t, shownSort)}
      </Button>
      <label className={s.statsSwitch} title={t("hosts.stats.hint")}>
        <Switch checked={stats} label={t("hosts.stats")} onChange={onChangeStats} />
        <span>{t("hosts.stats")}</span>
      </label>
      <Segmented<LocalPrefs["hosts_view"]>
        ariaLabel={t("hosts.view")}
        className={s.viewSwitch}
        value={view}
        options={[
          { value: "list", label: t("hosts.view.list"), title: t("hosts.view.list"), icon: "rows" },
          {
            value: "cards",
            label: t("dashboard.layout.cards"),
            title: t("dashboard.layout.cards"),
            icon: "squares-four",
          },
        ]}
        onChange={onChangeView}
      />
    </div>
  );
}
