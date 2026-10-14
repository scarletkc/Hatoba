import type {
  AiAllowance,
  AiAutoVerdict,
  AiChatgptSignIn,
  AiConversationDetail,
  AiConversationView,
  AiEntryView,
  AiFileChange,
  AiFilePreview,
  AiFileRevert,
  AiHostFile,
  AiHostListing,
  AiJudgeSampleOutcome,
  AiModel,
  AiModelRef,
  AiProviderInput,
  AiProviderView,
  AiSendInput,
  AiSendStarted,
  AiSettingsPatch,
  AiSettingsView,
  AiSideAnswer,
  AiSideEvent,
  AiSideQuestionInput,
  AiSkillCommand,
  AiTestResult,
  AiToolResultInput,
  AiToolRun,
  AiTurnContext,
  AiTurnEvent,
  AiSearchHit,
  AppError,
  AppInfo,
  BuiltinSkillView,
  CliCommandView,
  ConflictView,
  ConnectOptions,
  DashboardEvent,
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
  McpPromptArgumentInput,
  McpServerInput,
  McpServerStatus,
  McpServerView,
  McpToolInfo,
  McpProxyInfo,
  NotificationInput,
  ProbeResult,
  ProxyInput,
  ProxyView,
  QuickTarget,
  SearchProviderInput,
  SearchProviderView,
  SettingsPatch,
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
  TrayMenuInput,
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
  /** The arguments of each `hatoba …` launch the window has not handled yet, oldest first (HOST-13). Each is returned once, and only while the vault is unlocked. */
  launch_take(): Promise<string[][]>;
  cli_command_get(): Promise<CliCommandView>;
  /** Adds or removes the `hatoba` command. On macOS this may wait for an administrator's password. */
  cli_command_set(enabled: boolean): Promise<CliCommandView>;

  // vault
  vault_status(): Promise<VaultStatus>;
  /** Creates the local vault and returns the recovery code, formatted in dash-separated groups. */
  vault_create(password: string): Promise<string>;
  vault_unlock(password: string): Promise<void>;
  vault_unlock_biometric(): Promise<void>;
  /** SEC-11: unlock with the PIN. */
  vault_unlock_pin(pin: string): Promise<void>;
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
  /** SEC-11: set or replace the PIN after checking the master password. It needs the master password after a restart until `pin_set_persist`. */
  pin_enable(password: string, pin: string): Promise<void>;
  pin_disable(): Promise<void>;
  pin_set_persist(persist: boolean): Promise<void>;
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
  /** HOST-14: leave a host or a group out of the dashboard, or bring it back (device-local). */
  dashboard_set_shown(id: string, shown: boolean): Promise<void>;
  /** SEC-08: Rust copies the saved password to the clipboard and clears it after this device's delay. */
  host_copy_password(id: string): Promise<void>;
  groups_list(): Promise<GroupView[]>;
  group_save(input: GroupInput): Promise<GroupView>;
  group_delete(id: string): Promise<void>;
  tags_list(): Promise<TagCount[]>;
  hosts_probe(ids: string[]): Promise<ProbeResult[]>;
  /**
   * HOST-14: reads the resource usage of `host_ids` every `interval_secs` in the background, in place of the
   * earlier watch, until `dashboard_unwatch` with the id this resolves to, or the lock. Hosts both watches read
   * keep their connections.
   */
  dashboard_watch(host_ids: string[], interval_secs: number, onEvent: (event: DashboardEvent) => void): Promise<number>;
  /** Stops the watch; its connections close a minute later unless the page watches their hosts again. */
  dashboard_unwatch(watch_id: number): Promise<void>;
  /** **Retry**: signs in to a held host again, and tries an offline or failed one now. */
  dashboard_retry(host_id: string): Promise<void>;
  ssh_config_preview(): Promise<SshConfigCandidate[]>;
  /** Also imports the unencrypted identity files in `keyFiles`, the ones the preview listed and the user confirmed; no other key file is read. */
  ssh_config_import(aliases: string[], keyFiles: string[]): Promise<ImportResult>;

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
  /** Bytes written as they are, for the legacy mouse reports xterm sends as binary. */
  ssh_write_bytes(session_id: string, data: number[]): Promise<void>;
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
  /** Flow C: connect this device to a remote that already holds a vault, with that vault's master password, and merge this device's data into it. */
  sync_join(config: SyncConfigInput, password: string): Promise<void>;
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
  /** The master password step when the Worker already holds a vault: `sync_join` against that Worker. */
  deploy_join(handle: string, target: DeployTarget, password: string): Promise<void>;
  /** What "Update Worker" starts from. */
  deploy_upgrade_defaults(): Promise<UpgradeDefaults>;
  /** Step 2 of an upgrade, against the deployment started with `deploy_start`. */
  deploy_upgrade_inspect(handle: string, target: UpgradeTarget): Promise<UpgradePlan>;
  /** "Update Worker": the deployment steps as an upgrade. Run it again after a failure to continue; `deploy_check` asks again while step 8 waits. */
  deploy_upgrade(handle: string, target: UpgradeTarget, onProgress: (p: DeployProgress) => void): Promise<DeployOutcome>;

  // settings
  settings_get(): Promise<SettingsView>;
  /** Writes the fields `patch` sets over the stored settings; the others keep their stored values. */
  settings_save(patch: SettingsPatch): Promise<void>;
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
  /**
   * AI-52: signs in with ChatGPT in the system browser: a new account (`null`), which adds a ChatGPT
   * provider with its models, or a ChatGPT provider's account again. Resolves once the browser comes
   * back; rejects with `cancelled` after `ai_chatgpt_sign_in_cancel` or the lock, and with `ai` otherwise.
   */
  ai_chatgpt_sign_in(providerId: string | null): Promise<AiChatgptSignIn>;
  /** AI-52: cancels the sign-in that waits for the browser. */
  ai_chatgpt_sign_in_cancel(): Promise<void>;
  /**
   * AI-52: signs a ChatGPT provider out on this device. Resolves to whether OpenAI confirmed revoking the
   * session; the session is gone from this device either way.
   */
  ai_chatgpt_sign_out(providerId: string): Promise<boolean>;
  /** AI-14: the search providers behind `web_search`. */
  search_providers_list(): Promise<SearchProviderView[]>;
  search_provider_save(input: SearchProviderInput): Promise<SearchProviderView>;
  search_provider_delete(id: string): Promise<void>;
  search_provider_test(input: SearchProviderInput): Promise<AiTestResult>;
  /** The synced default model and search provider. */
  ai_settings_get(): Promise<AiSettingsView>;
  /** Writes the fields `patch` sets, like `settings_save`. */
  ai_settings_save(patch: AiSettingsPatch): Promise<void>;
  /** AI-23: every conversation, unsorted. */
  ai_conversations_list(): Promise<AiConversationView[]>;
  ai_conversation_get(id: string): Promise<AiConversationDetail>;
  ai_conversation_rename(id: string, title: string): Promise<AiConversationView>;
  ai_conversation_pin(id: string, pinned: boolean): Promise<AiConversationView>;
  ai_conversation_delete(id: string): Promise<void>;
  /** AI-43: copies the conversation up to the entry `entry_id`, or all of it with null, into a new conversation titled `title`. */
  ai_conversation_fork(id: string, entry_id: string | null, title: string): Promise<AiConversationView>;
  /** Stores the user's message and starts a turn; events stream to `onEvent` until `turn_ended` (§13.1). */
  ai_send(input: AiSendInput, onEvent: (event: AiTurnEvent) => void): Promise<AiSendStarted>;
  /** Retry after an error: sends the next request from the stored conversation on a new channel. */
  ai_retry(conversation_id: string, context: AiTurnContext, onEvent: (event: AiTurnEvent) => void): Promise<void>;
  /** Stores a result the frontend produced (`read_terminal`, `send_input`, a rejection). */
  ai_tool_result(conversation_id: string, tool_call_id: string, result: AiToolResultInput): Promise<AiEntryView>;
  /**
   * Runs a tool that runs in Rust (`run_command` and the file tools on `session_id`, `web_search`, `fetch_url`, ...)
   * and stores its result; says whether keeping a file snapshot dropped older ones (AI-41). `judged`: Auto mode
   * decided the call (AI-47), so the result is stored with the verdict Rust remembers for it.
   */
  ai_tool_run(conversation_id: string, tool_call_id: string, session_id: string | null, edited_arguments: string | null, judged: boolean): Promise<AiToolRun>;
  /**
   * AI-47: asks the judge of Auto mode whether the open call `tool_call_id` can run without the approval card, and
   * remembers the verdict for the call's result. `screen_tail`: for `send_input`, the last lines of the tab's screen.
   * Never fails for the judge's own errors: those come back as an `ask` verdict with a `failure`.
   */
  ai_judge(conversation_id: string, tool_call_id: string, screen_tail: string | null): Promise<AiAutoVerdict>;
  /**
   * AI-47: **Test** beside the judge in Settings → AI: the judge's verdict on each sample call, in order, with the
   * decision expected. `judge` is the model the pane shows, saved or not; null tests the saved judge. Rejects with
   * `invalid_input` (`judge_model`) when the judge names no listed model; the judge's own failures come back as
   * verdicts.
   */
  ai_judge_test(judge: AiModelRef | null): Promise<AiJudgeSampleOutcome[]>;
  /**
   * AI-39, AI-40: the change an open `edit_file` or `write_file` call would make to the file on `session_id`'s
   * host, for its approval card, with the arguments as the user edited them on the card. Rust reads the file
   * once per call and the call writes only over what it read.
   */
  ai_file_preview(conversation_id: string, tool_call_id: string, session_id: string | null, edited_arguments: string | null): Promise<AiFilePreview>;
  /** AI-41: the changes of the conversation's `edit_file` and `write_file` calls that this device keeps. */
  ai_file_changes(conversation_id: string): Promise<AiFileChange[]>;
  /**
   * AI-41: what the call `tool_call_id` of the response `response_id` changed, from the snapshot this device
   * keeps, from the file before the call to what it wrote; null once the change was reverted or dropped.
   */
  ai_file_change_diff(conversation_id: string, response_id: string, tool_call_id: string): Promise<AiFilePreview | null>;
  /**
   * AI-41: reverts the change of the call `tool_call_id` of the response `response_id` on the conversation's
   * tab `session_id`, only while the file holds what the call wrote, or with `restore` (Restore anyway), what
   * the diff of the last revert showed. A file the call created is deleted, so the panel asks first.
   */
  ai_file_revert(conversation_id: string, response_id: string, tool_call_id: string, session_id: string | null, restore: boolean): Promise<AiFileRevert>;
  /** AI-41: the server a revert on the tab `session_id` runs on, as `AiFileChange.server` names it; null while the tab is not connected. */
  ai_session_server(session_id: string | null): Promise<string | null>;
  /** Stops the turn: aborts the request and running tools, cancels calls without a result. */
  ai_stop(conversation_id: string): Promise<void>;
  /**
   * AI-21: summarizes the context with the given model and moves `context_start` to the summary. `instructions`:
   * what `/compact <instructions>` asks of the summary (AI-50).
   */
  ai_compact(conversation_id: string, context: AiTurnContext, instructions: string | null): Promise<AiEntryView>;
  /**
   * AI-42: asks a side question about the conversation without stopping its turn. The answer streams to
   * `onEvent` and comes back whole; nothing is stored. A newer side question of the conversation, `ai_side_stop`
   * and the lock stop it (`cancelled`).
   */
  ai_side_question(input: AiSideQuestionInput, onEvent: (event: AiSideEvent) => void): Promise<AiSideAnswer>;
  /** AI-42: stops the conversation's side question `question_id`, if it is on its way; its turn goes on, and so does a newer side question. */
  ai_side_stop(conversation_id: string, question_id: number): Promise<void>;

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
  /** AI-50: the skills the panel's `/` menu offers as commands, sorted by name. */
  ai_skill_commands(): Promise<AiSkillCommand[]>;
  /** AI-50: the `SKILL.md` of the enabled skill `name`, which a `/<skill>` command attaches; `not_found` otherwise. */
  ai_skill_text(name: string): Promise<string>;
  /** AI-51: the entries of the directory `path` on the host of the tab's session, for the `@` menu. */
  ai_host_list(session_id: string, path: string): Promise<AiHostListing>;
  /** AI-51: the text file `path` on the host of the tab's session, which an `@` mention attaches. */
  ai_host_read(session_id: string, path: string): Promise<AiHostFile>;
  /** AI-19: the approvals saved on this device, oldest first. */
  ai_allowances_list(): Promise<AiAllowance[]>;
  /** AI-19: saves an approval on this device; resolves to them all. */
  ai_allowance_add(allowance: AiAllowance): Promise<AiAllowance[]>;
  /** AI-19: removes a saved approval; resolves to what is left. */
  ai_allowance_remove(allowance: AiAllowance): Promise<AiAllowance[]>;

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
  /** Imports the skill at `path`, reading it again. `replace_id` replaces that skill; `rename` saves it under a new name. */
  skill_import(path: string, replace_id: string | null, rename: string | null): Promise<SkillView>;
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
  /**
   * AI-45: the MCP tool a `call_mcp_tool` call with `args` names, resolved as `ai_tool_run` resolves it, and
   * whether the call runs: one whose arguments do not match the tool's schema, or that names no tool of the
   * conversation or a deleted server's, gets its error result without asking.
   */
  mcp_proxy_info(conversation_id: string, args: string): Promise<McpProxyInfo>;
  /**
   * AI-50, AI-51: the panel's `/` or `@` menu opened. Starts the servers a conversation with `disabled` switched
   * off uses, as its next message would, and lists their prompts and resources again after a `list_changed`;
   * `ai://mcp-status` brings them. Resolves at once.
   */
  mcp_warm(disabled: string[]): Promise<void>;
  /** AI-50: gets a server's prompt `name` with its arguments, as the text a `/` command attaches; fails with `ai` and the reason. */
  mcp_prompt_get(server_id: string, name: string, args: McpPromptArgumentInput[]): Promise<string>;
  /** AI-51: reads a server's resource at `uri`, as the text an `@` mention attaches; fails with `ai` and the reason. */
  mcp_resource_read(server_id: string, uri: string): Promise<string>;
  /** AI-33: what pasted `mcpServers` / VS Code `servers` JSON would add. */
  mcp_import_preview(json: string): Promise<McpImportPreview>;
  /** Adds every importable server; env and header values move into the vault. */
  mcp_import(json: string): Promise<McpServerView[]>;
  /** `mcpServers` JSON with placeholders in place of env and header values. */
  mcp_export(): Promise<string>;

  // window (Windows custom title bar, WIN-01)
  window_snap_overlay(): Promise<void>;
  /**
   * Whether the Linux main window shows its painted chrome: false while maximized, fullscreen
   * or tiled, without a compositor, and on other platforms.
   */
  window_floating(): Promise<boolean>;
  /** WIN-10: the close prompt's choices, and the tray menu in the app's language. */
  window_hide_to_tray(): Promise<void>;
  app_quit(): Promise<void>;
  tray_set_menu(menu: TrayMenuInput): Promise<void>;
  /** AI-07: a system notification; a click on it sends `app://notification-click`. */
  notification_show(notification: NotificationInput): Promise<void>;
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
