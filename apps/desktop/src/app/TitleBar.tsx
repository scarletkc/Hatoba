import { useEffect, useRef, useState, type ReactNode } from "react";
import { Icon } from "@/components/controls";
import { useT } from "@/i18n";
import { api, isTauri } from "@/ipc/api";
import { cx } from "@/lib/cx";
import { useApp } from "./store";
import { useTabs, type TabStatus } from "./tabs";
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
      <button type="button" className={s.captionButton} title={t("window.minimize")} aria-label={t("window.minimize")} onClick={() => void currentWindow().then((w) => w?.minimize())}>
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
      <button type="button" className={cx(s.captionButton, s.captionClose)} title={t("window.close")} aria-label={t("window.close")} onClick={() => void currentWindow().then((w) => w?.close())}>
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

/** Tab bar merged with the title bar (design TabBar + WIN-01). */
export function TabBar({ homeLabel, homeIcon, onNewTab, onCloseTab }: {
  homeLabel: string;
  homeIcon: string;
  onNewTab: () => void;
  onCloseTab: (id: string) => void;
}) {
  const t = useT();
  const { tabs, active, activate } = useTabs();
  const collapsed = useApp((st) => st.sidebarCollapsed);
  const platform = useApp((st) => st.info.platform);

  return (
    <div className={s.tabbar} data-tauri-drag-region>
      {collapsed && platform === "macos" && <span style={{ width: 70, flex: "none" }} data-tauri-drag-region />}
      {collapsed && (
        <button type="button" className={s.newTab} title={t("window.toggleSidebar")} aria-label={t("window.toggleSidebar")} onClick={() => useApp.getState().toggleSidebar()}>
          <Icon name="sidebar-simple" size={16} />
        </button>
      )}
      <div className={s.tabs} role="tablist">
        <button
          type="button"
          role="tab"
          aria-selected={active === "home"}
          className={cx(s.tab, active === "home" && s.tabActive)}
          onClick={() => activate("home")}
        >
          <Icon name={homeIcon} className={s.tabIcon} />
          <span className={s.tabLabel}>{homeLabel}</span>
        </button>
        {tabs.length > 0 && <span className={s.sep} />}
        {tabs.map((tab) => (
          <button
            key={tab.id}
            type="button"
            role="tab"
            aria-selected={active === tab.id}
            className={cx(s.tab, active === tab.id && s.tabActive)}
            onClick={() => activate(tab.id)}
            onAuxClick={(e) => {
              if (e.button === 1) onCloseTab(tab.id);
            }}
            title={tab.title}
          >
            <span className={s.tabDot} style={{ background: STATUS_DOT[tab.status] }} />
            <span className={s.tabLabel}>{tab.title}</span>
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
      <button type="button" className={s.newTab} title={t("tabs.newTab")} aria-label={t("tabs.newTab")} onClick={onNewTab}>
        <Icon name="plus" />
      </button>
      <div className={s.drag} data-tauri-drag-region />
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
