import { create } from "zustand";
import { api } from "@/ipc/api";
import type { AppInfo, LocalPrefs, QuickTarget, SyncStatus, VaultStatus } from "@/ipc/types";
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
      /** A new host starts from this quick-connect target (HOST-12). */
      prefill?: QuickTarget;
    }
  | { kind: "keys" }
  | { kind: "sync" };

export type Phase = "boot" | "onboarding" | "locked" | "unlocked";

export type SettingsTab = "general" | "appearance" | "terminal" | "security" | "ai" | "about";

export const DEFAULT_PREFS: LocalPrefs = {
  language: "system",
  appearance: "system",
  density: "regular",
  host_probe: true,
  auto_update_check: false,
  ai_permission_mode: "manual",
  ai_bypass_confirmed: false,
  ai_tool_call_limit: 25,
  ai_panel_open: false,
  ai_panel_width: 380,
};

interface AppState {
  phase: Phase;
  info: AppInfo;
  prefs: LocalPrefs;
  vault: VaultStatus | null;
  sync: SyncStatus | null;
  page: Page;
  sidebarCollapsed: boolean;
  settingsOpen: boolean;
  /** The tab the settings window opens on; null reopens the last one. */
  settingsTab: SettingsTab | null;
  /** Bumped to ask the hosts page to focus its search field (Ctrl+Shift+K). */
  searchFocusTick: number;

  navigate(page: Page): void;
  setPrefs(prefs: LocalPrefs): Promise<void>;
  refreshVault(): Promise<VaultStatus>;
  setSync(status: SyncStatus): void;
  setPhase(phase: Phase): void;
  toggleSidebar(): void;
  openSettings(open: boolean, tab?: SettingsTab): void;
  focusSearch(): void;
  lock(): Promise<void>;
}

export const useApp = create<AppState>((set, get) => ({
  phase: "boot",
  info: { version: "0.0.0", platform: "web", mica: false },
  prefs: DEFAULT_PREFS,
  vault: null,
  sync: null,
  page: { kind: "hosts", filter: { kind: "all" } },
  sidebarCollapsed: false,
  settingsOpen: false,
  settingsTab: null,
  searchFocusTick: 0,

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
  focusSearch: () => set((s) => ({ searchFocusTick: s.searchFocusTick + 1 })),
  lock: async () => {
    await api.vault_lock();
    set({ phase: "locked", settingsOpen: false });
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
