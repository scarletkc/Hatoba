import { byteCount, HOST_NOTES_MAX_BYTES } from "@/lib/ai/instructions";
import { hostPort, reachableAddress } from "@/lib/address";
import { ENV_MAX_VARS, envRows, envTooLarge, validateEnvRows } from "@/lib/envVars";
import { defaultRightClick, defaultTerminalFont } from "@/lib/platform";
import { ZOOM_STEPS, zoomStep } from "@/lib/zoom";
import { formatTarget, sameTarget } from "@/session/quickConnect";
import { withCommand } from "@/session/suggest";
import { appError, type FrameHandler, type HatobaApi, type Unlisten } from "../api";
import type {
  AppError,
  ConnectionsEvent,
  EventMap,
  FileEntry,
  ForwardView,
  GroupView,
  HostView,
  QuickTarget,
  RunningForwardView,
  SshErrorKind,
  StarPrompt,
} from "../types";
import { FRAME_CLOSED, FRAME_DATA, FRAME_SESSION } from "../types";
import { createAiMock } from "./ai";
import { createAiExtensionsMock } from "./aiExtensions";
import { createAiMemoryMock } from "./aiMemory";
import { createAiSettingsMock } from "./aiSettings";
import { createDashboardMock } from "./dashboard";
import * as D from "./data";
import { mockStats } from "./data";
import { createEditsMock } from "./edits";
import { createKeysProxiesMock } from "./keysProxies";
import { mockLocalShells } from "./local";
import { applyPatch } from "./patch";
import { readStoredPrefs } from "./prefs";
import { delay, fail, frame, type MockDeps, type MockState } from "./shared";
import { FakeShell } from "./shell";
import { createSyncMock } from "./sync";
import { createTelnetMock } from "./telnet";
import { createVaultMock } from "./vault";
import { passedLocale, VISITOR, VISITOR_HOST, visitorPlatform } from "./visitor";
import { version as appVersion } from "../../../package.json";

/** The room between neighbouring places in the custom order, as `PLACE_STEP` in Rust (HOST-16). */
const PLACE_STEP = 1 << 16;

/** `?ssh=<kind>`: every connection fails with this SSH error kind and a detail like the real one. */
const SSH_DEMO_DETAIL: Partial<Record<SshErrorKind, (address: string, port: number) => string>> &
  Record<"timeout", (address: string, port: number) => string> = {
  timeout: (a, p) => `connect to ${a}:${p}: operation timed out`,
  refused: (a, p) => `connect to ${a}:${p}: connection refused`,
  dns: (a) => `failed to lookup address information: ${a}: nodename nor servname provided, or not known`,
  unreachable: (a, p) => `connect to ${a}:${p}: network is unreachable`,
  auth_failed: () => "authentication failed: no method succeeded (tried publickey, password)",
  disconnected: () => "the server closed the connection during the handshake",
  proxy_unreachable: () => "cannot reach the proxy 127.0.0.1:7890: connect to 127.0.0.1:7890: connection refused",
  proxy_auth: () => "the proxy proxy.corp.example.com:3128 rejected the username or password (HTTP 407)",
  proxy: (a, p) => `the proxy could not connect to ${a}:${p}: connection not allowed by its rules (0x02)`,
};

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
  let settings = structuredClone(D.SETTINGS);
  settings.terminal.font_family = defaultTerminalFont(platform);
  settings.terminal.right_click = defaultRightClick(platform);
  const lang = passedLocale();
  if (lang) state.prefs = { ...state.prefs, language: lang };
  const storePrefs = () => {
    if (!VISITOR) localStorage.setItem("hatoba.mock.prefs", JSON.stringify(state.prefs));
  };
  const starPrompt: StarPrompt = {
    first_seen_at: q.get("star") === "due" ? Date.now() - 2 * 86_400_000 : Date.now(),
    done: false,
  };
  let forwards: ForwardView[] = demo === "empty" ? [] : D.FORWARDS.map((f) => ({ ...f }));
  let recentTargets: QuickTarget[] = demo === "empty" ? [] : D.RECENT_TARGETS.map((r) => ({ ...r }));
  // TERM-15: each host's and local shell's commands, newest first; the demo starts every one with a few.
  const commandHistory = new Map<string, string[]>();
  let historySeeded = demo !== "empty";
  const historyOf = (owner: string) => commandHistory.get(owner) ?? (historySeeded ? [...D.COMMAND_HISTORY] : []);
  const trustedQuickHosts = new Set(recentTargets.map((r) => `${r.address}:${r.port}`));
  /**
   * SSH-16: each host's shared connection, with its tabs and forwards. `?tunnel=lost` drops a host's connection 5 s
   * after it opens, as a network failure would; `?fwd=busy` fails the auto-start forwards, as a port in use would.
   */
  const lostDemo = q.get("tunnel") === "lost";
  const busyDemo = q.get("fwd") === "busy";
  type MockConnection = {
    name: string;
    tabs: Set<string>;
    forwards: Map<string, Omit<RunningForwardView, "forward_id">>;
    failed: Map<string, AppError>;
  };
  const connections = new Map<string, MockConnection>();
  let connectionsVersion = 0;
  const connectionList = (): ConnectionsEvent => ({
    version: connectionsVersion,
    connections: [...connections.entries()].map(([host_id, c]) => ({
      host_id,
      host_name: c.name,
      forwards: [...c.forwards.entries()]
        .map(([forward_id, f]) => ({ forward_id, ...f }))
        .sort((a, b) => a.port - b.port),
      failed: [...c.failed.entries()].map(([forward_id, error]) => ({ forward_id, error })),
    })),
  });
  const connectionsChanged = () => {
    connectionsVersion++;
    emit("ssh://connections", connectionList());
  };
  /** The connection closes once no tab and no forward is left on it. */
  const closeIfUnused = (hostId: string) => {
    const c = connections.get(hostId);
    if (c && c.tabs.size === 0 && c.forwards.size === 0) connections.delete(hostId);
  };
  const runForward = (c: MockConnection, f: ForwardView): number => {
    const port = f.bind_port || 49152 + Math.floor(Math.random() * 1000);
    c.forwards.set(f.id, {
      name: f.name,
      kind: f.kind,
      port,
      address: f.kind === "remote" ? hostPort(f.bind_address, port) : reachableAddress(f.bind_address, port),
      dest: f.kind === "dynamic" ? "" : hostPort(f.dest_host, f.dest_port),
    });
    c.failed.delete(f.id);
    return port;
  };
  /** Opens the host's connection and starts its auto-start forwards on it (FWD-02). */
  const openConnection = (hostId: string, name: string): MockConnection => {
    const c: MockConnection = { name, tabs: new Set(), forwards: new Map(), failed: new Map() };
    connections.set(hostId, c);
    state.hosts = state.hosts.map((x) => (x.id === hostId ? { ...x, last_connected_at: Date.now() } : x));
    for (const f of forwards.filter((x) => x.host_id === hostId && x.auto_start)) {
      if (busyDemo) {
        c.failed.set(
          f.id,
          appError("ssh", `cannot listen on ${f.bind_address}:${f.bind_port}: address already in use`, {
            ssh_kind: "io",
          }),
        );
      } else runForward(c, f);
    }
    if (lostDemo) {
      setTimeout(() => {
        if (connections.get(hostId) !== c) return;
        connections.delete(hostId);
        for (const sid of c.tabs) {
          shells.get(sid)?.stop();
          shells.delete(sid);
          emit("ssh://state", {
            session_id: sid,
            host_id: hostId,
            state: "disconnected",
            latency_ms: null,
            error: appError("ssh", "keepalive timeout: no reply from the server", { ssh_kind: "disconnected" }),
            exit_status: null,
          });
        }
        if (c.forwards.size > 0) {
          emit("ssh://forwards-lost", {
            host_id: hostId,
            host_name: c.name,
            error: appError("ssh", "keepalive timeout: no reply from the server", { ssh_kind: "disconnected" }),
          });
        }
        connectionsChanged();
      }, 5000);
    }
    return c;
  };
  const leaveConnection = (sid: string) => {
    for (const [hostId, c] of connections) {
      if (!c.tabs.delete(sid)) continue;
      closeIfUnused(hostId);
      connectionsChanged();
    }
  };
  const stopForward = (hostId: string, forwardId: string) => {
    const c = connections.get(hostId);
    if (!c) return;
    const found = c.forwards.delete(forwardId) || c.failed.delete(forwardId);
    if (!found) return;
    closeIfUnused(hostId);
    connectionsChanged();
  };
  const listeners = new Map<string, Set<(p: unknown) => void>>();
  const shells = new Map<string, FakeShell>();
  const { unlock: unlockEdits, ...edits } = createEditsMock(emit, startTransfer, q.get("edit"));
  /** TERM-12: the mock resource usage of each session that shows it, by the id of its start. */
  const statsTimers = new Map<string, { id: number; timer: ReturnType<typeof setInterval> }>();
  let lastStatsId = 0;
  const stopStats = (sid: string) => {
    clearInterval(statsTimers.get(sid)?.timer);
    statsTimers.delete(sid);
  };
  let seq = 0;
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

  function emit<K extends keyof EventMap>(event: K, payload: EventMap[K]) {
    listeners.get(event)?.forEach((h) => h(payload));
  }

  const id = (p: string) => `${p}-${Date.now().toString(36)}-${(++seq).toString(36)}`;

  /** Numbers the hosts and Telnet hosts in `order` again, in that order (HOST-16). */
  function arrangeHosts(order: string[]) {
    const place = new Map(order.map((hid, i) => [hid, (i + 1) * PLACE_STEP]));
    state.hosts = state.hosts.map((h) => (place.has(h.id) ? { ...h, sort: place.get(h.id)! } : h));
    state.telnetHosts = state.telnetHosts.map((h) => (place.has(h.id) ? { ...h, sort: place.get(h.id)! } : h));
  }

  /** The place of a host being added, as `Vault::place_new_host` gives it: after `after`, or else last. */
  function placeNewHost(after?: string): number {
    const all = [...state.hosts, ...state.telnetHosts];
    if (after && all.find((h) => h.id === after)?.sort === 0) return 0;
    const placed = all.filter((h) => h.sort > 0).sort((a, b) => a.sort - b.sort);
    if (placed.length === 0) return 0;
    const i = after ? placed.findIndex((h) => h.id === after) : -1;
    if (i < 0 || i === placed.length - 1) return placed[placed.length - 1].sort + PLACE_STEP;
    if (placed[i + 1].sort - placed[i].sort >= 2) return Math.floor((placed[i].sort + placed[i + 1].sort) / 2);
    arrangeHosts(placed.map((h) => h.id));
    return placeNewHost(after);
  }

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

    hosts_list: async () => {
      needUnlocked();
      return state.hosts.map((h) => ({ ...h }));
    },
    host_get: async (hid) => {
      const h = state.hosts.find((x) => x.id === hid);
      if (!h) fail("not_found");
      return { ...h };
    },
    host_save: async (input) => {
      needUnlocked();
      if (!input.address.trim()) fail("invalid_input", "address is required", { field: "address" });
      if (byteCount(input.ai_notes) > HOST_NOTES_MAX_BYTES)
        fail("invalid_input", "AI notes are limited to 16 KB", { field: "ai_notes" });
      if (input.proxy_mode === "proxy" && !state.proxies.some((p) => p.id === input.proxy_id))
        fail("invalid_input", "proxy not found", { field: "proxy_id" });
      const env = input.env.map((v) => ({ name: v.name.trim(), value: v.value }));
      if (
        env.length > ENV_MAX_VARS ||
        validateEnvRows(envRows(env)).some(Boolean) ||
        envTooLarge(envRows(env)) ||
        env.some((v) => !v.name)
      )
        fail("invalid_input", "invalid environment variables", { field: "env" });
      const existing = input.id ? state.hosts.find((h) => h.id === input.id) : undefined;
      const view: HostView = {
        id: existing?.id ?? id("h"),
        name: input.name.trim() || Array.from(input.address.trim()).slice(0, 128).join(""),
        address: input.address.trim(),
        port: input.port || 22,
        username: input.username.trim(),
        auth_kind: input.auth_kind,
        has_password:
          input.auth_kind === "password" &&
          (input.password !== null ? input.password.length > 0 : !!existing?.has_password),
        key_id: input.auth_kind === "key" ? input.key_id : null,
        group_id: input.group_id,
        tags: input.tags,
        favorite: input.favorite,
        sort: existing?.sort ?? placeNewHost(),
        jump_host_id: input.jump_host_id,
        proxy_mode: input.proxy_mode,
        proxy_id: input.proxy_mode === "proxy" ? input.proxy_id : null,
        note: input.note,
        ai_notes: input.ai_notes,
        env,
        updated_at: Date.now(),
        last_connected_at: existing?.last_connected_at ?? null,
        os: existing?.os ?? null,
        show_stats: existing?.show_stats ?? false,
        dashboard: existing?.dashboard ?? true,
      };
      state.hosts = existing ? state.hosts.map((h) => (h.id === view.id ? view : h)) : [...state.hosts, view];
      touch();
      return view;
    },
    host_delete: async (hid) => {
      state.hosts = state.hosts
        .filter((h) => h.id !== hid)
        .map((h) => (h.jump_host_id === hid ? { ...h, jump_host_id: null } : h));
      // Its forwards go with it, and stop where they run; its tabs stay connected.
      forwards = forwards.filter((f) => f.host_id !== hid);
      const c = connections.get(hid);
      if (c && (c.forwards.size > 0 || c.failed.size > 0)) {
        c.forwards.clear();
        c.failed.clear();
        closeIfUnused(hid);
        connectionsChanged();
      }
      touch();
    },
    host_duplicate: async (hid) => {
      const h = state.hosts.find((x) => x.id === hid);
      if (!h) fail("not_found");
      const copy = {
        ...h,
        id: id("h"),
        name: `${h.name}-copy`,
        favorite: false,
        sort: placeNewHost(hid),
        last_connected_at: null,
        os: null,
        show_stats: false,
      };
      state.hosts = [...state.hosts, copy];
      touch();
      return copy;
    },
    hosts_arrange: async (order) => {
      needUnlocked();
      arrangeHosts(
        order.filter((hid) => state.hosts.some((h) => h.id === hid) || state.telnetHosts.some((h) => h.id === hid)),
      );
      touch();
    },
    host_set_favorite: async (hid, favorite) => {
      state.hosts = state.hosts.map((h) => (h.id === hid ? { ...h, favorite } : h));
      touch();
    },
    host_set_show_stats: async (hid, on) => {
      needUnlocked();
      state.hosts = state.hosts.map((h) => (h.id === hid ? { ...h, show_stats: on } : h));
    },
    dashboard_set_shown: async (itemId, shown) => {
      needUnlocked();
      if (!state.hosts.some((h) => h.id === itemId) && !state.groups.some((g) => g.id === itemId))
        fail("not_found", "host or group");
      state.hosts = state.hosts.map((h) => (h.id === itemId ? { ...h, dashboard: shown } : h));
      state.groups = state.groups.map((g) => (g.id === itemId ? { ...g, dashboard: shown } : g));
    },
    ...dashboard,
    host_copy_password: async () => {},
    groups_list: async () => state.groups.map((g) => ({ ...g })),
    group_save: async (input) => {
      const g: GroupView = {
        id: input.id ?? id("g"),
        name: input.name,
        parent_id: input.parent_id,
        sort: input.sort,
        dashboard: state.groups.find((x) => x.id === input.id)?.dashboard ?? true,
      };
      state.groups = input.id ? state.groups.map((x) => (x.id === g.id ? g : x)) : [...state.groups, g];
      touch();
      return g;
    },
    group_delete: async (gid) => {
      state.groups = state.groups.filter((g) => g.id !== gid);
      state.hosts = state.hosts.map((h) => (h.group_id === gid ? { ...h, group_id: null } : h));
      state.telnetHosts = state.telnetHosts.map((h) => (h.group_id === gid ? { ...h, group_id: null } : h));
      touch();
    },
    tags_list: async () => {
      const counts = new Map<string, number>();
      [...state.hosts, ...state.telnetHosts].forEach((h) =>
        h.tags.forEach((t) => counts.set(t, (counts.get(t) ?? 0) + 1)),
      );
      return [...counts.entries()]
        .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
        .map(([name, count]) => ({ name, count }));
    },
    hosts_probe: async (ids) => {
      await delay(250);
      return ids.map((hid) => {
        const online = D.PROBE[hid] ?? true;
        return { id: hid, online, latency_ms: online ? 20 + ((hid.length * 7) % 60) : null };
      });
    },
    hosts_import_preview: async (source, path) => {
      await delay(200);
      const candidates = D.importCandidates(source).map((c) => ({
        ...c,
        exists: state.hosts.some((h) => h.name.toLowerCase() === c.name.toLowerCase()),
      }));
      return { location: path ?? D.importLocation(source, platform), encrypted: false, candidates };
    },
    hosts_import: async (source, _path, ids, keyFiles) => {
      await delay(300);
      const chosen = D.importCandidates(source).filter((c) => ids.includes(c.id));
      for (const c of chosen) {
        // A folder becomes a group named after its path.
        let groupId: string | null = null;
        if (c.folder) {
          const found = state.groups.find((g) => !g.parent_id && g.name.toLowerCase() === c.folder?.toLowerCase());
          const g: GroupView = found ?? {
            id: id("g"),
            name: c.folder,
            parent_id: null,
            sort: state.groups.length,
            dashboard: true,
          };
          if (!found) state.groups.push(g);
          groupId = g.id;
        }
        state.hosts.push({
          ...D.HOSTS[0],
          id: id("h"),
          name: c.name,
          address: c.address,
          port: c.port,
          username: c.username,
          favorite: false,
          sort: placeNewHost(),
          tags: [],
          group_id: groupId,
          last_connected_at: null,
          os: null,
          key_id: null,
          auth_kind: "ask",
          proxy_mode: "device_default",
          proxy_id: null,
        });
      }
      touch();
      const warnings = chosen.flatMap((c) =>
        c.skipped_proxy ? [`${c.name}: ${c.skipped_proxy} was not imported; set a proxy or jump host for it`] : [],
      );
      return { hosts_created: chosen.length, keys_imported: keyFiles.length, warnings };
    },

    ssh_connect: async (hostId, _options, onFrame: FrameHandler) => {
      const h = state.hosts.find((x) => x.id === hostId);
      if (!h) fail("not_found");
      const sid = id("s");
      onFrame(frame(FRAME_SESSION, new TextEncoder().encode(sid)));
      const stateEvt = (s: EventMap["ssh://state"]["state"], extra: Partial<EventMap["ssh://state"]> = {}) =>
        emit("ssh://state", {
          session_id: sid,
          host_id: hostId,
          state: s,
          latency_ms: null,
          error: null,
          exit_status: null,
          ...extra,
        });
      setTimeout(() => stateEvt("connecting"), 0);
      if (D.FAILING_HOSTS.has(hostId) || sshDemo) {
        const enc = new TextEncoder();
        for (let i = 1; i <= 3; i++) {
          await delay(350);
          onFrame(
            frame(FRAME_DATA, enc.encode(`\x1b[90mConnecting to ${h.address}:${h.port} (attempt ${i})…\x1b[0m\r\n`)),
          );
        }
        await delay(400);
        const kind = sshDemo && sshDemo in SSH_DEMO_DETAIL ? (sshDemo as SshErrorKind) : "timeout";
        const detail = (SSH_DEMO_DETAIL[kind] ?? SSH_DEMO_DETAIL.timeout)(h.address, h.port);
        const error = appError("ssh", detail, { ssh_kind: kind });
        setTimeout(() => stateEvt("failed", { error }), 0);
        throw error;
      }
      const shared = connections.get(hostId);
      if (h.name === "homelab-nas" && !shared) {
        const accepted = await new Promise<boolean>((resolve) => {
          const rid = id("hk");
          pendingHostKeys.set(rid, resolve);
          emit("ssh://hostkey-prompt", {
            request_id: rid,
            session_id: sid,
            host: h.address,
            port: h.port,
            key_type: "ssh-ed25519",
            fingerprint: "SHA256:pX2v8KqLm4Tn7Wc1Rz9Ye3Hb6Ud0Sa5Jf2Gk8Qw",
            kind: "new",
            known_fingerprint: null,
            known_key_type: null,
          });
        });
        if (!accepted) {
          const error = appError("ssh", "host key rejected", { ssh_kind: "host_key_rejected" });
          setTimeout(() => stateEvt("failed", { error }), 0);
          throw error;
        }
      }
      await delay(500);
      const shell = new FakeShell(h, (bytes) => onFrame(frame(FRAME_DATA, bytes)));
      shells.set(sid, shell);
      // SSH-16: the tab joins the host's connection, or opens it.
      const c = shared ?? openConnection(hostId, h.name);
      c.tabs.add(sid);
      connectionsChanged();
      state.hosts = state.hosts.map((x) => (x.id === hostId ? { ...x, last_connected_at: Date.now() } : x));
      setTimeout(() => {
        stateEvt("connected", { latency_ms: 38 });
        shell.start();
      }, 0);
      shell.onExit = () => {
        onFrame(frame(FRAME_CLOSED, new TextEncoder().encode("exit")));
        stateEvt("disconnected", { exit_status: 0 });
        shells.delete(sid);
        leaveConnection(sid);
        stopStats(sid);
      };
      return sid;
    },
    ssh_connect_target: async (target, _options, onFrame: FrameHandler) => {
      needUnlocked();
      const sid = id("s");
      onFrame(frame(FRAME_SESSION, new TextEncoder().encode(sid)));
      const stateEvt = (s: EventMap["ssh://state"]["state"], extra: Partial<EventMap["ssh://state"]> = {}) =>
        emit("ssh://state", {
          session_id: sid,
          host_id: null,
          state: s,
          latency_ms: null,
          error: null,
          exit_status: null,
          ...extra,
        });
      const failWith = (error: AppError): never => {
        setTimeout(() => stateEvt("failed", { error }), 0);
        throw error;
      };
      setTimeout(() => stateEvt("connecting"), 0);
      await delay(400);
      if (target.address.includes("timeout"))
        failWith(appError("ssh", SSH_DEMO_DETAIL.timeout(target.address, target.port), { ssh_kind: "timeout" }));
      const endpoint = `${target.address}:${target.port}`;
      if (!trustedQuickHosts.has(endpoint)) {
        const accepted = await new Promise<boolean>((resolve) => {
          const rid = id("hk");
          pendingHostKeys.set(rid, resolve);
          emit("ssh://hostkey-prompt", {
            request_id: rid,
            session_id: sid,
            host: target.address,
            port: target.port,
            key_type: "ssh-ed25519",
            fingerprint: "SHA256:Qm3cT8vLk2Hs9Xa4Pz7Wd1Nf6Ry0Ub5Ej2Gi8Ko",
            kind: "new",
            known_fingerprint: null,
            known_key_type: null,
          });
        });
        if (!accepted) failWith(appError("ssh", "host key rejected", { ssh_kind: "host_key_rejected" }));
        trustedQuickHosts.add(endpoint);
      }
      const host = target.address.includes(":") ? `[${target.address}]` : target.address;
      if (target.address.endsWith(".ts.net")) {
        // Tailscale SSH's check mode: the server says where to sign in and holds the login until then.
        const rid = id("ab");
        const signedIn = await new Promise<boolean>((resolve) => {
          pendingAuth.set(rid, () => resolve(false));
          emit("ssh://auth-banner", {
            request_id: rid,
            session_id: sid,
            target: `${target.username}@${host}:${target.port}`,
            text: "# Tailscale SSH requires an additional check.\n# To authenticate, visit: https://login.tailscale.com/a/l7c4b9e2f1a3",
          });
          setTimeout(() => resolve(true), 8000);
        });
        if (pendingAuth.delete(rid)) emit("ssh://auth-prompt-closed", { request_id: rid });
        if (!signedIn) failWith(appError("ssh", "operation cancelled", { ssh_kind: "cancelled" }));
      } else {
        // No ssh-agent here: the server asks for the password.
        const answers = await new Promise<string[] | null>((resolve) => {
          const rid = id("ap");
          pendingAuth.set(rid, resolve);
          emit("ssh://auth-prompt", {
            request_id: rid,
            session_id: sid,
            name: target.address,
            instructions: "",
            prompts: [{ prompt: "Password: ", echo: false }],
            password: true,
            sudo: false,
            wrong: false,
            target: `${target.username}@${host}:${target.port}`,
          });
        });
        if (!answers) failWith(appError("ssh", "operation cancelled", { ssh_kind: "cancelled" }));
        if (answers![0] === "wrong")
          failWith(
            appError("ssh", "the server rejected the password (server accepts: publickey, password)", {
              ssh_kind: "auth_failed",
            }),
          );
      }
      await delay(300);
      const shellHost: HostView = {
        ...D.HOSTS[0],
        id: "",
        name: target.address,
        address: target.address,
        port: target.port,
        username: target.username,
      };
      const shell = new FakeShell(shellHost, (bytes) => onFrame(frame(FRAME_DATA, bytes)));
      shells.set(sid, shell);
      recentTargets = [target, ...recentTargets.filter((r) => !sameTarget(r, target))].slice(0, 8);
      setTimeout(() => {
        stateEvt("connected", { latency_ms: 21 });
        shell.start();
      }, 0);
      shell.onExit = () => {
        onFrame(frame(FRAME_CLOSED, new TextEncoder().encode("exit")));
        stateEvt("disconnected", { exit_status: 0 });
        shells.delete(sid);
        stopStats(sid);
      };
      return sid;
    },
    command_history_list: async (owner) => {
      needUnlocked();
      return historyOf(owner);
    },
    command_history_add: async (owner, command) => {
      needUnlocked();
      commandHistory.set(owner, withCommand(historyOf(owner), command));
    },
    command_history_remove: async (owner, command) => {
      needUnlocked();
      commandHistory.set(
        owner,
        historyOf(owner).filter((c) => c !== command),
      );
    },
    command_history_clear: async () => {
      needUnlocked();
      commandHistory.clear();
      historySeeded = false;
    },
    recent_targets_list: async () => {
      needUnlocked();
      return recentTargets.map((r) => ({ ...r }));
    },
    recent_target_remove: async (target) => {
      needUnlocked();
      recentTargets = recentTargets.filter((r) => !sameTarget(r, target));
      return recentTargets.map((r) => ({ ...r }));
    },
    ssh_write: async (sid, data) => {
      shells.get(sid)?.input(data);
    },
    // The sample shell never turns on mouse reporting, so no binary input reaches it.
    ssh_write_bytes: async () => {},
    ssh_resize: async () => {},
    ssh_disconnect: async (sid) => {
      shells.get(sid)?.stop();
      shells.delete(sid);
      leaveConnection(sid);
      stopStats(sid);
    },
    local_shells: async () => mockLocalShells(platform),
    local_open: async (shellId, _options, onFrame: FrameHandler) => {
      needUnlocked();
      const shell = mockLocalShells(platform).find((s) => s.id === shellId) ?? fail("not_found", "shell not found");
      // `?local=fail`: the shell does not start, as when a WSL distribution is broken.
      if (q.get("local") === "fail") fail("io", `the shell could not be started: ${shell.name} exited at once`);
      await delay(120);
      const sid = id("l");
      const fake = new FakeShell(null, (bytes) => onFrame(frame(FRAME_DATA, bytes)), {
        shell: shell.kind,
        user: "demo",
        machine: platform === "macos" ? "Demo-MacBook" : "DEMO-PC",
      });
      shells.set(sid, fake);
      setTimeout(() => {
        emit("ssh://state", {
          session_id: sid,
          host_id: null,
          state: "connected",
          latency_ms: null,
          error: null,
          exit_status: null,
        });
        fake.start();
      }, 0);
      fake.onExit = () => {
        onFrame(frame(FRAME_CLOSED, new Uint8Array()));
        emit("ssh://state", {
          session_id: sid,
          host_id: null,
          state: "disconnected",
          latency_ms: null,
          error: null,
          exit_status: 0,
        });
        shells.delete(sid);
      };
      return sid;
    },
    local_write: async (sid, data) => {
      shells.get(sid)?.input(data);
    },
    local_write_bytes: async () => {},
    local_resize: async () => {},
    local_close: async (sid) => {
      shells.get(sid)?.stop();
      shells.delete(sid);
    },
    ssh_stats_start: async (sid, onEvent) => {
      const shell = shells.get(sid) ?? fail("not_found", "session");
      const statsId = ++lastStatsId;
      await delay(300);
      if (shell.os === "freebsd" || shell.os === "windows") {
        onEvent({ kind: "unsupported", system: shell.os === "freebsd" ? "FreeBSD" : "Windows" });
        return statsId;
      }
      // Like the backend: a start that a later one overtook does not run.
      if ((statsTimers.get(sid)?.id ?? 0) > statsId) return statsId;
      stopStats(sid);
      const sample = mockStats();
      onEvent({ kind: "stats", stats: sample(true) });
      statsTimers.set(sid, {
        id: statsId,
        timer: setInterval(() => onEvent({ kind: "stats", stats: sample(false) }), 2000),
      });
      return statsId;
    },
    ssh_stats_stop: async (sid, statsId) => {
      if (statsTimers.get(sid)?.id === statsId) stopStats(sid);
    },
    ssh_suggestions: async (sid) => {
      needUnlocked();
      const shell = shells.get(sid) ?? fail("not_found", "session");
      await delay(200);
      if (shell.os === "freebsd" || shell.os === "windows") return null;
      return { history: D.SERVER_HISTORY, commands: D.SERVER_COMMANDS };
    },
    ssh_test: async (input) => {
      await delay(800);
      if (input.address === "172.31.40.8")
        return {
          ok: false,
          latency_ms: null,
          host_key_verified: false,
          error: appError("ssh", "timed out", { ssh_kind: "timeout" }),
        };
      return { ok: true, latency_ms: 38, host_key_verified: true, error: null };
    },
    hostkey_respond: async (rid, accept) => {
      pendingHostKeys.get(rid)?.(accept);
      pendingHostKeys.delete(rid);
    },
    auth_prompt_respond: async (rid, answers) => {
      pendingAuth.get(rid)?.(answers);
      pendingAuth.delete(rid);
    },

    forwards_list_all: async () => [...forwards],
    forward_save: async (input) => {
      const f: ForwardView = {
        ...input,
        id: input.id ?? id("f"),
        name: input.name.trim(),
        bind_address: input.bind_address || (input.kind === "remote" ? "localhost" : "127.0.0.1"),
        ...(input.kind === "dynamic" && { dest_host: "", dest_port: 0 }),
      };
      forwards = input.id ? forwards.map((x) => (x.id === f.id ? f : x)) : [...forwards, f];
      touch();
      // A running forward takes its rule's name.
      const running = connections.get(f.host_id)?.forwards.get(f.id);
      if (running && running.name !== f.name) {
        running.name = f.name;
        connectionsChanged();
      }
      return f;
    },
    forward_delete: async (fid) => {
      const f = forwards.find((x) => x.id === fid);
      forwards = forwards.filter((x) => x.id !== fid);
      touch();
      if (f) stopForward(f.host_id, fid);
    },
    forward_start: async (hostId, fid, options) => {
      needUnlocked();
      const h = state.hosts.find((x) => x.id === hostId) ?? fail("not_found", "host not found");
      const f = forwards.find((x) => x.id === fid && x.host_id === hostId) ?? fail("not_found", "forward not found");
      let c = connections.get(hostId);
      if (!c) {
        if (h.auth_kind === "ask" && !options.password)
          fail("invalid_input", "a password is required for this host", { field: "password" });
        await delay(600);
        c = openConnection(hostId, h.name);
      }
      const port = c.forwards.get(fid)?.port ?? runForward(c, f);
      connectionsChanged();
      return port;
    },
    forward_stop: async (hostId, fid) => stopForward(hostId, fid),
    connections_list: async () => connectionList(),
    sftp_home: async () => "/srv/api",
    sftp_list: async (_sid, path) => {
      await delay(120);
      if (path === "/srv/api") return D.API_FILES;
      if (path === "/home/deploy") return D.HOME_FILES;
      return [
        {
          name: "README",
          path: `${path.replace(/\/$/, "")}/README`,
          is_dir: false,
          is_symlink: false,
          size: 120,
          modified: Date.now(),
          permissions: "-rw-r--r--",
        },
      ] satisfies FileEntry[];
    },
    sftp_download: async (sid, remote) => startTransfer(sid, "download", remote.split("/").pop() ?? remote, 18_400_000),
    sftp_upload: async (sid, local) => startTransfer(sid, "upload", local.split(/[\\/]/).pop() ?? local, 61_600_000),
    sftp_rename: async () => {},
    sftp_remove: async () => {},
    sftp_mkdir: async () => {},
    transfer_cancel: async (tid) => {
      transfers.get(tid)?.();
    },
    ...edits,

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
    zmodem_directory: async (path) => {
      if (path !== null) localStorage.setItem("hatoba-zmodem-directory", path);
      return localStorage.getItem("hatoba-zmodem-directory") || "/home/demo/Downloads";
    },
    zmodem_download_begin: async () => {
      throw new Error("ZMODEM disk writes require the desktop app");
    },
    zmodem_download_write: async () => {},
    zmodem_download_finish: async () => null,
    save_text_file: async (path, contents) => {
      // Browser fallback: download the text instead of writing to `path`.
      const a = document.createElement("a");
      a.href = URL.createObjectURL(new Blob([contents], { type: "text/plain" }));
      a.download = path.split(/[\\/]/).pop() || "hatoba.txt";
      a.click();
      URL.revokeObjectURL(a.href);
    },

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

  function startTransfer(sid: string, direction: "upload" | "download", name: string, total: number) {
    const tid = id("t");
    let bytes = 0;
    const rate = 3_100_000;
    const timer = setInterval(() => {
      bytes = Math.min(total, bytes + rate / 5);
      const done = bytes >= total;
      emit("transfer://progress", {
        transfer_id: tid,
        session_id: sid,
        direction,
        name,
        bytes,
        total,
        bytes_per_sec: rate,
        state: done ? "done" : "running",
        error: null,
      });
      if (done) clearInterval(timer);
    }, 200);
    transfers.set(tid, () => {
      clearInterval(timer);
      emit("transfer://progress", {
        transfer_id: tid,
        session_id: sid,
        direction,
        name,
        bytes,
        total,
        bytes_per_sec: 0,
        state: "cancelled",
        error: null,
      });
    });
    return tid;
  }
}

const pendingHostKeys = new Map<string, (accept: boolean) => void>();
const pendingAuth = new Map<string, (answers: string[] | null) => void>();
const transfers = new Map<string, () => void>();
