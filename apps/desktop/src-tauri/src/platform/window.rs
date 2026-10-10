//! Main window creation (WIN-01 frameless title bar, WIN-07 Mica) and Snap Layouts helper.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const MAIN: &str = "main";

/// macOS traffic lights: x is the close button's left edge, y the extra height the title bar
/// container gets below the buttons (tao). The buttons are 14 x 16 pt, so their vertical center
/// sits y + 2 pt below the window top; y = 20 centers them in the 44 px title bar (center at 22).
/// x = 18 matches the sidebar's left padding; the three buttons end at x = 72, inside the 78 pt the
/// sidebar reserves (18 padding + 60 `.traffic`) and the 70 pt the collapsed tab bar reserves.
/// tao re-applies the position on every redraw and after leaving fullscreen.
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHT_X: f64 = 18.0;
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHT_Y: f64 = 20.0;

/// Creates the main window. Returns whether Windows 11 Mica was applied.
pub fn create_main_window(app: &AppHandle) -> tauri::Result<(WebviewWindow, bool)> {
    let builder = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::default())
        .title("Hatoba")
        .inner_size(1200.0, 760.0)
        .min_inner_size(860.0, 540.0)
        .center()
        .visible(false);

    // Windows / Linux: our own title bar merged with the tab bar (WIN-01).
    // macOS (P1): keep the native traffic lights over a transparent title bar, like the design.
    #[cfg(not(target_os = "macos"))]
    let builder = builder.decorations(false);
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(tauri::LogicalPosition::new(
            TRAFFIC_LIGHT_X,
            TRAFFIC_LIGHT_Y,
        ));

    // The page paints a solid background unless Mica is active, so a transparent window is safe on
    // Windows 10 too; it is required for Mica to show through on Windows 11.
    #[cfg(target_os = "windows")]
    let builder = builder.transparent(true).shadow(true);

    let window = builder.build()?;
    let mica = apply_material(&window);
    Ok((window, mica))
}

#[cfg(target_os = "windows")]
fn apply_material(window: &WebviewWindow) -> bool {
    match window_vibrancy::apply_mica(window, None) {
        Ok(()) => true,
        Err(e) => {
            tracing::info!("Mica unavailable, using solid background: {e}");
            false
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn apply_material(_window: &WebviewWindow) -> bool {
    false
}

/// Shows the main window once the WebView has painted, avoiding a white flash at startup.
pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Counts reopens, so a hide still waiting for a fullscreen exit can tell that the window was
/// brought back in the meantime and leave it shown.
#[cfg(target_os = "macos")]
static REOPENS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Hides the main window for the red button. A fullscreen window leaves fullscreen first: hidden in
/// place, it would leave its black Space behind and come back fullscreen. `is_fullscreen` may turn
/// false before the exit animation ends, so the hide waits for the animation as well, and is
/// dropped if the Dock brings the window back before then.
#[cfg(target_os = "macos")]
pub fn hide_main(window: tauri::Window) {
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    if !window.is_fullscreen().unwrap_or(false) {
        let _ = window.hide();
        return;
    }
    let reopens = REOPENS.load(Ordering::SeqCst);
    let _ = window.set_fullscreen(false);
    tauri::async_runtime::spawn(async move {
        for _ in 0..30 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if !window.is_fullscreen().unwrap_or(true) {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
        // A window that never left fullscreen stays shown rather than leave its Space behind.
        if REOPENS.load(Ordering::SeqCst) == reopens && !window.is_fullscreen().unwrap_or(true) {
            let _ = window.hide();
        }
    });
}

/// Brings the main window back after the red button hid it or after it was minimized (Dock click).
#[cfg(target_os = "macos")]
pub fn reopen_main(app: &AppHandle) {
    REOPENS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
    }
    show_main(app);
}

/// Opens the Windows 11 Snap Layouts flyout for the focused window by sending Win+Z.
///
/// WebView2 swallows the non-client hit-testing that normally triggers the flyout when hovering the
/// maximize button, so the custom caption button asks for it explicitly (WIN-01).
#[cfg(target_os = "windows")]
pub fn snap_overlay() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
        VIRTUAL_KEY, VK_LWIN,
    };
    const VK_Z: VIRTUAL_KEY = VIRTUAL_KEY(0x5A);
    let key = |vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [
        key(VK_LWIN, KEYBD_EVENT_FLAGS(0)),
        key(VK_Z, KEYBD_EVENT_FLAGS(0)),
        key(VK_Z, KEYEVENTF_KEYUP),
        key(VK_LWIN, KEYEVENTF_KEYUP),
    ];
    // SAFETY: `inputs` is a valid slice of INPUT structs and the size argument matches.
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(not(target_os = "windows"))]
pub fn snap_overlay() {}
