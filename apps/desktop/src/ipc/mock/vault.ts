import type { HatobaApi } from "../api";
import type { SyncConfigInput, SyncStatus, VaultStatus } from "../types";
import * as D from "./data";
import { delay, fail, type MockState } from "./shared";

type VaultApi = Pick<
  HatobaApi,
  | "vault_status"
  | "vault_create"
  | "vault_unlock"
  | "vault_unlock_biometric"
  | "vault_unlock_pin"
  | "vault_lock"
  | "vault_verify_password"
  | "vault_change_password"
  | "vault_recover"
  | "vault_restore_from_cloud"
  | "vault_export_backup"
  | "vault_rotate_recovery"
  | "biometric_enable"
  | "biometric_disable"
  | "pin_enable"
  | "pin_disable"
  | "pin_set_persist"
>;

export interface VaultMock extends VaultApi {
  unlocked(): boolean;
  needUnlocked(): void;
}

export function createVaultMock(ctx: {
  state: MockState;
  q: URLSearchParams;
  demo: string | null;
  syncKind(): SyncStatus["kind"];
  resetSyncOff(): void;
  resetSyncRestored(config: SyncConfigInput): void;
  onUnlock(): void;
  onLock(): void;
}): VaultMock {
  const { state, q, demo } = ctx;
  const recovery = "K7QF-2M9X-PL4D-8WRT-H3ZN-6VBE-Q1MA-7TCY";

  let vaultState: VaultStatus["state"] =
    demo === "onboarding" ? "uninitialized" : demo === "locked" ? "locked" : "unlocked";
  let biometric = q.get("hello") !== "off";
  const biometricRefusal = q.get("biometric");
  const pinDemo = q.get("pin");
  const pinState = {
    pin: pinDemo === "on" || pinDemo === "persist" ? "1234" : (null as string | null),
    persist: pinDemo === "persist",
    armed: true,
    failures: 0,
  };
  /** Any full unlock arms the PIN again and resets its failures. */
  const armPin = () => {
    pinState.armed = pinState.pin !== null;
    pinState.failures = 0;
  };
  let failed = 0;
  let retryAt: number | null = null;

  const unlocked = () => vaultState === "unlocked";
  const needUnlocked = () => {
    if (!unlocked()) fail("locked");
  };

  return {
    unlocked,
    needUnlocked,

    vault_status: async () => ({
      state: vaultState,
      failed_attempts: failed,
      retry_at: retryAt,
      sync_kind: ctx.syncKind(),
      biometric_available: true,
      biometric_enabled: biometric,
      pin_enabled: pinState.pin !== null,
      pin_persist: pinState.persist,
      pin_unlock: pinState.pin !== null && pinState.armed,
    }),
    vault_create: async (password) => {
      await delay(600);
      if (password.length < 8) fail("invalid_input", "password too short", { field: "password" });
      vaultState = "unlocked";
      state.hosts = [];
      state.telnetHosts = [];
      state.groups = [];
      state.keys = [];
      ctx.resetSyncOff();
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
      armPin();
      vaultState = "unlocked";
      ctx.onUnlock();
    },
    vault_unlock_biometric: async () => {
      await delay(700);
      if (biometricRefusal === "unavailable") fail("biometric_unavailable", "Touch ID unavailable");
      if (biometricRefusal === "stale") {
        // The backend removes an enrolment that no longer opens.
        biometric = false;
        fail("biometric_stale", "Touch ID key invalidated (fingerprints changed)");
      }
      if (biometricRefusal === "cancelled") fail("cancelled");
      armPin();
      vaultState = "unlocked";
    },
    vault_unlock_pin: async (pin) => {
      if (pinState.pin === null || !pinState.armed) fail("pin_unavailable");
      await delay(450);
      if (pin !== pinState.pin) {
        pinState.failures++;
        const attempts_left = Math.max(0, 5 - pinState.failures);
        if (attempts_left === 0) pinState.armed = false;
        fail("wrong_pin", "wrong PIN", { attempts_left });
      }
      armPin();
      vaultState = "unlocked";
    },
    vault_lock: async () => {
      vaultState = "locked";
      ctx.onLock();
    },
    vault_verify_password: async (password) => password.length >= 4,
    vault_change_password: async (current) => {
      await delay(500);
      if (current.length < 4) fail("wrong_password");
    },
    vault_recover: async (code) => {
      await delay(500);
      if (code.replace(/[-\s]/g, "").length !== 32) fail("wrong_recovery_code");
      armPin();
      vaultState = "unlocked";
    },
    vault_restore_from_cloud: async (config, password) => {
      await delay(1200);
      if (password.length < 4) fail("wrong_password");
      state.hosts = D.HOSTS.map((h) => ({ ...h }));
      state.groups = D.GROUPS.map((g) => ({ ...g }));
      state.keys = D.KEYS.map((k) => ({ ...k }));
      ctx.resetSyncRestored(config);
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
    pin_enable: async (password, pin) => {
      if (pin.length < 4) fail("invalid_input", "the PIN must have at least 4 characters", { field: "pin" });
      await delay(400);
      if (password.length < 4) fail("wrong_password");
      Object.assign(pinState, { pin, persist: false, armed: true, failures: 0 });
    },
    pin_disable: async () => {
      Object.assign(pinState, { pin: null, persist: false, armed: false, failures: 0 });
    },
    pin_set_persist: async (persist) => {
      pinState.persist = persist;
    },
  };
}
