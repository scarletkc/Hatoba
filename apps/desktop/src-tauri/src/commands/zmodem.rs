//! Local disk endpoints for the terminal's ZMODEM protocol, independent of SFTP.
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};

fn directory(app: &AppHandle) -> AppResult<PathBuf> {
    let config = app
        .path()
        .app_config_dir()
        .map_err(|e| AppError::invalid("path", e.to_string()))?
        .join("zmodem-directory.json");
    if config.exists() {
        let path: String = serde_json::from_slice(&std::fs::read(config)?)
            .map_err(|e| AppError::invalid("path", e.to_string()))?;
        return Ok(PathBuf::from(path));
    }
    dirs::download_dir()
        .ok_or_else(|| AppError::invalid("path", "Downloads directory is unavailable"))
}

#[tauri::command]
#[specta::specta]
pub fn zmodem_directory(
    app: AppHandle,
    state: State<'_, AppState>,
    path: Option<String>,
) -> AppResult<String> {
    state.with_unlocked(|_| Ok(()))?;
    if let Some(path) = path {
        let config = app
            .path()
            .app_config_dir()
            .map_err(|e| AppError::invalid("path", e.to_string()))?;
        std::fs::create_dir_all(&config)?;
        let config = config.join("zmodem-directory.json");
        if path.is_empty() {
            if config.exists() {
                std::fs::remove_file(config)?;
            }
        } else {
            let folder = PathBuf::from(&path);
            if !folder.is_absolute() || !folder.is_dir() {
                return Err(AppError::invalid(
                    "path",
                    "choose an existing absolute directory",
                ));
            }
            let json =
                serde_json::to_vec(&path).map_err(|e| AppError::invalid("path", e.to_string()))?;
            std::fs::write(config, json)?;
        }
    }
    Ok(directory(&app)?.to_string_lossy().into_owned())
}

#[tauri::command]
#[specta::specta]
pub fn zmodem_download_begin(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> AppResult<String> {
    state.with_unlocked(|_| {
        let folder = directory(&app)?;
        std::fs::create_dir_all(&folder)?;
        Ok(state
            .zmodem
            .begin(&folder, &name, uuid::Uuid::now_v7().to_string())?)
    })
}

#[tauri::command]
#[specta::specta]
pub fn zmodem_download_write(
    state: State<'_, AppState>,
    id: String,
    data: Vec<u8>,
) -> AppResult<()> {
    state.with_unlocked(|_| Ok(state.zmodem.write(&id, &data)?))
}

#[tauri::command]
#[specta::specta]
pub fn zmodem_download_finish(
    state: State<'_, AppState>,
    id: String,
    commit: bool,
) -> AppResult<Option<String>> {
    if !commit {
        return Ok(state.zmodem.finish(&id, false)?);
    }
    state.with_unlocked(|_| Ok(state.zmodem.finish(&id, true)?))
}
