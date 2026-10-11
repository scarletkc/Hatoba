import { Channel, invoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import type { HatobaApi } from "./api";
import type { AiTurnEvent, DeployProgress, StatsEvent, UpdateProgress } from "./types";

/**
 * Tauri implementation of {@link HatobaApi}. Command arguments use Tauri's default camelCase
 * mapping of the Rust snake_case parameter names.
 */
export function createTauriApi(): HatobaApi {
  const call = <T>(cmd: string, args?: Record<string, unknown>) => invoke<T>(cmd, args);

  return {
    app_info: () => call("app_info"),
    update_check: () => call("update_check"),
    update_install: (onProgress) => {
      const progress = new Channel<UpdateProgress>();
      progress.onmessage = onProgress;
      return call("update_install", { progress });
    },

    vault_status: () => call("vault_status"),
    vault_create: (password) => call("vault_create", { password }),
    vault_unlock: (password) => call("vault_unlock", { password }),
    vault_unlock_biometric: () => call("vault_unlock_biometric"),
    vault_lock: () => call("vault_lock"),
    vault_verify_password: (password) => call("vault_verify_password", { password }),
    vault_change_password: (current, next) => call("vault_change_password", { current, next }),
    vault_recover: (recoveryCode, newPassword) => call("vault_recover", { recoveryCode, newPassword }),
    vault_restore_from_cloud: (config, password) => call("vault_restore_from_cloud", { config, password }),
    vault_export_backup: (path) => call("vault_export_backup", { path }),
    vault_rotate_recovery: (password) => call("vault_rotate_recovery", { password }),
    biometric_enable: (password) => call("biometric_enable", { password }),
    biometric_disable: () => call("biometric_disable"),
    activity_ping: () => call("activity_ping"),

    hosts_list: () => call("hosts_list"),
    host_get: (id) => call("host_get", { id }),
    host_save: (input) => call("host_save", { input }),
    host_delete: (id) => call("host_delete", { id }),
    host_duplicate: (id) => call("host_duplicate", { id }),
    host_set_favorite: (id, favorite) => call("host_set_favorite", { id, favorite }),
    host_set_show_stats: (id, on) => call("host_set_show_stats", { id, on }),
    host_copy_password: (id) => call("host_copy_password", { id }),
    groups_list: () => call("groups_list"),
    group_save: (input) => call("group_save", { input }),
    group_delete: (id) => call("group_delete", { id }),
    tags_list: () => call("tags_list"),
    hosts_probe: (ids) => call("hosts_probe", { ids }),
    ssh_config_preview: () => call("ssh_config_preview"),
    ssh_config_import: (aliases, keyFiles) => call("ssh_config_import", { aliases, keyFiles }),

    proxies_list: () => call("proxies_list"),
    proxy_save: (input) => call("proxy_save", { input }),
    proxy_delete: (id) => call("proxy_delete", { id }),

    keys_list: () => call("keys_list"),
    key_import: (input) => call("key_import", { input }),
    key_generate: (input) => call("key_generate", { input }),
    key_rename: (id, name) => call("key_rename", { id, name }),
    key_delete: (id) => call("key_delete", { id }),
    key_public: (id) => call("key_public", { id }),
    key_deploy: (keyId, hostId) => call("key_deploy", { keyId, hostId }),

    ssh_connect: (hostId, options, onFrame) => {
      const channel = new Channel<ArrayBuffer | number[]>();
      channel.onmessage = (msg) => onFrame(msg instanceof ArrayBuffer ? new Uint8Array(msg) : Uint8Array.from(msg));
      return call("ssh_connect", { hostId, options, channel });
    },
    ssh_connect_target: (target, options, onFrame) => {
      const channel = new Channel<ArrayBuffer | number[]>();
      channel.onmessage = (msg) => onFrame(msg instanceof ArrayBuffer ? new Uint8Array(msg) : Uint8Array.from(msg));
      return call("ssh_connect_target", { target, options, channel });
    },
    recent_targets_list: () => call("recent_targets_list"),
    recent_target_remove: (target) => call("recent_target_remove", { target }),
    ssh_write: (sessionId, data) => call("ssh_write", { sessionId, data }),
    ssh_resize: (sessionId, cols, rows) => call("ssh_resize", { sessionId, cols, rows }),
    ssh_disconnect: (sessionId) => call("ssh_disconnect", { sessionId }),
    ssh_stats_start: (sessionId, onEvent) => {
      const channel = new Channel<StatsEvent>();
      channel.onmessage = onEvent;
      return call("ssh_stats_start", { sessionId, channel });
    },
    ssh_stats_stop: (sessionId, statsId) => call("ssh_stats_stop", { sessionId, statsId }),
    ssh_test: (input) => call("ssh_test", { input }),
    hostkey_respond: (requestId, accept) => call("hostkey_respond", { requestId, accept }),
    auth_prompt_respond: (requestId, answers) => call("auth_prompt_respond", { requestId, answers }),

    forwards_list: (hostId) => call("forwards_list", { hostId }),
    forward_save: (input) => call("forward_save", { input }),
    forward_delete: (id) => call("forward_delete", { id }),
    forward_start: (sessionId, forwardId) => call("forward_start", { sessionId, forwardId }),
    forward_stop: (sessionId, forwardId) => call("forward_stop", { sessionId, forwardId }),
    forwards_active: (sessionId) => call("forwards_active", { sessionId }),
    sftp_home: (sessionId) => call("sftp_home", { sessionId }),
    sftp_list: (sessionId, path) => call("sftp_list", { sessionId, path }),
    sftp_download: (sessionId, remotePath, localPath) => call("sftp_download", { sessionId, remotePath, localPath }),
    sftp_upload: (sessionId, localPath, remoteDir) => call("sftp_upload", { sessionId, localPath, remoteDir }),
    sftp_rename: (sessionId, from, to) => call("sftp_rename", { sessionId, from, to }),
    sftp_remove: (sessionId, path, isDir) => call("sftp_remove", { sessionId, path, isDir }),
    sftp_mkdir: (sessionId, path) => call("sftp_mkdir", { sessionId, path }),
    transfer_cancel: (transferId) => call("transfer_cancel", { transferId }),

    sync_status: () => call("sync_status"),
    sync_test: (config) => call("sync_test", { config }),
    sync_configure: (config, password) => call("sync_configure", { config, password }),
    sync_now: () => call("sync_now"),
    sync_login: (password) => call("sync_login", { password }),
    sync_set_auto: (enabled) => call("sync_set_auto", { enabled }),
    sync_check_worker: () => call("sync_check_worker"),
    sync_dismiss_worker_update: () => call("sync_dismiss_worker_update"),
    sync_disconnect: () => call("sync_disconnect"),
    sync_devices: () => call("sync_devices"),
    sync_revoke_device: (deviceId) => call("sync_revoke_device", { deviceId }),
    sync_conflicts: () => call("sync_conflicts"),
    sync_conflict_resolve: (id, action) => call("sync_conflict_resolve", { id, action }),

    deploy_start: (apiToken, accountId) => call("deploy_start", { apiToken, accountId }),
    deploy_inspect: (handle, target) => call("deploy_inspect", { handle, target }),
    deploy_run: (handle, target, onProgress) => {
      const progress = new Channel<DeployProgress>();
      progress.onmessage = onProgress;
      return call("deploy_run", { handle, target, progress });
    },
    deploy_check: (handle) => call("deploy_check", { handle }),
    deploy_cleanup: (handle) => call("deploy_cleanup", { handle }),
    deploy_cancel: (handle) => call("deploy_cancel", { handle }),
    deploy_setup: (handle, password) => call("deploy_setup", { handle, password }),
    deploy_upgrade_defaults: () => call("deploy_upgrade_defaults"),
    deploy_upgrade_inspect: (handle, target) => call("deploy_upgrade_inspect", { handle, target }),
    deploy_upgrade: (handle, target, onProgress) => {
      const progress = new Channel<DeployProgress>();
      progress.onmessage = onProgress;
      return call("deploy_upgrade", { handle, target, progress });
    },

    settings_get: () => call("settings_get"),
    settings_save: (settings) => call("settings_save", { settings }),
    prefs_get: () => call("prefs_get"),
    prefs_save: (prefs) => call("prefs_save", { prefs }),
    star_prompt_get: () => call("star_prompt_get"),
    star_prompt_done: () => call("star_prompt_done"),

    ai_providers_list: () => call("ai_providers_list"),
    ai_provider_save: (input) => call("ai_provider_save", { input }),
    ai_provider_delete: (id) => call("ai_provider_delete", { id }),
    ai_provider_models: (input) => call("ai_provider_models", { input }),
    ai_provider_test: (input) => call("ai_provider_test", { input }),
    search_providers_list: () => call("search_providers_list"),
    search_provider_save: (input) => call("search_provider_save", { input }),
    search_provider_delete: (id) => call("search_provider_delete", { id }),
    search_provider_test: (input) => call("search_provider_test", { input }),
    ai_settings_get: () => call("ai_settings_get"),
    ai_settings_save: (settings) => call("ai_settings_save", { settings }),
    ai_conversations_list: () => call("ai_conversations_list"),
    ai_conversation_get: (id) => call("ai_conversation_get", { id }),
    ai_conversation_rename: (id, title) => call("ai_conversation_rename", { id, title }),
    ai_conversation_pin: (id, pinned) => call("ai_conversation_pin", { id, pinned }),
    ai_conversation_delete: (id) => call("ai_conversation_delete", { id }),
    ai_send: (input, onEvent) => {
      const channel = new Channel<AiTurnEvent>();
      channel.onmessage = onEvent;
      return call("ai_send", { input, channel });
    },
    ai_retry: (conversationId, context, onEvent) => {
      const channel = new Channel<AiTurnEvent>();
      channel.onmessage = onEvent;
      return call("ai_retry", { conversationId, context, channel });
    },
    ai_tool_result: (conversationId, toolCallId, result) => call("ai_tool_result", { conversationId, toolCallId, result }),
    ai_tool_run: (conversationId, toolCallId, sessionId, editedArguments) =>
      call("ai_tool_run", { conversationId, toolCallId, sessionId, editedArguments }),
    ai_file_preview: (conversationId, toolCallId, sessionId, editedArguments) =>
      call("ai_file_preview", { conversationId, toolCallId, sessionId, editedArguments }),
    ai_stop: (conversationId) => call("ai_stop", { conversationId }),
    ai_compact: (conversationId, context) => call("ai_compact", { conversationId, context }),

    ai_search: (query) => call("ai_search", { query }),
    ai_edit_resend: (conversationId, entryId, text, context, onEvent) => {
      const channel = new Channel<AiTurnEvent>();
      channel.onmessage = onEvent;
      return call("ai_edit_resend", { conversationId, entryId, text, context, channel });
    },
    ai_read_dropped_files: (paths) => call("ai_read_dropped_files", { paths }),

    skills_list: () => call("skills_list"),
    skill_get: (id) => call("skill_get", { id }),
    skill_builtin_get: () => call("skill_builtin_get"),
    skill_save: (input) => call("skill_save", { input }),
    skill_delete: (id) => call("skill_delete", { id }),
    skill_set_enabled: (id, enabled) => call("skill_set_enabled", { id, enabled }),
    skill_import_preview: (path) => call("skill_import_preview", { path }),
    skill_import: (path, token, replaceId, rename) => call("skill_import", { path, token, replaceId, rename }),
    skill_export: (id, path) => call("skill_export", { id, path }),

    mcp_servers_list: () => call("mcp_servers_list"),
    mcp_server_save: (input) => call("mcp_server_save", { input }),
    mcp_server_delete: (id) => call("mcp_server_delete", { id }),
    mcp_server_set_enabled: (id, enabled) => call("mcp_server_set_enabled", { id, enabled }),
    mcp_server_status: (id) => call("mcp_server_status", { id }),
    mcp_server_start: (id) => call("mcp_server_start", { id }),
    mcp_server_stop: (id) => call("mcp_server_stop", { id }),
    mcp_set_always_allow: (serverId, tool, allow) => call("mcp_set_always_allow", { serverId, tool, allow }),
    mcp_tool_info: (conversationId, name) => call("mcp_tool_info", { conversationId, name }),
    mcp_import_preview: (json) => call("mcp_import_preview", { json }),
    mcp_import: (json) => call("mcp_import", { json }),
    mcp_export: () => call("mcp_export"),

    window_snap_overlay: () => call("window_snap_overlay"),
    save_text_file: (path, contents) => call("save_text_file", { path, contents }),

    listen: (event, handler) => tauriListen(event, (e) => handler(e.payload as never)),
  };
}
