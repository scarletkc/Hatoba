import { Channel } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import type { HatobaApi } from "./api";
import { commands } from "./bindings";
import type { AiSideEvent, AiTurnEvent, DashboardEvent, DeployProgress, StatsEvent, UpdateProgress } from "./types";

/** A command whose Rust function returns `()` resolves to `null`; the API declares `void`. */
const done = (result: Promise<null>): Promise<void> => result.then(() => undefined);

/**
 * Tauri implementation of {@link HatobaApi}, on the command functions tauri-specta generates, so
 * a changed command name or parameter list fails the type check here.
 */
export function createTauriApi(): HatobaApi {
  return {
    app_info: () => commands.appInfo(),
    update_check: () => commands.updateCheck(),
    update_install: (onProgress) => {
      const progress = new Channel<UpdateProgress>();
      progress.onmessage = onProgress;
      return done(commands.updateInstall(progress));
    },
    launch_take: () => commands.launchTake(),
    cli_command_get: () => commands.cliCommandGet(),
    cli_command_set: (enabled) => commands.cliCommandSet(enabled),

    vault_status: () => commands.vaultStatus(),
    vault_create: (password) => commands.vaultCreate(password),
    vault_unlock: (password) => done(commands.vaultUnlock(password)),
    vault_unlock_biometric: () => done(commands.vaultUnlockBiometric()),
    vault_unlock_pin: (pin) => done(commands.vaultUnlockPin(pin)),
    vault_lock: () => commands.vaultLock(),
    vault_verify_password: (password) => commands.vaultVerifyPassword(password),
    vault_change_password: (current, next) => done(commands.vaultChangePassword(current, next)),
    vault_recover: (recoveryCode, newPassword) => done(commands.vaultRecover(recoveryCode, newPassword)),
    vault_restore_from_cloud: (config, password) => done(commands.vaultRestoreFromCloud(config, password)),
    vault_export_backup: (path) => done(commands.vaultExportBackup(path)),
    vault_rotate_recovery: (password) => commands.vaultRotateRecovery(password),
    biometric_enable: (password) => done(commands.biometricEnable(password)),
    biometric_disable: () => done(commands.biometricDisable()),
    pin_enable: (password, pin) => done(commands.pinEnable(password, pin)),
    pin_disable: () => done(commands.pinDisable()),
    pin_set_persist: (persist) => done(commands.pinSetPersist(persist)),
    activity_ping: () => commands.activityPing(),

    hosts_list: () => commands.hostsList(),
    host_get: (id) => commands.hostGet(id),
    host_save: (input) => commands.hostSave(input),
    host_delete: (id) => done(commands.hostDelete(id)),
    host_duplicate: (id) => commands.hostDuplicate(id),
    host_set_favorite: (id, favorite) => done(commands.hostSetFavorite(id, favorite)),
    host_set_show_stats: (id, on) => done(commands.hostSetShowStats(id, on)),
    dashboard_set_shown: (id, shown) => done(commands.dashboardSetShown(id, shown)),
    host_copy_password: (id) => done(commands.hostCopyPassword(id)),
    groups_list: () => commands.groupsList(),
    group_save: (input) => commands.groupSave(input),
    group_delete: (id) => done(commands.groupDelete(id)),
    tags_list: () => commands.tagsList(),
    hosts_probe: (ids) => commands.hostsProbe(ids),
    dashboard_watch: (hostIds, intervalSecs, onEvent) => {
      const channel = new Channel<DashboardEvent>();
      channel.onmessage = onEvent;
      return commands.dashboardWatch(hostIds, intervalSecs, channel);
    },
    dashboard_unwatch: (watchId) => commands.dashboardUnwatch(watchId),
    dashboard_retry: (hostId) => commands.dashboardRetry(hostId),
    ssh_config_preview: () => commands.sshConfigPreview(),
    ssh_config_import: (aliases, keyFiles) => commands.sshConfigImport(aliases, keyFiles),

    proxies_list: () => commands.proxiesList(),
    proxy_save: (input) => commands.proxySave(input),
    proxy_delete: (id) => done(commands.proxyDelete(id)),

    keys_list: () => commands.keysList(),
    key_import: (input) => commands.keyImport(input),
    key_generate: (input) => commands.keyGenerate(input),
    key_rename: (id, name) => commands.keyRename(id, name),
    key_delete: (id) => done(commands.keyDelete(id)),
    key_public: (id) => commands.keyPublic(id),
    key_deploy: (keyId, hostId) => done(commands.keyDeploy(keyId, hostId)),

    ssh_connect: (hostId, options, onFrame) => {
      const channel = new Channel<ArrayBuffer | number[]>();
      channel.onmessage = (msg) => onFrame(msg instanceof ArrayBuffer ? new Uint8Array(msg) : Uint8Array.from(msg));
      return commands.sshConnect(hostId, options, channel);
    },
    ssh_connect_target: (target, options, onFrame) => {
      const channel = new Channel<ArrayBuffer | number[]>();
      channel.onmessage = (msg) => onFrame(msg instanceof ArrayBuffer ? new Uint8Array(msg) : Uint8Array.from(msg));
      return commands.sshConnectTarget(target, options, channel);
    },
    recent_targets_list: () => commands.recentTargetsList(),
    recent_target_remove: (target) => commands.recentTargetRemove(target),
    ssh_write: (sessionId, data) => done(commands.sshWrite(sessionId, data)),
    ssh_write_bytes: (sessionId, data) => done(commands.sshWriteBytes(sessionId, data)),
    ssh_resize: (sessionId, cols, rows) => done(commands.sshResize(sessionId, cols, rows)),
    ssh_disconnect: (sessionId) => done(commands.sshDisconnect(sessionId)),
    local_shells: () => commands.localShells(),
    local_open: (shellId, options, onFrame) => {
      const channel = new Channel<ArrayBuffer | number[]>();
      channel.onmessage = (msg) => onFrame(msg instanceof ArrayBuffer ? new Uint8Array(msg) : Uint8Array.from(msg));
      return commands.localOpen(shellId, options, channel);
    },
    local_write: (sessionId, data) => done(commands.localWrite(sessionId, data)),
    local_write_bytes: (sessionId, data) => done(commands.localWriteBytes(sessionId, data)),
    local_resize: (sessionId, cols, rows) => done(commands.localResize(sessionId, cols, rows)),
    local_close: (sessionId) => done(commands.localClose(sessionId)),
    ssh_stats_start: (sessionId, onEvent) => {
      const channel = new Channel<StatsEvent>();
      channel.onmessage = onEvent;
      return commands.sshStatsStart(sessionId, channel);
    },
    ssh_stats_stop: (sessionId, statsId) => commands.sshStatsStop(sessionId, statsId),
    ssh_test: (input) => commands.sshTest(input),
    hostkey_respond: (requestId, accept) => commands.hostkeyRespond(requestId, accept),
    auth_prompt_respond: (requestId, answers) => commands.authPromptRespond(requestId, answers),

    forwards_list: (hostId) => commands.forwardsList(hostId),
    forward_save: (input) => commands.forwardSave(input),
    forward_delete: (id) => done(commands.forwardDelete(id)),
    forward_start: (sessionId, forwardId) => commands.forwardStart(sessionId, forwardId),
    forward_stop: (sessionId, forwardId) => done(commands.forwardStop(sessionId, forwardId)),
    forwards_active: (sessionId) => commands.forwardsActive(sessionId),
    sftp_home: (sessionId) => commands.sftpHome(sessionId),
    sftp_list: (sessionId, path) => commands.sftpList(sessionId, path),
    sftp_download: (sessionId, remotePath, localPath) => commands.sftpDownload(sessionId, remotePath, localPath),
    sftp_upload: (sessionId, localPath, remoteDir) => commands.sftpUpload(sessionId, localPath, remoteDir),
    sftp_rename: (sessionId, from, to) => done(commands.sftpRename(sessionId, from, to)),
    sftp_remove: (sessionId, path, isDir) => done(commands.sftpRemove(sessionId, path, isDir)),
    sftp_mkdir: (sessionId, path) => done(commands.sftpMkdir(sessionId, path)),
    transfer_cancel: (transferId) => commands.transferCancel(transferId),

    sync_status: () => commands.syncStatus(),
    sync_test: (config) => commands.syncTest(config),
    sync_configure: (config, password) => done(commands.syncConfigure(config, password)),
    sync_join: (config, password) => done(commands.syncJoin(config, password)),
    sync_now: () => done(commands.syncNow()),
    sync_login: (password) => done(commands.syncLogin(password)),
    sync_set_auto: (enabled) => commands.syncSetAuto(enabled),
    sync_check_worker: () => commands.syncCheckWorker(),
    sync_dismiss_worker_update: () => done(commands.syncDismissWorkerUpdate()),
    sync_disconnect: () => done(commands.syncDisconnect()),
    sync_devices: () => commands.syncDevices(),
    sync_revoke_device: (deviceId) => done(commands.syncRevokeDevice(deviceId)),
    sync_conflicts: () => commands.syncConflicts(),
    sync_conflict_resolve: (id, action) => done(commands.syncConflictResolve(id, action)),

    deploy_start: (apiToken, accountId) => commands.deployStart(apiToken, accountId),
    deploy_inspect: (handle, target) => commands.deployInspect(handle, target),
    deploy_run: (handle, target, onProgress) => {
      const progress = new Channel<DeployProgress>();
      progress.onmessage = onProgress;
      return commands.deployRun(handle, target, progress);
    },
    deploy_check: (handle) => commands.deployCheck(handle),
    deploy_cleanup: (handle) => done(commands.deployCleanup(handle)),
    deploy_cancel: (handle) => commands.deployCancel(handle),
    deploy_setup: (handle, password) => done(commands.deploySetup(handle, password)),
    deploy_join: (handle, target, password) => done(commands.deployJoin(handle, target, password)),
    deploy_upgrade_defaults: () => commands.deployUpgradeDefaults(),
    deploy_upgrade_inspect: (handle, target) => commands.deployUpgradeInspect(handle, target),
    deploy_upgrade: (handle, target, onProgress) => {
      const progress = new Channel<DeployProgress>();
      progress.onmessage = onProgress;
      return commands.deployUpgrade(handle, target, progress);
    },

    settings_get: () => commands.settingsGet(),
    settings_save: (patch) => done(commands.settingsSave(patch)),
    prefs_get: () => commands.prefsGet(),
    prefs_save: (prefs) => done(commands.prefsSave(prefs)),
    star_prompt_get: () => commands.starPromptGet(),
    star_prompt_done: () => done(commands.starPromptDone()),

    ai_providers_list: () => commands.aiProvidersList(),
    ai_provider_save: (input) => commands.aiProviderSave(input),
    ai_provider_delete: (id) => done(commands.aiProviderDelete(id)),
    ai_provider_models: (input) => commands.aiProviderModels(input),
    ai_provider_test: (input) => commands.aiProviderTest(input),
    ai_chatgpt_sign_in: (providerId) => commands.aiChatgptSignIn(providerId),
    ai_chatgpt_sign_in_cancel: () => commands.aiChatgptSignInCancel(),
    ai_chatgpt_sign_out: (providerId) => commands.aiChatgptSignOut(providerId),
    search_providers_list: () => commands.searchProvidersList(),
    search_provider_save: (input) => commands.searchProviderSave(input),
    search_provider_delete: (id) => done(commands.searchProviderDelete(id)),
    search_provider_test: (input) => commands.searchProviderTest(input),
    ai_settings_get: () => commands.aiSettingsGet(),
    ai_settings_save: (patch) => done(commands.aiSettingsSave(patch)),
    ai_conversations_list: () => commands.aiConversationsList(),
    ai_conversation_get: (id) => commands.aiConversationGet(id),
    ai_conversation_rename: (id, title) => commands.aiConversationRename(id, title),
    ai_conversation_pin: (id, pinned) => commands.aiConversationPin(id, pinned),
    ai_conversation_delete: (id) => done(commands.aiConversationDelete(id)),
    ai_conversation_fork: (id, entryId, title) => commands.aiConversationFork(id, entryId, title),
    ai_send: (input, onEvent) => {
      const channel = new Channel<AiTurnEvent>();
      channel.onmessage = onEvent;
      return commands.aiSend(input, channel);
    },
    ai_retry: (conversationId, context, onEvent) => {
      const channel = new Channel<AiTurnEvent>();
      channel.onmessage = onEvent;
      return done(commands.aiRetry(conversationId, context, channel));
    },
    ai_tool_result: (conversationId, toolCallId, result) => commands.aiToolResult(conversationId, toolCallId, result),
    ai_tool_run: (conversationId, toolCallId, sessionId, editedArguments, judged) =>
      commands.aiToolRun(conversationId, toolCallId, sessionId, editedArguments, judged),
    ai_judge: (conversationId, toolCallId, screenTail) => commands.aiJudge(conversationId, toolCallId, screenTail),
    ai_judge_test: (judge) => commands.aiJudgeTest(judge),
    ai_file_preview: (conversationId, toolCallId, sessionId, editedArguments) =>
      commands.aiFilePreview(conversationId, toolCallId, sessionId, editedArguments),
    ai_file_changes: (conversationId) => commands.aiFileChanges(conversationId),
    ai_file_change_diff: (conversationId, responseId, toolCallId) => commands.aiFileChangeDiff(conversationId, responseId, toolCallId),
    ai_file_revert: (conversationId, responseId, toolCallId, sessionId, restore) =>
      commands.aiFileRevert(conversationId, responseId, toolCallId, sessionId, restore),
    ai_session_server: (sessionId) => commands.aiSessionServer(sessionId),
    ai_stop: (conversationId) => done(commands.aiStop(conversationId)),
    ai_compact: (conversationId, context, instructions) => commands.aiCompact(conversationId, context, instructions),
    ai_side_question: (input, onEvent) => {
      const channel = new Channel<AiSideEvent>();
      channel.onmessage = onEvent;
      return commands.aiSideQuestion(input, channel);
    },
    ai_side_stop: (conversationId, questionId) => done(commands.aiSideStop(conversationId, questionId)),

    ai_search: (query) => commands.aiSearch(query),
    ai_edit_resend: (conversationId, entryId, text, context, onEvent) => {
      const channel = new Channel<AiTurnEvent>();
      channel.onmessage = onEvent;
      return commands.aiEditResend(conversationId, entryId, text, context, channel);
    },
    ai_read_dropped_files: (paths) => commands.aiReadDroppedFiles(paths),
    ai_skill_commands: () => commands.aiSkillCommands(),
    ai_skill_text: (name) => commands.aiSkillText(name),
    ai_host_list: (sessionId, path) => commands.aiHostList(sessionId, path),
    ai_host_read: (sessionId, path) => commands.aiHostRead(sessionId, path),
    ai_allowances_list: () => commands.aiAllowancesList(),
    ai_allowance_add: (allowance) => commands.aiAllowanceAdd(allowance),
    ai_allowance_remove: (allowance) => commands.aiAllowanceRemove(allowance),

    skills_list: () => commands.skillsList(),
    skill_get: (id) => commands.skillGet(id),
    skill_builtin_get: () => commands.skillBuiltinGet(),
    skill_save: (input) => commands.skillSave(input),
    skill_delete: (id) => done(commands.skillDelete(id)),
    skill_set_enabled: (id, enabled) => done(commands.skillSetEnabled(id, enabled)),
    skill_import_preview: (path) => commands.skillImportPreview(path),
    skill_import: (path, replaceId, rename) => commands.skillImport(path, replaceId, rename),
    skill_export: (id, path) => done(commands.skillExport(id, path)),

    mcp_servers_list: () => commands.mcpServersList(),
    mcp_server_save: (input) => commands.mcpServerSave(input),
    mcp_server_delete: (id) => done(commands.mcpServerDelete(id)),
    mcp_server_set_enabled: (id, enabled) => done(commands.mcpServerSetEnabled(id, enabled)),
    mcp_server_status: (id) => commands.mcpServerStatus(id),
    mcp_server_start: (id) => commands.mcpServerStart(id),
    mcp_server_stop: (id) => done(commands.mcpServerStop(id)),
    mcp_set_always_allow: (serverId, tool, allow) => done(commands.mcpSetAlwaysAllow(serverId, tool, allow)),
    mcp_tool_info: (conversationId, name) => commands.mcpToolInfo(conversationId, name),
    mcp_proxy_info: (conversationId, args) => commands.mcpProxyInfo(conversationId, args),
    mcp_warm: (disabled) => done(commands.mcpWarm(disabled)),
    mcp_prompt_get: (serverId, name, args) => commands.mcpPromptGet(serverId, name, args),
    mcp_resource_read: (serverId, uri) => commands.mcpResourceRead(serverId, uri),
    mcp_import_preview: (json) => commands.mcpImportPreview(json),
    mcp_import: (json) => commands.mcpImport(json),
    mcp_export: () => commands.mcpExport(),

    window_snap_overlay: () => commands.windowSnapOverlay(),
    window_floating: () => commands.windowFloating(),
    window_hide_to_tray: () => commands.windowHideToTray(),
    app_quit: () => commands.appQuit(),
    tray_set_menu: (menu) => commands.traySetMenu(menu),
    notification_show: (notification) => commands.notificationShow(notification),
    save_text_file: (path, contents) => done(commands.saveTextFile(path, contents)),

    listen: (event, handler) => tauriListen(event, (e) => handler(e.payload as never)),
  };
}
