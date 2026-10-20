//! The command line (HOST-13): the arguments `hatoba …` was started with, and the `hatoba`
//! command itself (Settings → General), whose own commands other programs use the saved hosts
//! with (HOST-15).

use hatoba_core::Vault;
use serde_json::Value;
use tauri::{AppHandle, State};

use crate::commands::settings::stored_prefs;
use crate::dto::{CliActivity, CliCommandView};
use crate::error::{AppError, AppResult};
use crate::launch;
use crate::platform;
use crate::state::AppState;

/// Whether the command is on, kept in the local prefs beside the `LocalPrefs` fields. Those leave
/// it alone (`merge_prefs`), so only `cli_command_set` writes it.
const PREF: &str = "cli_command";
/// HOST-15: whether other programs may use the saved hosts through the command, kept the same way
/// and written only by `cli_access_set`.
const ACCESS_PREF: &str = "cli_access";

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
    view(&state.vault())
}

/// Adds or removes the command. On macOS this may wait for an administrator's password.
#[tauri::command]
#[specta::specta]
pub async fn cli_command_set(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> AppResult<CliCommandView> {
    blocking(move || {
        if enabled {
            platform::cli::install()
        } else {
            platform::cli::uninstall()
        }
    })
    .await?;
    set_flag(&mut state.vault(), PREF, enabled)?;
    crate::cli_access::sync(&app);
    view(&state.vault())
}

/// HOST-15: lets other programs use the saved hosts through the command, or stops them. Turning it
/// on puts the launcher in place again, so one from before HOST-15 runs the command's own
/// commands; on macOS that may wait for an administrator's password.
#[tauri::command]
#[specta::specta]
pub async fn cli_access_set(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> AppResult<CliCommandView> {
    if enabled && flag(&state.vault(), PREF)? {
        blocking(platform::cli::install).await?;
    }
    set_flag(&mut state.vault(), ACCESS_PREF, enabled)?;
    crate::cli_access::sync(&app);
    view(&state.vault())
}

/// What other programs did through the command since the unlock, newest first (HOST-15).
#[tauri::command]
#[specta::specta]
pub fn cli_activity_list(state: State<'_, AppState>) -> AppResult<Vec<CliActivity>> {
    state.with_unlocked(|_| Ok(state.cli.activity()))
}

async fn blocking(f: impl FnOnce() -> AppResult<()> + Send + 'static) -> AppResult<()> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
}

fn view(vault: &Vault) -> AppResult<CliCommandView> {
    let location = platform::cli::location()?;
    Ok(CliCommandView {
        enabled: flag(vault, PREF)?,
        on_path: platform::cli::on_path(&location),
        location: location.display().to_string(),
        access: flag(vault, ACCESS_PREF)?,
    })
}

/// Whether the app listens for the command's own commands (HOST-15): while the command and the
/// setting are both on. Readable while the vault is locked.
pub fn access_on(vault: &Vault) -> bool {
    flag(vault, PREF).unwrap_or(false) && flag(vault, ACCESS_PREF).unwrap_or(false)
}

fn enabled(vault: &Vault) -> AppResult<bool> {
    flag(vault, PREF)
}

fn flag(vault: &Vault, key: &str) -> AppResult<bool> {
    Ok(stored_prefs(vault.local_prefs()?.as_deref())
        .get(key)
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

fn set_flag(vault: &mut Vault, key: &str, on: bool) -> AppResult<()> {
    let mut prefs = stored_prefs(vault.local_prefs()?.as_deref());
    prefs.insert(key.to_owned(), Value::Bool(on));
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

    use super::{ACCESS_PREF, PREF, access_on, enabled, set_flag};
    use crate::commands::settings::merge_prefs;
    use crate::dto::LocalPrefs;

    /// Startup reads the setting before any unlock, so the vault here is never unlocked.
    #[test]
    fn the_setting_needs_no_unlock_and_survives_saving_the_other_prefs() {
        let mut vault = Vault::open_in_memory().unwrap();
        assert!(!enabled(&vault).unwrap());

        set_flag(&mut vault, PREF, true).unwrap();
        let json = merge_prefs(
            vault.local_prefs().unwrap().as_deref(),
            &LocalPrefs::default(),
        );
        vault.set_local_prefs(&json.unwrap()).unwrap();
        assert!(enabled(&vault).unwrap());

        set_flag(&mut vault, PREF, false).unwrap();
        assert!(!enabled(&vault).unwrap());
    }

    /// HOST-15: programs reach the hosts only while both switches are on.
    #[test]
    fn access_needs_the_command_and_the_setting() {
        let mut vault = Vault::open_in_memory().unwrap();
        assert!(!access_on(&vault));
        set_flag(&mut vault, ACCESS_PREF, true).unwrap();
        assert!(!access_on(&vault));
        set_flag(&mut vault, PREF, true).unwrap();
        assert!(access_on(&vault));
        set_flag(&mut vault, PREF, false).unwrap();
        assert!(!access_on(&vault));
    }
}
