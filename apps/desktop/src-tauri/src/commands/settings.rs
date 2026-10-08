//! Synced settings item (spec §5.1 `Settings`), device-local preferences, and the star prompt state.

use hatoba_core::model::{
    CursorStyle, Item, SETTINGS_ID, Settings, TerminalSettings as CoreTerminal, ThemeMode,
};
use hatoba_core::vault::Vault;
use tauri::{AppHandle, State};

use crate::dto::{
    CursorChoice, LocalPrefs, SettingsView, StarPrompt, TerminalSettings, ThemeChoice,
};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, now_ms};
use crate::{lock, sync};

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
    let model = Settings {
        terminal: CoreTerminal {
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
        },
        auto_lock_minutes: settings.auto_lock_minutes.min(24 * 60),
        lock_disconnects_sessions: settings.lock_disconnects_sessions,
        updated_at: 0,
    };
    state.with_unlocked(|v| {
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
    let json = serde_json::to_string(&prefs).map_err(|e| AppError::internal(e.to_string()))?;
    state.vault().set_local_prefs(&json)?;
    Ok(())
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
