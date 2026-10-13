//! Synced settings item (spec §5.1 `Settings`), device-local preferences, and the star prompt
//! state.

use hatoba_core::Vault;
use hatoba_core::model::{
    CursorStyle, Item, RightClick as CoreRightClick, SETTINGS_ID, Settings,
    TerminalSettings as CoreTerminal, ThemeMode,
};
use serde_json::{Map, Value};
use tauri::{AppHandle, State};

use crate::dto::{
    CursorChoice, LocalPrefs, RightClick, SettingsPatch, SettingsView, StarPrompt,
    TerminalSettings, TerminalSettingsPatch, ThemeChoice,
};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, now_ms};
use crate::{lock, sync};

/// Keys that builds before the terminal behaviour was synced kept in `local_prefs`.
const LEGACY_RIGHT_CLICK: &str = "right_click";
const LEGACY_CONFIRM_PASTE: &str = "confirm_multiline_paste";

const MAX_AUTO_LOCK_MINUTES: u32 = 24 * 60;

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
    patch: SettingsPatch,
) -> AppResult<()> {
    if state.with_unlocked(|v| save_settings(v, &patch))? {
        sync::local_change(&app);
    }
    Ok(())
}

/// Writes the fields `patch` sets over the stored settings and returns whether that changed them.
/// The other fields keep their stored values, which may be newer than what the window that sent
/// `patch` shows. Nothing is written when a field is invalid.
fn save_settings(v: &mut Vault, patch: &SettingsPatch) -> AppResult<bool> {
    let stored = v.settings();
    let mut settings = stored.clone();
    patch_terminal(&mut settings.terminal, &patch.terminal)?;
    if settings == stored {
        return Ok(false);
    }
    v.put(Some(SETTINGS_ID), Item::Settings(settings))?;
    Ok(true)
}

fn patch_terminal(t: &mut CoreTerminal, patch: &TerminalSettingsPatch) -> AppResult<()> {
    if let Some(font_family) = &patch.font_family {
        if font_family.trim().is_empty() || font_family.len() > 200 {
            return Err(AppError::invalid("font_family", "invalid font family"));
        }
        font_family.trim().clone_into(&mut t.font_family);
    }
    if let Some(font_size) = patch.font_size {
        if !(6..=48).contains(&font_size) {
            return Err(AppError::invalid(
                "font_size",
                "font size must be between 6 and 48",
            ));
        }
        t.font_size = font_size as u16;
    }
    if let Some(theme) = patch.theme {
        t.theme = match theme {
            ThemeChoice::Light => ThemeMode::Light,
            ThemeChoice::Dark => ThemeMode::Dark,
            ThemeChoice::System => ThemeMode::System,
        };
    }
    if let Some(cursor_style) = patch.cursor_style {
        t.cursor_style = match cursor_style {
            CursorChoice::Bar => CursorStyle::Bar,
            CursorChoice::Underline => CursorStyle::Underline,
            CursorChoice::Block => CursorStyle::Block,
        };
    }
    if let Some(scrollback) = patch.scrollback {
        if !(100..=200_000).contains(&scrollback) {
            return Err(AppError::invalid(
                "scrollback",
                "scrollback must be between 100 and 200000 lines",
            ));
        }
        t.scrollback = scrollback;
    }
    // A behaviour field the user never changed stays unset (`TerminalSettings::set_right_click`).
    if let Some(right_click) = patch.right_click {
        t.set_right_click(match right_click {
            RightClick::Menu => CoreRightClick::Menu,
            RightClick::CopyPaste => CoreRightClick::CopyPaste,
        });
    }
    if let Some(confirm) = patch.confirm_multiline_paste {
        t.set_confirm_multiline_paste(confirm);
    }
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
pub fn prefs_save(state: State<'_, AppState>, mut prefs: LocalPrefs) -> AppResult<()> {
    prefs.auto_lock_minutes = prefs.auto_lock_minutes.min(MAX_AUTO_LOCK_MINUTES);
    prefs.dashboard_interval_secs = prefs.dashboard_interval_secs.clamp(1, 60);
    for alert in [
        &mut prefs.dashboard_cpu_alert,
        &mut prefs.dashboard_mem_alert,
        &mut prefs.dashboard_disk_alert,
    ] {
        *alert = (*alert).min(100);
    }
    prefs.dashboard_hidden_columns.retain(|c| c.len() <= 32);
    prefs.dashboard_hidden_columns.truncate(32);
    {
        let mut vault = state.vault();
        let json = merge_prefs(vault.local_prefs()?.as_deref(), &prefs)?;
        vault.set_local_prefs(&json)?;
    }
    lock::refresh_policy(&state);
    Ok(())
}

/// Writes `prefs` over the stored JSON. Keys this build does not know stay: the terminal
/// behaviour older builds kept there waits for [`adopt_legacy_prefs`], which needs the vault
/// unlocked and the latest settings, and a newer build's prefs survive a downgrade.
pub(crate) fn merge_prefs(stored: Option<&str>, prefs: &LocalPrefs) -> AppResult<String> {
    let mut merged = stored_prefs(stored);
    if let Value::Object(fields) =
        serde_json::to_value(prefs).map_err(|e| AppError::internal(e.to_string()))?
    {
        merged.extend(fields);
    }
    serde_json::to_string(&merged).map_err(|e| AppError::internal(e.to_string()))
}

pub(crate) fn stored_prefs(json: Option<&str>) -> Map<String, Value> {
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

    use super::{
        adopt_legacy_prefs, adopt_legacy_prefs_after_unlock, merge_prefs, save_settings,
        settings_view,
    };
    use crate::dto::{Language, LocalPrefs, SettingsPatch, TerminalSettingsPatch};

    const PW: &str = "correct horse battery staple";

    /// A right-click value other than this platform's default.
    fn custom() -> RightClick {
        match RightClick::platform_default() {
            RightClick::Menu => RightClick::CopyPaste,
            RightClick::CopyPaste => RightClick::Menu,
        }
    }

    /// `local_prefs` as the last build that kept the terminal behaviour on the device wrote it.
    fn legacy_prefs(right_click: RightClick, confirm_multiline_paste: bool) -> String {
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
        vault
            .set_local_prefs(&legacy_prefs(custom(), false))
            .unwrap();

        assert!(adopt_legacy_prefs(&mut vault).unwrap());

        let settings = vault.settings();
        assert_eq!(settings.terminal.right_click, Some(custom()));
        assert_eq!(settings.terminal.confirm_multiline_paste, Some(false));
        assert_eq!(settings.terminal.font_size, 15);
        // A newer write, so it wins over the older copy on the server.
        assert!(settings.updated_at > before);
        let prefs = stored(&vault);
        assert!(prefs.get("right_click").is_none());
        assert!(prefs.get("confirm_multiline_paste").is_none());
        let kept = LocalPrefs::from_stored(&prefs.to_string());
        assert_eq!(kept.language, Language::Ja);
        assert!(!kept.host_probe);
    }

    #[test]
    fn a_value_already_recorded_is_kept() {
        let mut vault = vault();
        // Another device recorded the default and left the paste field unset.
        put_older_settings(
            &mut vault,
            json!({"right_click": RightClick::platform_default()}),
        );
        vault
            .set_local_prefs(&legacy_prefs(custom(), false))
            .unwrap();

        adopt_legacy_prefs(&mut vault).unwrap();

        let t = vault.settings().terminal;
        assert_eq!(t.right_click, Some(RightClick::platform_default()));
        assert_eq!(t.confirm_multiline_paste, Some(false));
    }

    #[test]
    fn runs_once() {
        let mut vault = vault();
        vault
            .set_local_prefs(&legacy_prefs(custom(), false))
            .unwrap();
        adopt_legacy_prefs(&mut vault).unwrap();

        // The user switches back on another device and the change syncs in.
        let mut settings = vault.settings();
        settings
            .terminal
            .set_right_click(RightClick::platform_default());
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
            .set_local_prefs(&legacy_prefs(RightClick::platform_default(), true))
            .unwrap();

        assert!(!adopt_legacy_prefs(&mut vault).unwrap());

        assert_eq!(vault.settings(), before);
        assert!(stored(&vault).get("right_click").is_none());

        // So a device with a customised value can still bring it over.
        vault
            .set_local_prefs(&legacy_prefs(custom(), true))
            .unwrap();
        adopt_legacy_prefs(&mut vault).unwrap();
        assert_eq!(vault.settings().terminal.right_click, Some(custom()));
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
        vault
            .set_local_prefs(&legacy_prefs(custom(), false))
            .unwrap();

        adopt_legacy_prefs(&mut vault).unwrap();

        let settings = vault.get(SETTINGS_ID).and_then(Item::as_settings).unwrap();
        assert_eq!(settings.terminal.right_click, Some(custom()));
        assert_eq!(settings.terminal.confirm_multiline_paste, Some(false));
    }

    #[test]
    fn after_unlock_it_waits_for_sync_when_sync_is_configured() {
        let mut vault = vault();
        vault
            .set_local_prefs(&legacy_prefs(custom(), false))
            .unwrap();
        let config = SyncConfig::Worker {
            url: "https://sync.example.workers.dev".into(),
            deployment: None,
        };
        vault.set_sync_config(Some(&config)).unwrap();
        let before = vault.settings();

        assert!(!adopt_legacy_prefs_after_unlock(&mut vault).unwrap());
        assert_eq!(vault.settings(), before);
        assert_eq!(stored(&vault)["right_click"], json!(custom()));

        // Without sync this device holds the only copy, so the move runs right away.
        vault.set_sync_config(None).unwrap();
        assert!(adopt_legacy_prefs_after_unlock(&mut vault).unwrap());
        assert_eq!(vault.settings().terminal.right_click, Some(custom()));
    }

    fn terminal(patch: TerminalSettingsPatch) -> SettingsPatch {
        SettingsPatch { terminal: patch }
    }

    #[test]
    fn a_save_from_a_stale_window_keeps_the_fields_it_leaves_out() {
        let mut vault = vault();
        // Settings → Terminal reads the settings, then a sync round pulls another device's changes.
        let shown = settings_view(&vault.settings()).terminal;
        let mut pulled = vault.settings();
        pulled.terminal.scrollback = 50_000;
        pulled.terminal.set_right_click(custom());
        vault
            .put(Some(SETTINGS_ID), Item::Settings(pulled))
            .unwrap();

        // The user changes the font size in that window, which sends only that field.
        let bigger: SettingsPatch =
            serde_json::from_value(json!({"terminal": {"font_size": shown.font_size + 1}}))
                .unwrap();
        assert!(save_settings(&mut vault, &bigger).unwrap());

        let t = vault.settings().terminal;
        assert_eq!(u32::from(t.font_size), shown.font_size + 1);
        assert_eq!(t.scrollback, 50_000);
        assert_eq!(t.right_click, Some(custom()));
        assert_eq!(t.font_family, shown.font_family);
    }

    #[test]
    fn a_patch_checks_only_the_fields_it_sets() {
        let mut vault = vault();
        // Another device stored a size this one would refuse to save.
        put_older_settings(&mut vault, json!({"font_size": 60}));
        let scrollback = terminal(TerminalSettingsPatch {
            scrollback: Some(5_000),
            ..TerminalSettingsPatch::default()
        });
        assert!(save_settings(&mut vault, &scrollback).unwrap());
        let t = vault.settings().terminal;
        assert_eq!((t.font_size, t.scrollback), (60, 5_000));

        // An invalid field is refused, and the valid ones sent with it are not written either.
        let before = vault.settings();
        let font = || Some("Fira Code".to_owned());
        for (patch, field) in [
            (
                TerminalSettingsPatch {
                    font_family: font(),
                    font_size: Some(5),
                    ..TerminalSettingsPatch::default()
                },
                "font_size",
            ),
            (
                TerminalSettingsPatch {
                    font_family: font(),
                    scrollback: Some(99),
                    ..TerminalSettingsPatch::default()
                },
                "scrollback",
            ),
            (
                TerminalSettingsPatch {
                    font_family: Some("  ".into()),
                    ..TerminalSettingsPatch::default()
                },
                "font_family",
            ),
        ] {
            let err = save_settings(&mut vault, &terminal(patch)).unwrap_err();
            assert_eq!(err.field.as_deref(), Some(field));
        }
        assert_eq!(vault.settings(), before);
    }

    #[test]
    fn a_patch_that_changes_nothing_writes_nothing() {
        let mut vault = vault();
        let before = vault.settings();
        // The window sends back what it shows, the defaults of the unset behaviour fields included.
        let shown = settings_view(&before).terminal;
        let same = terminal(TerminalSettingsPatch {
            font_family: Some(shown.font_family.clone()),
            font_size: Some(shown.font_size),
            right_click: Some(shown.right_click),
            confirm_multiline_paste: Some(shown.confirm_multiline_paste),
            ..TerminalSettingsPatch::default()
        });
        assert!(!save_settings(&mut vault, &same).unwrap());
        assert!(!save_settings(&mut vault, &SettingsPatch::default()).unwrap());
        assert_eq!(vault.settings(), before);
        assert_eq!(vault.settings().terminal.right_click, None);
    }

    #[test]
    fn saving_prefs_keeps_the_keys_waiting_for_unlock() {
        // The language changes on the lock screen before the first unlock after the upgrade.
        let prefs = LocalPrefs {
            language: Language::En,
            ..LocalPrefs::from_stored(&legacy_prefs(custom(), false))
        };
        let json = merge_prefs(Some(&legacy_prefs(custom(), false)), &prefs).unwrap();
        let saved: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(saved["language"], "en");
        assert_eq!(saved["right_click"], json!(custom()));
        assert_eq!(saved["confirm_multiline_paste"], false);
        let fresh: Value =
            serde_json::from_str(&merge_prefs(None, &LocalPrefs::default()).unwrap()).unwrap();
        assert_eq!(fresh, serde_json::to_value(LocalPrefs::default()).unwrap());
    }
}
