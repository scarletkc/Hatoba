//! Terminal colour schemes the user made (TERM-06). Each is its own `terminal_scheme` item, so a
//! build that does not know the type leaves it alone, and settings pick one by its item id.

use hatoba_core::model::{
    Item, SchemeColors as CoreColors, SchemeMode as CoreMode, TerminalScheme,
};
use hatoba_core::vault::Vault;
use tauri::{AppHandle, State};

use crate::dto::{SchemeColors, SchemeMode, TerminalSchemeInput, TerminalSchemeView};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::sync;

const MAX_NAME_CHARS: usize = 64;

/// `#rrggbb` in lowercase, from `#RRGGBB` or `#rrggbb`.
fn hex_color(field: &str, value: &str) -> AppResult<String> {
    let value = value.trim();
    let digits = value.strip_prefix('#').unwrap_or_default();
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AppError::invalid(field, "colour must be #rrggbb"));
    }
    Ok(value.to_ascii_lowercase())
}

macro_rules! palette {
    ($($field:ident),* $(,)?) => {
        fn colors_view(c: &CoreColors) -> SchemeColors {
            SchemeColors { $($field: c.$field.clone()),* }
        }

        fn colors_from_input(c: &SchemeColors) -> AppResult<CoreColors> {
            Ok(CoreColors { $($field: hex_color(stringify!($field), &c.$field)?),* })
        }

        /// Every colour in a fixed order, for the conflict view.
        pub fn palette_summary(c: &CoreColors) -> String {
            [$(c.$field.as_str()),*].join(" ")
        }
    };
}

palette!(
    foreground,
    background,
    cursor,
    selection,
    black,
    red,
    green,
    yellow,
    blue,
    magenta,
    cyan,
    white,
    bright_black,
    bright_red,
    bright_green,
    bright_yellow,
    bright_blue,
    bright_magenta,
    bright_cyan,
    bright_white,
);

pub fn scheme_view(id: &str, s: &TerminalScheme) -> TerminalSchemeView {
    TerminalSchemeView {
        id: id.to_owned(),
        name: s.name.clone(),
        mode: match s.mode {
            CoreMode::Dark => SchemeMode::Dark,
            CoreMode::Light => SchemeMode::Light,
        },
        colors: colors_view(&s.colors),
        updated_at: s.updated_at,
    }
}

/// Checks the editor's form and builds the stored model.
fn scheme_from_input(input: &TerminalSchemeInput) -> AppResult<TerminalScheme> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::invalid("name", "name is required"));
    }
    if name.chars().count() > MAX_NAME_CHARS || name.contains(char::is_control) {
        return Err(AppError::invalid("name", "name is too long"));
    }
    Ok(TerminalScheme {
        name: name.to_owned(),
        mode: match input.mode {
            SchemeMode::Dark => CoreMode::Dark,
            SchemeMode::Light => CoreMode::Light,
        },
        colors: colors_from_input(&input.colors)?,
        updated_at: 0,
    })
}

fn find_scheme(v: &Vault, id: &str) -> AppResult<TerminalScheme> {
    v.get(id)
        .and_then(Item::as_terminal_scheme)
        .cloned()
        .ok_or_else(|| AppError::not_found("colour scheme"))
}

fn list(v: &Vault) -> Vec<TerminalSchemeView> {
    let mut schemes: Vec<TerminalSchemeView> = v
        .terminal_schemes()
        .iter()
        .map(|(id, s)| scheme_view(id, s))
        .collect();
    schemes.sort_by_cached_key(|s| s.name.to_lowercase());
    schemes
}

fn save(v: &mut Vault, input: &TerminalSchemeInput) -> AppResult<TerminalSchemeView> {
    if let Some(id) = &input.id {
        find_scheme(v, id)?;
    }
    let scheme = scheme_from_input(input)?;
    let id = v.put(input.id.as_deref(), Item::TerminalScheme(scheme))?;
    Ok(scheme_view(&id, &find_scheme(v, &id)?))
}

/// Deletes a scheme. Settings that picked it read it as the mode's default scheme, as they do
/// an id another build does not know, so they are left as they are.
fn remove(v: &mut Vault, id: &str) -> AppResult<()> {
    find_scheme(v, id)?;
    v.delete(id)?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn terminal_schemes_list(state: State<'_, AppState>) -> AppResult<Vec<TerminalSchemeView>> {
    state.with_unlocked(|v| Ok(list(v)))
}

#[tauri::command]
#[specta::specta]
pub fn terminal_scheme_save(
    app: AppHandle,
    state: State<'_, AppState>,
    input: TerminalSchemeInput,
) -> AppResult<TerminalSchemeView> {
    let view = state.with_unlocked(|v| save(v, &input))?;
    sync::local_change(&app);
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub fn terminal_scheme_delete(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    state.with_unlocked(|v| remove(v, &id))?;
    sync::local_change(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use hatoba_core::KdfParams;
    use hatoba_core::model::{SETTINGS_ID, Settings};

    use super::*;

    fn vault() -> Vault {
        let mut vault = Vault::open_in_memory().unwrap();
        vault
            .create_with_params("correct horse battery staple", KdfParams::for_tests())
            .unwrap();
        vault
    }

    fn colors(background: &str) -> SchemeColors {
        let c = |hex: &str| hex.to_owned();
        SchemeColors {
            foreground: c("#F8F8F2"),
            background: c(background),
            cursor: c("#f8f8f2"),
            selection: c("#44475a"),
            black: c("#21222c"),
            red: c("#ff5555"),
            green: c("#50fa7b"),
            yellow: c("#f1fa8c"),
            blue: c("#bd93f9"),
            magenta: c("#ff79c6"),
            cyan: c("#8be9fd"),
            white: c("#f8f8f2"),
            bright_black: c("#6272a4"),
            bright_red: c("#ff6e6e"),
            bright_green: c("#69ff94"),
            bright_yellow: c("#ffffa5"),
            bright_blue: c("#d6acff"),
            bright_magenta: c("#ff92df"),
            bright_cyan: c("#a4ffff"),
            bright_white: c("#ffffff"),
        }
    }

    fn input(name: &str, background: &str) -> TerminalSchemeInput {
        TerminalSchemeInput {
            id: None,
            name: name.into(),
            mode: SchemeMode::Dark,
            colors: colors(background),
        }
    }

    #[test]
    fn schemes_are_saved_listed_and_deleted() {
        let mut vault = vault();
        let saved = save(&mut vault, &input(" Dracula (Copy) ", "#282A36")).unwrap();
        assert_eq!(saved.name, "Dracula (Copy)");
        assert_eq!(saved.mode, SchemeMode::Dark);
        // Colours are stored in lowercase.
        assert_eq!(saved.colors.background, "#282a36");
        assert_eq!(saved.colors.foreground, "#f8f8f2");
        // The id is one the settings accept as a scheme id.
        assert!(super::super::settings::is_scheme_id(&saved.id));

        let mut edit = input("Alucard", "#f8f8f2");
        edit.id = Some(saved.id.clone());
        edit.mode = SchemeMode::Light;
        let edited = save(&mut vault, &edit).unwrap();
        assert_eq!(edited.id, saved.id);
        save(&mut vault, &input("blood", "#000000")).unwrap();
        let names: Vec<_> = list(&vault).into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["Alucard", "blood"]);

        remove(&mut vault, &saved.id).unwrap();
        assert_eq!(list(&vault).len(), 1);
        assert_eq!(
            remove(&mut vault, &saved.id).unwrap_err().code,
            crate::error::ErrorCode::NotFound
        );
        let mut missing = input("x", "#000000");
        missing.id = Some(saved.id);
        assert!(save(&mut vault, &missing).is_err());
    }

    #[test]
    fn invalid_schemes_are_refused() {
        let mut vault = vault();
        for (scheme, field) in [
            (input("  ", "#000000"), "name"),
            (input(&"x".repeat(65), "#000000"), "name"),
            (input("x", "000000"), "background"),
            (input("x", "#00000"), "background"),
            (input("x", "#00000g"), "background"),
            (input("x", "rgb(0, 0, 0)"), "background"),
        ] {
            let err = save(&mut vault, &scheme).unwrap_err();
            assert_eq!(err.field.as_deref(), Some(field));
        }
        assert!(list(&vault).is_empty());
    }

    #[test]
    fn deleting_a_picked_scheme_leaves_the_settings_alone() {
        let mut vault = vault();
        let saved = save(&mut vault, &input("Mine", "#101010")).unwrap();
        let mut settings = Settings::default();
        settings.terminal.dark_scheme = Some(saved.id.clone());
        vault
            .put(Some(SETTINGS_ID), Item::Settings(settings))
            .unwrap();
        remove(&mut vault, &saved.id).unwrap();
        // The id now names nothing, which the app reads as its default dark scheme.
        assert_eq!(vault.settings().terminal.dark_scheme, Some(saved.id));
    }
}
