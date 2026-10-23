//! Main window creation (WIN-01 frameless title bar, WIN-07 Mica) and Snap Layouts helper.

use tauri::{
    AppHandle, LogicalSize, Manager, Monitor, PhysicalPosition, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

use crate::dto::ZoomView;

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

/// The initial and minimum size of the visible window surface.
const INITIAL_SIZE: (f64, f64) = (1200.0, 760.0);
const MIN_SIZE: (f64, f64) = (860.0, 540.0);

/// What the window adds around the visible surface on each axis: Linux's transparent margin, so
/// the surface the CSS insets by `WIN_PAD` keeps the sizes above.
#[cfg(target_os = "linux")]
const MARGIN: f64 = 2.0 * super::linux_chrome::WIN_PAD as f64;
#[cfg(not(target_os = "linux"))]
const MARGIN: f64 = 0.0;

/// TERM-13: the interface zoom levels in percent, a subset of the browsers' steps.
pub const ZOOM_STEPS: [u32; 10] = [67, 75, 80, 90, 100, 110, 125, 150, 175, 200];

#[cfg(target_os = "linux")]
pub use super::linux_chrome::floating;

/// Whether the main window shows the painted Linux chrome; other platforms have none.
#[cfg(not(target_os = "linux"))]
pub fn floating() -> bool {
    false
}

/// Creates the main window. Returns whether Windows 11 Mica was applied.
pub fn create_main_window(app: &AppHandle) -> tauri::Result<(WebviewWindow, bool)> {
    let builder = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::default())
        .title("Hatoba")
        .inner_size(INITIAL_SIZE.0 + MARGIN, INITIAL_SIZE.1 + MARGIN)
        .min_inner_size(MIN_SIZE.0 + MARGIN, MIN_SIZE.1 + MARGIN)
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

    // Linux draws no chrome around an undecorated window, so the app paints its own (rounded
    // corners, shadow, hairline ring) on a surface inset by WIN_PAD inside a transparent
    // window. linux_chrome::attach manages that margin and decides when the chrome is on.
    #[cfg(target_os = "linux")]
    let builder = builder.transparent(true);

    let window = builder.build()?;
    #[cfg(target_os = "linux")]
    super::linux_chrome::attach(&window)?;
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

/// The zoom step nearest to `percent`.
pub fn zoom_step(percent: u32) -> u32 {
    ZOOM_STEPS
        .into_iter()
        .min_by_key(|s| s.abs_diff(percent))
        .unwrap_or(100)
}

/// The smallest window at `percent`, margin included. Zooming in shrinks the page the window
/// holds, so the minimum grows with the zoom and the layout keeps its own minimum of `MIN_SIZE`.
fn zoom_min_size(percent: u32) -> (f64, f64) {
    let zoom = f64::from(percent) / 100.0;
    (MIN_SIZE.0 * zoom + MARGIN, MIN_SIZE.1 * zoom + MARGIN)
}

/// The largest zoom step whose minimum window fits a work area of `width` x `height` logical
/// pixels. 100% always does, as the window allowed it before there was a zoom.
fn zoom_limit(width: f64, height: f64) -> u32 {
    ZOOM_STEPS
        .into_iter()
        .filter(|&s| {
            let (w, h) = zoom_min_size(s);
            s <= 100 || (w <= width && h <= height)
        })
        .max()
        .unwrap_or(100)
}

/// Zooms the main window's page to the step nearest `percent` (TERM-13), at most the largest one
/// whose minimum window fits the screen the window is on. Grows the window to the new minimum when
/// it is smaller, keeping it inside that screen's work area.
pub fn apply_zoom(window: &WebviewWindow, percent: u32) -> tauri::Result<ZoomView> {
    let monitor = window.current_monitor()?;
    *ZOOM_SCREEN.lock().unwrap_or_else(|e| e.into_inner()) = monitor.as_ref().map(screen_key);
    let max = monitor
        .as_ref()
        .map_or(ZOOM_STEPS[ZOOM_STEPS.len() - 1], |m| {
            let area = m.work_area().size.to_logical::<f64>(m.scale_factor());
            zoom_limit(area.width, area.height)
        });
    let zoom = zoom_step(percent).min(max);
    window.set_zoom(f64::from(zoom) / 100.0)?;
    let (min_w, min_h) = zoom_min_size(zoom);
    window.set_min_size(Some(LogicalSize::new(min_w, min_h)))?;
    if !window.is_maximized()? && !window.is_fullscreen()? {
        grow_to(window, monitor.as_ref(), (min_w, min_h))?;
    }
    Ok(ZoomView { zoom, max })
}

/// A screen as the zoom limit sees it: its work area's position and size, and its scale's bits.
type ScreenKey = (i32, i32, u32, u32, u64);

/// The screen the zoom limit was last worked out for.
static ZOOM_SCREEN: std::sync::Mutex<Option<ScreenKey>> = std::sync::Mutex::new(None);

fn screen_key(m: &Monitor) -> ScreenKey {
    let area = m.work_area();
    (
        area.position.x,
        area.position.y,
        area.size.width,
        area.size.height,
        m.scale_factor().to_bits(),
    )
}

/// TERM-13: once the main window is on another screen, or its screen changed scale, zooms it again
/// from `percent`, the device's preference, under that screen's limit: a window zoomed on a large
/// screen and moved to a smaller one would otherwise keep a minimum size that does not fit there.
/// Returns the new zoom, which the page learns from `window://zoom`; `None` while on the same
/// screen.
pub fn follow_screen(window: &WebviewWindow, percent: impl FnOnce() -> u32) -> Option<ZoomView> {
    let key = window
        .current_monitor()
        .ok()
        .flatten()
        .map(|m| screen_key(&m));
    if key.is_none() || *ZOOM_SCREEN.lock().unwrap_or_else(|e| e.into_inner()) == key {
        return None;
    }
    apply_zoom(window, percent()).ok()
}

fn grow_to(
    window: &WebviewWindow,
    monitor: Option<&Monitor>,
    (min_w, min_h): (f64, f64),
) -> tauri::Result<()> {
    let scale = window.scale_factor()?;
    let inner = window.inner_size()?;
    let size = inner.to_logical::<f64>(scale);
    if size.width >= min_w && size.height >= min_h {
        return Ok(());
    }
    let grown =
        LogicalSize::new(size.width.max(min_w), size.height.max(min_h)).to_physical::<i32>(scale);
    // The frame around the inner size, which the new outer size keeps. Read before the resize:
    // `set_size` may apply after it returns.
    let outer = window.outer_size()?;
    let frame = (
        outer.width.saturating_sub(inner.width) as i32,
        outer.height.saturating_sub(inner.height) as i32,
    );
    window.set_size(grown)?;
    let Some(area) = monitor.map(Monitor::work_area) else {
        return Ok(());
    };
    let pos = window.outer_position()?;
    let right = area.position.x + area.size.width as i32;
    let bottom = area.position.y + area.size.height as i32;
    let x = pos
        .x
        .min(right - grown.width - frame.0)
        .max(area.position.x);
    let y = pos
        .y
        .min(bottom - grown.height - frame.1)
        .max(area.position.y);
    if (x, y) != (pos.x, pos.y) {
        window.set_position(PhysicalPosition::new(x, y))?;
    }
    Ok(())
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

/// Brings the main window back after the red button hid it or after it was minimized: a Dock
/// click, or another launch (HOST-13).
pub fn reopen_main(app: &AppHandle) {
    #[cfg(target_os = "macos")]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_snaps_to_the_nearest_step() {
        assert_eq!(zoom_step(100), 100);
        assert_eq!(zoom_step(130), 125);
        assert_eq!(zoom_step(10), 67);
        assert_eq!(zoom_step(500), 200);
    }

    /// TERM-13: zooming in stops at the largest step whose minimum window fits the work area.
    #[test]
    fn zoom_stops_where_the_minimum_window_no_longer_fits() {
        // 4K at 100% scaling, less the taskbar.
        assert_eq!(zoom_limit(3840.0, 2112.0), 200);
        // 1366 x 768 at 100%.
        assert_eq!(zoom_limit(1366.0, 728.0), 125);
        // 1920 x 1080 at 150%.
        assert_eq!(zoom_limit(1280.0, 672.0), 110);
        // Smaller than the minimum window: 100% stays.
        assert_eq!(zoom_limit(800.0, 500.0), 100);
    }
}
