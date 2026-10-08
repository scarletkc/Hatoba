import { useEffect, useMemo, useRef, useState, type MouseEvent } from "react";
import { useVaultData } from "@/app/data";
import { Icon, IconButton, LinkButton, Spinner } from "@/components/controls";
import { confirm, Menu, useMenu, type MenuEntry } from "@/components/overlay";
import { formatRelative, useT } from "@/i18n";
import type { AiConversationView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { deleteConversation, loadHistory, openConversation, pinConversation, renameConversation } from "./actions";
import { sortConversations } from "./models";
import { useAi } from "./store";
import s from "./History.module.css";

/** AI-23: conversations by last activity, pinned first, with title, host and time. */
export function History({ slotId, currentId, onClose }: { slotId: string; currentId: string | null; onClose: () => void }) {
  const t = useT();
  const history = useAi((st) => st.history);
  const failed = useAi((st) => st.historyFailed);
  const hosts = useVaultData((st) => st.hosts);
  const [renaming, setRenaming] = useState<string | null>(null);
  const menu = useMenu();
  const [menuEntries, setMenuEntries] = useState<MenuEntry[]>([]);
  const list = useMemo(() => (history ? sortConversations(history) : null), [history]);

  useEffect(() => {
    void loadHistory();
  }, []);

  const open = (c: AiConversationView) => {
    void openConversation(slotId, c.id);
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
      { label: t("btn.rename"), icon: "pencil-simple", onSelect: () => setRenaming(c.id) },
      { label: t(c.pinned ? "ai.history.unpin" : "ai.history.pin"), icon: c.pinned ? "push-pin-slash" : "push-pin", onSelect: () => void pinConversation(c.id, !c.pinned) },
      { kind: "separator" },
      { label: t("ai.history.delete"), icon: "trash", danger: true, onSelect: () => void remove(c) },
    ]);
    if (anchor) menu.openBelow(anchor, true);
    else menu.openAt({ x: e.clientX, y: e.clientY });
  };

  return (
    <div className={s.history}>
      <div className={s.head}>
        <button type="button" className={s.back} onClick={onClose} title={t("btn.back")} aria-label={t("btn.back")}>
          <Icon name="caret-left" size={13} />
        </button>
        <span className={s.title}>{t("ai.history")}</span>
      </div>
      <div className={s.list} role="list">
        {list === null && (
          <div className={s.message}>
            <Spinner size={13} />
            {t("ai.loading")}
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
          <div className={s.empty}>
            <Icon name="chats" size={26} className={s.emptyIcon} />
            <div className={s.emptyTitle}>{t("ai.history.empty")}</div>
            <div className={s.emptyBody}>{t("ai.history.emptyBody")}</div>
          </div>
        )}
        {list?.map((c) => {
          const host = hosts.find((h) => h.id === c.host_id);
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
                    {c.pinned && <Icon name="push-pin" fill size={11} className={s.pin} />}
                    <span className={s.titleText}>{c.title || t("ai.untitled")}</span>
                  </div>
                )}
                <div className={s.rowMeta}>
                  {host ? (
                    <>
                      <Icon name="hard-drives" size={11} />
                      <span className={s.metaHost}>{host.name}</span>
                    </>
                  ) : (
                    <span className={s.metaHost}>{c.host_id ? t("ai.history.hostGone") : t("ai.history.noHost")}</span>
                  )}
                  <span className={s.dot}>·</span>
                  <span className={s.time}>{formatRelative(t.locale, c.last_activity)}</span>
                </div>
              </div>
              <IconButton
                icon="dots-three"
                label={t("ai.history.more")}
                className={s.more}
                onClick={(e) => showMenu(e, c, e.currentTarget)}
              />
            </div>
          );
        })}
      </div>
      {menu.anchor && <Menu anchor={menu.anchor} entries={menuEntries} onClose={menu.close} minWidth={168} />}
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
      maxLength={200}
      aria-label={t("ai.history.renameLabel")}
      onChange={(e) => setValue(e.target.value)}
      onClick={(e) => e.stopPropagation()}
      onBlur={() => finish(value)}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter" && !e.nativeEvent.isComposing) {
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
