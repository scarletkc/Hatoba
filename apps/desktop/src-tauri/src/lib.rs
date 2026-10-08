//! Hatoba desktop shell: Tauri commands, events and channels on top of `hatoba-core` (vault,
//! crypto, sync) and `hatoba-ssh` (sessions, SFTP). The WebView never receives secrets (spec §3.2).

mod commands;
mod convert;
mod dto;
mod error;
mod lock;
mod logging;
mod platform;
mod ssh;
mod state;
mod sync;
mod update;

use commands::{
    app, forwards, hosts, keys, settings, sftp, ssh as ssh_cmd, sync as sync_cmd, vault,
};
use tauri::Manager;
use tauri_specta::{collect_commands, collect_events};

/// The command/event registry; also used to export TypeScript bindings.
pub fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new()
        .error_handling(tauri_specta::ErrorHandlingMode::Throw)
        // i64 timestamps (Unix ms) and u64 sizes stay far below 2^53.
        .dangerously_cast_bigints_to_number()
        .commands(collect_commands![
            app::app_info,
            app::window_snap_overlay,
            app::save_text_file,
            app::activity_ping,
            app::update_check,
            vault::vault_status,
            vault::vault_create,
            vault::vault_unlock,
            vault::vault_unlock_biometric,
            vault::vault_lock,
            vault::vault_verify_password,
            vault::vault_change_password,
            vault::vault_recover,
            vault::vault_restore_from_cloud,
            vault::vault_export_backup,
            vault::vault_rotate_recovery,
            vault::biometric_enable,
            vault::biometric_disable,
            hosts::hosts_list,
            hosts::host_get,
            hosts::host_save,
            hosts::host_delete,
            hosts::host_duplicate,
            hosts::host_set_favorite,
            hosts::host_copy_password,
            hosts::groups_list,
            hosts::group_save,
            hosts::group_delete,
            hosts::tags_list,
            hosts::hosts_probe,
            hosts::ssh_config_preview,
            hosts::ssh_config_import,
            keys::keys_list,
            keys::key_import,
            keys::key_generate,
            keys::key_rename,
            keys::key_delete,
            keys::key_public,
            keys::key_deploy,
            ssh_cmd::ssh_connect,
            ssh_cmd::ssh_write,
            ssh_cmd::ssh_resize,
            ssh_cmd::ssh_disconnect,
            ssh_cmd::ssh_test,
            ssh_cmd::hostkey_respond,
            ssh_cmd::auth_prompt_respond,
            forwards::forwards_list,
            forwards::forward_save,
            forwards::forward_delete,
            forwards::forward_start,
            forwards::forward_stop,
            forwards::forwards_active,
            sftp::sftp_home,
            sftp::sftp_list,
            sftp::sftp_download,
            sftp::sftp_upload,
            sftp::sftp_rename,
            sftp::sftp_remove,
            sftp::sftp_mkdir,
            sftp::transfer_cancel,
            sync_cmd::sync_status,
            sync_cmd::sync_test,
            sync_cmd::sync_configure,
            sync_cmd::sync_now,
            sync_cmd::sync_login,
            sync_cmd::sync_set_auto,
            sync_cmd::sync_disconnect,
            sync_cmd::sync_devices,
            sync_cmd::sync_revoke_device,
            sync_cmd::sync_conflicts,
            sync_cmd::sync_conflict_resolve,
            settings::settings_get,
            settings::settings_save,
            settings::prefs_get,
            settings::prefs_save,
        ])
        .events(collect_events![
            dto::VaultLockedEvent,
            dto::SyncStatus,
            dto::HostKeyPrompt,
            dto::AuthPrompt,
            dto::SessionStateEvent,
            dto::TransferProgressEvent,
            dto::ForwardStatusEvent,
        ])
}

pub fn run() {
    let builder = specta_builder();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            let handle = app.handle().clone();

            let data_dir = app.path().app_data_dir()?;
            let log_dir = app.path().app_log_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            std::fs::create_dir_all(&log_dir)?;
            if let Some(guard) = logging::init(&log_dir) {
                // Keep the non-blocking log writer alive for the life of the app.
                app.manage(guard);
            }

            let vault = hatoba_core::vault::Vault::open(&data_dir.join("vault.db"))?;
            let (_window, mica) = platform::window::create_main_window(&handle)?;
            app.manage(state::AppState::new(vault, mica));

            lock::spawn_watchers(handle.clone());
            sync::spawn_scheduler(handle.clone());
            platform::window::show_main(&handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Focused(true) = event {
                // §6.3: sync when the window regains focus.
                sync::trigger(window.app_handle(), sync::Trigger::Focus);
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Hatoba");
}

#[cfg(test)]
mod tests {
    /// Regenerates `src/ipc/bindings.ts` from the Rust types (`cargo test -p hatoba-desktop export_bindings`).
    #[test]
    fn export_bindings() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/ipc/bindings.ts");
        super::specta_builder()
            .export(
                specta_typescript::Typescript::default().header(
                    "// Generated by tauri-specta from src-tauri. Do not edit.\n// @ts-nocheck\n",
                ),
                path,
            )
            .expect("export TypeScript bindings");
    }
}
