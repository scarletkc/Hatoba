import type { FrameHandler, HatobaApi, Unlisten } from "../api";
import type {
  AppError,
  EventMap,
  FileEntry,
  ForwardView,
  GroupView,
  HostView,
  KeyView,
  LocalPrefs,
  QuickTarget,
  SshErrorKind,
  StarPrompt,
  SyncStatus,
  VaultStatus,
} from "../types";
import { sameTarget } from "@/features/hosts/quickConnect";
import { FRAME_CLOSED, FRAME_DATA, FRAME_SESSION } from "../types";
import { createAiMock } from "./ai";
import { createAiExtensionsMock } from "./aiExtensions";
import { createAiSettingsMock } from "./aiSettings";
import * as D from "./data";
import { FakeShell } from "./shell";

/** `?ssh=<kind>`: every connection fails with this SSH error kind and a detail like the real one. */
const SSH_DEMO_DETAIL: Partial<Record<SshErrorKind, (address: string, port: number) => string>> & Record<"timeout", (address: string, port: number) => string> = {
  timeout: (a, p) => `connect to ${a}:${p}: operation timed out`,
  refused: (a, p) => `connect to ${a}:${p}: connection refused`,
  dns: (a) => `failed to lookup address information: ${a}: nodename nor servname provided, or not known`,
  unreachable: (a, p) => `connect to ${a}:${p}: network is unreachable`,
  auth_failed: () => "authentication failed: no method succeeded (tried publickey, password)",
  disconnected: () => "the server closed the connection during the handshake",
};

/**
 * In-browser stand-in for the Rust backend so the UI can be developed and reviewed with the
 * design's sample data (`pnpm dev`). URL parameters select demo states:
 *   ?state=onboarding | locked | empty      ?sync=none | syncing | offline | conflict | auth
 *   ?platform=windows | macos | linux       ?update=available | offline | error
 *   ?deploy=fail | waiting | vault | foreign | nosub | accounts | permission | nobundle
 *   ?star=due
 *   ?worker=available | required | custom | app
 *   ?ssh=timeout | refused | dns | unreachable | auth_failed | disconnected
 * Quick connect (HOST-12) asks to trust a new host key once per address, then for the password
 * (anything but "wrong" is accepted); targets whose address contains "timeout" fail with a timeout.
 * Connecting to staging-web-02 always fails with a timeout; with `?ssh`, every host fails with that
 * SSH error kind (`?ssh=fail` or another value: a timeout), so the error card shows (SSH-05).
 * Without `?update`, the update check finds no release, as GitHub does before the first one.
 * The in-app deployment accepts any API token of 20 or more characters.
 * With `?star=due`, the star prompt's day has passed, so it shows after the first connection.
 * `?worker` shows a Worker update notice (§6.7, Upgrades): `available` on a Worker the app
 * deployed, `required` on one deployed another way (sync paused), `custom` on a custom domain,
 * and `app` for a Worker that needs a newer Hatoba. "Update Worker" honours `?deploy=fail`,
 * `waiting`, `permission`, `nobundle`, and `accounts`, and also `missing`, `novault`, `newer`, and
 * `foreign` for the review.
 * Nothing here is secure; it never runs inside the Tauri app.
 */
export function createMockApi(): HatobaApi {
  const q = new URLSearchParams(location.search);
  const demo = q.get("state");
  const syncDemo = q.get("sync");
  const updateDemo = q.get("update");
  const deployDemo = q.get("deploy");
  const sshDemo = q.get("ssh");
  const workerDemo = q.get("worker");
  const version = "0.1.0-dev";
  /** The Worker version this build "bundles". */
  const BUNDLED_WORKER = "0.2.0";

  let hosts: HostView[] = demo === "empty" ? [] : D.HOSTS.map((h) => ({ ...h }));
  let groups: GroupView[] = demo === "empty" ? [] : D.GROUPS.map((g) => ({ ...g }));
  let keys: KeyView[] = demo === "empty" ? [] : D.KEYS.map((k) => ({ ...k }));
  let settings = structuredClone(D.SETTINGS);
  let prefs: LocalPrefs = loadPrefs();
  const starPrompt: StarPrompt = { first_seen_at: q.get("star") === "due" ? Date.now() - 2 * 86_400_000 : Date.now(), done: false };
  let conflicts = syncDemo === "conflict" ? [...D.CONFLICTS] : [];
  let devices = [...D.DEVICES];
  let forwards: ForwardView[] = demo === "empty" ? [] : D.FORWARDS.map((f) => ({ ...f }));
  let recentTargets: QuickTarget[] = demo === "empty" ? [] : D.RECENT_TARGETS.map((r) => ({ ...r }));
  const trustedQuickHosts = new Set(recentTargets.map((r) => `${r.address}:${r.port}`));
  const activeForwards = new Map<string, number>();
  const dropSessionForwards = (sid: string) => {
    for (const key of [...activeForwards.keys()]) if (key.startsWith(`${sid}/`)) activeForwards.delete(key);
  };
  let vaultState: VaultStatus["state"] =
    demo === "onboarding" ? "uninitialized" : demo === "locked" ? "locked" : "unlocked";
  let biometric = q.get("hello") !== "off";
  let failed = 0;
  let retryAt: number | null = null;
  let sync: SyncStatus = makeSync();
  let deployment: { handle: string; failed: boolean; waited: boolean; url: string | null; upgrade: boolean } | null = null;
  const listeners = new Map<string, Set<(p: unknown) => void>>();
  const shells = new Map<string, FakeShell>();
  let seq = 0;

  function makeSync(): SyncStatus {
    const s = D.syncStatus();
    if (demo === "empty" || syncDemo === "none") return { ...s, kind: "none", state: "off", endpoint: null, database: null, last_synced_at: null, counts: null };
    if (syncDemo === "syncing") return { ...s, state: "syncing", pending: 3 };
    if (syncDemo === "offline") return { ...s, state: "offline", pending: 3 };
    if (syncDemo === "auth") return { ...s, state: "auth_failed" };
    if (syncDemo === "conflict") return { ...s, conflicts: D.CONFLICTS.length };
    if (workerDemo === "available") return { ...s, worker_update: { kind: "available", version: "0.1.0", bundled: BUNDLED_WORKER } };
    if (workerDemo === "required" || workerDemo === "custom")
      return {
        ...s,
        state: "paused",
        pending: 3,
        endpoint: workerDemo === "custom" ? "sync.example.com" : s.endpoint,
        worker_update: { kind: "required", version: "0.0.9", bundled: deployDemo === "nobundle" ? null : BUNDLED_WORKER },
      };
    if (workerDemo === "app") return { ...s, state: "paused", pending: 3, worker_update: { kind: "app_required", version: "1.0.0", bundled: BUNDLED_WORKER } };
    return s;
  }

  function loadPrefs(): LocalPrefs {
    try {
      return { ...D.PREFS, ...JSON.parse(localStorage.getItem("hatoba.mock.prefs") ?? "{}") };
    } catch {
      return { ...D.PREFS };
    }
  }

  function emit<K extends keyof EventMap>(event: K, payload: EventMap[K]) {
    listeners.get(event)?.forEach((h) => h(payload));
  }

  function fail(code: AppError["code"], detail: string = code, extra: Partial<AppError> = {}): never {
    throw { code, detail, ...extra } satisfies AppError;
  }

  function needUnlocked() {
    if (vaultState !== "unlocked") fail("locked");
  }

  function needDeployment(handle: string) {
    if (deployment?.handle !== handle) fail("cancelled", "the deployment has ended");
    return deployment;
  }

  const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const id = (p: string) => `${p}-${Date.now().toString(36)}-${(++seq).toString(36)}`;

  function keyViews(): KeyView[] {
    return keys.map((k) => ({ ...k, used_by: hosts.filter((h) => h.auth_kind === "key" && h.key_id === k.id).map((h) => h.id) }));
  }

  function setSync(patch: Partial<SyncStatus>) {
    sync = { ...sync, ...patch };
    emit("sync://status", sync);
  }

  function touch() {
    if (sync.kind === "none") return;
    if (sync.state === "paused") return setSync({ pending: sync.pending + 1 });
    setSync({ state: "syncing", pending: sync.pending + 1 });
    setTimeout(() => setSync({ state: "idle", pending: 0, last_synced_at: Date.now() }), 900);
  }

  /** The Worker answers with the bundled version: the notice goes, and a paused sync pushes what waited. */
  function upgraded() {
    const paused = sync.state === "paused";
    setSync({ worker_update: null, state: paused ? "syncing" : sync.state });
    if (paused) setTimeout(() => setSync({ state: "idle", pending: 0, last_synced_at: Date.now() }), 900);
  }

  const recovery = "K7QF-2M9X-PL4D-8WRT-H3ZN-6VBE-Q1MA-7TCY";

  const aiSettings = createAiSettingsMock();
  const aiExtensions = createAiExtensionsMock((event, payload) => emit(event, payload), {
    version,
    builtinEnabled: async () => (await aiSettings.ai_settings_get()).builtin_skill_enabled,
  });
  return {
    ...aiSettings,
    ...aiExtensions,
    ...createAiMock({
      providers: aiSettings.ai_providers_list,
      settings: aiSettings.ai_settings_get,
      hosts: async () => hosts,
      mcp: { servers: aiExtensions.mcp_servers_list, status: aiExtensions.mcp_server_status, start: aiExtensions.mcp_server_start, toolInfo: aiExtensions.mcp_tool_info },
    }),
    app_info: async () => {
      const p = q.get("platform");
      const platform = p === "macos" || p === "linux" || p === "windows" ? p : "windows";
      return { version, platform, mica: false };
    },
    update_check: async () => {
      await delay(800);
      if (updateDemo === "offline") fail("sync_offline", "GitHub could not be reached");
      if (updateDemo === "error") fail("internal", "GitHub answered HTTP 403");
      if (updateDemo === "available")
        return { current_version: version, latest_version: "0.2.0", release_url: "https://github.com/scarletkc/Hatoba/releases/tag/v0.2.0", update_available: true };
      return { current_version: version, latest_version: null, release_url: null, update_available: false };
    },

    vault_status: async () => ({
      state: vaultState,
      failed_attempts: failed,
      retry_at: retryAt,
      sync_kind: sync.kind,
      biometric_available: true,
      biometric_enabled: biometric,
    }),
    vault_create: async (password) => {
      await delay(600);
      if (password.length < 8) fail("invalid_input", "password too short", { field: "password" });
      vaultState = "unlocked";
      hosts = [];
      groups = [];
      keys = [];
      sync = { ...sync, kind: "none", state: "off", endpoint: null, database: null, last_synced_at: null, counts: null };
      return recovery;
    },
    vault_unlock: async (password) => {
      if (retryAt && Date.now() < retryAt) fail("throttled", "throttled", { retry_at: retryAt });
      await delay(450);
      if (password !== "hatoba" && password.length < 4) {
        failed++;
        if (failed >= 3) retryAt = Date.now() + 2 ** (failed - 3) * 1000;
        fail("wrong_password");
      }
      failed = 0;
      retryAt = null;
      vaultState = "unlocked";
    },
    vault_unlock_biometric: async () => {
      await delay(700);
      vaultState = "unlocked";
    },
    vault_lock: async () => {
      vaultState = "locked";
    },
    vault_verify_password: async (password) => password.length >= 4,
    vault_change_password: async (current) => {
      await delay(500);
      if (current.length < 4) fail("wrong_password");
    },
    vault_recover: async (code) => {
      await delay(500);
      if (code.replace(/[-\s]/g, "").length !== 32) fail("wrong_recovery_code");
      vaultState = "unlocked";
    },
    vault_restore_from_cloud: async (_config, password) => {
      await delay(1200);
      if (password.length < 4) fail("wrong_password");
      hosts = D.HOSTS.map((h) => ({ ...h }));
      groups = D.GROUPS.map((g) => ({ ...g }));
      keys = D.KEYS.map((k) => ({ ...k }));
      sync = D.syncStatus();
      vaultState = "unlocked";
    },
    vault_export_backup: async () => {
      await delay(300);
    },
    vault_rotate_recovery: async (password) => {
      if (password.length < 4) fail("wrong_password");
      return "M2QX-9KPL-D4WR-T8ZN-H36V-BEQ1-MA7T-CYK7";
    },
    biometric_enable: async (password) => {
      await delay(300);
      if (password.length < 4) fail("wrong_password");
      biometric = true;
    },
    biometric_disable: async () => {
      biometric = false;
    },
    activity_ping: async () => {},

    hosts_list: async () => {
      needUnlocked();
      return hosts.map((h) => ({ ...h }));
    },
    host_get: async (hid) => {
      const h = hosts.find((x) => x.id === hid);
      if (!h) fail("not_found");
      return { ...h };
    },
    host_save: async (input) => {
      needUnlocked();
      if (!input.name.trim()) fail("invalid_input", "name is required", { field: "name" });
      if (!input.address.trim()) fail("invalid_input", "address is required", { field: "address" });
      if ([...input.ai_notes].length > 2_000) fail("invalid_input", "AI notes are limited to 2,000 characters", { field: "ai_notes" });
      const existing = input.id ? hosts.find((h) => h.id === input.id) : undefined;
      const view: HostView = {
        id: existing?.id ?? id("h"),
        name: input.name.trim(),
        address: input.address.trim(),
        port: input.port || 22,
        username: input.username.trim(),
        auth_kind: input.auth_kind,
        has_password: input.auth_kind === "password" && (input.password !== null ? input.password.length > 0 : !!existing?.has_password),
        key_id: input.auth_kind === "key" ? input.key_id : null,
        group_id: input.group_id,
        tags: input.tags,
        favorite: input.favorite,
        jump_host_id: input.jump_host_id,
        note: input.note,
        ai_notes: input.ai_notes,
        updated_at: Date.now(),
        last_connected_at: existing?.last_connected_at ?? null,
        os: existing?.os ?? null,
      };
      hosts = existing ? hosts.map((h) => (h.id === view.id ? view : h)) : [...hosts, view];
      touch();
      return view;
    },
    host_delete: async (hid) => {
      hosts = hosts.filter((h) => h.id !== hid).map((h) => (h.jump_host_id === hid ? { ...h, jump_host_id: null } : h));
      touch();
    },
    host_duplicate: async (hid) => {
      const h = hosts.find((x) => x.id === hid);
      if (!h) fail("not_found");
      const copy = { ...h, id: id("h"), name: `${h.name}-copy`, favorite: false, last_connected_at: null, os: null };
      hosts = [...hosts, copy];
      touch();
      return copy;
    },
    host_set_favorite: async (hid, favorite) => {
      hosts = hosts.map((h) => (h.id === hid ? { ...h, favorite } : h));
      touch();
    },
    host_copy_password: async () => {},
    groups_list: async () => groups.map((g) => ({ ...g })),
    group_save: async (input) => {
      const g: GroupView = { id: input.id ?? id("g"), name: input.name, parent_id: input.parent_id, sort: input.sort };
      groups = input.id ? groups.map((x) => (x.id === g.id ? g : x)) : [...groups, g];
      touch();
      return g;
    },
    group_delete: async (gid) => {
      groups = groups.filter((g) => g.id !== gid);
      hosts = hosts.map((h) => (h.group_id === gid ? { ...h, group_id: null } : h));
      touch();
    },
    tags_list: async () => {
      const counts = new Map<string, number>();
      hosts.forEach((h) => h.tags.forEach((t) => counts.set(t, (counts.get(t) ?? 0) + 1)));
      return [...counts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([name, count]) => ({ name, count }));
    },
    hosts_probe: async (ids) => {
      await delay(250);
      return ids.map((hid) => {
        const online = D.PROBE[hid] ?? true;
        return { id: hid, online, latency_ms: online ? 20 + ((hid.length * 7) % 60) : null };
      });
    },
    ssh_config_preview: async () => [
      { alias: "github-runner", address: "10.0.4.20", port: 22, username: "runner", identity_file: "~/.ssh/id_ed25519", proxy_jump: null, exists: false },
      { alias: "bastion-tokyo", address: "bastion.tky.example.net", port: 2222, username: "ops", identity_file: null, proxy_jump: null, exists: true },
      { alias: "minecraft", address: "mc.example.org", port: 22, username: "mc", identity_file: null, proxy_jump: "bastion-tokyo", exists: false },
    ],
    ssh_config_import: async (aliases) => {
      await delay(300);
      aliases.forEach((a) => hosts.push({ ...D.HOSTS[0], id: id("h"), name: a, favorite: false, tags: [], group_id: null, last_connected_at: null, os: null, key_id: null, auth_kind: "ask" }));
      touch();
      return { hosts_created: aliases.length, keys_imported: 0, warnings: [] };
    },

    keys_list: async () => {
      needUnlocked();
      return keyViews();
    },
    key_import: async (input) => {
      await delay(300);
      const text = input.private_key ?? "";
      if (!input.path && !text.includes("PRIVATE KEY") && !text.includes("PuTTY-User-Key-File"))
        fail("key_parse", "unsupported format", { key_kind: "unsupported_format" });
      if (text.includes("ENCRYPTED") && !input.passphrase) fail("key_parse", "passphrase required", { key_kind: "passphrase_required" });
      const k: KeyView = { ...D.KEYS[1], id: id("k"), name: input.name || "imported-key", created_at: Date.now(), updated_at: Date.now(), has_passphrase: !!input.passphrase, used_by: [] };
      keys = [...keys, k];
      touch();
      return k;
    },
    key_generate: async (input) => {
      await delay(input.algorithm === "rsa" ? 1500 : 300);
      const k: KeyView = {
        id: id("k"),
        name: input.name,
        algorithm: input.algorithm,
        bits: input.algorithm === "rsa" ? 4096 : 256,
        public_key: `${input.algorithm === "rsa" ? "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAACAQ" : "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI"}${Math.random().toString(36).slice(2, 30)} ${input.comment}`,
        fingerprint: `SHA256:${btoa(String(Math.random())).replace(/=/g, "").slice(0, 43)}`,
        comment: input.comment,
        has_passphrase: !!input.passphrase,
        created_at: Date.now(),
        updated_at: Date.now(),
        used_by: [],
      };
      keys = [...keys, k];
      touch();
      return k;
    },
    key_rename: async (kid, name) => {
      keys = keys.map((k) => (k.id === kid ? { ...k, name, updated_at: Date.now() } : k));
      touch();
      return keyViews().find((k) => k.id === kid)!;
    },
    key_delete: async (kid) => {
      keys = keys.filter((k) => k.id !== kid);
      hosts = hosts.map((h) => (h.key_id === kid ? { ...h, key_id: null, auth_kind: "ask" } : h));
      touch();
    },
    key_public: async (kid) => keys.find((k) => k.id === kid)?.public_key ?? fail("not_found"),
    key_deploy: async () => {
      await delay(900);
    },

    ssh_connect: async (hostId, _options, onFrame: FrameHandler) => {
      const h = hosts.find((x) => x.id === hostId);
      if (!h) fail("not_found");
      const sid = id("s");
      onFrame(frame(FRAME_SESSION, new TextEncoder().encode(sid)));
      const state = (s: EventMap["ssh://state"]["state"], extra: Partial<EventMap["ssh://state"]> = {}) =>
        emit("ssh://state", { session_id: sid, host_id: hostId, state: s, latency_ms: null, error: null, exit_status: null, ...extra });
      setTimeout(() => state("connecting"), 0);
      if (D.FAILING_HOSTS.has(hostId) || sshDemo) {
        const enc = new TextEncoder();
        for (let i = 1; i <= 3; i++) {
          await delay(350);
          onFrame(frame(FRAME_DATA, enc.encode(`\x1b[90mConnecting to ${h.address}:${h.port} (attempt ${i})…\x1b[0m\r\n`)));
        }
        await delay(400);
        const kind = sshDemo && sshDemo in SSH_DEMO_DETAIL ? (sshDemo as SshErrorKind) : "timeout";
        const detail = (SSH_DEMO_DETAIL[kind] ?? SSH_DEMO_DETAIL.timeout)(h.address, h.port);
        const error: AppError = { code: "ssh", detail, ssh_kind: kind };
        setTimeout(() => state("failed", { error }), 0);
        throw error;
      }
      if (h.name === "homelab-nas") {
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
          const error: AppError = { code: "ssh", detail: "host key rejected", ssh_kind: "host_key_rejected" };
          setTimeout(() => state("failed", { error }), 0);
          throw error;
        }
      }
      await delay(500);
      const shell = new FakeShell(h, (bytes) => onFrame(frame(FRAME_DATA, bytes)));
      shells.set(sid, shell);
      hosts = hosts.map((x) => (x.id === hostId ? { ...x, last_connected_at: Date.now() } : x));
      setTimeout(() => {
        state("connected", { latency_ms: 38 });
        shell.start();
      }, 0);
      shell.onExit = () => {
        onFrame(frame(FRAME_CLOSED, new TextEncoder().encode("exit")));
        state("disconnected", { exit_status: 0 });
        shells.delete(sid);
        dropSessionForwards(sid);
      };
      // FWD-02: like the real backend, auto-start forwards come up with the connection and are announced
      // by events that can fire before the UI has the session id.
      for (const f of forwards.filter((x) => x.host_id === hostId && x.auto_start)) {
        const port = f.bind_port || 49152 + Math.floor(Math.random() * 1000);
        activeForwards.set(`${sid}/${f.id}`, port);
        emit("ssh://forward", { session_id: sid, forward_id: f.id, state: "running", local_port: port, error: null });
      }
      return sid;
    },
    ssh_connect_target: async (target, _options, onFrame: FrameHandler) => {
      needUnlocked();
      const sid = id("s");
      onFrame(frame(FRAME_SESSION, new TextEncoder().encode(sid)));
      const state = (s: EventMap["ssh://state"]["state"], extra: Partial<EventMap["ssh://state"]> = {}) =>
        emit("ssh://state", { session_id: sid, host_id: null, state: s, latency_ms: null, error: null, exit_status: null, ...extra });
      const failWith = (error: AppError): never => {
        setTimeout(() => state("failed", { error }), 0);
        throw error;
      };
      setTimeout(() => state("connecting"), 0);
      await delay(400);
      if (target.address.includes("timeout"))
        failWith({ code: "ssh", detail: SSH_DEMO_DETAIL.timeout(target.address, target.port), ssh_kind: "timeout" });
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
        if (!accepted) failWith({ code: "ssh", detail: "host key rejected", ssh_kind: "host_key_rejected" });
        trustedQuickHosts.add(endpoint);
      }
      // No ssh-agent here: the server asks for the password.
      const answers = await new Promise<string[] | null>((resolve) => {
        const rid = id("ap");
        pendingAuth.set(rid, resolve);
        const host = target.address.includes(":") ? `[${target.address}]` : target.address;
        emit("ssh://auth-prompt", {
          request_id: rid,
          session_id: sid,
          name: target.address,
          instructions: "",
          prompts: [{ prompt: "Password: ", echo: false }],
          password: true,
          target: `${target.username}@${host}:${target.port}`,
        });
      });
      if (!answers) failWith({ code: "ssh", detail: "operation cancelled", ssh_kind: "cancelled" });
      if (answers![0] === "wrong")
        failWith({ code: "ssh", detail: "the server rejected the password (server accepts: publickey, password)", ssh_kind: "auth_failed" });
      await delay(300);
      const shellHost: HostView = { ...D.HOSTS[0], id: "", name: target.address, address: target.address, port: target.port, username: target.username };
      const shell = new FakeShell(shellHost, (bytes) => onFrame(frame(FRAME_DATA, bytes)));
      shells.set(sid, shell);
      recentTargets = [target, ...recentTargets.filter((r) => !sameTarget(r, target))].slice(0, 8);
      setTimeout(() => {
        state("connected", { latency_ms: 21 });
        shell.start();
      }, 0);
      shell.onExit = () => {
        onFrame(frame(FRAME_CLOSED, new TextEncoder().encode("exit")));
        state("disconnected", { exit_status: 0 });
        shells.delete(sid);
      };
      return sid;
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
    ssh_resize: async () => {},
    ssh_disconnect: async (sid) => {
      shells.get(sid)?.stop();
      shells.delete(sid);
      dropSessionForwards(sid);
    },
    ssh_test: async (input) => {
      await delay(800);
      if (input.address === "172.31.40.8")
        return { ok: false, latency_ms: null, host_key_verified: false, error: { code: "ssh", detail: "timed out", ssh_kind: "timeout" } };
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

    forwards_list: async (hostId) => forwards.filter((f) => f.host_id === hostId),
    forward_save: async (input) => {
      const f: ForwardView = { ...input, id: input.id ?? id("f"), bind_address: input.bind_address || "127.0.0.1" };
      forwards = input.id ? forwards.map((x) => (x.id === f.id ? f : x)) : [...forwards, f];
      touch();
      return f;
    },
    forward_delete: async (fid) => {
      forwards = forwards.filter((f) => f.id !== fid);
      touch();
    },
    forward_start: async (sid, fid) => {
      const f = forwards.find((x) => x.id === fid) ?? fail("not_found");
      const port = f.bind_port || 49152 + Math.floor(Math.random() * 1000);
      activeForwards.set(`${sid}/${fid}`, port);
      emit("ssh://forward", { session_id: sid, forward_id: fid, state: "running", local_port: port, error: null });
      return port;
    },
    forward_stop: async (sid, fid) => {
      activeForwards.delete(`${sid}/${fid}`);
      emit("ssh://forward", { session_id: sid, forward_id: fid, state: "stopped", local_port: null, error: null });
    },
    forwards_active: async (sid) =>
      [...activeForwards.entries()].filter(([k]) => k.startsWith(`${sid}/`)).map(([k, port]) => [k.split("/")[1], port] as [string, number]),
    sftp_home: async () => "/srv/api",
    sftp_list: async (_sid, path) => {
      await delay(120);
      if (path === "/srv/api") return D.API_FILES;
      if (path === "/home/deploy") return D.HOME_FILES;
      return [
        { name: "README", path: `${path.replace(/\/$/, "")}/README`, is_dir: false, is_symlink: false, size: 120, modified: Date.now(), permissions: "-rw-r--r--" },
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

    sync_status: async () => sync,
    sync_test: async (config) => {
      await delay(700);
      if (config.kind === "d1") {
        if (config.api_token.length < 20)
          return { ok: false, initialized: false, version: null, latency_ms: null, databases: null, error: { code: "sync", detail: "token lacks D1 edit permission" } };
        return { ok: true, initialized: demo === "onboarding", version: null, latency_ms: 64, databases: [{ id: "3f8a2c91-d04e-4b7a-a1e6-c5d2b9f07e13", name: "hatoba", region: "APAC" }], error: null };
      }
      if (!/^https?:\/\//.test(config.url) && !config.url.includes("."))
        return { ok: false, initialized: false, version: null, latency_ms: null, databases: null, error: { code: "sync_offline", detail: "dns" } };
      // First-launch demo (flow B) restores from a remote that already holds a vault.
      return { ok: true, initialized: demo === "onboarding", version: "0.1.0", latency_ms: 64, databases: null, error: null };
    },
    sync_configure: async (config, password) => {
      await delay(1200);
      if (password.length < 4) fail("wrong_password");
      sync = {
        ...D.syncStatus(),
        endpoint: config.kind === "worker" ? config.url.replace(/^https?:\/\//, "") : null,
        database: config.kind === "d1" ? "hatoba" : "hatoba · APAC",
        last_synced_at: Date.now(),
        counts: { hosts: hosts.length, keys: keys.length, groups: groups.length },
      };
      emit("sync://status", sync);
    },
    sync_now: async () => {
      if (sync.kind === "none") return;
      const paused = sync.state === "paused";
      setSync({ state: "syncing" });
      await delay(900);
      // A paused round only rereads /v1/health.
      if (paused) return setSync({ state: "paused" });
      setSync({ state: "idle", pending: 0, last_synced_at: Date.now() });
    },
    sync_login: async (password) => {
      await delay(600);
      if (password.length < 4) fail("wrong_password");
      setSync({ state: "idle", last_synced_at: Date.now() });
    },
    sync_set_auto: async (enabled) => setSync({ auto_sync: enabled }),
    sync_check_worker: async () => {
      await delay(300);
      emit("sync://status", sync);
    },
    sync_dismiss_worker_update: async () => {
      if (sync.worker_update?.kind === "available") setSync({ worker_update: null });
    },
    sync_disconnect: async () => {
      sync = { ...sync, kind: "none", state: "off", endpoint: null, database: null, last_synced_at: null, counts: null, conflicts: 0 };
      emit("sync://status", sync);
    },
    sync_devices: async () => devices,
    sync_revoke_device: async (did) => {
      devices = devices.filter((d) => d.device_id !== did);
    },
    sync_conflicts: async () => conflicts,
    sync_conflict_resolve: async (cid) => {
      conflicts = conflicts.filter((c) => c.id !== cid);
      setSync({ conflicts: conflicts.length });
    },

    deploy_start: async (token, accountId) => {
      await delay(700);
      if (deployDemo === "nobundle") fail("no_worker_bundle");
      if (token.length < 20) fail("cloudflare_token");
      deployment = { handle: id("deploy"), failed: false, waited: false, url: null, upgrade: false };
      const accounts =
        deployDemo === "accounts"
          ? [
              { id: "0123456789abcdef0123456789abcdef", name: "Personal" },
              { id: "fedcba9876543210fedcba9876543210", name: "Work" },
            ]
          : accountId
            ? []
            : [{ id: "0123456789abcdef0123456789abcdef", name: "Personal" }];
      return { handle: deployment.handle, account_owned: !!accountId && deployDemo !== "accounts", accounts, worker_name: "hatoba-sync", database_name: "hatoba" };
    },
    deploy_inspect: async (handle, target) => {
      needDeployment(handle);
      await delay(800);
      if (deployDemo === "permission") fail("cloudflare_permission", "token lacks D1 Edit", { permission: "d1" });
      const fresh = target.worker_name !== "hatoba-sync";
      return {
        subdomain: deployDemo === "nosub" ? null : "kc",
        worker: fresh ? "create" : deployDemo === "vault" ? "has_vault" : deployDemo === "foreign" ? "foreign" : "create",
        database: "create",
        database_name: target.database_name,
      };
    },
    deploy_run: async (handle, target, onProgress) => {
      const d = needDeployment(handle);
      const steps = ["inspect", "create_database", "migrate", "upload", "setup_token", "route", "wait"] as const;
      for (const step of steps) {
        onProgress({ step, status: "running" });
        await delay(step === "wait" ? 1500 : 600);
        if (step === "upload" && deployDemo === "fail" && !d.failed) {
          d.failed = true;
          fail("cloudflare", "HTTP 500, code 10013", { cf_code: 10013 });
        }
        if (step === "wait" && deployDemo === "waiting" && !d.waited) {
          d.waited = true;
          d.url = `https://${target.worker_name}.${target.subdomain ?? "kc"}.workers.dev`;
          return { url: d.url, ready: false };
        }
        onProgress({ step, status: d.failed && step === "create_database" ? "skipped" : "done" });
      }
      d.url = `https://${target.worker_name}.${target.subdomain ?? "kc"}.workers.dev`;
      return { url: d.url, ready: true };
    },
    deploy_check: async (handle) => {
      const d = needDeployment(handle);
      await delay(900);
      if (d.upgrade) upgraded();
      return true;
    },
    deploy_cleanup: async (handle) => {
      const d = needDeployment(handle);
      await delay(900);
      d.failed = false;
    },
    deploy_cancel: async (handle) => {
      if (deployment?.handle === handle) deployment = null;
    },
    deploy_setup: async (handle, password) => {
      const d = needDeployment(handle);
      await delay(1200);
      if (password.length < 4) fail("wrong_password");
      sync = {
        ...D.syncStatus(),
        endpoint: (d.url ?? "").replace(/^https?:\/\//, ""),
        last_synced_at: Date.now(),
        counts: { hosts: hosts.length, keys: keys.length, groups: groups.length },
      };
      deployment = null;
      emit("sync://status", sync);
    },

    deploy_upgrade_defaults: async () => {
      if (sync.kind !== "worker") fail("sync", "sync does not use a Worker");
      const url = `https://${sync.endpoint}`;
      const workersDev = url.endsWith(".workers.dev");
      return workerDemo === "available"
        ? { url, account_id: "0123456789abcdef0123456789abcdef", worker_name: "hatoba-sync", deployed_by_app: true }
        : { url, account_id: null, worker_name: workersDev ? (sync.endpoint ?? "").split(".")[0] : null, deployed_by_app: false };
    },
    deploy_upgrade_inspect: async (handle, target) => {
      needDeployment(handle);
      await delay(800);
      if (deployDemo === "permission") fail("cloudflare_permission", "token lacks D1 Edit", { permission: "d1" });
      const version = sync.worker_update?.version ?? null;
      const stop = target.worker_name !== "hatoba-sync" ? "missing" : ({ missing: "missing", novault: "no_vault", newer: "newer", foreign: "foreign" } as const)[deployDemo ?? ""];
      return {
        worker: stop ?? "upgrade",
        database_name: stop ? null : "hatoba",
        migrations: 1,
        route: !!sync.endpoint?.endsWith(".workers.dev"),
        version: stop === "newer" ? "0.3.0" : version,
        bundled: BUNDLED_WORKER,
      };
    },
    deploy_upgrade: async (handle, _target, onProgress) => {
      const d = needDeployment(handle);
      d.upgrade = true;
      const route = !!sync.endpoint?.endsWith(".workers.dev");
      for (const step of ["inspect", "migrate", "upload", "route", "wait"] as const) {
        if (step === "route" && !route) {
          onProgress({ step, status: "skipped" });
          continue;
        }
        onProgress({ step, status: "running" });
        await delay(step === "wait" ? 1500 : 600);
        if (step === "upload" && deployDemo === "fail" && !d.failed) {
          d.failed = true;
          fail("cloudflare", "HTTP 500, code 10013", { cf_code: 10013 });
        }
        if (step === "wait" && deployDemo === "waiting" && !d.waited) {
          d.waited = true;
          return { url: `https://${sync.endpoint}`, ready: false };
        }
        onProgress({ step, status: d.failed && step === "migrate" ? "skipped" : "done" });
      }
      upgraded();
      return { url: `https://${sync.endpoint}`, ready: true };
    },

    settings_get: async () => structuredClone(settings),
    settings_save: async (next) => {
      settings = structuredClone(next);
      touch();
    },
    prefs_get: async () => prefs,
    prefs_save: async (next) => {
      prefs = next;
      localStorage.setItem("hatoba.mock.prefs", JSON.stringify(next));
    },
    star_prompt_get: async () => ({ ...starPrompt }),
    star_prompt_done: async () => {
      starPrompt.done = true;
    },

    window_snap_overlay: async () => {},
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
      emit("transfer://progress", { transfer_id: tid, session_id: sid, direction, name, bytes, total, bytes_per_sec: rate, state: done ? "done" : "running", error: null });
      if (done) clearInterval(timer);
    }, 200);
    transfers.set(tid, () => {
      clearInterval(timer);
      emit("transfer://progress", { transfer_id: tid, session_id: sid, direction, name, bytes, total, bytes_per_sec: 0, state: "cancelled", error: null });
    });
    return tid;
  }
}

const pendingHostKeys = new Map<string, (accept: boolean) => void>();
const pendingAuth = new Map<string, (answers: string[] | null) => void>();
const transfers = new Map<string, () => void>();

function frame(tag: number, payload: Uint8Array): Uint8Array {
  const out = new Uint8Array(payload.length + 1);
  out[0] = tag;
  out.set(payload, 1);
  return out;
}
