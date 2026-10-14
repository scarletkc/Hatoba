//! The system tray and the window's close button on Windows and Linux (WIN-10).
//!
//! SSH sessions, forwards and sync keep running without a window, so the close button can hide
//! it to the tray instead of quitting. The device pref `close_action` asks each time (the
//! default), hides or quits; without a tray it always quits. The tray menu opens the window,
//! locks the vault and quits, and a left click opens the window (Linux shows the menu instead).
//! The WebView sets the menu's labels, so they follow the app's language.
//!
//! macOS keeps its own close button, which hides the window (MAC-06), and the Dock, so it has no
//! tray.

#[cfg(not(target_os = "macos"))]
pub use imp::*;

#[cfg(not(target_os = "macos"))]
mod imp {
    use std::sync::atomic::{AtomicU8, Ordering};

    use tauri::image::Image;
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use tauri::{AppHandle, CloseRequestApi, Manager, Window, Wry};
    use tauri_specta::Event;

    use crate::dto::{CloseAction, ClosePromptEvent, LocalPrefs, LockReason, TrayMenuInput};
    use crate::state::AppState;
    use crate::{lock, platform};

    /// Sharp at the tray's size: 16 px at 100% on Windows; Linux panels scale the larger one.
    #[cfg(windows)]
    const ICON: Image<'static> = tauri::include_image!("icons/32x32.png");
    #[cfg(not(windows))]
    const ICON: Image<'static> = tauri::include_image!("icons/128x128.png");

    /// The menu items whose labels and state the WebView sets. Managed once the tray exists.
    struct Tray {
        open: MenuItem<Wry>,
        lock: MenuItem<Wry>,
        quit: MenuItem<Wry>,
    }

    /// The pref, copied out of the device-local prefs so that the close button reads it without
    /// the vault's mutex.
    static CLOSE_ACTION: AtomicU8 = AtomicU8::new(CloseAction::Ask as u8);

    /// Reads the pref, which is readable while locked. Runs at startup and whenever the prefs
    /// are saved.
    pub fn refresh_close_action(state: &AppState) {
        let action = state
            .vault()
            .local_prefs()
            .ok()
            .flatten()
            .map_or(CloseAction::Ask, |json| {
                LocalPrefs::from_stored(&json).close_action
            });
        CLOSE_ACTION.store(action as u8, Ordering::Relaxed);
    }

    fn close_action() -> CloseAction {
        match CLOSE_ACTION.load(Ordering::Relaxed) {
            x if x == CloseAction::Tray as u8 => CloseAction::Tray,
            x if x == CloseAction::Quit as u8 => CloseAction::Quit,
            _ => CloseAction::Ask,
        }
    }

    /// Creates the tray, in English until the WebView sets the labels. Without one (no tray
    /// host, or a menu that would not build) Hatoba runs on, and the close button quits.
    pub fn init(app: &AppHandle) {
        if let Err(e) = build(app) {
            tracing::warn!("no system tray: {e}");
        }
    }

    fn build(app: &AppHandle) -> tauri::Result<()> {
        let open = MenuItem::with_id(app, "open", "Open Hatoba", true, None::<&str>)?;
        let lock = MenuItem::with_id(app, "lock", "Lock", false, None::<&str>)?;
        let quit = MenuItem::with_id(app, "quit", "Quit Hatoba", true, None::<&str>)?;
        let menu = Menu::with_items(
            app,
            &[&open, &lock, &PredefinedMenuItem::separator(app)?, &quit],
        )?;
        TrayIconBuilder::with_id("main")
            .icon(ICON)
            .tooltip("Hatoba")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_menu_event(|app, event| match event.id().as_ref() {
                "open" => platform::window::reopen_main(app),
                "lock" => {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        lock::lock_vault(&app, LockReason::Manual).await;
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
                    platform::window::reopen_main(tray.app_handle());
                }
            })
            .build(app)?;
        app.manage(Tray { open, lock, quit });
        #[cfg(target_os = "linux")]
        refresh_host();
        Ok(())
    }

    /// Sets the menu's labels and whether **Lock** is enabled: the WebView calls this when the
    /// language changes and when the vault locks or unlocks.
    pub fn set_menu(app: &AppHandle, menu: &TrayMenuInput) {
        let Some(tray) = app.try_state::<Tray>() else {
            return;
        };
        let _ = tray.open.set_text(&menu.open);
        let _ = tray.lock.set_text(&menu.lock);
        let _ = tray.lock.set_enabled(menu.can_lock);
        let _ = tray.quit.set_text(&menu.quit);
    }

    /// Whether a hidden window can come back from the tray.
    fn tray_shown(app: &AppHandle) -> bool {
        app.try_state::<Tray>().is_some() && host_running()
    }

    #[cfg(not(target_os = "linux"))]
    fn host_running() -> bool {
        true
    }

    /// A Linux tray icon shows only where a StatusNotifier host runs, which GNOME lacks without
    /// an extension, and the icon is created either way. [`refresh_host`] asks the session bus
    /// off the main thread, so the close button never waits on it.
    #[cfg(target_os = "linux")]
    static HOST_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    #[cfg(target_os = "linux")]
    fn host_running() -> bool {
        HOST_RUNNING.load(Ordering::Relaxed)
    }

    /// Asks again at startup and whenever the window gains focus, which comes before a click on
    /// its close button.
    #[cfg(target_os = "linux")]
    pub fn refresh_host() {
        tauri::async_runtime::spawn_blocking(|| {
            let running = host_registered().unwrap_or(false);
            HOST_RUNNING.store(running, Ordering::Relaxed);
        });
    }

    #[cfg(target_os = "linux")]
    fn host_registered() -> zbus::Result<bool> {
        use std::sync::OnceLock;
        use zbus::blocking::{Connection, Proxy, proxy::Builder};

        // One connection for the life of the app; without a session bus there is no tray.
        static BUS: OnceLock<Option<Connection>> = OnceLock::new();
        let Some(bus) = BUS.get_or_init(|| Connection::session().ok()) else {
            return Ok(false);
        };
        let watcher: Proxy<'_> = Builder::new(bus)
            .destination("org.kde.StatusNotifierWatcher")?
            .path("/StatusNotifierWatcher")?
            .interface("org.kde.StatusNotifierWatcher")?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()?;
        watcher.get_property("IsStatusNotifierHostRegistered")
    }

    /// The main window's close button, Alt+F4 and the taskbar's Close: asks, hides the window,
    /// or lets it close, which quits.
    pub fn close_requested(window: &Window, api: &CloseRequestApi) {
        let app = window.app_handle();
        let action = close_action();
        if action == CloseAction::Quit || !tray_shown(app) {
            return;
        }
        api.prevent_close();
        if action == CloseAction::Tray {
            let _ = window.hide();
        } else {
            // A minimized window, closed from the taskbar, shows the question.
            platform::window::reopen_main(app);
            let _ = ClosePromptEvent.emit(app);
        }
    }

    /// The close prompt's **Minimize to Tray**.
    pub fn hide_to_tray(app: &AppHandle) {
        if let Some(window) = app.get_webview_window(platform::window::MAIN) {
            let _ = window.hide();
        }
    }

    #[cfg(test)]
    mod tests {
        use hatoba_core::vault::Vault;

        use super::*;

        #[test]
        fn the_close_button_follows_the_device_pref() {
            // Readable before the vault exists, as on the onboarding screen.
            let state = AppState::new(Vault::open_in_memory().unwrap(), false);
            refresh_close_action(&state);
            assert_eq!(close_action(), CloseAction::Ask);

            for (stored, action) in [
                (r#"{"close_action":"tray"}"#, CloseAction::Tray),
                (r#"{"close_action":"quit"}"#, CloseAction::Quit),
                // A value another build wrote asks.
                (r#"{"close_action":"minimize"}"#, CloseAction::Ask),
            ] {
                state.vault().set_local_prefs(stored).unwrap();
                refresh_close_action(&state);
                assert_eq!(close_action(), action);
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub fn set_menu(_app: &tauri::AppHandle, _menu: &crate::dto::TrayMenuInput) {}

#[cfg(target_os = "macos")]
pub fn hide_to_tray(_app: &tauri::AppHandle) {}
