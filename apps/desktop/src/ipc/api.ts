import type {
  AiConversationDetail,
  AiConversationView,
  AiEntryView,
  AiModel,
  AiProviderInput,
  AiProviderView,
  AiSendInput,
  AiSendStarted,
  AiSettingsView,
  AiTestResult,
  AiToolResultInput,
  AiTurnContext,
  AiTurnEvent,
  AiSearchHit,
  AppError,
  AppInfo,
  BuiltinSkillView,
  ConflictView,
  ConnectOptions,
  DeployOutcome,
  DeployPlan,
  DeployProgress,
  DeployStart,
  DeployTarget,
  DeviceView,
  DroppedFile,
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
  McpImportPreview,
  McpServerInput,
  McpServerStatus,
  McpServerView,
  McpToolInfo,
  ProbeResult,
  ProxyInput,
  ProxyView,
  QuickTarget,
  SearchProviderInput,
  SearchProviderView,
  SettingsView,
  SkillDetail,
  SkillImportPreview,
  SkillInput,
  SkillView,
  SshConfigCandidate,
  StarPrompt,
  StatsEvent,
  SyncConfigInput,
  SyncStatus,
  SyncTestResult,
  TagCount,
  TestResult,
  UpdateCheck,
  UpdateProgress,
  UpgradeDefaults,
  UpgradePlan,
  UpgradeTarget,
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
  /** Asks for a newer release (spec §11). Without a release that carries update information, Hatoba is up to date. */
  update_check(): Promise<UpdateCheck>;
  /**
   * Downloads the update the last check found, checks its signature, and installs it. Hatoba then
   * closes and the installer starts the new version, so this settles only when that fails.
   */
  update_install(onProgress: (p: UpdateProgress) => void): Promise<void>;

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
  /** TERM-12: show the server's resource usage in this host's terminals (device-local). */
  host_set_show_stats(id: string, on: boolean): Promise<void>;
  /** SEC-08: Rust copies the saved password to the clipboard and clears it after 30 s. */
  host_copy_password(id: string): Promise<void>;
  groups_list(): Promise<GroupView[]>;
  group_save(input: GroupInput): Promise<GroupView>;
  group_delete(id: string): Promise<void>;
  tags_list(): Promise<TagCount[]>;
  hosts_probe(ids: string[]): Promise<ProbeResult[]>;
  ssh_config_preview(): Promise<SshConfigCandidate[]>;
  ssh_config_import(aliases: string[]): Promise<ImportResult>;

  // proxies (SSH-13)
  proxies_list(): Promise<ProxyView[]>;
  proxy_save(input: ProxyInput): Promise<ProxyView>;
  /** Hosts that named the proxy go back to the device default; this device stops using it as its default. */
  proxy_delete(id: string): Promise<void>;

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
  /** Quick connect (HOST-12): a terminal on a target that is not saved as a host. */
  ssh_connect_target(target: QuickTarget, options: ConnectOptions, onFrame: FrameHandler): Promise<string>;
  /** Recent quick-connect targets on this device, newest first. */
  recent_targets_list(): Promise<QuickTarget[]>;
  /** Forgets a recent target; resolves to what is left. */
  recent_target_remove(target: QuickTarget): Promise<QuickTarget[]>;
  ssh_write(session_id: string, data: string): Promise<void>;
  ssh_resize(session_id: string, cols: number, rows: number): Promise<void>;
  ssh_disconnect(session_id: string): Promise<void>;
  /**
   * TERM-12: reads the server's resource usage every 2 s until `ssh_stats_stop` with the id this
   * resolves to, or the session ends. When starts overlap, the one sent last keeps running.
   */
  ssh_stats_start(session_id: string, onEvent: (event: StatsEvent) => void): Promise<number>;
  /** Stops the sampling started as `stats_id`; a later one keeps running. */
  ssh_stats_stop(session_id: string, stats_id: number): Promise<void>;
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
  /** The status page opened: rereads the Worker's `/v1/health`, which updates `worker_update` and may pause or resume sync. */
  sync_check_worker(): Promise<void>;
  /** Hides "Worker update available" until the app bundles a newer Worker. */
  sync_dismiss_worker_update(): Promise<void>;
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
  /** What "Update Worker" starts from. */
  deploy_upgrade_defaults(): Promise<UpgradeDefaults>;
  /** Step 2 of an upgrade, against the deployment started with `deploy_start`. */
  deploy_upgrade_inspect(handle: string, target: UpgradeTarget): Promise<UpgradePlan>;
  /** "Update Worker": the deployment steps as an upgrade. Run it again after a failure to continue; `deploy_check` asks again while step 8 waits. */
  deploy_upgrade(handle: string, target: UpgradeTarget, onProgress: (p: DeployProgress) => void): Promise<DeployOutcome>;

  // settings
  settings_get(): Promise<SettingsView>;
  settings_save(settings: SettingsView): Promise<void>;
  prefs_get(): Promise<LocalPrefs>;
  prefs_save(prefs: LocalPrefs): Promise<void>;
  /** The sidebar's star prompt (spec §9). The first call records when this device started waiting. */
  star_prompt_get(): Promise<StarPrompt>;
  /** The prompt never shows again. */
  star_prompt_done(): Promise<void>;

  // AI assistant (§13)
  /** AI-01: providers. API keys go one way to Rust and never come back. */
  ai_providers_list(): Promise<AiProviderView[]>;
  ai_provider_save(input: AiProviderInput): Promise<AiProviderView>;
  ai_provider_delete(id: string): Promise<void>;
  /** AI-03: the provider's model list, for a saved or unsaved provider. Rejects with `ai`. */
  ai_provider_models(input: AiProviderInput): Promise<AiModel[]>;
  /** AI-04: a minimal request to the first model, or the model list when there is none. */
  ai_provider_test(input: AiProviderInput): Promise<AiTestResult>;
  /** AI-14: the search providers behind `web_search`. */
  search_providers_list(): Promise<SearchProviderView[]>;
  search_provider_save(input: SearchProviderInput): Promise<SearchProviderView>;
  search_provider_delete(id: string): Promise<void>;
  search_provider_test(input: SearchProviderInput): Promise<AiTestResult>;
  /** The synced default model and search provider. */
  ai_settings_get(): Promise<AiSettingsView>;
  ai_settings_save(settings: AiSettingsView): Promise<void>;
  /** AI-23: every conversation, unsorted. */
  ai_conversations_list(): Promise<AiConversationView[]>;
  ai_conversation_get(id: string): Promise<AiConversationDetail>;
  ai_conversation_rename(id: string, title: string): Promise<AiConversationView>;
  ai_conversation_pin(id: string, pinned: boolean): Promise<AiConversationView>;
  ai_conversation_delete(id: string): Promise<void>;
  /** Stores the user's message and starts a turn; events stream to `onEvent` until `turn_ended` (§13.1). */
  ai_send(input: AiSendInput, onEvent: (event: AiTurnEvent) => void): Promise<AiSendStarted>;
  /** Retry after an error: sends the next request from the stored conversation on a new channel. */
  ai_retry(conversation_id: string, context: AiTurnContext, onEvent: (event: AiTurnEvent) => void): Promise<void>;
  /** Stores a result the frontend produced (`read_terminal`, `send_input`, a rejection). */
  ai_tool_result(conversation_id: string, tool_call_id: string, result: AiToolResultInput): Promise<AiEntryView>;
  /** Runs a tool that runs in Rust (`run_command` on `session_id`, `web_search`, `fetch_url`, ...) and stores its result. */
  ai_tool_run(conversation_id: string, tool_call_id: string, session_id: string | null, edited_arguments: string | null): Promise<AiEntryView>;
  /** Stops the turn: aborts the request and running tools, cancels calls without a result. */
  ai_stop(conversation_id: string): Promise<void>;
  /** AI-21: summarizes the context with the given model and moves `context_start` to the summary. */
  ai_compact(conversation_id: string, context: AiTurnContext): Promise<AiEntryView>;

  /** AI-24: conversations whose title or message text contains `query` (case-insensitive). */
  ai_search(query: string): Promise<AiSearchHit[]>;
  /** AI-26: replaces the user's message `entry_id` with `text`, deletes every entry after it, and starts a turn. */
  ai_edit_resend(conversation_id: string, entry_id: string, text: string, context: AiTurnContext, onEvent: (event: AiTurnEvent) => void): Promise<AiSendStarted>;
  /**
   * AI-35, desktop app: reads text files dropped on the window, which its webview hands over as paths only.
   * Only paths of the window's last drop, each once; any other path refuses the request. One result per path,
   * in order, named by the file's base name.
   */
  ai_read_dropped_files(paths: string[]): Promise<DroppedFile[]>;

  // AI skills (§13.8)
  skills_list(): Promise<SkillView[]>;
  skill_get(id: string): Promise<SkillDetail>;
  /** The built-in `hatoba` skill (AI-34), with this app's version, for the viewer in Settings. */
  skill_builtin_get(): Promise<BuiltinSkillView>;
  /** Validates like an import (AI-27); rejects with `invalid_input` naming the field, also for the reserved name `hatoba`. */
  skill_save(input: SkillInput): Promise<SkillView>;
  skill_delete(id: string): Promise<void>;
  skill_set_enabled(id: string, enabled: boolean): Promise<void>;
  /** Reads a folder or `.zip` the user picked; nothing is saved. */
  skill_import_preview(path: string): Promise<SkillImportPreview>;
  /**
   * Imports what the preview showed: reads the path again and refuses it (`invalid_input`) when it no longer
   * matches the preview's `token`. `replace_id` replaces that skill; `rename` saves it under a new name.
   */
  skill_import(path: string, token: string, replace_id: string | null, rename: string | null): Promise<SkillView>;
  /** Writes the skill as a `.zip` to a path the user picked. */
  skill_export(id: string, path: string): Promise<void>;

  // MCP servers (§13.9)
  mcp_servers_list(): Promise<McpServerView[]>;
  mcp_server_save(input: McpServerInput): Promise<McpServerView>;
  mcp_server_delete(id: string): Promise<void>;
  /** On this device only (AI-29). */
  mcp_server_set_enabled(id: string, enabled: boolean): Promise<void>;
  mcp_server_status(id: string): Promise<McpServerStatus>;
  /** Starts (or restarts) the server now and lists its tools. */
  mcp_server_start(id: string): Promise<McpServerStatus>;
  mcp_server_stop(id: string): Promise<void>;
  /** AI-31 Always allow on this device: one tool (the server's own name), or every tool with `tool` null. */
  mcp_set_always_allow(server_id: string, tool: string | null, allow: boolean): Promise<void>;
  /**
   * The MCP tool behind a name the model called in the conversation, resolved as `ai_tool_run` resolves it:
   * from the offer of the conversation's latest request (so it still answers after the server stopped),
   * else from the running servers. Null when neither has it, or its server was deleted.
   */
  mcp_tool_info(conversation_id: string, name: string): Promise<McpToolInfo | null>;
  /** AI-33: what pasted `mcpServers` / VS Code `servers` JSON would add. */
  mcp_import_preview(json: string): Promise<McpImportPreview>;
  /** Adds every importable server; env and header values move into the vault. */
  mcp_import(json: string): Promise<McpServerView[]>;
  /** `mcpServers` JSON with placeholders in place of env and header values. */
  mcp_export(): Promise<string>;

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
