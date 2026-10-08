import { Channel, invoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import type { HatobaApi } from "./api";

/**
 * Tauri implementation of {@link HatobaApi}. Command arguments use Tauri's default camelCase
 * mapping of the Rust snake_case parameter names.
 */
export function createTauriApi(): HatobaApi {
  const call = <T>(cmd: string, args?: Record<string, unknown>) => invoke<T>(cmd, args);

  return {
    app_info: () => call("app_info"),
    update_check: () => call("update_check"),

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
    host_copy_password: (id) => call("host_copy_password", { id }),
    groups_list: () => call("groups_list"),
    group_save: (input) => call("group_save", { input }),
    group_delete: (id) => call("group_delete", { id }),
    tags_list: () => call("tags_list"),
    hosts_probe: (ids) => call("hosts_probe", { ids }),
    ssh_config_preview: () => call("ssh_config_preview"),
    ssh_config_import: (aliases) => call("ssh_config_import", { aliases }),

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
    ssh_write: (sessionId, data) => call("ssh_write", { sessionId, data }),
    ssh_resize: (sessionId, cols, rows) => call("ssh_resize", { sessionId, cols, rows }),
    ssh_disconnect: (sessionId) => call("ssh_disconnect", { sessionId }),
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
    sync_disconnect: () => call("sync_disconnect"),
    sync_devices: () => call("sync_devices"),
    sync_revoke_device: (deviceId) => call("sync_revoke_device", { deviceId }),
    sync_conflicts: () => call("sync_conflicts"),
    sync_conflict_resolve: (id, action) => call("sync_conflict_resolve", { id, action }),

    settings_get: () => call("settings_get"),
    settings_save: (settings) => call("settings_save", { settings }),
    prefs_get: () => call("prefs_get"),
    prefs_save: (prefs) => call("prefs_save", { prefs }),

    window_snap_overlay: () => call("window_snap_overlay"),
    save_text_file: (path, contents) => call("save_text_file", { path, contents }),

    listen: (event, handler) => tauriListen(event, (e) => handler(e.payload as never)),
  };
}
