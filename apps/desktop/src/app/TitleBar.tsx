import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
} from "react";
import { Icon, Menu, useMenu, type MenuEntry } from "@hatoba/ui";
import { AiPanelButton } from "@/features/ai/AiPanel";
import { tabActivity, useAi } from "@/features/ai/store";
import { useT } from "@/i18n";
import { api, isTauri } from "@/ipc/api";
import type { LocalShellKind, LocalShellView } from "@/ipc/types";
import { OsIcon } from "@/lib/OsIcon";
import { shortcutLabel } from "@/lib/platform";
import { cx } from "@/lib/cx";
import { motion } from "@/lib/motion";
import { openLocalTerminal } from "@/session/connect";
import { loadLocalShells, preferredShell, shellName } from "@/session/localShells";
import { getSession } from "@/session/session";
import { useVaultData } from "./data";
import { useApp } from "./store";
import { useTabDrag } from "./tabDrag";
import { useTabs, type TabStatus } from "./tabs";
import { useOfferedUpdate } from "./update";
import s from "./TitleBar.module.css";

/** Area that drags the window; double-click toggles maximize (handled by Tauri). */
export function TitlebarDrag({ className, children }: { className?: string; children?: ReactNode }) {
  return (
    <div className={className} data-tauri-drag-region>
      {children}
    </div>
  );
}

async function currentWindow() {
  if (!isTauri()) return null;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  return getCurrentWindow();
}

/** Windows-style minimize / maximize / close (WIN-01). Hovering maximize shows Snap Layouts. */
export function WindowControls() {
  const t = useT();
  const platform = useApp((st) => st.info.platform);
  const [maximized, setMaximized] = useState(false);
  const [focused, setFocused] = useState(true);
  const snapTimer = useRef<number | undefined>(undefined);

  useEffect(() => {
    let disposed = false;
    const unlisten: (() => void)[] = [];
    void currentWindow().then(async (w) => {
      if (!w || disposed) return;
      setMaximized(await w.isMaximized());
      unlisten.push(await w.onResized(() => void w.isMaximized().then(setMaximized)));
      unlisten.push(await w.onFocusChanged(({ payload }) => setFocused(payload)));
    });
    return () => {
      disposed = true;
      unlisten.forEach((u) => u());
    };
  }, []);

  if (platform === "macos") return null;
  // Glyphs are drawn with CSS so they look the same with or without Segoe Fluent Icons installed.
  const glyph = (className: string) => <span className={className} />;

  return (
    <div className={cx(s.caption, !focused && s.captionInactive)}>
      <button
        type="button"
        className={s.captionButton}
        title={t("window.minimize")}
        aria-label={t("window.minimize")}
        onClick={() => void currentWindow().then((w) => w?.minimize())}
      >
        {glyph(s.glyphMin)}
      </button>
      <button
        type="button"
        className={s.captionButton}
        title={maximized ? t("window.restore") : t("window.maximize")}
        aria-label={maximized ? t("window.restore") : t("window.maximize")}
        onClick={() => void currentWindow().then((w) => w?.toggleMaximize())}
        onMouseEnter={() => {
          if (platform !== "windows") return;
          snapTimer.current = window.setTimeout(() => void api.window_snap_overlay().catch(() => {}), 650);
        }}
        onMouseLeave={() => window.clearTimeout(snapTimer.current)}
      >
        {maximized ? glyph(s.glyphRestore) : glyph(s.glyphMax)}
      </button>
      <button
        type="button"
        className={cx(s.captionButton, s.captionClose)}
        title={t("window.close")}
        aria-label={t("window.close")}
        onClick={() => void currentWindow().then((w) => w?.close())}
      >
        {glyph(s.glyphClose)}
      </button>
    </div>
  );
}

const STATUS_DOT: Record<TabStatus, string> = {
  connecting: "var(--orange)",
  connected: "var(--green)",
  failed: "var(--red)",
  disconnected: "var(--fg3)",
};

/**
 * HOST-11: the tab host's OS, with the session status as the badge. A quick connection has no host, so
 * no OS; a local tab shows a terminal.
 */
function TabOsIcon({
  hostId,
  local,
  telnet,
  status,
}: {
  hostId: string | null;
  local: boolean;
  telnet: boolean;
  status: TabStatus;
}) {
  const os = useVaultData((st) => (hostId ? (st.hosts.find((h) => h.id === hostId)?.os ?? null) : null));
  return (
    <OsIcon os={os} badge={STATUS_DOT[status]} icon={local ? "terminal-window" : telnet ? "network" : undefined} />
  );
}

const SHELL_ICON: Record<LocalShellKind, string> = {
  unix: "terminal-window",
  powershell: "terminal-window",
  cmd: "terminal",
  git_bash: "git-branch",
  wsl: "linux-logo",
};

/**
 * LOCAL-01: the menu beside New Tab, as in Windows Terminal: this computer's shells, read again each
 * time it opens so a shell installed meanwhile shows up, then New Tab's host list.
 */
function useLocalMenu(onNewTab: () => void) {
  const t = useT();
  const menu = useMenu();
  const platform = useApp((st) => st.info.platform);
  const preferred = useApp((st) => st.prefs.local_shell);
  const [shells, setShells] = useState<LocalShellView[] | null>(null);
  const open = (button: HTMLElement) => {
    menu.openBelow(button);
    setShells(null);
    void loadLocalShells()
      .then(setShells)
      .catch(() => setShells([]));
  };
  const first = shells ? preferredShell(shells, preferred) : undefined;
  const entries: MenuEntry[] = [
    { kind: "header", label: t("terminal.local.title") },
    ...(shells === null
      ? [{ label: t("terminal.local.loading"), disabled: true, onSelect: () => {} }]
      : shells.length === 0
        ? [{ label: t("terminal.local.noShell"), disabled: true, onSelect: () => {} }]
        : shells.map((shell) => ({
            label: shellName(shell, t),
            icon: SHELL_ICON[shell.kind],
            hint: shell.id === first?.id ? shortcutLabel(platform, "Ctrl+Shift+`", "⌃⇧`") : undefined,
            onSelect: () => void openLocalTerminal(shell),
          }))),
    { kind: "separator" },
    {
      label: t("tabs.hosts"),
      icon: "hard-drives",
      hint: shortcutLabel(platform, "Ctrl+Shift+T", "⌘T"),
      onSelect: onNewTab,
    },
  ];
  return { menu, open, entries };
}

/** AI-07: a tab whose conversation, or one it left in the background, is running a turn or waiting for approval. */
function AiActivity({ slotId }: { slotId: string }) {
  const t = useT();
  const activity = useAi((st) => tabActivity(st, slotId));
  if (activity === "idle") return null;
  const label = t(activity === "waiting" ? "ai.tabWaiting" : "ai.tabRunning");
  return (
    <span
      className={cx(s.tabAi, activity === "waiting" ? s.tabAiWaiting : s.tabAiRunning)}
      title={label}
      aria-label={label}
      role="img"
    >
      <Icon name={activity === "waiting" ? "hand-palm" : "sparkle"} fill={activity === "waiting"} />
    </span>
  );
}

/** The label, with a hidden copy at the selected weight that keeps the tab's width. */
function TabLabel({ text }: { text: string }) {
  return (
    <span className={s.tabLabel} data-label={text}>
      <span>{text}</span>
    </span>
  );
}

/**
 * A session's tab opens out from nothing as it is added, pushing the buttons after it along. Played
 * from script, not a CSS animation: React moves a tab's element when the tabs are reordered, and
 * that would play a CSS animation again.
 */
function useOpenNewTabs(list: RefObject<HTMLDivElement | null>, tabs: { id: string }[]) {
  const known = useRef<Set<string> | null>(null);
  useLayoutEffect(() => {
    const ids = tabs.map((tab) => tab.id);
    const before = known.current;
    known.current = new Set(ids);
    // The tabs there when the bar first shows are not new.
    if (!before || !list.current) return;
    const { duration, easing } = motion("--motion-base");
    if (!duration) return;
    for (const id of ids) {
      if (before.has(id)) continue;
      const el = list.current.querySelector<HTMLElement>(`[data-tab-id="${CSS.escape(id)}"]`);
      el?.animate?.(
        [
          { maxWidth: "0px", opacity: 0 },
          { maxWidth: "200px", opacity: 1 },
        ],
        { duration, easing },
      );
    }
  }, [list, tabs]);
}

/**
 * Lock and Settings, which act on the whole app, in the title bar every tab shares. Lock stays farthest from the
 * AI button and the window controls, as it can end every session (SEC-03).
 */
function AppButtons() {
  const t = useT();
  const platform = useApp((st) => st.info.platform);
  const updateAvailable = useOfferedUpdate() !== null;
  const lockLabel = `${t("sidebar.lock")} (${shortcutLabel(platform, "Ctrl+Shift+L", "⌘L")})`;
  const settingsLabel = `${updateAvailable ? t("sidebar.settingsUpdate") : t("sidebar.settings")} (${shortcutLabel(platform, "Ctrl+,", "⌘,")})`;
  return (
    <>
      <button
        type="button"
        className={s.barButton}
        title={lockLabel}
        aria-label={lockLabel}
        onClick={() => void useApp.getState().lock()}
      >
        <Icon name="lock-simple" size={16} />
      </button>
      <button
        type="button"
        className={s.barButton}
        title={settingsLabel}
        aria-label={settingsLabel}
        onClick={() => useApp.getState().openSettings(true, updateAvailable ? "about" : undefined)}
      >
        <Icon name="gear-six" size={16} />
        {updateAvailable && <span className={s.updateDot} />}
      </button>
    </>
  );
}

/** Tab bar merged with the title bar (design TabBar + WIN-01). */
export function TabBar({ onNewTab, onCloseTab }: { onNewTab: () => void; onCloseTab: (id: string) => void }) {
  const t = useT();
  const { tabs, active, activate } = useTabs();
  const local = useLocalMenu(onNewTab);
  const drag = useTabDrag(tabs);
  const list = useRef<HTMLDivElement>(null);
  useOpenNewTabs(list, tabs);

  // Left and Right select the tab beside, and the focus follows it there; Tab reaches only the selected tab.
  const onTabKey = (e: KeyboardEvent) => {
    if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
    e.preventDefault();
    const ids = ["home", ...tabs.map((tab) => tab.id)];
    const i = ids.indexOf(active);
    const next = ids[(i + (e.key === "ArrowRight" ? 1 : ids.length - 1)) % ids.length];
    activate(next);
    // After the tab's terminal took the focus as it showed. The tabs are in the order of `ids`.
    requestAnimationFrame(() =>
      list.current?.querySelectorAll<HTMLElement>(':scope > [role="tab"]')[ids.indexOf(next)]?.focus(),
    );
  };

  return (
    <div className={s.tabbar} data-tauri-drag-region>
      <div ref={list} className={s.tabs} role="tablist" onKeyDown={onTabKey}>
        <button
          type="button"
          role="tab"
          aria-selected={active === "home"}
          aria-label={t("tabs.home")}
          title={t("tabs.home")}
          tabIndex={active === "home" ? 0 : -1}
          className={cx(s.tab, s.homeTab, active === "home" && s.tabActive)}
          // A press leaves the focus where it is, as with browser tabs; a session tab gives it to its terminal.
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => activate("home")}
        >
          <Icon name="house" className={s.tabIcon} />
          <AiActivity slotId="home" />
        </button>
        {tabs.length > 0 && <span className={s.sep} />}
        {tabs.map((tab) => (
          <button
            key={tab.id}
            type="button"
            role="tab"
            data-tab-id={tab.id}
            aria-selected={active === tab.id}
            tabIndex={active === tab.id ? 0 : -1}
            className={cx(s.tab, active === tab.id && s.tabActive)}
            onPointerDown={(e) => drag.down(e, tab.id)}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => {
              activate(tab.id);
              // Typing goes to the terminal, even after a click on the tab already selected or Enter on it.
              getSession(tab.id)?.focus();
            }}
            onAuxClick={(e) => {
              if (e.button === 1) onCloseTab(tab.id);
            }}
            title={tab.title}
          >
            <TabOsIcon hostId={tab.hostId} local={!!tab.local} telnet={!!tab.telnet} status={tab.status} />
            <TabLabel text={tab.title} />
            <AiActivity slotId={tab.id} />
            <span
              role="button"
              tabIndex={-1}
              aria-label={t("tabs.close")}
              title={t("tabs.close")}
              className={s.tabClose}
              onClick={(e) => {
                e.stopPropagation();
                onCloseTab(tab.id);
              }}
            >
              <Icon name="x" />
            </span>
          </button>
        ))}
      </div>
      <button
        type="button"
        className={s.newTab}
        title={t("tabs.newTab")}
        aria-label={t("tabs.newTab")}
        onClick={onNewTab}
      >
        <Icon name="plus" />
      </button>
      <button
        type="button"
        className={cx(s.newTab, s.newTabMenu)}
        title={t("tabs.newMenu")}
        aria-label={t("tabs.newMenu")}
        aria-haspopup="menu"
        aria-expanded={!!local.menu.anchor}
        onClick={(e) => local.open(e.currentTarget)}
      >
        <Icon name="caret-down" size={12} />
      </button>
      {local.menu.anchor && (
        <Menu anchor={local.menu.anchor} entries={local.entries} onClose={local.menu.close} minWidth={220} />
      )}
      <div className={s.drag} data-tauri-drag-region />
      <AppButtons />
      <AiPanelButton className={s.aiButton} activeClassName={s.aiButtonOn} dotClassName={s.aiDot} />
      <WindowControls />
    </div>
  );
}

/**
 * Top strip for full-window screens without a tab bar (unlock, onboarding): a drag region with the
 * Windows caption buttons on the right. On macOS the native traffic lights sit on top of it.
 */
export function WindowChrome({ transparent = true }: { transparent?: boolean }) {
  return (
    <div
      data-tauri-drag-region
      style={{
        position: "absolute",
        top: 0,
        left: 0,
        right: 0,
        height: "var(--titlebar-height)",
        display: "flex",
        justifyContent: "flex-end",
        background: transparent ? "transparent" : "var(--win)",
        zIndex: 10,
      }}
    >
      <WindowControls />
    </div>
  );
}
