import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from "react";
import { useVaultData } from "@/app/data";
import { useApp } from "@/app/store";
import { useTabs, type SessionTab } from "@/app/tabs";
import { Button, Icon, IconButton, LinkButton, Spinner } from "@/components/controls";
import { reconnectSession } from "@/features/terminal/connect";
import { shellName } from "@/features/terminal/localShells";
import { useT } from "@/i18n";
import type { HostView } from "@/ipc/types";
import { shortcutLabel } from "@/lib/platform";
import { cx } from "@/lib/cx";
import {
  attachDroppedPaths,
  attachFiles,
  connectConversation,
  continueHomeConversation,
  focusInput,
  newConversation,
  reloadSlot,
  stopSideQuestion,
  stopTurn,
  toggleAiPanel,
} from "./actions";
import { dialectOf } from "./commandRules";
import { Composer } from "./Composer";
import { usePanelFileDrop } from "./fileDrop";
import { History } from "./History";
import { hostInfo, MessageList } from "./Messages";
import { chats, conversationModel, hasModels } from "./models";
import { connectionFor, movedFrom, placeLabel } from "./place";
import { sideRunning } from "./side";
import { blankSlot, HOME_SLOT, offersHomeConversation, useAi, type Slot } from "./store";
import { approvalOf } from "./turn";
import s from "./AiPanel.module.css";

export const PANEL_DEFAULT_WIDTH = 380;
const PANEL_MIN = 300;
const PANEL_MAX = 900;
/** Room the page or terminal keeps beside the panel (`.panel`'s max-width). */
const CONTENT_MIN = 480;

const clampWidth = (w: number) => Math.round(Math.min(PANEL_MAX, Math.max(PANEL_MIN, w)));

/** The AI panel at the right edge of the window, below the tab bar (§9, §13). It shows the active tab's conversation. */
export function AiPanel({ slotId }: { slotId: string }) {
  const t = useT();
  const prefWidth = useApp((st) => st.prefs.ai_panel_width);
  const defaultMode = useApp((st) => st.prefs.ai_permission_mode);
  const stored = useAi((st) => st.slots[slotId]);
  const blank = useMemo(() => blankSlot(defaultMode), [defaultMode]);
  const slot = stored ?? blank;
  const catalog = useAi((st) => st.catalog);
  const tab = useTabs((st) => (slotId === HOME_SLOT ? undefined : st.tabs.find((x) => x.id === slotId)));
  const hosts = useVaultData((st) => st.hosts);
  const tabHost = tab ? hosts.find((h) => h.id === tab.hostId) : undefined;
  // §7.5: a local tab is this computer, named so wherever a tab's host is named.
  const local = tab?.local ? { name: t("terminal.local.thisComputer"), target: shellName(tab.local, t), local: true, dialect: dialectOf(tab.local.kind) } : null;
  const [view, setView] = useState<"chat" | "history">("chat");
  const [dragWidth, setDragWidth] = useState<number | null>(null);
  const panelRef = useRef<HTMLElement>(null);

  useEffect(() => setView("chat"), [slotId]);
  // A request to focus the input box (Ask AI, New conversation) needs the conversation in view.
  const focusTick = useAi((st) => st.focusTick);
  useEffect(() => setView("chat"), [focusTick]);

  const model = conversationModel(slot.entries, catalog.providers, catalog.settings, slot.model);
  const ready = catalog.loaded && hasModels(catalog.providers);
  const busy = !!slot.turn || slot.remoteRunning;
  // AI-35: text files dropped on the panel go with the next message.
  const drop = usePanelFileDrop(panelRef, ready, {
    onFiles: (files) => void attachFiles(slotId, files),
    onPaths: (paths) => void attachDroppedPaths(slotId, paths),
  });

  const saveWidth = (width: number) => {
    const app = useApp.getState();
    if (width !== app.prefs.ai_panel_width) void app.setPrefs({ ...app.prefs, ai_panel_width: width }).catch(() => {});
  };

  /** Drag the left edge to resize; the width is a device preference. */
  const onResizeStart = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const handle = e.currentTarget;
    const startX = e.clientX;
    const startWidth = panelRef.current?.getBoundingClientRect().width ?? prefWidth;
    // Saves only the width the panel can show, so it doesn't grow wider than it was dragged later.
    const room = (panelRef.current?.parentElement?.clientWidth ?? Infinity) - CONTENT_MIN;
    const fit = (w: number) => clampWidth(Math.min(w, room));
    let width = fit(startWidth);
    handle.setPointerCapture(e.pointerId);
    document.documentElement.classList.add(s.resizing);
    const move = (ev: globalThis.PointerEvent) => {
      width = fit(startWidth + (startX - ev.clientX));
      setDragWidth(width);
    };
    const end = () => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", end);
      handle.removeEventListener("pointercancel", end);
      document.documentElement.classList.remove(s.resizing);
      setDragWidth(null);
      saveWidth(width);
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", end);
    handle.addEventListener("pointercancel", end);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "Escape" || e.defaultPrevented) return;
    // AI-42: a side question on its way stops first, and only it.
    if (sideRunning(slot.side)) {
      e.preventDefault();
      stopSideQuestion(slotId);
    } else if (busy) {
      // AI-18: Esc while focus is in the panel stops the turn.
      e.preventDefault();
      void stopTurn(slotId);
    }
  };

  const title = slot.conversation?.title || (slot.conversationId ? t("ai.untitled") : t("ai.newConversation"));
  const hostLabel = tab ? (local ? `${local.name} · ${local.target}` : (tabHost?.name ?? tab.title)) : t("ai.noTerminal");

  return (
    <aside
      ref={panelRef}
      className={s.panel}
      style={{ width: dragWidth ?? clampWidth(prefWidth || PANEL_DEFAULT_WIDTH) }}
      aria-label={t("ai.title")}
      onKeyDown={onKeyDown}
      {...drop.handlers}
    >
      {drop.dragging && (
        <div className={s.drop} aria-hidden>
          <Icon name="file-arrow-up" size={24} />
          <span>{t("ai.attach.drop")}</span>
        </div>
      )}
      <div
        className={s.resize}
        role="separator"
        aria-orientation="vertical"
        aria-label={t("ai.resize")}
        title={t("ai.resize")}
        onPointerDown={onResizeStart}
        onDoubleClick={() => saveWidth(PANEL_DEFAULT_WIDTH)}
      />
      <div className={s.header}>
        <Icon name="sparkle" size={14} className={s.headerIcon} />
        <div className={s.heading}>
          <span className={s.title} title={title}>
            {title}
          </span>
          <span className={s.host} title={tabHost ? `${tabHost.username}@${tabHost.address}:${tabHost.port}` : undefined}>
            {hostLabel}
          </span>
        </div>
        <IconButton
          icon="clock-counter-clockwise"
          label={t("ai.history")}
          active={view === "history"}
          className={s.iconBtn}
          onClick={() => setView(view === "history" ? "chat" : "history")}
        />
        <IconButton
          icon="note-pencil"
          label={t("ai.newConversation")}
          className={s.iconBtn}
          disabled={!slot.conversationId && slot.entries.length === 0 && !busy}
          onClick={() => {
            setView("chat");
            newConversation(slotId);
          }}
        />
        <IconButton icon="x" label={t("ai.close")} className={s.iconBtn} onClick={() => toggleAiPanel(false)} />
      </div>

      {view === "history" ? (
        <History
          slotId={slotId}
          currentId={slot.conversationId}
          onClose={() => {
            setView("chat");
            focusInput();
          }}
        />
      ) : !catalog.loaded ? (
        <div className={s.center}>
          <Spinner size={14} />
        </div>
      ) : !ready ? (
        <NoProvider none={!catalog.providers.some(chats)} />
      ) : (
        <>
          <Notices slotId={slotId} slot={slot} tab={tab} tabHost={tabHost} hosts={hosts} />
          {slot.loadFailed ? (
            <div className={s.center}>
              <span>{t("ai.loadFailed")}</span>
              <LinkButton icon="arrows-clockwise" onClick={() => void reloadSlot(slotId)}>
                {t("btn.retry")}
              </LinkButton>
            </div>
          ) : (
            <MessageList
              key={`${slotId}:${slot.conversationId ?? ""}`}
              slotId={slotId}
              slot={slot}
              host={local ?? hostInfo(tabHost)}
              empty={<EmptyConversation tab={tab} host={tabHost} />}
            />
          )}
          <Composer slotId={slotId} slot={slot} model={model} tools={!!tab} />
        </>
      )}
    </aside>
  );
}

function Notice({ icon, tone, children, action }: { icon: ReactNode; tone?: "warn"; children: ReactNode; action?: ReactNode }) {
  return (
    <div className={cx(s.notice, tone === "warn" && s.noticeWarn)} role="status">
      <span className={s.noticeIcon}>{icon}</span>
      <span className={s.noticeText}>{children}</span>
      {action}
    </div>
  );
}

/**
 * The tab's state: no terminal (AI-09), disconnected (AI-08), a conversation from another host or
 * quick-connect target, or the home tab's conversation to continue here (AI-09).
 */
function Notices({ slotId, slot, tab, tabHost, hosts }: { slotId: string; slot: Slot; tab: SessionTab | undefined; tabHost: HostView | undefined; hosts: HostView[] }) {
  const t = useT();
  const device = useApp((st) => st.info.device_name);
  // The title of the home tab's conversation while this tab offers to continue it (AI-09).
  const homeTitle = useAi((st) => (tab && offersHomeConversation(slot, st.slots[HOME_SLOT]) ? (st.slots[HOME_SLOT]?.conversation?.title ?? "") : null));
  const items: ReactNode[] = [];
  if (!tab) {
    const connection = slot.conversation ? connectionFor(slot.conversation, hosts, device) : null;
    if (connection)
      items.push(
        <Notice
          key="tab"
          icon={<Icon name="terminal-window" size={14} />}
          action={
            <Button size="xs" icon="plugs-connected" onClick={() => void connectConversation(connection.source)}>
              {connection.source === "local" ? t("ai.openLocal") : t("ai.connectTo", { host: connection.name })}
            </Button>
          }
        >
          {t("ai.notice.noTab")}
        </Notice>,
      );
    else if (slot.entries.length > 0) items.push(<Notice key="tab" icon={<Icon name="terminal-window" size={14} />}>{t("ai.notice.noTab")}</Notice>);
  } else {
    const name = tab.local ? `${t("terminal.local.thisComputer")} · ${tab.title}` : (tabHost?.name ?? tab.title);
    if (tab.status === "disconnected" || tab.status === "failed")
      items.push(
        <Notice
          key="state"
          tone="warn"
          icon={<Icon name="plugs" size={14} />}
          action={
            <Button size="xs" icon="arrow-clockwise" onClick={() => void reconnectSession(tab.id)}>
              {t(tab.local ? "terminal.local.restart" : "terminal.reconnect")}
            </Button>
          }
        >
          {t("ai.notice.disconnected", { host: name })}
        </Notice>,
      );
    else if (tab.status === "connecting") items.push(<Notice key="state" icon={<Spinner size={12} />}>{t("ai.notice.connecting", { host: name })}</Notice>);
    if (homeTitle !== null)
      items.push(
        <Notice
          key="home"
          icon={<Icon name="chats" size={14} />}
          action={
            <Button size="xs" icon="arrow-right" onClick={() => continueHomeConversation(slotId)}>
              {t("ai.continueHere")}
            </Button>
          }
        >
          {t("ai.notice.homeConversation", { title: homeTitle || t("ai.untitled") })}
        </Notice>,
      );
    // Only for a move the next message notes, named as its divider will name them.
    const move = slot.conversation ? movedFrom(slot.conversation, tab, hosts, device) : null;
    if (move)
      items.push(
        <Notice key="move" icon={<Icon name="arrows-left-right" size={14} />}>
          {t("ai.notice.moveHost", { from: move.from ? placeLabel(t, move.from, device) : t("ai.history.hostGone"), to: placeLabel(t, move.to, device) })}
        </Notice>,
      );
  }
  return items.length > 0 ? <div className={s.notices}>{items}</div> : null;
}

function EmptyConversation({ tab, host }: { tab: SessionTab | undefined; host: HostView | undefined }) {
  const t = useT();
  return (
    <div className={s.empty}>
      <div className={s.emptyIcon}>
        <Icon name="sparkle" size={20} />
      </div>
      <div className={s.emptyTitle}>
        {tab ? t("ai.empty.title", { host: tab.local ? t("terminal.local.thisComputer") : (host?.name ?? tab.title) }) : t("ai.empty.homeTitle")}
      </div>
      <div className={s.emptyBody}>{tab ? t("ai.empty.body") : t("ai.empty.homeBody")}</div>
    </div>
  );
}

/** §9: no provider configured, with a link to Settings → AI. */
function NoProvider({ none }: { none: boolean }) {
  const t = useT();
  return (
    <div className={s.empty}>
      <div className={s.emptyIcon}>
        <Icon name="plug" size={20} />
      </div>
      <div className={s.emptyTitle}>{none ? t("ai.noProvider.title") : t("ai.noModels.title")}</div>
      {!none && <div className={s.emptyBody}>{t("ai.noModels.body")}</div>}
      <Button variant="primary" size="sm" icon="gear-six" onClick={() => useApp.getState().openSettings(true, "ai")}>
        {t("ai.noProvider.open")}
      </Button>
    </div>
  );
}

/** The tab bar's AI button (§9) and its shortcut hint. */
export function AiPanelButton({ className, activeClassName, dotClassName }: { className: string; activeClassName: string; dotClassName: string }) {
  const t = useT();
  const open = useApp((st) => st.prefs.ai_panel_open);
  const platform = useApp((st) => st.info.platform);
  const waiting = useAi((st) => Object.values(st.slots).some((x) => !!approvalOf(x.turn)));
  const label = `${t(open ? "ai.hide" : "ai.show")} (${shortcutLabel(platform, "Ctrl+Shift+A", "⌘⇧A")})`;
  return (
    <button type="button" className={cx(className, open && activeClassName)} title={label} aria-label={label} aria-pressed={open} onClick={() => toggleAiPanel()}>
      <Icon name="sparkle" size={16} />
      {waiting && !open && <span className={dotClassName} />}
    </button>
  );
}
