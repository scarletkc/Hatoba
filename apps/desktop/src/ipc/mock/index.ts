import { formatTarget } from "@/session/quickConnect";
import type { HatobaApi, Unlisten } from "../api";
import type { EventMap } from "../types";
import { createAiMock } from "./ai";
import { createAiExtensionsMock } from "./aiExtensions";
import { createAiMemoryMock } from "./aiMemory";
import { createAiSettingsMock } from "./aiSettings";
import { createAppMock } from "./app";
import { createDashboardMock } from "./dashboard";
import * as D from "./data";
import { createEditsMock } from "./edits";
import { createHostsMock } from "./hosts";
import { createKeysProxiesMock } from "./keysProxies";
import { createLocalMock } from "./local";
import { readStoredPrefs } from "./prefs";
import { createSftpMock } from "./sftp";
import type { MockDeps, MockState } from "./shared";
import { FakeShell } from "./shell";
import { createSshMock } from "./ssh";
import { createSyncMock } from "./sync";
import { createTelnetMock } from "./telnet";
import { createVaultMock } from "./vault";
import { passedLocale, VISITOR, visitorPlatform } from "./visitor";
import { version as appVersion } from "../../../package.json";

/**
 * In-browser stand-in for the Rust backend so the UI can be developed and reviewed with the
 * sample data (`pnpm dev`). URL parameters select demo states:
 *   ?state=onboarding | locked | empty      ?sync=none | syncing | offline | conflict | auth | too_large
 *   ?platform=windows | macos | linux       ?update=available | badsig | dropped | offline | error
 *   ?manager=scoop | aur | other            (the package manager that installed Hatoba)
 *   ?deploy=fail | waiting | vault | foreign | nosub | accounts | permission | nobundle
 *   ?star=due
 *   ?worker=available | required | custom | app
 *   ?ssh=timeout | refused | dns | unreachable | auth_failed | disconnected | proxy_unreachable | proxy_auth | proxy
 *   ?launch=<command line>                  ?cli=offpath
 *   ?close=prompt                           ?hello=off   ?pin=on | persist
 *   ?edit=conflict | fail | held            (SFTP-05, see `createEditsMock`)
 *   ?biometric=unavailable | stale | cancelled   (how Windows Hello or Touch ID fails)
 *   ?lang=en | zh-CN | zh-TW | ja
 * Quick connect (HOST-12) asks to trust a new host key once per address, then for the password
 * (anything but "wrong" is accepted); targets whose address contains "timeout" fail with a timeout.
 * Connecting to staging-web-02 always fails with a timeout; with `?ssh`, every host fails with that
 * SSH error kind (`?ssh=fail` or another value: a timeout), so the error card shows (SSH-05).
 * Without `?update`, the update check finds no release, as GitHub does before the first one.
 * `available`, `badsig` and `dropped` find v0.2.0. Installing it downloads for a few seconds and
 * then stays at "installing", where the real app closes; `badsig` fails the signature check and
 * `dropped` loses the connection partway through the download. With `?manager`, the About page
 * leaves updates to that package manager.
 * The in-app deployment accepts any API token of 20 or more characters.
 * `?launch` is what followed `hatoba` on the command line (HOST-13), which the window takes once
 * it is unlocked, e.g. `?launch=root@db` or `?launch=ssh -p 2222 deploy@10.0.0.5`. The `hatoba`
 * command starts off; with `?cli=offpath`, its folder is not on the PATH, as on Linux before the
 * next sign-in.
 * With `?platform=macos`, the page draws the traffic lights where the native window has them.
 * With `?star=due`, the star prompt's day has passed, so it shows after the first connection.
 * With `?close=prompt`, the window's close button asks whether to minimize to the tray (WIN-10),
 * once the window listens for it.
 * With `?pin`, a PIN is set (SEC-11): `on` works until the app restarts, `persist` after one too.
 * The mock's PIN is 1234 until another is set; five wrong ones in a row turn it off until an
 * unlock with the master password.
 * `?worker` shows a Worker update notice (§6.7, Upgrades): `available` on a Worker the app
 * deployed, `required` on one deployed another way (sync paused), `custom` on a custom domain,
 * and `app` for a Worker that needs a newer Hatoba. "Update Worker" honours `?deploy=fail`,
 * `waiting`, `permission`, `nobundle`, and `accounts`, and also `missing`, `novault`, `newer`, and
 * `foreign` for the review.
 * `?lang` sets the language pref and the language of the sample data until the page reloads.
 * Built with `--mode demo`, it is the website's demo instead (see `VISITOR`), where the parameters
 * still work.
 * Nothing here is secure; it never runs inside the Tauri app.
 */
export function createMockApi(): HatobaApi {
  const q = new URLSearchParams(location.search);
  const platformParam = q.get("platform");
  const platform =
    platformParam === "macos" || platformParam === "linux" || platformParam === "windows"
      ? platformParam
      : VISITOR
        ? visitorPlatform()
        : "windows";
  const demo = q.get("state");
  const syncDemo = q.get("sync");
  const updateDemo = q.get("update");
  const managerParam = q.get("manager");
  const packageManager =
    managerParam === "scoop" || managerParam === "aur" || managerParam === "other" ? managerParam : null;
  const deployDemo = q.get("deploy");
  const sshDemo = q.get("ssh");
  const workerDemo = q.get("worker");
  let closeDemo = q.get("close") === "prompt";
  const version = VISITOR ? appVersion : "0.1.0-dev";

  const state: MockState = {
    hosts: demo === "empty" ? [] : D.HOSTS.map((h) => ({ ...h })),
    telnetHosts: demo === "empty" || VISITOR ? [] : D.TELNET_HOSTS.map((h) => ({ ...h })),
    groups: demo === "empty" ? [] : D.GROUPS.map((g) => ({ ...g })),
    keys: demo === "empty" ? [] : D.KEYS.map((k) => ({ ...k })),
    proxies: demo === "empty" ? [] : D.PROXIES.map((p) => ({ ...p })),
    // The demo keeps prefs only until the page reloads, so a reload starts it over.
    prefs: VISITOR
      ? { ...D.PREFS, ai_panel_open: true }
      : readStoredPrefs(() => localStorage.getItem("hatoba.mock.prefs")),
  };
  const lang = passedLocale();
  if (lang) state.prefs = { ...state.prefs, language: lang };
  const storePrefs = () => {
    if (!VISITOR) localStorage.setItem("hatoba.mock.prefs", JSON.stringify(state.prefs));
  };

  const listeners = new Map<string, Set<(p: unknown) => void>>();
  const shells = new Map<string, FakeShell>();
  let seq = 0;

  function emit<K extends keyof EventMap>(event: K, payload: EventMap[K]) {
    listeners.get(event)?.forEach((h) => h(payload));
  }

  const id = (p: string) => `${p}-${Date.now().toString(36)}-${(++seq).toString(36)}`;

  const { startTransfer, ...sftpApi } = createSftpMock({ emit, id });
  const { unlock: unlockEdits, ...edits } = createEditsMock(emit, startTransfer, q.get("edit"));

  const {
    kind: syncKind,
    touch,
    resetOff,
    resetRestored,
    ...syncApi
  } = createSyncMock({
    state,
    emit,
    id,
    demo,
    syncDemo,
    deployDemo,
    workerDemo,
  });
  let dashboardStopAll = () => {};
  const { unlocked, needUnlocked, ...vaultApi } = createVaultMock({
    state,
    q,
    demo,
    syncKind,
    resetSyncOff: resetOff,
    resetSyncRestored: resetRestored,
    onUnlock: unlockEdits,
    onLock: () => dashboardStopAll(),
  });
  const dashboard = createDashboardMock({
    host: (hid) => state.hosts.find((h) => h.id === hid),
    unlocked,
  });
  dashboardStopAll = dashboard.stopAll;

  const { onHostDeleted, ...sshApi } = createSshMock({
    state,
    emit,
    needUnlocked,
    id,
    touch,
    shells,
    q,
    demo,
    sshDemo,
  });
  const { placeNewHost, ...hostsApi } = createHostsMock({
    state,
    needUnlocked,
    id,
    touch,
    platform,
    onHostDeleted,
  });
  const localApi = createLocalMock({ emit, needUnlocked, id, platform, shells, q });
  const appApi = createAppMock({
    state,
    needUnlocked,
    touch,
    storePrefs,
    q,
    platform,
    version,
    packageManager,
    updateDemo,
  });

  const deps: MockDeps = { state, emit, needUnlocked, id, touch, placeNewHost, storePrefs };
  const keysProxiesApi = createKeysProxiesMock(deps);
  const telnetApi = createTelnetMock(deps);

  const aiSettings = createAiSettingsMock();
  const aiExtensions = createAiExtensionsMock((event, payload) => emit(event, payload), {
    version,
    builtinEnabled: async () => (await aiSettings.ai_settings_get()).builtin_skill_enabled,
  });
  const { memory_tool, ...aiMemory } = createAiMemoryMock();
  // Loaded only for the demo: it reaches into the app's stores, which the mock's tests leave out.
  if (VISITOR) void import("./visitorStart").then((m) => m.startVisitor());
  return {
    ...aiSettings,
    ...aiExtensions,
    ...aiMemory,
    ...createAiMock({
      providers: aiSettings.ai_providers_list,
      settings: aiSettings.ai_settings_get,
      hosts: async () => state.hosts,
      sessionServer: (sid) => {
        const shell = shells.get(sid);
        return shell?.host ? formatTarget(shell.host) : shell ? "local" : null;
      },
      mcp: {
        servers: aiExtensions.mcp_servers_list,
        status: aiExtensions.mcp_server_status,
        start: aiExtensions.mcp_server_start,
        toolInfo: aiExtensions.mcp_tool_info,
      },
      memory: memory_tool,
    }),
    ...vaultApi,
    ...syncApi,
    ...keysProxiesApi,
    ...telnetApi,
    ...appApi,
    ...hostsApi,
    ...dashboard,
    ...sshApi,
    ...localApi,
    ...sftpApi,
    ...edits,
    listen: async (event, handler): Promise<Unlisten> => {
      let set = listeners.get(event);
      if (!set) listeners.set(event, (set = new Set()));
      const h = handler as (p: unknown) => void;
      set.add(h);
      if (event === "window://close-prompt" && closeDemo) {
        closeDemo = false;
        setTimeout(() => emit("window://close-prompt", null), 500);
      }
      return () => set.delete(h);
    },
  };
}
