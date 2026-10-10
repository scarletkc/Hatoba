import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from "react";
import { useVaultData } from "@/app/data";
import { useApp } from "@/app/store";
import { useTabs, type SessionTab } from "@/app/tabs";
import { Button, Icon, IconButton, LinkButton, Spinner } from "@/components/controls";
import { confirm, Menu, useMenu } from "@/components/overlay";
import { reconnectSession } from "@/features/terminal/connect";
import { useT } from "@/i18n";
import type { AiPermissionMode, HostView } from "@/ipc/types";
import { shortcutLabel } from "@/lib/platform";
import { cx } from "@/lib/cx";
import { attachDroppedPaths, attachFiles, connectConversation, continueHomeConversation, focusInput, newConversation, reloadSlot, setSlotMode, stopTurn, toggleAiPanel } from "./actions";
import { Composer } from "./Composer";
import { usePanelFileDrop } from "./fileDrop";
import { History } from "./History";
import { hostInfo, MessageList } from "./Messages";
import { conversationModel, hasModels } from "./models";
import { blankSlot, HOME_SLOT, offersHomeConversation, useAi, type Slot } from "./store";
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
  const convHost = slot.conversation?.host_id ? hosts.find((h) => h.id === slot.conversation?.host_id) : undefined;
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
    // AI-18: Esc while focus is in the panel stops the turn.
    if (e.key === "Escape" && !e.defaultPrevented && busy) {
      e.preventDefault();
      void stopTurn(slotId);
    }
  };

  const title = slot.conversation?.title || (slot.conversationId ? t("ai.untitled") : t("ai.newConversation"));
  const hostLabel = tab ? (tabHost?.name ?? tab.title) : t("ai.noTerminal");

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
          <Icon name="file-arrow-up" size={22} />
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
        <ModeButton slotId={slotId} mode={slot.mode} />
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
        <NoProvider none={catalog.providers.length === 0} />
      ) : (
        <>
          <Notices slotId={slotId} slot={slot} tab={tab} tabHost={tabHost} convHost={convHost} />
          {slot.loadFailed ? (
            <div className={s.center}>
              <span>{t("ai.loadFailed")}</span>
              <LinkButton icon="arrows-clockwise" onClick={() => void reloadSlot(slotId)}>
                {t("btn.retry")}
              </LinkButton>
            </div>
          ) : (
            <MessageList key={`${slotId}:${slot.conversationId ?? ""}`} slotId={slotId} slot={slot} host={hostInfo(tabHost)} empty={<EmptyConversation tab={tab} host={tabHost} mode={slot.mode} />} />
          )}
          <Composer slotId={slotId} slot={slot} model={model} tools={!!tab} />
        </>
      )}
    </aside>
  );
}

/** AI-16: the conversation's permission mode. The first switch to bypass on a device asks first. */
function ModeButton({ slotId, mode }: { slotId: string; mode: AiPermissionMode }) {
  const t = useT();
  const menu = useMenu();
  const ref = useRef<HTMLButtonElement>(null);

  const choose = async (next: AiPermissionMode) => {
    if (next === mode) return;
    if (next === "bypass" && !useApp.getState().prefs.ai_bypass_confirmed) {
      const ok = await confirm({
        title: t("ai.bypass.title"),
        body: t("ai.bypass.body"),
        confirmLabel: t("ai.bypass.confirm"),
        danger: true,
        icon: "lightning",
        iconColor: "var(--orange)",
      });
      if (!ok) return;
      const app = useApp.getState();
      void app.setPrefs({ ...app.prefs, ai_bypass_confirmed: true }).catch(() => {});
    }
    setSlotMode(slotId, next);
  };

  const label = mode === "bypass" ? t("ai.mode.bypass") : t("ai.mode.manual");
  return (
    <>
      <button
        ref={ref}
        type="button"
        className={cx(s.mode, mode === "bypass" && s.modeBypass)}
        aria-haspopup="menu"
        aria-expanded={!!menu.anchor}
        aria-label={`${t("ai.mode")}: ${label}`}
        title={`${t("ai.mode")}: ${label}`}
        onClick={() => ref.current && menu.openBelow(ref.current, true)}
      >
        <Icon name={mode === "bypass" ? "lightning" : "shield-check"} size={14} />
        {mode === "bypass" && <span>{t("ai.mode.bypassShort")}</span>}
      </button>
      {menu.anchor && (
        <Menu
          anchor={menu.anchor}
          onClose={menu.close}
          minWidth={230}
          entries={[
            { kind: "header", label: t("ai.mode.menuTitle") },
            { label: t("ai.mode.manual"), icon: "shield-check", checked: mode === "manual", onSelect: () => void choose("manual") },
            { label: t("ai.mode.bypass"), icon: "lightning", checked: mode === "bypass", onSelect: () => void choose("bypass") },
          ]}
        />
      )}
    </>
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
 * The tab's state: no terminal (AI-09), disconnected (AI-08), a conversation from another host, or the
 * home tab's conversation to continue here (AI-09).
 */
function Notices({ slotId, slot, tab, tabHost, convHost }: { slotId: string; slot: Slot; tab: SessionTab | undefined; tabHost: HostView | undefined; convHost: HostView | undefined }) {
  const t = useT();
  const home = useAi((st) => (tab ? st.slots[HOME_SLOT] : undefined));
  const items: ReactNode[] = [];
  if (!tab) {
    if (convHost)
      items.push(
        <Notice
          key="tab"
          icon={<Icon name="terminal-window" size={14} />}
          action={
            <Button size="xs" icon="plugs-connected" onClick={() => void connectConversation(convHost.id)}>
              {t("ai.connectTo", { host: convHost.name })}
            </Button>
          }
        >
          {t("ai.notice.noTab")}
        </Notice>,
      );
    else if (slot.entries.length > 0) items.push(<Notice key="tab" icon={<Icon name="terminal-window" size={14} />}>{t("ai.notice.noTab")}</Notice>);
  } else {
    const name = tabHost?.name ?? tab.title;
    if (tab.status === "disconnected" || tab.status === "failed")
      items.push(
        <Notice
          key="state"
          tone="warn"
          icon={<Icon name="plugs" size={14} />}
          action={
            <Button size="xs" icon="arrow-clockwise" onClick={() => void reconnectSession(tab.id)}>
              {t("terminal.reconnect")}
            </Button>
          }
        >
          {t("ai.notice.disconnected", { host: name })}
        </Notice>,
      );
    else if (tab.status === "connecting") items.push(<Notice key="state" icon={<Spinner size={12} />}>{t("ai.notice.connecting", { host: name })}</Notice>);
    if (home && offersHomeConversation(slot, home))
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
          {t("ai.notice.homeConversation", { title: home.conversation?.title || t("ai.untitled") })}
        </Notice>,
      );
    const convHostId = slot.conversation?.host_id;
    if (convHostId && convHostId !== tab.hostId)
      items.push(
        <Notice key="move" icon={<Icon name="arrows-left-right" size={14} />}>
          {t("ai.notice.moveHost", { from: convHost?.name ?? t("ai.history.hostGone"), to: name })}
        </Notice>,
      );
  }
  return items.length > 0 ? <div className={s.notices}>{items}</div> : null;
}

function EmptyConversation({ tab, host, mode }: { tab: SessionTab | undefined; host: HostView | undefined; mode: AiPermissionMode }) {
  const t = useT();
  return (
    <div className={s.empty}>
      <div className={s.emptyIcon}>
        <Icon name="sparkle" size={20} />
      </div>
      <div className={s.emptyTitle}>{tab ? t("ai.empty.title", { host: host?.name ?? tab.title }) : t("ai.empty.homeTitle")}</div>
      <div className={s.emptyBody}>{tab ? t("ai.empty.body") : t("ai.empty.homeBody")}</div>
      {tab && <div className={s.emptyNote}>{mode === "bypass" ? t("ai.empty.bypass") : t("ai.empty.manual")}</div>}
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
      <div className={s.emptyBody}>{none ? t("ai.noProvider.body") : t("ai.noModels.body")}</div>
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
  const waiting = useAi((st) => Object.values(st.slots).some((x) => x.turn?.call?.state === "approval" || x.turn?.call?.state === "limit"));
  const label = `${t(open ? "ai.hide" : "ai.show")} (${shortcutLabel(platform, "Ctrl+Shift+A", "⌘⇧A")})`;
  return (
    <button type="button" className={cx(className, open && activeClassName)} title={label} aria-label={label} aria-pressed={open} onClick={() => toggleAiPanel()}>
      <Icon name="sparkle" size={15} />
      {waiting && !open && <span className={dotClassName} />}
    </button>
  );
}
