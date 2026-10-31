import { create } from "zustand";
import { api } from "@/ipc/api";
import type {
  AppInfo,
  LocalPrefs,
  QuickTarget,
  SyncStatus,
  VaultLockedEvent,
  VaultStatus,
  ZoomView,
} from "@/ipc/types";
import { detectLocale, setLocale, type Locale } from "@/i18n";
import type { HostDraft } from "@/features/hosts/ProtocolRow";
import { applyZoom } from "@/lib/zoom";

/** What the "home" tab shows. Session tabs are tracked in `tabs.ts`. */
export type Page =
  /** Every host, or the hosts of `group`; favorites and tags narrow it in `useHostsUi`. */
  | { kind: "hosts"; group: string | null }
  | {
      kind: "host-edit";
      hostId: string | null;
      groupId: string | null;
      /** The group whose hosts Back and Save return to, or null for every host. */
      back: string | null;
      /** A new host starts from this quick-connect target (HOST-12). */
      prefill?: QuickTarget;
      /** A new host starts from what the Telnet editor's form had when its protocol was switched. */
      draft?: HostDraft;
    }
  /** A Telnet host's editor (TELNET-01); a new one may start from the SSH editor's form. */
  | {
      kind: "telnet-edit";
      hostId: string | null;
      groupId: string | null;
      /** The group whose hosts Back and Save return to, or null for every host. */
      back: string | null;
      draft?: HostDraft;
    }
  | { kind: "keys" }
  | { kind: "sync" }
  /** Every host's port forwards (FWD-05), with the section of `hostId` in view. */
  | { kind: "forwards"; hostId: string | null };

export type Phase = "boot" | "onboarding" | "locked" | "unlocked";

export type SettingsTab = "general" | "appearance" | "terminal" | "proxies" | "security" | "ai" | "about";
/** A section of a Settings tab that `openSettings` can scroll to; the section clears it once it has. */
export type SettingsSection = "memory";

export const DEFAULT_PREFS: LocalPrefs = {
  language: "system",
  appearance: "system",
  density: "regular",
  ui_zoom: 100,
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
  hosts_view: "list",
  hosts_stats: false,
  dashboard_hidden_columns: [],
  dashboard_interval_secs: 3,
  dashboard_cpu_alert: 90,
  dashboard_mem_alert: 90,
  dashboard_disk_alert: 90,
  close_action: "ask",
  local_shell: null,
  sftp_editor: null,
};

interface AppState {
  phase: Phase;
  info: AppInfo;
  /** Whether the Linux window shows its painted chrome right now (`:root[data-floating]`). */
  floating: boolean;
  prefs: LocalPrefs;
  /** The interface zoom in effect, which the screen may hold below `prefs.ui_zoom` (TERM-13). */
  zoom: ZoomView;
  vault: VaultStatus | null;
  /** Why the vault last locked while the app ran; null when it started locked. */
  lockReason: VaultLockedEvent["reason"] | null;
  sync: SyncStatus | null;
  page: Page;
  settingsOpen: boolean;
  /** The tab the settings window opens on or switches to; null keeps the last one. The window clears it once taken. */
  settingsTab: SettingsTab | null;
  settingsSection: SettingsSection | null;
  /** Bumped to ask the hosts page to focus its search field (Ctrl+Shift+K). */
  searchFocusTick: number;
  /** The text the search field takes with that focus, or null to keep its own (HOST-13). */
  searchFocusText: string | null;
  /** Bumped after each sync status that may follow a pull, so open views of synced data re-read it. */
  syncTick: number;

  navigate(page: Page): void;
  setPrefs(prefs: LocalPrefs): Promise<void>;
  /** Zooms the interface to the step nearest `percent` that the screen allows, and keeps it as this device's preference. */
  setZoom(percent: number): Promise<ZoomView>;
  refreshVault(): Promise<VaultStatus>;
  setSync(status: SyncStatus): void;
  setPhase(phase: Phase): void;
  /** Opens or closes Settings; `section` scrolls to a section of `tab`, such as `memory` in AI (AI-53). */
  openSettings(open: boolean, tab?: SettingsTab, section?: SettingsSection): void;
  focusSearch(text?: string): void;
  refreshSynced(): void;
  lock(): Promise<void>;
}

export const useApp = create<AppState>((set, get) => ({
  phase: "boot",
  info: { version: "0.0.0", platform: "web", mica: false, local_user: "", device_name: "", package_manager: null },
  floating: false,
  prefs: DEFAULT_PREFS,
  zoom: { zoom: 100, max: 200 },
  vault: null,
  lockReason: null,
  sync: null,
  page: { kind: "hosts", group: null },
  settingsOpen: false,
  settingsTab: null,
  settingsSection: null,
  searchFocusTick: 0,
  searchFocusText: null,
  syncTick: 0,

  navigate: (page) => set({ page }),
  setPrefs: async (prefs) => {
    set({ prefs });
    applyPrefs(prefs);
    await api.prefs_save(prefs);
  },
  setZoom: async (percent) => {
    const zoom = await api.window_zoom(percent);
    set({ zoom });
    applyZoom(zoom.zoom);
    if (zoom.zoom !== get().prefs.ui_zoom) await get().setPrefs({ ...get().prefs, ui_zoom: zoom.zoom });
    return zoom;
  },
  refreshVault: async () => {
    const vault = await api.vault_status();
    set({ vault });
    return vault;
  },
  setSync: (sync) => set({ sync }),
  setPhase: (phase) => set({ phase }),
  openSettings: (settingsOpen, tab, section) =>
    set({ settingsOpen, settingsTab: tab ?? null, settingsSection: section ?? null }),
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
  const theme = dark ? "dark" : "light";
  if (root.dataset.theme !== theme) {
    // Every color changes at once: controls fading on their own (base.css) would trail the page.
    root.dataset.themeSwitch = "";
    root.dataset.theme = theme;
    void document.body?.offsetHeight;
    delete root.dataset.themeSwitch;
  }
  root.dataset.density = prefs.density;
  setLocale(resolveLocale(prefs));
}

darkQuery?.addEventListener("change", () => applyPrefs(useApp.getState().prefs));

export function isDarkTheme(): boolean {
  return document.documentElement.dataset.theme === "dark";
}
