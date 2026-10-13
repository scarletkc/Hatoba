//! System tray (issue #189): Hatoba keeps SSH sessions, tunnels and the sync scheduler running
//! without a window, so it lives in the notification area while the window is closed.
//!
//! The tray shows the app icon, a left click toggles the main window, and the menu offers Open,
//! Lock vault and Quit. Creating it can fail (a Linux session without a StatusNotifier); Hatoba
//! then simply runs without a tray, and close-to-tray falls back to the ordinary quit.

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::dto::{Language, LocalPrefs};
use crate::{lock, platform, state};

/// Managed once the tray exists, so close-to-tray knows it can hide the window.
#[derive(Debug, Default)]
struct TrayPresent;

struct MenuText {
    open: &'static str,
    lock: &'static str,
    quit: &'static str,
}

const EN: MenuText = MenuText {
    open: "Open Hatoba",
    lock: "Lock vault",
    quit: "Quit Hatoba",
};
const ZH: MenuText = MenuText {
    open: "打开 Hatoba",
    lock: "锁定保险库",
    quit: "退出 Hatoba",
};
const JA: MenuText = MenuText {
    open: "Hatoba を開く",
    lock: "Vault をロック",
    quit: "Hatoba を終了",
};

/// The menu follows the language pref; `System` keeps English until the tray learns to re-read it.
fn menu_text(language: Language) -> MenuText {
    match language {
        Language::ZhCn => ZH,
        Language::Ja => JA,
        Language::System | Language::En => EN,
    }
}

/// Whether the tray was created. Without one there is nothing to bring a hidden window back.
pub fn present(app: &AppHandle) -> bool {
    app.try_state::<TrayPresent>().is_some()
}

/// Whether the window's close button should hide it instead of quitting (device pref, off by
/// default), which needs a tray to reopen from.
pub fn close_to_tray(app: &AppHandle) -> bool {
    let Some(state) = app.try_state::<state::AppState>() else {
        return false;
    };
    let pref_on = state
        .vault()
        .local_prefs()
        .ok()
        .flatten()
        .map(|json| LocalPrefs::from_stored(&json).close_to_tray)
        .unwrap_or(false);
    pref_on && present(app)
}

/// Builds the tray. Called once from `setup`, after the app state exists (the language pref).
pub fn init(app: &AppHandle) {
    let language = app
        .try_state::<state::AppState>()
        .and_then(|state| state.vault().local_prefs().ok().flatten())
        .map(|json| LocalPrefs::from_stored(&json).language)
        .unwrap_or(Language::System);
    let text = menu_text(language);

    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "open", text.open, true, None::<&str>)
                .expect("tray menu item"),
            &MenuItem::with_id(app, "lock", text.lock, true, None::<&str>)
                .expect("tray menu item"),
            &MenuItem::with_id(app, "quit", text.quit, true, None::<&str>)
                .expect("tray menu item"),
        ],
    )
    .expect("tray menu");

    let tray = TrayIconBuilder::with_id("hatoba")
        .tooltip("Hatoba")
        .menu(&menu)
        // A left click toggles the window; the menu belongs to the right click.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => platform::window::reopen_main(app),
            "lock" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    lock::lock_vault(&app, crate::dto::LockReason::Manual).await;
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_main(tray.app_handle());
            }
        });

    // The app icon ships with the bundle, so the tray needs no extra asset.
    let tray = match app.default_window_icon() {
        Some(icon) => tray.icon(icon.clone()),
        None => tray,
    };

    match tray.build(app) {
        Ok(_) => {
            app.manage(TrayPresent);
            tracing::info!("system tray created");
        }
        Err(e) => tracing::warn!("no system tray, closing the window quits: {e}"),
    }
}

/// A left click shows a hidden or minimized window, and hides a visible one (macOS's red button
/// keeps the app too, so the behaviour matches across platforms).
fn toggle_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(platform::window::MAIN) {
        if window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false) {
            let _ = window.hide();
        } else {
            platform::window::reopen_main(app);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EN, JA, ZH, menu_text};
    use crate::dto::Language;

    #[test]
    fn the_menu_follows_the_language_pref() {
        assert_eq!(menu_text(Language::En).open, EN.open);
        assert_eq!(menu_text(Language::ZhCn).lock, ZH.lock);
        assert_eq!(menu_text(Language::Ja).quit, JA.quit);
        assert_eq!(menu_text(Language::System).open, EN.open);
    }
}
