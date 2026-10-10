//! App info and window helpers.

use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use crate::dto::{AppInfo, UpdateCheck, UpdateProgress};
use crate::error::{AppError, AppResult};
use crate::platform;
use crate::state::AppState;
use crate::update;

#[tauri::command]
#[specta::specta]
pub fn app_info(app: AppHandle, state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        platform: platform::platform(),
        mica: state.mica,
    }
}

/// Asks GitHub whether a newer version exists (Settings → About). Runs only when the user
/// checks, or after unlock when they turned on the automatic check.
#[tauri::command]
#[specta::specta]
pub async fn update_check(app: AppHandle, state: State<'_, AppState>) -> AppResult<UpdateCheck> {
    update::check(&app, &state.updates).await
}

/// Downloads and installs the update the last check found. Hatoba closes, which ends every SSH
/// session, and the installer starts the new version.
#[tauri::command]
#[specta::specta]
pub async fn update_install(app: AppHandle, progress: Channel<UpdateProgress>) -> AppResult<()> {
    update::install(&app, &progress).await
}

/// Opens the Windows 11 Snap Layouts flyout (hovering the custom maximize button, WIN-01).
#[tauri::command]
#[specta::specta]
pub fn window_snap_overlay() {
    platform::window::snap_overlay();
}

/// The extensions `save_text_file` writes: the recovery code (.txt), a conversation export (.md,
/// AI-25) and the MCP server export (.json).
const TEXT_EXTENSIONS: [&str; 3] = ["txt", "md", "json"];

/// Writes text to a path the user chose in the native save dialog.
#[tauri::command]
#[specta::specta]
pub fn save_text_file(path: String, contents: String) -> AppResult<()> {
    let path = std::path::PathBuf::from(path);
    // Only plain-text files at an absolute, user-chosen location: this command is not a general
    // file-write primitive for the WebView.
    if !path.is_absolute() || path.is_dir() || !is_text_file(&path) {
        return Err(AppError::invalid(
            "path",
            "choose a .txt, .md or .json file location",
        ));
    }
    std::fs::write(&path, contents)?;
    Ok(())
}

fn is_text_file(path: &std::path::Path) -> bool {
    path.extension()
        .is_some_and(|e| TEXT_EXTENSIONS.iter().any(|t| e.eq_ignore_ascii_case(t)))
}

/// Feeds the idle auto-lock timer (SEC-02).
#[tauri::command]
#[specta::specta]
pub fn activity_ping(state: State<'_, AppState>) {
    state.touch_activity();
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn saves_only_the_text_files_the_app_exports() {
        for name in ["code.txt", "Chat.MD", "mcp-servers.json"] {
            assert!(is_text_file(Path::new(name)), "{name}");
        }
        for name in ["run.exe", "notes", "config.json.bat", ".md"] {
            assert!(!is_text_file(Path::new(name)), "{name}");
        }
    }
}
