//! Synced settings item (spec §5.1 `Settings`), device-local preferences, and the star prompt state.

use hatoba_core::Vault;
use hatoba_core::model::{
    CursorStyle, Item, RightClick as CoreRightClick, SETTINGS_ID, Settings,
    TerminalSettings as CoreTerminal, ThemeMode,
};
use serde_json::{Map, Value};
use tauri::{AppHandle, State};

use crate::dto::{
    CursorChoice, LocalPrefs, RightClick, SettingsView, StarPrompt, TerminalSettings, ThemeChoice,
};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, now_ms};
use crate::{lock, sync};

/// Keys that builds before the terminal behaviour was synced kept in `local_prefs`.
const LEGACY_RIGHT_CLICK: &str = "right_click";
const LEGACY_CONFIRM_PASTE: &str = "confirm_multiline_paste";

pub fn settings_view(s: &Settings) -> SettingsView {
    SettingsView {
        terminal: TerminalSettings {
            font_family: s.terminal.font_family.clone(),
            font_size: s.terminal.font_size.into(),
            theme: match s.terminal.theme {
                ThemeMode::Light => ThemeChoice::Light,
                ThemeMode::Dark => ThemeChoice::Dark,
                ThemeMode::System => ThemeChoice::System,
            },
            cursor_style: match s.terminal.cursor_style {
                CursorStyle::Bar => CursorChoice::Bar,
                CursorStyle::Underline => CursorChoice::Underline,
                CursorStyle::Block => CursorChoice::Block,
            },
            scrollback: s.terminal.scrollback,
            right_click: match s.terminal.right_click() {
                CoreRightClick::Menu => RightClick::Menu,
                CoreRightClick::CopyPaste => RightClick::CopyPaste,
            },
            confirm_multiline_paste: s.terminal.confirm_multiline_paste(),
        },
        auto_lock_minutes: s.auto_lock_minutes,
        lock_disconnects_sessions: s.lock_disconnects_sessions,
    }
}

#[tauri::command]
#[specta::specta]
pub fn settings_get(state: State<'_, AppState>) -> AppResult<SettingsView> {
    state.with_unlocked(|v| Ok(settings_view(&v.settings())))
}

#[tauri::command]
#[specta::specta]
pub fn settings_save(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: SettingsView,
) -> AppResult<()> {
    let t = &settings.terminal;
    if t.font_family.trim().is_empty() || t.font_family.len() > 200 {
        return Err(AppError::invalid("font_family", "invalid font family"));
    }
    if !(6..=48).contains(&t.font_size) {
        return Err(AppError::invalid(
            "font_size",
            "font size must be between 6 and 48",
        ));
    }
    if !(100..=200_000).contains(&t.scrollback) {
        return Err(AppError::invalid(
            "scrollback",
            "scrollback must be between 100 and 200000 lines",
        ));
    }
    state.with_unlocked(|v| {
        // The behaviour fields start from the stored ones, so a field the user never changed
        // stays unset (`TerminalSettings::set_right_click`).
        let mut terminal = CoreTerminal {
            font_family: t.font_family.trim().to_owned(),
            font_size: t.font_size as u16,
            theme: match t.theme {
                ThemeChoice::Light => ThemeMode::Light,
                ThemeChoice::Dark => ThemeMode::Dark,
                ThemeChoice::System => ThemeMode::System,
            },
            cursor_style: match t.cursor_style {
                CursorChoice::Bar => CursorStyle::Bar,
                CursorChoice::Underline => CursorStyle::Underline,
                CursorChoice::Block => CursorStyle::Block,
            },
            scrollback: t.scrollback,
            ..v.settings().terminal
        };
        terminal.set_right_click(match t.right_click {
            RightClick::Menu => CoreRightClick::Menu,
            RightClick::CopyPaste => CoreRightClick::CopyPaste,
        });
        terminal.set_confirm_multiline_paste(t.confirm_multiline_paste);
        let model = Settings {
            terminal,
            auto_lock_minutes: settings.auto_lock_minutes.min(24 * 60),
            lock_disconnects_sessions: settings.lock_disconnects_sessions,
            updated_at: 0,
        };
        v.put(Some(SETTINGS_ID), Item::Settings(model))?;
        Ok(())
    })?;
    lock::refresh_policy(&state);
    sync::local_change(&app);
    Ok(())
}

/// Readable while locked: the unlock screen needs the language.
#[tauri::command]
#[specta::specta]
pub fn prefs_get(state: State<'_, AppState>) -> AppResult<LocalPrefs> {
    let json = state.vault().local_prefs()?;
    Ok(json
        .map(|j| LocalPrefs::from_stored(&j))
        .unwrap_or_default())
}

#[tauri::command]
#[specta::specta]
pub fn prefs_save(state: State<'_, AppState>, prefs: LocalPrefs) -> AppResult<()> {
    let mut vault = state.vault();
    let json = merge_prefs(vault.local_prefs()?.as_deref(), &prefs)?;
    vault.set_local_prefs(&json)?;
    Ok(())
}

/// Writes `prefs` over the stored JSON. Keys this build does not know stay: the terminal
/// behaviour older builds kept there waits for [`adopt_legacy_prefs`], which needs the vault
/// unlocked and the latest settings, and a newer build's prefs survive a downgrade.
fn merge_prefs(stored: Option<&str>, prefs: &LocalPrefs) -> AppResult<String> {
    let mut merged = stored_prefs(stored);
    if let Value::Object(fields) =
        serde_json::to_value(prefs).map_err(|e| AppError::internal(e.to_string()))?
    {
        merged.extend(fields);
    }
    serde_json::to_string(&merged).map_err(|e| AppError::internal(e.to_string()))
}

fn stored_prefs(json: Option<&str>) -> Map<String, Value> {
    match json.map(serde_json::from_str) {
        Some(Ok(Value::Object(map))) => map,
        _ => Map::new(),
    }
}

/// Moves the right-click and multi-line paste prefs that older builds kept in `local_prefs`
/// into the synced settings ([`Vault::adopt_device_terminal_prefs`]), then removes the keys from
/// `local_prefs`, so it does its work once. Returns whether it wrote the settings.
///
/// It must run on the latest settings: right after unlock when sync is off
/// ([`adopt_legacy_prefs_after_unlock`]), and otherwise after each successful sync round
/// (`sync::run_round`). Until then [`merge_prefs`] keeps the keys.
pub fn adopt_legacy_prefs(vault: &mut Vault) -> AppResult<bool> {
    let mut stored = stored_prefs(vault.local_prefs()?.as_deref());
    let right_click = stored.remove(LEGACY_RIGHT_CLICK);
    let confirm_paste = stored.remove(LEGACY_CONFIRM_PASTE);
    if right_click.is_none() && confirm_paste.is_none() {
        return Ok(false);
    }
    let wrote = vault.adopt_device_terminal_prefs(
        right_click.and_then(|v| serde_json::from_value(v).ok()),
        confirm_paste.as_ref().and_then(Value::as_bool),
    )?;
    let json = serde_json::to_string(&stored).map_err(|e| AppError::internal(e.to_string()))?;
    vault.set_local_prefs(&json)?;
    Ok(wrote)
}

/// The move after unlock. Without sync this device holds the only copy of the settings, so it
/// runs now; with sync configured it waits for the first successful round, which pulls any
/// newer settings first.
pub fn adopt_legacy_prefs_after_unlock(vault: &mut Vault) -> AppResult<bool> {
    if vault.sync_config()?.is_some() {
        return Ok(false);
    }
    adopt_legacy_prefs(vault)
}

/// The sidebar's star prompt (spec §9). The first read records when this device started waiting.
#[tauri::command]
#[specta::specta]
pub fn star_prompt_get(state: State<'_, AppState>) -> AppResult<StarPrompt> {
    load_star_prompt(&mut state.vault())
}

/// The user starred, opened the bug report form, or closed the prompt: it never shows again.
#[tauri::command]
#[specta::specta]
pub fn star_prompt_done(state: State<'_, AppState>) -> AppResult<()> {
    let mut vault = state.vault();
    let mut prompt = load_star_prompt(&mut vault)?;
    prompt.done = true;
    store_star_prompt(&mut vault, &prompt)
}

/// A missing or unreadable state starts the wait now.
fn load_star_prompt(vault: &mut Vault) -> AppResult<StarPrompt> {
    if let Some(prompt) = vault
        .star_prompt()?
        .and_then(|j| serde_json::from_str(&j).ok())
    {
        return Ok(prompt);
    }
    let prompt = StarPrompt {
        first_seen_at: now_ms(),
        done: false,
    };
    store_star_prompt(vault, &prompt)?;
    Ok(prompt)
}

fn store_star_prompt(vault: &mut Vault, prompt: &StarPrompt) -> AppResult<()> {
    let json = serde_json::to_string(prompt).map_err(|e| AppError::internal(e.to_string()))?;
    vault.set_star_prompt(&json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use hatoba_core::model::{Item, RightClick, SETTINGS_ID};
    use hatoba_core::sync::SyncConfig;
    use hatoba_core::{KdfParams, Vault};
    use serde_json::{Value, json};

    use super::{adopt_legacy_prefs, adopt_legacy_prefs_after_unlock, merge_prefs};
    use crate::dto::{Language, LocalPrefs};

    const PW: &str = "correct horse battery staple";

    /// `local_prefs` as the last build that kept the terminal behaviour on the device wrote it.
    fn legacy_prefs(right_click: &str, confirm_multiline_paste: bool) -> String {
        json!({
            "language": "ja",
            "appearance": "dark",
            "density": "compact",
            "right_click": right_click,
            "host_probe": false,
            "confirm_multiline_paste": confirm_multiline_paste,
            "auto_update_check": true,
        })
        .to_string()
    }

    fn vault() -> Vault {
        let mut vault = Vault::open_in_memory().unwrap();
        vault
            .create_with_params(PW, KdfParams::for_tests())
            .unwrap();
        vault
    }

    /// Stores a settings item as a build without the terminal behaviour fields wrote it.
    fn put_older_settings(vault: &mut Vault, terminal: Value) {
        let plaintext = json!({"type": "settings", "terminal": terminal}).to_string();
        let item = Item::from_plaintext(plaintext.as_bytes()).unwrap();
        vault.put(Some(SETTINGS_ID), item).unwrap();
    }

    fn stored(vault: &Vault) -> Value {
        serde_json::from_str(&vault.local_prefs().unwrap().unwrap()).unwrap()
    }

    #[test]
    fn device_values_move_into_settings_from_an_older_build() {
        let mut vault = vault();
        put_older_settings(&mut vault, json!({"font_size": 15}));
        let before = vault.settings().updated_at;
        vault.set_local_prefs(&legacy_prefs("menu", false)).unwrap();

        assert!(adopt_legacy_prefs(&mut vault).unwrap());

        let settings = vault.settings();
        assert_eq!(settings.terminal.right_click, Some(RightClick::Menu));
        assert_eq!(settings.terminal.confirm_multiline_paste, Some(false));
        assert_eq!(settings.terminal.font_size, 15);
        // A newer write, so it wins over the older copy on the server.
        assert!(settings.updated_at > before);
        let prefs = stored(&vault);
        assert!(prefs.get("right_click").is_none());
        assert!(prefs.get("confirm_multiline_paste").is_none());
        let kept = LocalPrefs::from_stored(&prefs.to_string());
        assert_eq!(kept.language, Language::Ja);
        assert!(!kept.host_probe && kept.auto_update_check);
    }

    #[test]
    fn a_value_already_recorded_is_kept() {
        let mut vault = vault();
        // Another device recorded copy/paste (the default) and left the paste field unset.
        put_older_settings(&mut vault, json!({"right_click": "copy_paste"}));
        vault.set_local_prefs(&legacy_prefs("menu", false)).unwrap();

        adopt_legacy_prefs(&mut vault).unwrap();

        let t = vault.settings().terminal;
        assert_eq!(t.right_click, Some(RightClick::CopyPaste));
        assert_eq!(t.confirm_multiline_paste, Some(false));
    }

    #[test]
    fn runs_once() {
        let mut vault = vault();
        vault.set_local_prefs(&legacy_prefs("menu", false)).unwrap();
        adopt_legacy_prefs(&mut vault).unwrap();

        // The user switches back on another device and the change syncs in.
        let mut settings = vault.settings();
        settings.terminal.set_right_click(RightClick::CopyPaste);
        settings.terminal.set_confirm_multiline_paste(true);
        vault.put(None, Item::Settings(settings)).unwrap();
        let after_change = vault.settings();

        assert!(!adopt_legacy_prefs(&mut vault).unwrap());
        assert_eq!(vault.settings(), after_change);
    }

    #[test]
    fn default_values_leave_the_fields_unset() {
        let mut vault = vault();
        let before = vault.settings();
        vault
            .set_local_prefs(&legacy_prefs("copy_paste", true))
            .unwrap();

        assert!(!adopt_legacy_prefs(&mut vault).unwrap());

        assert_eq!(vault.settings(), before);
        assert!(stored(&vault).get("right_click").is_none());

        // So a device with a customised value can still bring it over.
        vault.set_local_prefs(&legacy_prefs("menu", true)).unwrap();
        adopt_legacy_prefs(&mut vault).unwrap();
        assert_eq!(
            vault.settings().terminal.right_click,
            Some(RightClick::Menu)
        );
    }

    #[test]
    fn creates_the_settings_item_when_none_is_stored() {
        // A vault restored from a backup that holds no settings item.
        let path = std::env::temp_dir().join(format!(
            "hatoba-settings-test-{}.hatoba",
            hatoba_core::new_id()
        ));
        vault().export_backup(&path).unwrap();
        let mut backup: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        backup["items"]
            .as_array_mut()
            .unwrap()
            .retain(|item| item["id"] != SETTINGS_ID);
        std::fs::write(&path, backup.to_string()).unwrap();
        let mut vault = Vault::open_in_memory().unwrap();
        let imported = vault.import_backup_into_new_vault(&path, PW);
        std::fs::remove_file(&path).unwrap();
        imported.unwrap();
        assert!(vault.get(SETTINGS_ID).is_none());
        vault.set_local_prefs(&legacy_prefs("menu", false)).unwrap();

        adopt_legacy_prefs(&mut vault).unwrap();

        let settings = vault.get(SETTINGS_ID).and_then(Item::as_settings).unwrap();
        assert_eq!(settings.terminal.right_click, Some(RightClick::Menu));
        assert_eq!(settings.terminal.confirm_multiline_paste, Some(false));
    }

    #[test]
    fn after_unlock_it_waits_for_sync_when_sync_is_configured() {
        let mut vault = vault();
        vault.set_local_prefs(&legacy_prefs("menu", false)).unwrap();
        let config = SyncConfig::Worker {
            url: "https://sync.example.workers.dev".into(),
        };
        vault.set_sync_config(Some(&config)).unwrap();
        let before = vault.settings();

        assert!(!adopt_legacy_prefs_after_unlock(&mut vault).unwrap());
        assert_eq!(vault.settings(), before);
        assert_eq!(stored(&vault)["right_click"], "menu");

        // Without sync this device holds the only copy, so the move runs right away.
        vault.set_sync_config(None).unwrap();
        assert!(adopt_legacy_prefs_after_unlock(&mut vault).unwrap());
        assert_eq!(
            vault.settings().terminal.right_click,
            Some(RightClick::Menu)
        );
    }

    #[test]
    fn saving_prefs_keeps_the_keys_waiting_for_unlock() {
        // The language changes on the lock screen before the first unlock after the upgrade.
        let prefs = LocalPrefs {
            language: Language::En,
            ..LocalPrefs::from_stored(&legacy_prefs("menu", false))
        };
        let json = merge_prefs(Some(&legacy_prefs("menu", false)), &prefs).unwrap();
        let saved: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(saved["language"], "en");
        assert_eq!(saved["right_click"], "menu");
        assert_eq!(saved["confirm_multiline_paste"], false);
        let fresh: Value =
            serde_json::from_str(&merge_prefs(None, &LocalPrefs::default()).unwrap()).unwrap();
        assert_eq!(fresh, serde_json::to_value(LocalPrefs::default()).unwrap());
    }
}
