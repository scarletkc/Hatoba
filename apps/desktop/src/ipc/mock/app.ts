import { defaultRightClick, defaultTerminalFont } from "@/lib/platform";
import { ZOOM_STEPS, zoomStep } from "@/lib/zoom";
import type { HatobaApi } from "../api";
import type { PackageManager, Platform, StarPrompt } from "../types";
import * as D from "./data";
import { applyPatch } from "./patch";
import { delay, fail, type MockDeps } from "./shared";
import { VISITOR, VISITOR_HOST } from "./visitor";

type AppApi = Pick<
  HatobaApi,
  | "app_info"
  | "launch_take"
  | "cli_command_get"
  | "cli_command_set"
  | "cli_access_set"
  | "cli_activity_list"
  | "update_check"
  | "update_install"
  | "activity_ping"
  | "settings_get"
  | "settings_save"
  | "prefs_get"
  | "prefs_save"
  | "star_prompt_get"
  | "star_prompt_done"
  | "window_snap_overlay"
  | "window_floating"
  | "window_zoom"
  | "window_hide_to_tray"
  | "app_quit"
  | "tray_set_menu"
  | "notification_show"
  | "save_text_file"
>;

export interface AppMockDeps extends Pick<MockDeps, "state" | "needUnlocked" | "touch" | "storePrefs"> {
  q: URLSearchParams;
  platform: Platform;
  version: string;
  packageManager: PackageManager | null;
  updateDemo: string | null;
}

export function createAppMock(ctx: AppMockDeps): AppApi {
  const { state, needUnlocked, touch, storePrefs, q, platform, version, packageManager, updateDemo } = ctx;

  let settings = structuredClone(D.SETTINGS);
  settings.terminal.font_family = defaultTerminalFont(platform);
  settings.terminal.right_click = defaultRightClick(platform);
  const starPrompt: StarPrompt = {
    first_seen_at: q.get("star") === "due" ? Date.now() - 2 * 86_400_000 : Date.now(),
    done: false,
  };
  const launches = q.get("launch")?.trim() ? [q.get("launch")!.trim().split(/\s+/)] : VISITOR ? [[VISITOR_HOST]] : [];
  let cliCommand = q.get("cli") === "access";
  let cliAccess = cliCommand;
  const cliView = () => ({
    enabled: cliCommand,
    location:
      platform === "windows"
        ? "C:\\Users\\demo\\AppData\\Local\\Hatoba\\bin"
        : platform === "macos"
          ? "/usr/local/bin/hatoba"
          : "/home/demo/.local/bin/hatoba",
    on_path: q.get("cli") !== "offpath",
    access: cliAccess,
  });

  return {
    app_info: async () => ({
      version,
      platform,
      mica: false,
      local_user: "demo",
      device_name: platform === "macos" ? "Demo-MacBook" : "DEMO-PC",
      package_manager: packageManager,
    }),
    launch_take: async () => {
      needUnlocked();
      return launches.splice(0);
    },
    cli_command_get: async () => cliView(),
    cli_command_set: async (enabled) => {
      await delay(300);
      cliCommand = enabled;
      return cliView();
    },
    cli_access_set: async (enabled) => {
      await delay(300);
      cliAccess = enabled;
      return cliView();
    },
    cli_activity_list: async () => {
      needUnlocked();
      return cliAccess ? D.cliActivity() : [];
    },
    update_check: async () => {
      await delay(800);
      if (updateDemo === "offline") fail("sync_offline", "GitHub could not be reached");
      if (updateDemo === "error") fail("internal", "the update endpoint answered HTTP 403");
      if (updateDemo === "available" || updateDemo === "badsig" || updateDemo === "dropped")
        return { current_version: version, update: D.AVAILABLE_UPDATE };
      return { current_version: version, update: null };
    },
    update_install: async (onProgress) => {
      const total = 14_680_064;
      for (let downloaded = 0; downloaded < total; downloaded += 524_288) {
        if (updateDemo === "dropped" && downloaded > total * 0.4) fail("sync_offline", "GitHub could not be reached");
        onProgress({ kind: "downloading", downloaded, total });
        await delay(100);
      }
      onProgress({ kind: "downloading", downloaded: total, total });
      if (updateDemo === "badsig") fail("update_signature", "Minisign error: the signature verification failed");
      onProgress({ kind: "installing" });
      // The real app closes here, and the installer starts the new version.
      return new Promise<void>(() => {});
    },
    activity_ping: async () => {},

    settings_get: async () => structuredClone(settings),
    settings_save: async (patch) => {
      const terminal = applyPatch(settings.terminal, patch.terminal ?? {});
      if (JSON.stringify(terminal) === JSON.stringify(settings.terminal)) return;
      settings = { ...settings, terminal };
      touch();
    },
    prefs_get: async () => state.prefs,
    prefs_save: async (next) => {
      state.prefs = next;
      storePrefs();
    },
    star_prompt_get: async () => ({ ...starPrompt }),
    star_prompt_done: async () => {
      starPrompt.done = true;
    },

    window_snap_overlay: async () => {},
    window_floating: async () => false,
    window_zoom: async (percent) => ({ zoom: zoomStep(percent), max: ZOOM_STEPS[ZOOM_STEPS.length - 1] }),
    window_hide_to_tray: async () => {},
    app_quit: async () => {},
    tray_set_menu: async () => {},
    notification_show: async () => {},
    save_text_file: async (path, contents) => {
      // Browser fallback: download the text instead of writing to `path`.
      const a = document.createElement("a");
      a.href = URL.createObjectURL(new Blob([contents], { type: "text/plain" }));
      a.download = path.split(/[\\/]/).pop() || "hatoba.txt";
      a.click();
      URL.revokeObjectURL(a.href);
    },
  };
}
