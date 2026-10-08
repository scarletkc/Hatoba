import "@xterm/xterm/css/xterm.css";
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type MouseEvent } from "react";
import { useVaultData } from "@/app/data";
import { useApp } from "@/app/store";
import type { SessionTab } from "@/app/tabs";
import { Menu, useMenu, type MenuEntry } from "@/components/overlay";
import { askAi as askAiAbout } from "@/features/ai/actions";
import { SftpPanel } from "@/features/sftp/SftpPanel";
import { useT } from "@/i18n";
import { shortcutLabel } from "@/lib/platform";
import { cx } from "@/lib/cx";
import { editSessionHost, reconnectSession } from "./connect";
import { countRunning, useForwardRuns } from "./forwards";
import { ForwardsPopover } from "./ForwardsPopover";
import { FindBar } from "./FindBar";
import { patchInfo, useSessionInfo } from "./info";
import { ConnectingOverlay, DisconnectedBanner, ErrorCard } from "./Overlays";
import { ensureSession, type LiveSession } from "./session";
import { StatusBar } from "./StatusBar";
import { useTermSettings, useTerminalChrome } from "./theme";
import s from "./TerminalView.module.css";

/** One terminal session (design §03 / §03b). Inactive tabs stay mounted so their sessions keep running. */
export function TerminalView({ tab, active }: { tab: SessionTab; active: boolean }) {
  const session = ensureSession(tab.id, tab.hostId);
  if (!session) return null;
  return <TerminalBody tab={tab} active={active} session={session} />;
}

function TerminalBody({ tab, active, session }: { tab: SessionTab; active: boolean; session: LiveSession }) {
  const t = useT();
  const platform = useApp((st) => st.info.platform);
  const info = useSessionInfo(tab.id);
  const host = useVaultData((st) => st.hosts.find((h) => h.id === tab.hostId));
  const jumpName = useVaultData((st) => st.hosts.find((h) => h.id === host?.jump_host_id)?.name ?? null);
  const chrome = useTerminalChrome();

  const containerRef = useRef<HTMLDivElement>(null);
  const [findOpen, setFindOpen] = useState(false);
  const menu = useMenu();
  const [menuEntries, setMenuEntries] = useState<MenuEntry[]>([]);
  const forwardsMenu = useMenu();
  const forwardRuns = useForwardRuns(tab.sessionId);

  // The xterm instance lives in the session registry; React only gives it a place in the DOM.
  useLayoutEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    session.attach(el);
    return () => session.detach(el);
  }, [session]);

  useEffect(() => {
    session.setActive(active);
  }, [session, active]);

  // Ctrl+Shift+F inside the terminal asks for the find bar.
  const seenFindTick = useRef(info.findTick);
  useEffect(() => {
    if (info.findTick !== seenFindTick.current) {
      seenFindTick.current = info.findTick;
      setFindOpen(true);
    }
  }, [info.findTick]);

  const closeFind = useCallback(() => {
    setFindOpen(false);
    session.focus();
  }, [session]);

  const connected = tab.status === "connected";
  const closeForwards = forwardsMenu.close;
  useEffect(() => {
    if (!active || !connected) closeForwards(); // a popover must not outlive its tab being visible / connected
  }, [active, connected, closeForwards]);
  const target = host ? `${host.username}@${host.address}:${host.port}` : null;

  const buildMenu = (context: boolean): MenuEntry[] => {
    const hasSelection = session.term.hasSelection();
    const copy: MenuEntry = {
      label: t("terminal.menu.copy"),
      icon: "copy",
      disabled: !hasSelection,
      onSelect: () => void session.copySelection(true).then(() => session.focus()),
    };
    const paste: MenuEntry = {
      label: t("terminal.menu.paste"),
      icon: "clipboard-text",
      disabled: !connected,
      onSelect: () => void session.pasteFromClipboard(),
    };
    const selectAll: MenuEntry = {
      label: t("terminal.menu.selectAll"),
      icon: "selection-all",
      onSelect: () => {
        session.term.selectAll();
        session.focus();
      },
    };
    const clear: MenuEntry = {
      label: t("terminal.menu.clear"),
      icon: "eraser",
      onSelect: () => {
        session.term.clear();
        session.focus();
      },
    };
    const find: MenuEntry = {
      label: t("terminal.menu.find"),
      icon: "magnifying-glass",
      onSelect: () => setFindOpen(true),
    };
    // AI-10: opens the AI panel on this tab, where the selection shows as a chip that goes with the next message.
    const selection = hasSelection ? session.term.getSelection() : "";
    const askAi: MenuEntry = {
      label: t("terminal.menu.askAi"),
      icon: "sparkle",
      disabled: !selection.trim(),
      onSelect: () => askAiAbout(tab.id, selection),
    };
    if (context) return [copy, paste, selectAll, { kind: "separator" }, askAi, { kind: "separator" }, clear, find];
    return [
      { label: t("terminal.menu.reconnect"), icon: "arrow-clockwise", disabled: tab.status === "connecting", onSelect: () => void reconnectSession(tab.id) },
      { label: t("terminal.menu.disconnect"), icon: "plugs", disabled: !connected, onSelect: () => void session.disconnect() },
      { kind: "separator" },
      copy,
      paste,
      clear,
      selectAll,
      { kind: "separator" },
      find,
      // Also here: with right click set to copy/paste there is no context menu.
      askAi,
      { kind: "separator" },
      { label: t("terminal.menu.editHost"), icon: "pencil-simple", disabled: !host, onSelect: () => editSessionHost(tab.hostId) },
    ];
  };

  // WIN-05: right click is either PuTTY-style copy/paste or a context menu.
  const onContextMenu = (e: MouseEvent) => {
    e.preventDefault();
    if (session.remoteTracksMouse && !e.shiftKey) return;
    if (useTermSettings.getState().settings.right_click === "copy_paste") {
      if (session.term.hasSelection()) void session.copySelection(true);
      else void session.pasteFromClipboard();
      return;
    }
    setMenuEntries(buildMenu(true));
    menu.openAt({ x: e.clientX, y: e.clientY });
  };

  return (
    <div className={cx(s.root, !active && s.hidden)} inert={!active}>
      <StatusBar
        status={tab.status}
        target={target}
        via={jumpName}
        latencyMs={info.latencyMs}
        findOpen={findOpen}
        sftpOpen={info.sftpOpen}
        forwardCount={countRunning(forwardRuns)}
        forwardsOpen={!!forwardsMenu.anchor}
        findHint={shortcutLabel(platform, "Ctrl+Shift+F", "⌘F")}
        onFind={() => (findOpen ? closeFind() : setFindOpen(true))}
        onForwards={(button) => (forwardsMenu.anchor ? forwardsMenu.close() : forwardsMenu.openBelow(button, true))}
        onToggleSftp={() => {
          patchInfo(tab.id, { sftpOpen: !info.sftpOpen });
          session.focus(); // keep typing in the terminal after toggling the panel
        }}
        onMore={(button) => {
          setMenuEntries(buildMenu(false));
          menu.openBelow(button, true);
        }}
      />
      <div className={s.main}>
        <div className={s.termArea} style={{ background: chrome.background }}>
          <div ref={containerRef} className={s.term} onContextMenu={onContextMenu} />
          {findOpen && <FindBar session={session} onClose={closeFind} />}
          {tab.status === "connecting" && <ConnectingOverlay name={host?.name ?? tab.title} />}
          {tab.status === "failed" && (
            <ErrorCard
              name={host?.name ?? tab.title}
              host={host}
              error={info.error}
              at={info.errorAt}
              attempts={info.attempts}
              onRetry={() => void reconnectSession(tab.id)}
              onEdit={() => editSessionHost(tab.hostId)}
            />
          )}
          {tab.status === "disconnected" && (
            <DisconnectedBanner reason={info.reason} onReconnect={() => void reconnectSession(tab.id)} />
          )}
        </div>
        {info.sftpOpen && connected && tab.sessionId && (
          <SftpPanel key={tab.sessionId} sessionId={tab.sessionId} hostName={host?.name ?? tab.title} active={active} />
        )}
      </div>
      {forwardsMenu.anchor && tab.sessionId && connected && (
        <ForwardsPopover
          anchor={forwardsMenu.anchor}
          sessionId={tab.sessionId}
          hostId={tab.hostId}
          onClose={(restoreFocus) => {
            forwardsMenu.close();
            if (restoreFocus) session.focus();
          }}
          onManage={() => editSessionHost(tab.hostId)}
        />
      )}
      {menu.anchor && <Menu anchor={menu.anchor} entries={menuEntries} onClose={menu.close} minWidth={168} />}
    </div>
  );
}
