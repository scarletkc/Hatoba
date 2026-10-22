import { useEffect, useMemo, useRef, useState, type MouseEvent } from "react";
import { useVaultData } from "@/app/data";
import { useApp } from "@/app/store";
import { Icon, IconButton, LinkButton } from "@/components/controls";
import { EmptyState, SearchField } from "@/components/layout";
import { confirm, Menu, useMenu, type MenuEntry } from "@/components/overlay";
import { StatusLine } from "@/components/patterns";
import { formatRelative, useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { AiConversationView, AiSearchHit, HostView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { isImeEvent } from "@/lib/ime";
import { conversationBusy, deleteConversation, exportConversation, forkConversation, loadHistory, openConversation, pinConversation, renameConversation } from "./actions";
import { MAX_TITLE_CHARS } from "./fork";
import { sortConversations } from "./models";
import { conversationPlace, placeName } from "./place";
import { highlightParts, oneLine, SEARCH_DEBOUNCE_MS } from "./search";
import { conversationActivity, useAi } from "./store";
import s from "./History.module.css";

type Search = { state: "idle" } | { state: "searching" } | { state: "done"; query: string; hits: AiSearchHit[] } | { state: "failed" };

/** AI-24: matches for `query`, asked of Rust once typing pauses. */
function useSearch(query: string): [Search, () => void] {
  const [result, setResult] = useState<Search>({ state: "idle" });
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setResult({ state: "idle" });
      return;
    }
    let live = true;
    setResult((r) => (r.state === "done" ? r : { state: "searching" }));
    const timer = window.setTimeout(() => {
      setResult((r) => (r.state === "done" ? r : { state: "searching" }));
      api
        .ai_search(q)
        .then((hits) => live && setResult({ state: "done", query: q, hits }))
        .catch(() => live && setResult({ state: "failed" }));
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [query, retry]);
  return [result, () => setRetry((n) => n + 1)];
}

/** AI-23: conversations by last activity, pinned first, with title, host or quick-connect target, and time. */
export function History({ slotId, currentId, onClose }: { slotId: string; currentId: string | null; onClose: () => void }) {
  const t = useT();
  const history = useAi((st) => st.history);
  const failed = useAi((st) => st.historyFailed);
  const hosts = useVaultData((st) => st.hosts);
  const [renaming, setRenaming] = useState<string | null>(null);
  const menu = useMenu();
  const [menuEntries, setMenuEntries] = useState<MenuEntry[]>([]);
  /** The row whose ⋯ button opened the menu. */
  const [menuRow, setMenuRow] = useState<string | null>(null);
  const list = useMemo(() => (history ? sortConversations(history) : null), [history]);
  const [query, setQuery] = useState("");
  const [search, retrySearch] = useSearch(query);
  const searching = query.trim() !== "";

  useEffect(() => {
    void loadHistory();
  }, []);

  const open = (c: AiConversationView) => {
    void openConversation(slotId, c.id);
    onClose();
  };

  // A hit opens its conversation and scrolls to the matching entry.
  const openHit = (hit: AiSearchHit) => {
    void openConversation(slotId, hit.conversation_id, hit.entry_id);
    onClose();
  };

  const fork = (c: AiConversationView) => {
    void forkConversation(slotId, c.id, null);
    onClose();
  };

  const remove = async (c: AiConversationView) => {
    const ok = await confirm({
      title: t("ai.history.deleteTitle", { title: c.title || t("ai.untitled") }),
      body: t("ai.history.deleteBody"),
      confirmLabel: t("btn.delete"),
      danger: true,
    });
    if (ok) await deleteConversation(c.id);
  };

  const showMenu = (e: MouseEvent, c: AiConversationView, anchor?: HTMLElement) => {
    e.preventDefault();
    e.stopPropagation();
    setMenuEntries([
      { label: t("ai.history.open"), icon: "chat-circle-dots", onSelect: () => open(c) },
      // AI-43: a copy of the whole conversation, which opens here; not while it is busy in its tab or a fork of it is being made.
      { label: t("ai.history.fork"), icon: "git-fork", disabled: conversationBusy(c.id) || useAi.getState().forking.includes(c.id), onSelect: () => fork(c) },
      { label: t("btn.rename"), icon: "pencil-simple", onSelect: () => setRenaming(c.id) },
      { label: t(c.pinned ? "ai.history.unpin" : "ai.history.pin"), icon: c.pinned ? "push-pin-slash" : "push-pin", onSelect: () => void pinConversation(c.id, !c.pinned) },
      { label: t("ai.history.export"), icon: "export", onSelect: () => void exportConversation(c.id) },
      { kind: "separator" },
      { label: t("ai.history.delete"), icon: "trash", danger: true, onSelect: () => void remove(c) },
    ]);
    setMenuRow(anchor ? c.id : null);
    if (anchor) menu.openBelow(anchor, true);
    else menu.openAt({ x: e.clientX, y: e.clientY });
  };

  return (
    <div className={s.history}>
      <div className={s.head}>
        <IconButton small icon="caret-left" label={t("btn.back")} size={14} onClick={onClose} />
        <span className={s.title}>{t("ai.history")}</span>
      </div>
      <SearchField fill value={query} onChange={setQuery} placeholder={t("ai.search")} clearLabel={t("ai.search.clear")} className={s.search} />
      {searching ? (
        <SearchResults search={search} query={query.trim()} history={history} hosts={hosts} currentId={currentId} onOpen={openHit} onRetry={retrySearch} />
      ) : (
        <div className={s.list} role="list">
          {list === null && (
            <div className={s.message}>
              <StatusLine tone="progress">{t("ai.loading")}</StatusLine>
            </div>
          )}
          {list !== null && failed && (
            <div className={s.message}>
              {t("ai.history.failed")}
              <LinkButton icon="arrows-clockwise" onClick={() => void loadHistory()}>
                {t("btn.retry")}
              </LinkButton>
            </div>
          )}
          {list?.length === 0 && !failed && (
            <EmptyState compact icon="chats" title={t("ai.history.empty")} />
          )}
          {list?.map((c) => {
            const place = conversationPlace(c, hosts);
            const menuOpen = !!menu.anchor?.trigger && menuRow === c.id;
            return (
              <div
                key={c.id}
                role="listitem"
                className={cx(s.row, c.id === currentId && s.rowCurrent)}
                onClick={() => renaming !== c.id && open(c)}
                onContextMenu={(e) => showMenu(e, c)}
              >
                <div className={s.rowMain}>
                  {renaming === c.id ? (
                    <TitleInput
                      initial={c.title}
                      onDone={(title) => {
                        setRenaming(null);
                        if (title !== null && title.trim() && title.trim() !== c.title) void renameConversation(c.id, title.trim());
                      }}
                    />
                  ) : (
                    <div className={s.rowTitle}>
                      {c.pinned && <Icon name="push-pin" fill size={12} className={s.pin} />}
                      <span className={s.titleText}>{c.title || t("ai.untitled")}</span>
                      <Activity id={c.id} />
                    </div>
                  )}
                  <div className={s.rowMeta}>
                    {place.kind === "host" && <Icon name="hard-drives" size={12} />}
                    {place.kind === "target" && <Icon name="terminal-window" size={12} />}
                    {place.kind === "local" && <Icon name="desktop" size={12} />}
                    <span className={s.metaHost}>{placeName(t, place, useApp.getState().info.device_name)}</span>
                    <span className={s.dot}>·</span>
                    <span className={s.time}>{formatRelative(t.locale, c.last_activity)}</span>
                  </div>
                </div>
                <IconButton
                  icon="dots-three"
                  label={t("ai.history.more")}
                  className={s.more}
                  active={menuOpen}
                  aria-haspopup="menu"
                  aria-expanded={menuOpen}
                  onClick={(e) => showMenu(e, c, e.currentTarget)}
                />
              </div>
            );
          })}
        </div>
      )}
      {menu.anchor && <Menu anchor={menu.anchor} entries={menuEntries} onClose={menu.close} minWidth={168} />}
    </div>
  );
}

/**
 * AI-07: a conversation whose turn is running or waiting for approval, in a tab or in the background, marked as
 * the tab bar marks its tab, so that it can be found and opened again.
 */
function Activity({ id }: { id: string }) {
  const t = useT();
  const activity = useAi((st) => conversationActivity(st, id));
  if (activity === "idle") return null;
  const label = t(activity === "waiting" ? "ai.tabWaiting" : "ai.tabRunning");
  return (
    <span className={cx(s.activity, activity === "waiting" ? s.activityWaiting : s.activityRunning)} title={label} aria-label={label} role="img">
      <Icon name={activity === "waiting" ? "hand-palm" : "sparkle"} fill={activity === "waiting"} size={12} />
    </span>
  );
}

function Highlighted({ text, query }: { text: string; query: string }) {
  return (
    <>
      {highlightParts(text, query).map((p, i) =>
        p.match ? (
          <mark key={i} className={s.mark}>
            {p.text}
          </mark>
        ) : (
          p.text
        ),
      )}
    </>
  );
}

function SearchResults({
  search,
  query,
  history,
  hosts,
  currentId,
  onOpen,
  onRetry,
}: {
  search: Search;
  query: string;
  history: AiConversationView[] | null;
  hosts: HostView[];
  currentId: string | null;
  onOpen: (hit: AiSearchHit) => void;
  onRetry: () => void;
}) {
  const t = useT();
  if (search.state === "idle" || search.state === "searching")
    return (
      <div className={s.list}>
        <div className={s.message}>
          <StatusLine tone="progress">{t("ai.search.searching")}</StatusLine>
        </div>
      </div>
    );
  if (search.state === "failed")
    return (
      <div className={s.list}>
        <div className={s.message}>
          {t("ai.search.failed")}
          <LinkButton icon="arrows-clockwise" onClick={onRetry}>
            {t("btn.retry")}
          </LinkButton>
        </div>
      </div>
    );
  if (search.hits.length === 0)
    return (
      <div className={s.list}>
        <div className={s.message}>{t("ai.search.none", { q: query })}</div>
      </div>
    );
  return (
    <div className={s.list} role="list">
      {search.hits.map((hit) => {
        const c = history?.find((x) => x.id === hit.conversation_id);
        const title = c?.title || t("ai.untitled");
        return (
          <button
            key={hit.conversation_id}
            type="button"
            role="listitem"
            className={cx(s.row, s.hit, hit.conversation_id === currentId && s.rowCurrent)}
            onClick={() => onOpen(hit)}
          >
            <div className={s.rowMain}>
              <div className={s.rowTitle}>
                {c?.pinned && <Icon name="push-pin" fill size={12} className={s.pin} />}
                <span className={s.titleText}>
                  <Highlighted text={title} query={search.query} />
                </span>
                <Activity id={hit.conversation_id} />
              </div>
              {hit.entry_id && (
                <div className={s.snippet}>
                  <Highlighted text={oneLine(hit.snippet)} query={search.query} />
                </div>
              )}
              {c && (
                <div className={s.rowMeta}>
                  <span className={s.metaHost}>{placeName(t, conversationPlace(c, hosts), useApp.getState().info.device_name)}</span>
                  <span className={s.dot}>·</span>
                  <span className={s.time}>{formatRelative(t.locale, c.last_activity)}</span>
                </div>
              )}
            </div>
          </button>
        );
      })}
    </div>
  );
}

/** Inline rename: Enter or blur saves, Esc cancels. */
function TitleInput({ initial, onDone }: { initial: string; onDone: (title: string | null) => void }) {
  const t = useT();
  const ref = useRef<HTMLInputElement>(null);
  const done = useRef(false);
  const [value, setValue] = useState(initial);
  useEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);
  const finish = (v: string | null) => {
    if (done.current) return;
    done.current = true;
    onDone(v);
  };
  return (
    <input
      ref={ref}
      className={s.renameInput}
      value={value}
      maxLength={MAX_TITLE_CHARS}
      aria-label={t("ai.history.renameLabel")}
      onChange={(e) => setValue(e.target.value)}
      onClick={(e) => e.stopPropagation()}
      onBlur={() => finish(value)}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter" && !isImeEvent(e)) {
          e.preventDefault();
          finish(value);
        } else if (e.key === "Escape") {
          e.preventDefault();
          finish(null);
        }
      }}
    />
  );
}
