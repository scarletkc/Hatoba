//! Hatoba desktop shell: Tauri commands, events and channels on top of `hatoba-core` (vault,
//! crypto, sync) and `hatoba-ssh` (sessions, SFTP). The WebView never receives secrets (spec §3.2).

mod ai;
mod commands;
mod convert;
mod deploy;
mod dropped;
mod dto;
mod error;
mod lock;
mod logging;
mod mcp;
mod platform;
mod ssh;
mod state;
mod sync;
mod update;

use commands::{
    ai as ai_cmd, app, deploy as deploy_cmd, forwards, hosts, keys, mcp as mcp_cmd, proxies, quick,
    settings, sftp, skills, ssh as ssh_cmd, sync as sync_cmd, vault,
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
            app::update_install,
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
            hosts::host_set_show_stats,
            hosts::host_copy_password,
            hosts::groups_list,
            hosts::group_save,
            hosts::group_delete,
            hosts::tags_list,
            hosts::hosts_probe,
            hosts::ssh_config_preview,
            hosts::ssh_config_import,
            proxies::proxies_list,
            proxies::proxy_save,
            proxies::proxy_delete,
            keys::keys_list,
            keys::key_import,
            keys::key_generate,
            keys::key_rename,
            keys::key_delete,
            keys::key_public,
            keys::key_deploy,
            ssh_cmd::ssh_connect,
            ssh_cmd::ssh_connect_target,
            quick::recent_targets_list,
            quick::recent_target_remove,
            ssh_cmd::ssh_write,
            ssh_cmd::ssh_resize,
            ssh_cmd::ssh_disconnect,
            ssh_cmd::ssh_stats_start,
            ssh_cmd::ssh_stats_stop,
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
            sync_cmd::sync_check_worker,
            sync_cmd::sync_dismiss_worker_update,
            sync_cmd::sync_set_auto,
            sync_cmd::sync_disconnect,
            sync_cmd::sync_devices,
            sync_cmd::sync_revoke_device,
            sync_cmd::sync_conflicts,
            sync_cmd::sync_conflict_resolve,
            deploy_cmd::deploy_start,
            deploy_cmd::deploy_inspect,
            deploy_cmd::deploy_run,
            deploy_cmd::deploy_check,
            deploy_cmd::deploy_cleanup,
            deploy_cmd::deploy_cancel,
            deploy_cmd::deploy_setup,
            deploy_cmd::deploy_upgrade_defaults,
            deploy_cmd::deploy_upgrade_inspect,
            deploy_cmd::deploy_upgrade,
            settings::settings_get,
            settings::settings_save,
            settings::prefs_get,
            settings::prefs_save,
            settings::star_prompt_get,
            settings::star_prompt_done,
            ai_cmd::ai_providers_list,
            ai_cmd::ai_provider_save,
            ai_cmd::ai_provider_delete,
            ai_cmd::ai_provider_models,
            ai_cmd::ai_provider_test,
            ai_cmd::search_providers_list,
            ai_cmd::search_provider_save,
            ai_cmd::search_provider_delete,
            ai_cmd::search_provider_test,
            ai_cmd::ai_settings_get,
            ai_cmd::ai_settings_save,
            ai_cmd::ai_conversations_list,
            ai_cmd::ai_conversation_get,
            ai_cmd::ai_conversation_rename,
            ai_cmd::ai_conversation_pin,
            ai_cmd::ai_conversation_delete,
            ai_cmd::ai_send,
            ai_cmd::ai_retry,
            ai_cmd::ai_tool_result,
            ai_cmd::ai_tool_run,
            ai_cmd::ai_file_preview,
            ai_cmd::ai_stop,
            ai_cmd::ai_compact,
            ai_cmd::ai_search,
            ai_cmd::ai_edit_resend,
            ai_cmd::ai_read_dropped_files,
            skills::skills_list,
            skills::skill_get,
            skills::skill_builtin_get,
            skills::skill_save,
            skills::skill_delete,
            skills::skill_set_enabled,
            skills::skill_import_preview,
            skills::skill_import,
            skills::skill_export,
            mcp_cmd::mcp_servers_list,
            mcp_cmd::mcp_server_save,
            mcp_cmd::mcp_server_delete,
            mcp_cmd::mcp_server_set_enabled,
            mcp_cmd::mcp_server_status,
            mcp_cmd::mcp_server_start,
            mcp_cmd::mcp_server_stop,
            mcp_cmd::mcp_set_always_allow,
            mcp_cmd::mcp_tool_info,
            mcp_cmd::mcp_import_preview,
            mcp_cmd::mcp_import,
            mcp_cmd::mcp_export,
        ])
        .events(collect_events![
            dto::VaultLockedEvent,
            dto::SyncStatus,
            dto::HostKeyPrompt,
            dto::AuthPrompt,
            dto::SessionStateEvent,
            dto::TransferProgressEvent,
            dto::ForwardStatusEvent,
            dto::McpServerStatus,
        ])
}

pub fn run() {
    let builder = specta_builder();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            let handle = app.handle().clone();

            // §5.2: the vault holds device-local state, so keep it out of the roaming profile.
            let data_dir = app.path().app_local_data_dir()?;
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
            app.state::<state::AppState>()
                .mcp
                .set_events(std::sync::Arc::new(mcp_cmd::AppMcpEvents(handle.clone())));

            lock::spawn_watchers(handle.clone());
            sync::spawn_scheduler(handle.clone());
            platform::window::show_main(&handle);
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // §6.3: sync when the window regains focus.
            tauri::WindowEvent::Focused(true) => {
                sync::trigger(window.app_handle(), sync::Trigger::Focus);
            }
            // AI-35: the AI panel may read the files of the last drop, and no others.
            tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) => {
                if let Some(state) = window.app_handle().try_state::<state::AppState>() {
                    state.dropped.record(paths);
                }
            }
            // macOS: the red button hides the window and keeps the app, its sessions and the
            // vault running, like other Mac apps. A destroyed last window would exit the app.
            // The Dock icon brings the window back (`RunEvent::Reopen`); Cmd+Q still quits.
            #[cfg(target_os = "macos")]
            tauri::WindowEvent::CloseRequested { api, .. }
                if window.label() == platform::window::MAIN =>
            {
                api.prevent_close();
                platform::window::hide_main(window.clone());
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("error while building Hatoba")
        .run(|app, event| match event {
            tauri::RunEvent::Exit => {
                if let Some(state) = app.try_state::<state::AppState>() {
                    // AI-32: MCP servers stop when the app quits.
                    tauri::async_runtime::block_on(state.mcp.shutdown());
                }
            }
            // macOS: a Dock click shows the window again. Every click counts, including one while
            // the window still leaves fullscreen and is visible, so it cancels a pending hide;
            // showing a visible window changes nothing.
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => platform::window::reopen_main(app),
            _ => {}
        });
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
