import { create } from "zustand";
import { api } from "@/ipc/api";
import type { AppInfo, LocalPrefs, QuickTarget, SyncStatus, VaultLockedEvent, VaultStatus } from "@/ipc/types";
import { detectLocale, setLocale, type Locale } from "@/i18n";

export type HostFilter =
  | { kind: "all" }
  | { kind: "favorites" }
  | { kind: "recent" }
  | { kind: "group"; id: string }
  | { kind: "tag"; name: string };

/** What the "home" tab shows. Session tabs are tracked in `tabs.ts`. */
export type Page =
  | { kind: "hosts"; filter: HostFilter }
  | {
      kind: "host-edit";
      hostId: string | null;
      groupId: string | null;
      back: HostFilter;
      /** The editor was opened from the dashboard, which Back and Save return to with the `back` filter (HOST-14). */
      fromDashboard?: boolean;
      /** A new host starts from this quick-connect target (HOST-12). */
      prefill?: QuickTarget;
    }
  | { kind: "keys" }
  | { kind: "sync" }
  /** The status and resource usage of the hosts a filter shows (HOST-14). */
  | { kind: "dashboard"; filter: HostFilter };

export type Phase = "boot" | "onboarding" | "locked" | "unlocked";

export type SettingsTab = "general" | "appearance" | "terminal" | "proxies" | "security" | "ai" | "about";

export const DEFAULT_PREFS: LocalPrefs = {
  language: "system",
  appearance: "system",
  density: "regular",
  host_probe: true,
  auto_update_check: true,
  ai_permission_mode: "manual",
  ai_bypass_confirmed: false,
  ai_notifications: true,
  ai_panel_open: false,
  ai_panel_width: 380,
  default_proxy_id: null,
  auto_lock_minutes: 15,
  lock_on_sleep: true,
  lock_disconnects_sessions: false,
  clipboard_clear_secs: 90,
  dashboard_layout: "cards",
  dashboard_hidden_columns: [],
  dashboard_interval_secs: 3,
  dashboard_cpu_alert: 90,
  dashboard_mem_alert: 90,
  dashboard_disk_alert: 90,
  close_action: "ask",
};

interface AppState {
  phase: Phase;
  info: AppInfo;
  /** Whether the Linux window shows its painted chrome right now (`:root[data-floating]`). */
  floating: boolean;
  prefs: LocalPrefs;
  vault: VaultStatus | null;
  /** Why the vault last locked while the app ran; null when it started locked. */
  lockReason: VaultLockedEvent["reason"] | null;
  sync: SyncStatus | null;
  page: Page;
  sidebarCollapsed: boolean;
  settingsOpen: boolean;
  /** The tab the settings window opens on or switches to; null keeps the last one. The window clears it once taken. */
  settingsTab: SettingsTab | null;
  /** Bumped to ask the hosts page to focus its search field (Ctrl+Shift+K). */
  searchFocusTick: number;
  /** The text the search field takes with that focus, or null to keep its own (HOST-13). */
  searchFocusText: string | null;
  /** Bumped after each sync status that may follow a pull, so open views of synced data re-read it. */
  syncTick: number;

  navigate(page: Page): void;
  setPrefs(prefs: LocalPrefs): Promise<void>;
  refreshVault(): Promise<VaultStatus>;
  setSync(status: SyncStatus): void;
  setPhase(phase: Phase): void;
  toggleSidebar(): void;
  openSettings(open: boolean, tab?: SettingsTab): void;
  focusSearch(text?: string): void;
  refreshSynced(): void;
  lock(): Promise<void>;
}

export const useApp = create<AppState>((set, get) => ({
  phase: "boot",
  info: { version: "0.0.0", platform: "web", mica: false, local_user: "" },
  floating: false,
  prefs: DEFAULT_PREFS,
  vault: null,
  lockReason: null,
  sync: null,
  page: { kind: "hosts", filter: { kind: "all" } },
  sidebarCollapsed: false,
  settingsOpen: false,
  settingsTab: null,
  searchFocusTick: 0,
  searchFocusText: null,
  syncTick: 0,

  navigate: (page) => set({ page }),
  setPrefs: async (prefs) => {
    set({ prefs });
    applyPrefs(prefs);
    await api.prefs_save(prefs);
  },
  refreshVault: async () => {
    const vault = await api.vault_status();
    set({ vault });
    return vault;
  },
  setSync: (sync) => set({ sync }),
  setPhase: (phase) => set({ phase }),
  toggleSidebar: () => set({ sidebarCollapsed: !get().sidebarCollapsed }),
  openSettings: (settingsOpen, tab) => set({ settingsOpen, settingsTab: tab ?? null }),
  focusSearch: (text) => set((s) => ({ searchFocusTick: s.searchFocusTick + 1, searchFocusText: text ?? null })),
  refreshSynced: () => set((s) => ({ syncTick: s.syncTick + 1 })),
  lock: async () => {
    await api.vault_lock();
    set({ phase: "locked", lockReason: "manual", settingsOpen: false });
  },
}));

export function resolveLocale(prefs: LocalPrefs): Locale {
  return prefs.language === "system" ? detectLocale() : prefs.language;
}

const darkQuery = typeof window !== "undefined" ? window.matchMedia("(prefers-color-scheme: dark)") : null;

/** Apply theme (WIN-08: follows the system live), density and language to the document. */
export function applyPrefs(prefs: LocalPrefs) {
  const root = document.documentElement;
  const dark = prefs.appearance === "dark" || (prefs.appearance === "system" && !!darkQuery?.matches);
  root.dataset.theme = dark ? "dark" : "light";
  root.dataset.density = prefs.density;
  setLocale(resolveLocale(prefs));
}

darkQuery?.addEventListener("change", () => applyPrefs(useApp.getState().prefs));

export function isDarkTheme(): boolean {
  return document.documentElement.dataset.theme === "dark";
}
