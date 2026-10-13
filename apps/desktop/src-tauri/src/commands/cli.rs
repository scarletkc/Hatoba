//! The command line (HOST-13): the arguments `hatoba …` was started with, and the `hatoba`
//! command itself (Settings → General).

use hatoba_core::Vault;
use serde_json::Value;
use tauri::State;

use crate::commands::settings::stored_prefs;
use crate::dto::CliCommandView;
use crate::error::{AppError, AppResult};
use crate::launch;
use crate::platform;
use crate::state::AppState;

/// Whether the command is on, kept in the local prefs beside the `LocalPrefs` fields. Those leave
/// it alone (`merge_prefs`), so only `cli_command_set` writes it.
const PREF: &str = "cli_command";

/// The arguments of the launches the window has not handled yet, oldest first. Each is taken
/// once, and only while the vault is unlocked: a launch during a lock waits for the unlock.
#[tauri::command]
#[specta::specta]
pub fn launch_take(state: State<'_, AppState>) -> AppResult<Vec<Vec<String>>> {
    state.with_unlocked(|_| Ok(launch::QUEUE.take()))
}

#[tauri::command]
#[specta::specta]
pub fn cli_command_get(state: State<'_, AppState>) -> AppResult<CliCommandView> {
    let enabled = enabled(&state.vault())?;
    view(enabled)
}

/// Adds or removes the command. On macOS this may wait for an administrator's password.
#[tauri::command]
#[specta::specta]
pub async fn cli_command_set(
    state: State<'_, AppState>,
    enabled: bool,
) -> AppResult<CliCommandView> {
    tauri::async_runtime::spawn_blocking(move || {
        if enabled {
            platform::cli::install()
        } else {
            platform::cli::uninstall()
        }
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))??;
    set_enabled(&mut state.vault(), enabled)?;
    view(enabled)
}

fn view(enabled: bool) -> AppResult<CliCommandView> {
    let location = platform::cli::location()?;
    Ok(CliCommandView {
        enabled,
        on_path: platform::cli::on_path(&location),
        location: location.display().to_string(),
    })
}

fn enabled(vault: &Vault) -> AppResult<bool> {
    Ok(stored_prefs(vault.local_prefs()?.as_deref())
        .get(PREF)
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

fn set_enabled(vault: &mut Vault, enabled: bool) -> AppResult<()> {
    let mut prefs = stored_prefs(vault.local_prefs()?.as_deref());
    prefs.insert(PREF.to_owned(), Value::Bool(enabled));
    let json = serde_json::to_string(&prefs).map_err(|e| AppError::internal(e.to_string()))?;
    Ok(vault.set_local_prefs(&json)?)
}

/// At startup, puts the command back when it is on but gone: an uninstall before a reinstall
/// took it off the `PATH`, or the AppImage moved. Only release builds on Windows and Linux do
/// this; macOS would ask for an administrator's password at launch.
pub fn restore(state: &AppState) {
    if cfg!(debug_assertions) || cfg!(target_os = "macos") {
        return;
    }
    if !enabled(&state.vault()).unwrap_or(false) {
        return;
    }
    std::thread::spawn(|| {
        if let Err(e) = platform::cli::install() {
            tracing::warn!("restoring the hatoba command failed: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use hatoba_core::Vault;

    use super::{enabled, set_enabled};
    use crate::commands::settings::merge_prefs;
    use crate::dto::LocalPrefs;

    /// Startup reads the setting before any unlock, so the vault here is never unlocked.
    #[test]
    fn the_setting_needs_no_unlock_and_survives_saving_the_other_prefs() {
        let mut vault = Vault::open_in_memory().unwrap();
        assert!(!enabled(&vault).unwrap());

        set_enabled(&mut vault, true).unwrap();
        let json = merge_prefs(
            vault.local_prefs().unwrap().as_deref(),
            &LocalPrefs::default(),
        );
        vault.set_local_prefs(&json.unwrap()).unwrap();
        assert!(enabled(&vault).unwrap());

        set_enabled(&mut vault, false).unwrap();
        assert!(!enabled(&vault).unwrap());
    }
}
