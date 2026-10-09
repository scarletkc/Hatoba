import { useCallback, useEffect, useRef } from "react";
import { ConfirmHost, setOverlaysLocked, ToastHost } from "@/components/overlay";
import { toggleAiPanel } from "@/features/ai/actions";
import { AiPanel } from "@/features/ai/AiPanel";
import { useAiLifecycle } from "@/features/ai/lifecycle";
import { HostEditPage } from "@/features/hosts/HostEditPage";
import { HostsPage } from "@/features/hosts/HostsPage";
import { KeysPage } from "@/features/keys/KeysPage";
import { Onboarding } from "@/features/onboarding/Onboarding";
import { SettingsWindow } from "@/features/settings/SettingsWindow";
import { SyncPage } from "@/features/sync/SyncPage";
import { SessionDialogs } from "@/features/terminal/SessionDialogs";
import { TerminalView } from "@/features/terminal/TerminalView";
import { closeSessionTab } from "@/features/terminal/connect";
import { UnlockScreen } from "@/features/unlock/UnlockScreen";
import { useT, type MessageKey } from "@/i18n";
import { api } from "@/ipc/api";
import { ContextMenuHost } from "./ContextMenu";
import { useVaultData } from "./data";
import { useShortcuts, type ShortcutAction } from "./shortcuts";
import { Sidebar } from "./Sidebar";
import { useApp, type HostFilter, type Page } from "./store";
import { useTabs } from "./tabs";
import { TabBar } from "./TitleBar";
import { useStartupUpdateCheck } from "./update";
import s from "./App.module.css";

// Synchronous with the phase change, so no dialog shows above the lock screen for a frame.
useApp.subscribe((st) => setOverlaysLocked(st.phase === "locked"));

export function App() {
  const phase = useApp((st) => st.phase);
  const settingsOpen = useApp((st) => st.settingsOpen);
  const hasSessions = useTabs((st) => st.tabs.length > 0);
  const platform = useApp((st) => st.info.platform);

  useGlobalEvents();
  useAiLifecycle();
  useActivityPing(phase === "unlocked");
  useStartupUpdateCheck(phase === "unlocked");

  if (phase === "boot") return null;
  if (phase === "onboarding")
    return (
      <>
        <Onboarding />
        <ConfirmHost />
        <ToastHost />
        <ContextMenuHost platform={platform} />
      </>
    );

  const locked = phase === "locked";
  return (
    <>
      {/* SEC-03: sessions stay mounted (and connected) behind the lock screen, but are hidden. */}
      {(!locked || hasSessions) && (
        <div className={s.main} aria-hidden={locked} style={locked ? { visibility: "hidden" } : undefined}>
          <Main />
        </div>
      )}
      {locked && <UnlockScreen />}
      {!locked && settingsOpen && <SettingsWindow onClose={() => useApp.getState().openSettings(false)} />}
      {!locked && <SessionDialogs />}
      <ConfirmHost />
      <ToastHost />
      <ContextMenuHost platform={platform} />
    </>
  );
}

function Main() {
  const t = useT();
  const page = useApp((st) => st.page);
  const collapsed = useApp((st) => st.sidebarCollapsed);
  const platform = useApp((st) => st.info.platform);
  const aiOpen = useApp((st) => st.prefs.ai_panel_open);
  const { tabs, active } = useTabs();
  const groups = useVaultData((st) => st.groups);
  // Only the terminals stay mounted behind the lock screen; the home page and its dialogs go.
  const locked = useApp((st) => st.phase === "locked");

  const home = homeTab(page, (k) => t(k), (id) => groups.find((g) => g.id === id)?.name);

  const onShortcut = useCallback((action: ShortcutAction) => {
    const app = useApp.getState();
    const tabsState = useTabs.getState();
    switch (action) {
      case "search":
      case "newTab":
        if (app.page.kind !== "hosts") app.navigate({ kind: "hosts", filter: { kind: "all" } });
        tabsState.activate("home");
        app.focusSearch();
        break;
      case "closeTab":
        if (tabsState.active !== "home") void closeSessionTab(tabsState.active);
        break;
      case "nextTab":
        tabsState.cycle(1);
        break;
      case "prevTab":
        tabsState.cycle(-1);
        break;
      case "settings":
        app.openSettings(true);
        break;
      case "lock":
        void app.lock();
        break;
      case "aiPanel":
        toggleAiPanel();
        break;
    }
  }, []);
  useShortcuts(platform, onShortcut);

  return (
    <div className={s.window}>
      {!collapsed && <Sidebar />}
      <div className={s.content}>
        <TabBar
          homeLabel={home.label}
          homeIcon={home.icon}
          onNewTab={() => onShortcut("newTab")}
          onCloseTab={(id) => void closeSessionTab(id)}
        />
        <div className={s.workspace}>
          <div className={s.body}>
            {active === "home" && !locked && <HomePage page={page} />}
            {tabs.map((tab) => (
              <TerminalView key={tab.id} tab={tab} active={active === tab.id} />
            ))}
          </div>
          {/* §9: the AI panel shows the active tab's conversation (AI-07). */}
          {aiOpen && <AiPanel slotId={active} />}
        </div>
      </div>
    </div>
  );
}

function HomePage({ page }: { page: Page }) {
  switch (page.kind) {
    case "hosts":
      return <HostsPage filter={page.filter} />;
    case "host-edit":
      return (
        <HostEditPage
          key={page.hostId ?? `new:${page.prefill ? JSON.stringify(page.prefill) : ""}`}
          hostId={page.hostId}
          groupId={page.groupId}
          back={page.back}
          prefill={page.prefill}
        />
      );
    case "keys":
      return <KeysPage />;
    case "sync":
      return <SyncPage />;
  }
}

function filterLabel(f: HostFilter, t: (k: MessageKey) => string, groupName: (id: string) => string | undefined) {
  switch (f.kind) {
    case "all":
      return { label: t("sidebar.all"), icon: "hard-drives" };
    case "favorites":
      return { label: t("sidebar.favorites"), icon: "star" };
    case "recent":
      return { label: t("sidebar.recent"), icon: "clock-counter-clockwise" };
    case "group":
      return { label: groupName(f.id) ?? t("sidebar.groups"), icon: "folder-simple" };
    case "tag":
      return { label: f.name, icon: "tag" };
  }
}

function homeTab(page: Page, t: (k: MessageKey) => string, groupName: (id: string) => string | undefined) {
  switch (page.kind) {
    case "hosts":
      return filterLabel(page.filter, t, groupName);
    case "host-edit":
      return { label: t(page.hostId ? "hosts.editHost" : "hosts.newHost"), icon: "pencil-simple" };
    case "keys":
      return { label: t("sidebar.keys"), icon: "key" };
    case "sync":
      return { label: t("sidebar.sync"), icon: "cloud" };
  }
}

/** Backend → UI events (§10.2) that affect the whole app. */
function useGlobalEvents() {
  useEffect(() => {
    const subs = [
      api.listen("vault://locked", () => {
        useApp.getState().setPhase("locked");
        useApp.getState().openSettings(false);
        void useApp.getState().refreshVault();
      }),
      api.listen("sync://status", (status) => {
        useApp.getState().setSync(status);
        // Pulls may have changed hosts/keys; refresh what the UI shows.
        if (status.state === "idle" && useApp.getState().phase === "unlocked") void useVaultData.getState().reload();
      }),
    ];
    return () => subs.forEach((p) => void p.then((un) => un()));
  }, []);
}

/** Feeds the backend's idle auto-lock timer (SEC-02) with user activity, at most every 20 s. */
function useActivityPing(enabled: boolean) {
  const last = useRef(0);
  useEffect(() => {
    if (!enabled) return;
    const ping = () => {
      const now = Date.now();
      if (now - last.current < 20_000) return;
      last.current = now;
      void api.activity_ping().catch(() => {});
    };
    window.addEventListener("keydown", ping, true);
    window.addEventListener("mousedown", ping, true);
    window.addEventListener("wheel", ping, { capture: true, passive: true });
    return () => {
      window.removeEventListener("keydown", ping, true);
      window.removeEventListener("mousedown", ping, true);
      window.removeEventListener("wheel", ping, true);
    };
  }, [enabled]);
}
