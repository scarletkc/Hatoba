//! The system tray and the window's close button on Windows and Linux (WIN-10).
//!
//! SSH sessions, forwards and sync keep running without a window, so the close button can hide
//! it to the tray instead of quitting. The device pref `close_action` asks each time (the
//! default), hides or quits; without a tray it always quits. The tray menu opens the window,
//! locks the vault and quits, and a left click opens the window (Linux shows the menu instead).
//! While a check has found a newer release, it also offers **Update to vX.Y.Z**, which opens the
//! window to install it from Settings → About. The running forwards (FWD-05) follow under
//! **Port Forwarding**, each with **Stop**, and with its address to copy when it listens on this
//! computer. The WebView sets the menu's labels, so they follow the app's language.
//!
//! macOS keeps its own close button, which hides the window (MAC-06), and the Dock, so it has no
//! tray.

#[cfg(not(target_os = "macos"))]
pub use imp::*;

#[cfg(not(target_os = "macos"))]
mod imp {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU8, Ordering};

    use tauri::image::Image;
    use tauri::menu::{IsMenuItem, Menu, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use tauri::{AppHandle, CloseRequestApi, Manager, Window, Wry};
    use tauri_plugin_clipboard_manager::ClipboardExt;
    use tauri_specta::Event;

    use crate::dto::{
        CloseAction, ClosePromptEvent, ForwardKind, HostConnectionView, LocalPrefs, LockReason,
        RunningForwardView, TrayMenuInput, TrayUpdateEvent,
    };
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
        /// In the menu, after **Lock**, only while the WebView names a newer release.
        update: MenuItem<Wry>,
        quit: MenuItem<Wry>,
        shown: Mutex<Shown>,
    }

    /// What the menu shows besides **Open**, **Lock** and **Quit**. Each change builds the menu
    /// anew: Windows keeps a menu as wide as its widest item ever was, a stopped forward's too.
    struct Shown {
        update: bool,
        /// FWD-05: the running forwards.
        heading: String,
        copy: String,
        stop: String,
        /// With `{addr}` for where a remote forward listens on the server.
        on_server: String,
        socks: String,
        /// Of the `hosts` list (`ConnectionsEvent::version`).
        version: u64,
        hosts: Vec<HostConnectionView>,
        /// What the menu was last built from: a change to tabs or failed forwards alone does not
        /// build it again.
        built: Option<Vec<String>>,
    }

    const TRAY: &str = "main";
    const UPDATE: &str = "update";
    /// Followed by the address to copy.
    const FORWARD_COPY: &str = "forward-copy:";
    /// Followed by `host_id:forward_id`.
    const FORWARD_STOP: &str = "forward-stop:";

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
        let update = MenuItem::with_id(app, UPDATE, "Update", true, None::<&str>)?;
        let quit = MenuItem::with_id(app, "quit", "Quit Hatoba", true, None::<&str>)?;
        let menu = Menu::with_items(
            app,
            &[&open, &lock, &PredefinedMenuItem::separator(app)?, &quit],
        )?;
        TrayIconBuilder::with_id(TRAY)
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
                UPDATE => {
                    platform::window::reopen_main(app);
                    let _ = TrayUpdateEvent.emit(app);
                }
                "quit" => app.exit(0),
                id => forward_action(app, id),
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
        app.manage(Tray {
            open,
            lock,
            update,
            quit,
            shown: Mutex::new(Shown {
                update: false,
                heading: "Port Forwarding".to_owned(),
                copy: "Copy Address".to_owned(),
                stop: "Stop".to_owned(),
                on_server: "{addr} on the server".to_owned(),
                socks: "SOCKS proxy".to_owned(),
                version: 0,
                hosts: Vec::new(),
                built: None,
            }),
        });
        #[cfg(target_os = "linux")]
        refresh_host();
        Ok(())
    }

    /// Sets the menu's labels, whether **Lock** is enabled, and **Update to vX.Y.Z**: the WebView
    /// calls this when the language changes, when the vault locks or unlocks, and when an update
    /// check or install changes what the menu offers.
    pub fn set_menu(app: &AppHandle, menu: &TrayMenuInput) {
        let Some(tray) = app.try_state::<Tray>() else {
            return;
        };
        let _ = tray.open.set_text(&menu.open);
        let _ = tray.lock.set_text(&menu.lock);
        let _ = tray.lock.set_enabled(menu.can_lock);
        let _ = tray.quit.set_text(&menu.quit);
        if let Some(label) = &menu.update {
            let _ = tray.update.set_text(label);
            let _ = tray.update.set_enabled(menu.can_update);
        }
        let update = menu.update.is_some();
        let (heading, copy, stop, on_server, socks) = (
            menu.forwards.clone(),
            menu.copy_address.clone(),
            menu.stop_forward.clone(),
            menu.on_server.clone(),
            menu.socks_proxy.clone(),
        );
        on_shown(app, move |shown| {
            shown.update = update;
            shown.heading = heading;
            shown.copy = copy;
            shown.stop = stop;
            shown.on_server = on_server;
            shown.socks = socks;
        });
    }

    /// FWD-05: the hosts' connections changed; a list older than the one shown changes nothing.
    pub fn set_forwards(app: &AppHandle, version: u64, hosts: &[HostConnectionView]) {
        let hosts = hosts.to_vec();
        on_shown(app, move |shown| {
            if version >= shown.version {
                shown.version = version;
                shown.hosts = hosts;
            }
        });
    }

    /// Changes what the menu shows, and shows it, on the main thread. Elsewhere each menu change
    /// waits for the main thread, which could be waiting for the lock.
    fn on_shown(app: &AppHandle, change: impl FnOnce(&mut Shown) + Send + 'static) {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            let Some(tray) = handle.try_state::<Tray>() else {
                return;
            };
            let mut shown = tray.shown.lock().unwrap_or_else(|p| p.into_inner());
            change(&mut shown);
            if let Err(e) = refresh(&handle, &tray, &mut shown) {
                tracing::warn!("tray menu not updated: {e}");
            }
        });
    }

    /// Builds the menu: **Open**, **Lock**, **Update to vX.Y.Z** while there is one, a separator,
    /// the running forwards, and **Quit**.
    fn refresh(app: &AppHandle, tray: &Tray, shown: &mut Shown) -> tauri::Result<()> {
        let Some(icon) = app.tray_by_id(TRAY) else {
            return Ok(());
        };
        let entries = entries(shown);
        if shown.built.as_ref() == Some(&entries) {
            return Ok(());
        }
        let separator = PredefinedMenuItem::separator(app)?;
        let forwards = forward_items(app, shown)?;
        let mut items: Vec<&dyn IsMenuItem<Wry>> = vec![&tray.open, &tray.lock];
        if shown.update {
            items.push(&tray.update);
        }
        items.push(&separator);
        items.extend(forwards.iter().map(|i| i as &dyn IsMenuItem<Wry>));
        items.push(&tray.quit);
        icon.set_menu(Some(Menu::with_items(app, &items)?))?;
        shown.built = Some(entries);
        Ok(())
    }

    /// Everything the built menu shows besides the labels the WebView sets on the fixed items.
    fn entries(shown: &Shown) -> Vec<String> {
        let mut entries = vec![
            shown.update.to_string(),
            shown.heading.clone(),
            shown.copy.clone(),
            shown.stop.clone(),
        ];
        for host in &shown.hosts {
            for forward in &host.forwards {
                entries.push(format!(
                    "{}:{} {} {}",
                    host.host_id,
                    forward.forward_id,
                    forward.address,
                    submenu_label(shown, host, forward)
                ));
            }
        }
        entries
    }

    /// The heading, a submenu for each running forward (see [`submenu_label`]), and a separator;
    /// nothing while none runs.
    fn forward_items(app: &AppHandle, forwards: &Shown) -> tauri::Result<Vec<MenuItemKind<Wry>>> {
        if forwards.hosts.iter().all(|h| h.forwards.is_empty()) {
            return Ok(Vec::new());
        }
        let heading = MenuItem::new(app, &forwards.heading, false, None::<&str>)?;
        let mut items = vec![MenuItemKind::MenuItem(heading)];
        for host in &forwards.hosts {
            for forward in &host.forwards {
                let stop_id = format!("{FORWARD_STOP}{}:{}", host.host_id, forward.forward_id);
                let stop = MenuItem::with_id(app, stop_id, &forwards.stop, true, None::<&str>)?;
                let label = submenu_label(forwards, host, forward);
                // A remote forward's address is on the server: nothing here connects to it.
                let entry = if forward.kind == ForwardKind::Remote {
                    Submenu::with_items(app, label, true, &[&stop])?
                } else {
                    let copy_id = format!("{FORWARD_COPY}{}", forward.address);
                    let copy = MenuItem::with_id(app, copy_id, &forwards.copy, true, None::<&str>)?;
                    Submenu::with_items(app, label, true, &[&copy, &stop])?
                };
                items.push(MenuItemKind::Submenu(entry));
            }
        }
        items.push(MenuItemKind::Predefined(PredefinedMenuItem::separator(
            app,
        )?));
        Ok(items)
    }

    /// A running forward's submenu: its host's name, its own if it has one, and its addresses.
    fn submenu_label(
        forwards: &Shown,
        host: &HostConnectionView,
        forward: &RunningForwardView,
    ) -> String {
        let addresses = addresses(forwards, forward);
        if forward.name.is_empty() {
            format!("{}  {addresses}", host.host_name)
        } else {
            format!("{} · {}  {addresses}", host.host_name, forward.name)
        }
    }

    /// Where a running forward listens and where it leads.
    fn addresses(forwards: &Shown, forward: &RunningForwardView) -> String {
        match forward.kind {
            ForwardKind::Local => format!("{} → {}", forward.address, forward.dest),
            ForwardKind::Remote => format!(
                "{} → {}",
                forwards.on_server.replace("{addr}", &forward.address),
                forward.dest
            ),
            ForwardKind::Dynamic => format!("{} ({})", forward.address, forwards.socks),
        }
    }

    /// A running forward's **Copy Address** or **Stop**.
    fn forward_action(app: &AppHandle, id: &str) {
        if let Some(address) = id.strip_prefix(FORWARD_COPY) {
            if let Err(e) = app.clipboard().write_text(address) {
                tracing::warn!("forward address not copied: {e}");
            }
        } else if let Some((host_id, forward_id)) = id
            .strip_prefix(FORWARD_STOP)
            .and_then(|ids| ids.split_once(':'))
        {
            crate::connection::stop(app, host_id, forward_id);
        }
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

        #[test]
        fn a_named_forward_shows_its_name_after_its_host() {
            let shown = Shown {
                update: false,
                heading: String::new(),
                copy: String::new(),
                stop: String::new(),
                on_server: "{addr} on the server".to_owned(),
                socks: "SOCKS proxy".to_owned(),
                version: 0,
                hosts: Vec::new(),
                built: None,
            };
            let host = HostConnectionView {
                host_id: "h1".to_owned(),
                host_name: "db".to_owned(),
                forwards: Vec::new(),
                failed: Vec::new(),
            };
            let forward = |name: &str| RunningForwardView {
                forward_id: "f1".to_owned(),
                name: name.to_owned(),
                kind: ForwardKind::Local,
                port: 15432,
                address: "localhost:15432".to_owned(),
                dest: "127.0.0.1:5432".to_owned(),
            };
            assert_eq!(
                submenu_label(&shown, &host, &forward("Postgres")),
                "db · Postgres  localhost:15432 → 127.0.0.1:5432"
            );
            assert_eq!(
                submenu_label(&shown, &host, &forward("")),
                "db  localhost:15432 → 127.0.0.1:5432"
            );
        }
    }
}

#[cfg(target_os = "macos")]
pub fn set_menu(_app: &tauri::AppHandle, _menu: &crate::dto::TrayMenuInput) {}

#[cfg(target_os = "macos")]
pub fn hide_to_tray(_app: &tauri::AppHandle) {}

#[cfg(target_os = "macos")]
pub fn set_forwards(
    _app: &tauri::AppHandle,
    _version: u64,
    _hosts: &[crate::dto::HostConnectionView],
) {
}
