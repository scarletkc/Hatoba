import { useEffect } from "react";
import { useApp } from "@/app/store";
import { useTabs } from "@/app/tabs";
import { api } from "@/ipc/api";
import { detachSlot, loadCatalog, refreshAfterSync, reloadAll, reloadSlot, resetForLock } from "./actions";
import { applyMcpStatus, loadMcp, resetMcp, useMcp } from "./mcp";
import { useAi } from "./store";

/**
 * Keeps the assistant in step with the rest of the app. Mounted once at the top of the app, so it
 * also runs while the panel is closed and behind the lock screen.
 */
export function useAiLifecycle() {
  const phase = useApp((st) => st.phase);

  // Unlock: providers and the open conversations (their text is dropped while locked).
  useEffect(() => {
    if (phase === "unlocked") {
      void loadCatalog();
      reloadAll();
      void loadMcp();
    } else if (phase === "locked") {
      resetForLock();
      resetMcp();
    }
  }, [phase]);

  useEffect(() => {
    const subs = [
      // §13.1: locking stops every turn in Rust; reset what was in flight here.
      api.listen("vault://locked", () => {
        resetForLock();
        resetMcp();
      }),
      // §13.7: a pull may bring entries, titles and providers from other devices.
      api.listen("sync://status", (status) => {
        if (status.state !== "idle" || useApp.getState().phase !== "unlocked") return;
        refreshAfterSync();
        if (useMcp.getState().servers) void loadMcp();
      }),
      // AI-32: servers start, stop and fail on their own; the tools menu shows it live.
      api.listen("ai://mcp-status", (status) => {
        if (useApp.getState().phase === "unlocked") applyMcpStatus(status);
      }),
    ];
    return () => subs.forEach((p) => void p.then((un) => un()));
  }, []);

  // AI-08: closing a tab detaches its conversation (it stays in history).
  useEffect(
    () =>
      useTabs.subscribe((st, prev) => {
        if (st.tabs === prev.tabs) return;
        const open = new Set(st.tabs.map((x) => x.id));
        const { slots, selections } = useAi.getState();
        for (const tab of prev.tabs) if (!open.has(tab.id) && (slots[tab.id] || selections[tab.id])) detachSlot(tab.id);
      }),
    [],
  );

  // Settings → AI may have changed providers, the default model, or MCP servers.
  useEffect(
    () =>
      useApp.subscribe((st, prev) => {
        if (!prev.settingsOpen || st.settingsOpen || st.phase !== "unlocked") return;
        void loadCatalog();
        void loadMcp();
      }),
    [],
  );

  // A turn an earlier page started runs on in Rust without events here: poll until it ends.
  const remote = useAi((st) => Object.values(st.slots).some((s) => s.remoteRunning));
  useEffect(() => {
    if (!remote) return;
    const timer = window.setInterval(() => {
      for (const [id, s] of Object.entries(useAi.getState().slots)) if (s.remoteRunning) void reloadSlot(id);
    }, 2000);
    return () => window.clearInterval(timer);
  }, [remote]);
}
