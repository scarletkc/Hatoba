//! The command history behind command suggestions (TERM-14, TERM-15): the commands typed in each
//! saved host's terminals and each local shell, kept sealed on this device and never synced.
//! `hatoba-core` keeps them; this module checks the owners the frontend names.

use hatoba_core::LOCAL_SHELL_OWNER_PREFIX;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// The longest local shell id an owner names, in characters.
const MAX_SHELL_ID_CHARS: usize = 200;

/// Checks that `owner` names a saved host by its item id, or a local shell as `local:` and its id.
fn check_owner(owner: &str) -> AppResult<()> {
    let plausible = match owner.strip_prefix(LOCAL_SHELL_OWNER_PREFIX) {
        Some(shell) => (1..=MAX_SHELL_ID_CHARS).contains(&shell.chars().count()),
        None => is_item_id(owner),
    };
    if plausible {
        Ok(())
    } else {
        Err(AppError::invalid(
            "owner",
            "name a saved host by its id, or a local shell as local:<shell id>",
        ))
    }
}

/// An item id as the sync Worker accepts them: 1 to 64 ASCII letters, digits, `-` and `_`.
fn is_item_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The commands typed on this device in a saved host's terminals or in a local shell, newest
/// first. `owner` is the host's id (SSH or Telnet), or `local:` and the shell's id
/// (`LocalShellView.id`). A host or shell without a history has an empty one.
#[tauri::command]
#[specta::specta]
pub fn command_history_list(state: State<'_, AppState>, owner: String) -> AppResult<Vec<String>> {
    check_owner(&owner)?;
    state.with_unlocked(|v| {
        let mut commands = v.command_history(&owner)?;
        Ok(std::mem::take(&mut *commands))
    })
}

/// Puts a command that Enter sent to the shell at the top of the history of `owner` (as in
/// `command_history_list`). A command already there moves up, and past 1,000 commands the oldest
/// goes. Nothing is kept for a command that is blank, starts with a space or is longer than 4,096
/// characters, nor for a host that is no longer saved. A quick connection keeps its commands in
/// its tab and never sends them here.
#[tauri::command]
#[specta::specta]
pub fn command_history_add(
    state: State<'_, AppState>,
    owner: String,
    command: String,
) -> AppResult<()> {
    check_owner(&owner)?;
    state.with_unlocked(|v| {
        v.command_history_add(&owner, &command)?;
        Ok(())
    })
}

/// Removes a command from the history of `owner` (as in `command_history_list`), as Shift+Delete
/// does in the suggestion list. A command the history does not have changes nothing.
#[tauri::command]
#[specta::specta]
pub fn command_history_remove(
    state: State<'_, AppState>,
    owner: String,
    command: String,
) -> AppResult<()> {
    check_owner(&owner)?;
    state.with_unlocked(|v| {
        v.command_history_remove(&owner, &command)?;
        Ok(())
    })
}

/// Deletes the command history of every saved host and local shell on this device (Settings →
/// Terminal → Clear Command History).
#[tauri::command]
#[specta::specta]
pub fn command_history_clear(state: State<'_, AppState>) -> AppResult<()> {
    state.with_unlocked(|v| Ok(v.command_history_clear()?))
}

#[cfg(test)]
mod tests {
    use super::check_owner;

    #[test]
    fn owners_are_hosts_or_local_shells() {
        let longest_shell = format!("local:{}", "é".repeat(200));
        for owner in [
            "0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b",
            "local:pwsh",
            "local:wsl:Ubuntu-24.04",
            "local:/bin/zsh",
            longest_shell.as_str(),
        ] {
            assert!(check_owner(owner).is_ok(), "{owner}");
        }
        let long_id = "x".repeat(65);
        let long_shell = format!("local:{}", "x".repeat(201));
        for owner in [
            "",
            "local:",
            "Local:pwsh",
            "host id",
            "a/b",
            long_id.as_str(),
            long_shell.as_str(),
        ] {
            let err = check_owner(owner).unwrap_err();
            assert_eq!(err.field.as_deref(), Some("owner"), "{owner}");
        }
    }
}
