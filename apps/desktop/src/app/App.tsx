import { useCallback, useEffect, useLayoutEffect, useRef } from "react";
import { ConfirmHost, ToastHost, setOverlaysLocked } from "@hatoba/ui";
import { toggleAiPanel } from "@/features/ai/actions";
import { AiPanel } from "@/features/ai/AiPanel";
import { EditFilesHost } from "@/features/ai/calls/EditFiles";
import { useAiLifecycle } from "@/features/ai/lifecycle";
import { clearReadouts } from "@/features/dashboard/readings";
import { ForwardsPage } from "@/features/hosts/ForwardsPage";
import { HostEditPage } from "@/features/hosts/HostEditPage";
import { TelnetEditPage } from "@/features/hosts/TelnetEditPage";
import { HostsPage } from "@/features/hosts/HostsPage";
import { useHostsUi } from "@/features/hosts/ui";
import { KeysPage } from "@/features/keys/KeysPage";
import { Onboarding } from "@/features/onboarding/Onboarding";
import { SettingsWindow } from "@/features/settings/SettingsWindow";
import { SyncPage } from "@/features/sync/SyncPage";
import { EditDialogs } from "@/features/sftp/EditDialogs";
import { SessionDialogs } from "@/features/terminal/SessionDialogs";
import { TerminalView } from "@/features/terminal/TerminalView";
import { UnlockScreen } from "@/features/unlock/UnlockScreen";
import { api } from "@/ipc/api";
import type { SyncState } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { applyZoom } from "@/lib/zoom";
import { closeSessionTab, openLocalTerminal } from "@/session/connect";
import { startForwardListeners } from "@/session/forwards";
import { ClosePrompt, useTrayMenu } from "./ClosePrompt";
import { ContextMenuHost } from "./ContextMenu";
import { ResizeHandles } from "./ResizeHandles";
import { useVaultData } from "./data";
import { useLaunches } from "./launch";
import { useShortcuts, type ShortcutAction } from "./shortcuts";
import { useZoomKeys } from "./zoomKeys";
import { ZoomHud } from "./ZoomHud";
import { Sidebar } from "./Sidebar";
import { StarPrompt } from "./StarPrompt";
import { TrafficLights } from "./TrafficLights";
import { useApp, type Page } from "./store";
import { useTabs } from "./tabs";
import { TabBar } from "./TitleBar";
import { useAutoUpdateCheck } from "./update";
import s from "./App.module.css";

// Synchronous with the phase change, so no dialog shows above the lock screen for a frame.
useApp.subscribe((st) => setOverlaysLocked(st.phase === "locked"));

export function App() {
  const phase = useApp((st) => st.phase);
  const settingsOpen = useApp((st) => st.settingsOpen);
  const hasSessions = useTabs((st) => st.tabs.length > 0);
  const platform = useApp((st) => st.info.platform);

  useGlobalEvents();
  useWindowChrome();
  useZoomKeys(platform);
  useAiLifecycle();
  useActivityPing(phase === "unlocked");
  useAutoUpdateCheck(phase === "unlocked");
  useLaunches(phase === "unlocked");
  useTrayMenu();
  // FWD-05: wired once unlocked, and kept, so a connection lost behind the lock screen still notifies.
  useEffect(() => {
    if (phase === "unlocked") startForwardListeners();
  }, [phase]);

  if (phase === "boot") return null;
  if (phase === "onboarding")
    return (
      <>
        <Onboarding />
        <ConfirmHost />
        <ClosePrompt />
        <ToastHost />
        <ZoomHud />
        <ContextMenuHost platform={platform} />
        <ResizeHandles platform={platform} />
        <TrafficLights platform={platform} />
      </>
    );

  const locked = phase === "locked";
  return (
    <>
      {/* SEC-03: sessions stay mounted (and connected) behind the lock screen, but are hidden. */}
      {(!locked || hasSessions) && (
        <div
          className={s.main}
          aria-hidden={locked}
          data-locked={locked || undefined}
          style={locked ? { visibility: "hidden" } : undefined}
        >
          <Main />
        </div>
      )}
      {locked && <UnlockScreen />}
      {!locked && settingsOpen && <SettingsWindow onClose={() => useApp.getState().openSettings(false)} />}
      {!locked && <SessionDialogs />}
      <EditDialogs locked={locked} />
      <EditFilesHost />
      <ConfirmHost />
      <ClosePrompt />
      <ToastHost />
      <ZoomHud />
      <ContextMenuHost platform={platform} />
      <ResizeHandles platform={platform} />
      <TrafficLights platform={platform} />
    </>
  );
}

function Main() {
  const page = useApp((st) => st.page);
  const platform = useApp((st) => st.info.platform);
  const aiOpen = useApp((st) => st.prefs.ai_panel_open);
  const { tabs, active } = useTabs();
  // Only the terminals stay mounted behind the lock screen; the home page and its dialogs go.
  const locked = useApp((st) => st.phase === "locked");
  // The navigation rail, on the home tab only (§9).
  const rail = active === "home" && !locked;

  const onShortcut = useCallback((action: ShortcutAction) => {
    const app = useApp.getState();
    // SEC-03: behind the lock screen the tabs stay as they are.
    if (app.phase === "locked") return;
    const tabsState = useTabs.getState();
    switch (action) {
      case "search":
      case "newTab":
        if (app.page.kind !== "hosts") {
          app.navigate({ kind: "hosts", group: null });
          useHostsUi.getState().clearFilters();
        }
        tabsState.activate("home");
        app.focusSearch();
        break;
      case "newLocal":
        void openLocalTerminal();
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
      <TabBar onNewTab={() => onShortcut("newTab")} onCloseTab={(id) => void closeSessionTab(id)} />
      <div className={s.workspace}>
        {/* The rail navigates the home tab, so a session's tab has the window's whole width. */}
        {rail && <Sidebar />}
        <div className={cx(s.content, rail && s.contentBeside)}>
          <div className={s.body}>
            {rail && (
              // A new page fades in, keyed so that going from one to another plays it again.
              <div key={page.kind} className={s.page}>
                <HomePage page={page} />
              </div>
            )}
            {tabs.map((tab) => (
              <TerminalView key={tab.id} tab={tab} active={active === tab.id} />
            ))}
            {rail && <StarPrompt />}
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
      return <HostsPage group={page.group} />;
    case "host-edit":
      return (
        <HostEditPage
          key={page.hostId ?? `new:${page.prefill ? JSON.stringify(page.prefill) : ""}${page.draft ? ":draft" : ""}`}
          hostId={page.hostId}
          groupId={page.groupId}
          back={page.back}
          prefill={page.prefill}
          draft={page.draft}
        />
      );
    case "telnet-edit":
      return (
        <TelnetEditPage
          key={page.hostId ?? `new-telnet${page.draft ? ":draft" : ""}`}
          hostId={page.hostId}
          groupId={page.groupId}
          back={page.back}
          draft={page.draft}
        />
      );
    case "keys":
      return <KeysPage />;
    case "sync":
      return <SyncPage />;
    case "forwards":
      return <ForwardsPage hostId={page.hostId} />;
  }
}

/** Backend → UI events (§10.2) that affect the whole app. */
function useGlobalEvents() {
  useEffect(() => {
    // The state of the last sync status, to tell when a round ends.
    let lastSync: SyncState | null = null;
    const subs = [
      api.listen("vault://locked", ({ reason }) => {
        // HOST-14: the readings are not kept behind the lock screen.
        clearReadouts();
        useApp.setState({ lockReason: reason });
        useApp.getState().setPhase("locked");
        useApp.getState().openSettings(false);
        void useApp.getState().refreshVault();
      }),
      api.listen("sync://status", (status) => {
        useApp.getState().setSync(status);
        // Pulls may have changed hosts, keys and settings; refresh what the UI shows. A round that
        // fails after pulling has changed them too.
        const roundEnded = lastSync === "syncing" && status.state !== "syncing";
        lastSync = status.state;
        if ((status.state === "idle" || roundEnded) && useApp.getState().phase === "unlocked") {
          void useVaultData.getState().reload();
          useApp.getState().refreshSynced();
        }
      }),
      api.listen("window://zoom", (zoom) => {
        useApp.setState({ zoom });
        applyZoom(zoom.zoom);
      }),
      api
        .listen("window://chrome", ({ floating }) => useApp.setState({ floating }))
        // A chrome change between the boot-time read and this listener would be lost, so
        // re-read once the listener is registered. Errors are ignored: the next event wins.
        .then((un) => {
          void api
            .window_floating()
            .then((floating) => useApp.setState({ floating }))
            .catch(() => {});
          return un;
        }),
    ];
    return () => subs.forEach((p) => void p.then((un) => un()));
  }, []);
}

/**
 * Mirrors the store's floating state to :root[data-floating], which gates the painted window
 * chrome's CSS (Linux). A layout effect puts the attribute on before the first painted frame.
 */
function useWindowChrome() {
  const floating = useApp((st) => st.floating);
  useLayoutEffect(() => {
    const root = document.documentElement;
    if (floating) root.dataset.floating = "";
    else delete root.dataset.floating;
  }, [floating]);
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
