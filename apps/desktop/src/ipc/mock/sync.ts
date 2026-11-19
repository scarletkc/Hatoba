import { appError, type HatobaApi } from "../api";
import type { SyncStatus } from "../types";
import * as D from "./data";
import { delay, fail, type MockEmit, type MockState } from "./shared";

type SyncApi = Pick<
  HatobaApi,
  | "sync_status"
  | "sync_test"
  | "sync_configure"
  | "sync_join"
  | "sync_now"
  | "sync_login"
  | "sync_set_auto"
  | "sync_check_worker"
  | "sync_dismiss_worker_update"
  | "sync_disconnect"
  | "sync_devices"
  | "sync_revoke_device"
  | "sync_conflicts"
  | "sync_conflict_resolve"
  | "deploy_start"
  | "deploy_inspect"
  | "deploy_run"
  | "deploy_check"
  | "deploy_cleanup"
  | "deploy_cancel"
  | "deploy_setup"
  | "deploy_join"
  | "deploy_upgrade_defaults"
  | "deploy_upgrade_inspect"
  | "deploy_upgrade"
>;

export interface SyncMock extends SyncApi {
  kind(): SyncStatus["kind"];
  touch(): void;
  resetOff(): void;
  resetRestored(): void;
}

export function createSyncMock(ctx: {
  state: MockState;
  emit: MockEmit;
  id(prefix: string): string;
  demo: string | null;
  syncDemo: string | null;
  deployDemo: string | null;
  workerDemo: string | null;
}): SyncMock {
  /** The Worker version this build "bundles". */
  const BUNDLED_WORKER = "0.2.0";
  const { state, emit, id, demo, syncDemo, deployDemo, workerDemo } = ctx;

  let conflicts = syncDemo === "conflict" ? [...D.CONFLICTS] : [];
  let devices = [...D.DEVICES];
  let sync: SyncStatus = makeSync();
  let deployment: { handle: string; failed: boolean; waited: boolean; url: string | null; upgrade: boolean } | null =
    null;

  function makeSync(): SyncStatus {
    const s = D.syncStatus();
    if (demo === "empty" || syncDemo === "none")
      return { ...s, kind: "none", state: "off", endpoint: null, database: null, last_synced_at: null, counts: null };
    if (syncDemo === "syncing") return { ...s, state: "syncing", pending: 3 };
    if (syncDemo === "offline") return { ...s, state: "offline", pending: 3 };
    if (syncDemo === "auth") return { ...s, state: "auth_failed" };
    if (syncDemo === "conflict") return { ...s, conflicts: D.CONFLICTS.length };
    if (syncDemo === "too_large") {
      const host = D.HOSTS[0];
      return {
        ...s,
        pending: 2,
        too_large: [
          { item_id: host.id, item_type: "host", item_name: host.name },
          { item_id: "settings", item_type: "settings", item_name: "settings" },
        ],
      };
    }
    if (workerDemo === "available")
      return { ...s, worker_update: { kind: "available", version: "0.1.0", bundled: BUNDLED_WORKER } };
    if (workerDemo === "required" || workerDemo === "custom")
      return {
        ...s,
        state: "paused",
        pending: 3,
        endpoint: workerDemo === "custom" ? "sync.example.com" : s.endpoint,
        worker_update: {
          kind: "required",
          version: "0.0.9",
          bundled: deployDemo === "nobundle" ? null : BUNDLED_WORKER,
        },
      };
    if (workerDemo === "app")
      return {
        ...s,
        state: "paused",
        pending: 3,
        worker_update: { kind: "app_required", version: "1.0.0", bundled: BUNDLED_WORKER },
      };
    return s;
  }

  function setSync(patch: Partial<SyncStatus>) {
    sync = { ...sync, ...patch };
    emit("sync://status", sync);
  }

  function touch() {
    if (sync.kind === "none") return;
    if (sync.state === "paused") return setSync({ pending: sync.pending + 1 });
    setSync({ state: "syncing", pending: sync.pending + 1 });
    setTimeout(() => setSync({ state: "idle", pending: 0, last_synced_at: Date.now(), too_large: [] }), 900);
  }

  /** The Worker answers with the bundled version: the notice goes, and a paused sync pushes what waited. */
  function upgraded() {
    const paused = sync.state === "paused";
    setSync({ worker_update: null, state: paused ? "syncing" : sync.state });
    if (paused) setTimeout(() => setSync({ state: "idle", pending: 0, last_synced_at: Date.now() }), 900);
  }

  function needDeployment(handle: string) {
    if (deployment?.handle !== handle) fail("cancelled", "the deployment has ended");
    return deployment;
  }

  const itemCounts = () => ({
    hosts: state.hosts.length,
    keys: state.keys.length,
    groups: state.groups.length,
  });

  return {
    kind: () => sync.kind,
    touch,
    resetOff: () => {
      sync = {
        ...sync,
        kind: "none",
        state: "off",
        endpoint: null,
        database: null,
        last_synced_at: null,
        counts: null,
      };
    },
    resetRestored: () => {
      sync = D.syncStatus();
    },

    sync_status: async () => sync,
    sync_test: async (config) => {
      await delay(700);
      if (config.kind === "d1") {
        if (config.api_token.length < 20)
          return {
            ok: false,
            initialized: false,
            version: null,
            latency_ms: null,
            databases: null,
            error: appError("sync", "token lacks D1 edit permission"),
          };
        return {
          ok: true,
          initialized: demo === "onboarding" || deployDemo === "vault",
          version: null,
          latency_ms: 64,
          databases: [{ id: "3f8a2c91-d04e-4b7a-a1e6-c5d2b9f07e13", name: "hatoba", region: "APAC" }],
          error: null,
        };
      }
      if (!/^https?:\/\//.test(config.url) && !config.url.includes("."))
        return {
          ok: false,
          initialized: false,
          version: null,
          latency_ms: null,
          databases: null,
          error: appError("sync_offline", "dns"),
        };
      // First-launch demo (flow B) restores from a remote that already holds a vault, and so
      // does `?deploy=vault` (flow C).
      return {
        ok: true,
        initialized: demo === "onboarding" || deployDemo === "vault",
        version: "0.1.0",
        latency_ms: 64,
        databases: null,
        error: null,
      };
    },
    sync_configure: async (config, password) => {
      await delay(1200);
      if (password.length < 4) fail("wrong_password");
      sync = {
        ...D.syncStatus(),
        endpoint: config.kind === "worker" ? config.url.replace(/^https?:\/\//, "") : null,
        database: config.kind === "d1" ? "hatoba" : "hatoba · APAC",
        last_synced_at: Date.now(),
        counts: itemCounts(),
      };
      emit("sync://status", sync);
    },
    sync_join: async (config, password) => {
      await delay(1200);
      if (password.length < 4) fail("wrong_password");
      sync = {
        ...D.syncStatus(),
        endpoint: config.kind === "worker" ? config.url.replace(/^https?:\/\//, "") : null,
        database: config.kind === "d1" ? "hatoba" : "hatoba · APAC",
        last_synced_at: Date.now(),
        counts: itemCounts(),
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
      sync = {
        ...sync,
        kind: "none",
        state: "off",
        endpoint: null,
        database: null,
        last_synced_at: null,
        counts: null,
        conflicts: 0,
      };
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
      return {
        handle: deployment.handle,
        account_owned: !!accountId && deployDemo !== "accounts",
        accounts,
        worker_name: "hatoba-sync",
        database_name: "hatoba",
      };
    },
    deploy_inspect: async (handle, target) => {
      needDeployment(handle);
      await delay(800);
      if (deployDemo === "permission") fail("cloudflare_permission", "token lacks D1 Edit", { permission: "d1" });
      const fresh = target.worker_name !== "hatoba-sync";
      return {
        subdomain: deployDemo === "nosub" ? null : "kc",
        worker: fresh
          ? "create"
          : deployDemo === "vault"
            ? "has_vault"
            : deployDemo === "foreign"
              ? "foreign"
              : "create",
        database: "create",
        database_name: target.database_name,
        vault_url: deployDemo === "vault" && !fresh ? `https://${target.worker_name}.kc.workers.dev` : null,
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
        counts: itemCounts(),
      };
      deployment = null;
      emit("sync://status", sync);
    },
    deploy_join: async (handle, target, password) => {
      needDeployment(handle);
      await delay(1200);
      if (password.length < 4) fail("wrong_password");
      sync = {
        ...D.syncStatus(),
        endpoint: `${target.worker_name}.kc.workers.dev`,
        last_synced_at: Date.now(),
        counts: itemCounts(),
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
        : {
            url,
            account_id: null,
            worker_name: workersDev ? (sync.endpoint ?? "").split(".")[0] : null,
            deployed_by_app: false,
          };
    },
    deploy_upgrade_inspect: async (handle, target) => {
      needDeployment(handle);
      await delay(800);
      if (deployDemo === "permission") fail("cloudflare_permission", "token lacks D1 Edit", { permission: "d1" });
      const version = sync.worker_update?.version ?? null;
      const stop =
        target.worker_name !== "hatoba-sync"
          ? "missing"
          : ({ missing: "missing", novault: "no_vault", newer: "newer", foreign: "foreign" } as const)[
              deployDemo ?? ""
            ];
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
  };
}
