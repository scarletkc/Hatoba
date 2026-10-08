import type {
  AppError,
  AppInfo,
  ConflictView,
  ConnectOptions,
  DeviceView,
  EventMap,
  FileEntry,
  ForwardInput,
  ForwardView,
  GroupInput,
  GroupView,
  HostInput,
  HostView,
  ImportResult,
  KeyGenerateInput,
  KeyImportInput,
  KeyView,
  LocalPrefs,
  ProbeResult,
  SettingsView,
  SshConfigCandidate,
  DeployOutcome,
  DeployPlan,
  DeployProgress,
  DeployStart,
  DeployTarget,
  SyncConfigInput,
  SyncStatus,
  SyncTestResult,
  TagCount,
  TestResult,
  UpdateCheck,
  VaultStatus,
} from "./types";

/** Receives raw terminal frames (see FRAME_* in types.ts). */
export type FrameHandler = (frame: Uint8Array) => void;
export type Unlisten = () => void;

/**
 * Everything the UI can ask the backend to do (spec §10.1). Each method maps 1:1 to a Tauri
 * command of the same snake_case name. Methods reject with {@link AppError}.
 */
export interface HatobaApi {
  app_info(): Promise<AppInfo>;
  /** Asks GitHub Releases for the newest stable release. A repository without releases is up to date. */
  update_check(): Promise<UpdateCheck>;

  // vault
  vault_status(): Promise<VaultStatus>;
  /** Creates the local vault and returns the recovery code, formatted in dash-separated groups. */
  vault_create(password: string): Promise<string>;
  vault_unlock(password: string): Promise<void>;
  vault_unlock_biometric(): Promise<void>;
  vault_lock(): Promise<void>;
  vault_verify_password(password: string): Promise<boolean>;
  vault_change_password(current: string, next: string): Promise<void>;
  /** VAULT-06: reset the master password with the recovery code. Works offline. */
  vault_recover(recovery_code: string, new_password: string): Promise<void>;
  /** Flow B: first launch on a new device, restore the vault from the cloud. */
  vault_restore_from_cloud(config: SyncConfigInput, password: string): Promise<void>;
  /** VAULT-07: write an encrypted backup to `path` (chosen with the save dialog). */
  vault_export_backup(path: string): Promise<void>;
  /** The vault's recovery code cannot be shown again; this rotates it and returns the new one. */
  vault_rotate_recovery(password: string): Promise<string>;
  biometric_enable(password: string): Promise<void>;
  biometric_disable(): Promise<void>;
  /** Resets the idle auto-lock timer (SEC-02). Called on user activity, throttled by the UI. */
  activity_ping(): Promise<void>;

  // hosts
  hosts_list(): Promise<HostView[]>;
  host_get(id: string): Promise<HostView>;
  host_save(input: HostInput): Promise<HostView>;
  host_delete(id: string): Promise<void>;
  host_duplicate(id: string): Promise<HostView>;
  host_set_favorite(id: string, favorite: boolean): Promise<void>;
  /** SEC-08: Rust copies the saved password to the clipboard and clears it after 30 s. */
  host_copy_password(id: string): Promise<void>;
  groups_list(): Promise<GroupView[]>;
  group_save(input: GroupInput): Promise<GroupView>;
  group_delete(id: string): Promise<void>;
  tags_list(): Promise<TagCount[]>;
  hosts_probe(ids: string[]): Promise<ProbeResult[]>;
  ssh_config_preview(): Promise<SshConfigCandidate[]>;
  ssh_config_import(aliases: string[]): Promise<ImportResult>;

  // keys
  keys_list(): Promise<KeyView[]>;
  key_import(input: KeyImportInput): Promise<KeyView>;
  key_generate(input: KeyGenerateInput): Promise<KeyView>;
  key_rename(id: string, name: string): Promise<KeyView>;
  key_delete(id: string): Promise<void>;
  key_public(id: string): Promise<string>;
  /** KEY-06: append the public key to `~/.ssh/authorized_keys` on a host. */
  key_deploy(key_id: string, host_id: string): Promise<void>;

  // ssh
  ssh_connect(host_id: string, options: ConnectOptions, onFrame: FrameHandler): Promise<string>;
  ssh_write(session_id: string, data: string): Promise<void>;
  ssh_resize(session_id: string, cols: number, rows: number): Promise<void>;
  ssh_disconnect(session_id: string): Promise<void>;
  /** Test a (possibly unsaved) host: connect, authenticate, disconnect. */
  ssh_test(input: HostInput): Promise<TestResult>;
  hostkey_respond(request_id: string, accept: boolean): Promise<void>;
  auth_prompt_respond(request_id: string, answers: string[] | null): Promise<void>;

  // port forwarding (FWD-01/02)
  forwards_list(host_id: string): Promise<ForwardView[]>;
  forward_save(input: ForwardInput): Promise<ForwardView>;
  forward_delete(id: string): Promise<void>;
  /** Starts a saved forward on a live session; resolves to the bound local port. */
  forward_start(session_id: string, forward_id: string): Promise<number>;
  forward_stop(session_id: string, forward_id: string): Promise<void>;
  /** `[forward_id, local_port]` pairs running on a session. */
  forwards_active(session_id: string): Promise<[string, number][]>;

  // sftp
  sftp_home(session_id: string): Promise<string>;
  sftp_list(session_id: string, path: string): Promise<FileEntry[]>;
  sftp_download(session_id: string, remote_path: string, local_path: string): Promise<string>;
  sftp_upload(session_id: string, local_path: string, remote_dir: string): Promise<string>;
  sftp_rename(session_id: string, from: string, to: string): Promise<void>;
  sftp_remove(session_id: string, path: string, is_dir: boolean): Promise<void>;
  sftp_mkdir(session_id: string, path: string): Promise<void>;
  transfer_cancel(transfer_id: string): Promise<void>;

  // sync
  sync_status(): Promise<SyncStatus>;
  sync_test(config: SyncConfigInput): Promise<SyncTestResult>;
  /** Flow A: initialise the remote (Setup Token) and push the local vault. Needs the master password. */
  sync_configure(config: SyncConfigInput, password: string): Promise<void>;
  sync_now(): Promise<void>;
  /** Re-authenticate after the session expired or was revoked ("auth_failed"). */
  sync_login(password: string): Promise<void>;
  sync_set_auto(enabled: boolean): Promise<void>;
  sync_disconnect(): Promise<void>;
  sync_devices(): Promise<DeviceView[]>;
  sync_revoke_device(device_id: string): Promise<void>;
  sync_conflicts(): Promise<ConflictView[]>;
  /** `keep`: accept the automatic resolution. `restore`: re-apply the losing version as a new edit. */
  sync_conflict_resolve(id: number, action: "keep" | "restore"): Promise<void>;

  // in-app deployment (§6.7)
  /** Step 1: checks the API token, which stays in Rust from here on. Rejects with `no_worker_bundle` in builds without the Worker. */
  deploy_start(api_token: string, account_id: string | null): Promise<DeployStart>;
  /** Step 2: what the deployment will do, before anything is written. */
  deploy_inspect(handle: string, target: DeployTarget): Promise<DeployPlan>;
  /** Steps 2 to 8. Run it again after a failure to continue. */
  deploy_run(handle: string, target: DeployTarget, onProgress: (p: DeployProgress) => void): Promise<DeployOutcome>;
  /** "Check again" while step 8 waits for the Worker. */
  deploy_check(handle: string): Promise<boolean>;
  /** Deletes only what this deployment created. */
  deploy_cleanup(handle: string): Promise<void>;
  /** Ends the deployment and wipes its tokens. */
  deploy_cancel(handle: string): Promise<void>;
  /** The master password step after a deployment: initialises the new Worker and enables sync. */
  deploy_setup(handle: string, password: string): Promise<void>;

  // settings
  settings_get(): Promise<SettingsView>;
  settings_save(settings: SettingsView): Promise<void>;
  prefs_get(): Promise<LocalPrefs>;
  prefs_save(prefs: LocalPrefs): Promise<void>;

  // window (Windows custom title bar, WIN-01)
  window_snap_overlay(): Promise<void>;
  /** Write a text file to a path the user picked in the native save dialog (recovery code "Save as Text"). */
  save_text_file(path: string, contents: string): Promise<void>;

  // events
  listen<K extends keyof EventMap>(event: K, handler: (payload: EventMap[K]) => void): Promise<Unlisten>;
}

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function isAppError(e: unknown): e is AppError {
  return typeof e === "object" && e !== null && "code" in e && "detail" in e;
}

/** Normalise anything thrown by a command into an {@link AppError}. */
export function toAppError(e: unknown): AppError {
  if (isAppError(e)) return e;
  return { code: "internal", detail: e instanceof Error ? e.message : String(e) };
}

let impl: Promise<HatobaApi> | null = null;

/** Cached promise so concurrent first calls share one backend instance. */
function load(): Promise<HatobaApi> {
  impl ??= isTauri()
    ? import("./tauri").then((m) => m.createTauriApi())
    : import("./mock").then((m) => m.createMockApi());
  return impl;
}

/**
 * The backend. Calls are forwarded lazily so the Tauri or mock implementation is chosen at runtime
 * (the mock lets the UI run in a plain browser with the design's sample data).
 */
export const api: HatobaApi = new Proxy({} as HatobaApi, {
  get(_target, prop: string) {
    return async (...args: unknown[]) => {
      const backend = await load();
      const fn = (backend as unknown as Record<string, (...a: unknown[]) => Promise<unknown>>)[prop];
      if (typeof fn !== "function") throw toAppError(new Error(`unknown command ${prop}`));
      try {
        return await fn.apply(backend, args);
      } catch (e) {
        throw toAppError(e);
      }
    };
  },
});
