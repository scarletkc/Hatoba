//! Painted window chrome on Linux.
//!
//! An undecorated, transparent window gets no rounding, shadow or resize border from the
//! compositor, so the app paints its own chrome on a surface inset by `WIN_PAD` (the CSS side
//! lives in `:root[data-floating]` rules). This module keeps the transparent margin honest:
//!
//! - `set_shadow_width` tells the compositor where the painted surface actually ends, so
//!   snapping and tiling line up with the visible window instead of leaving a margin-wide gap.
//! - The input shape is cut to the painted surface plus a `RESIZE_BAND` ring, so the rest of
//!   the margin passes clicks through to the windows behind. The WebView's own 5 px resize
//!   hit-test sits in that dead outer band, so `ResizeHandles.tsx` renders handles inside the
//!   surviving ring and starts resize drags from there.
//! - The chrome turns off while maximized, fullscreen or tiled (the margin would look wrong),
//!   and without a compositor, where the margin renders opaque instead of transparent.
//!
//! Tiling is only seen when the WM reports it: xdg-shell's tiled states on Wayland, and on X11
//! Mutter's `_GTK_EDGE_CONSTRAINTS` or a vertical-only maximize. KWin on X11 keeps quick tiling
//! to itself, so a quick-tiled window there keeps its chrome, as GTK's own client-side
//! decorations do.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use gtk::gdk;
use gtk::glib::Propagation;
use gtk::prelude::*;
use tauri::Manager;
use tauri_specta::Event;

use crate::dto::WindowChromeEvent;

/// The transparent margin around the painted window surface, in px. Keep in step with
/// `--win-pad` in `src/styles/tokens.css`.
pub const WIN_PAD: i32 = 22;

/// The resize band just outside the painted edge, in px. Keep in step with `--win-resize` in
/// `src/styles/tokens.css`.
pub const RESIZE_BAND: i32 = 8;

static FLOATING: AtomicBool = AtomicBool::new(false);

/// Whether the main window currently shows its painted chrome.
pub fn floating() -> bool {
    FLOATING.load(Ordering::Relaxed)
}

/// Floating means composited and clear of every WM state the painted chrome cannot follow:
/// maximized, fullscreen and any tiled edge (GDK has no "isFullscreen" that sees a WM-initiated
/// fullscreen, so the state flags are the source of truth).
fn is_floating(composited: bool, state: gdk::WindowState) -> bool {
    use gdk::WindowState as W;
    composited
        && !state.intersects(
            W::MAXIMIZED
                | W::FULLSCREEN
                | W::TILED
                | W::TOP_TILED
                | W::RIGHT_TILED
                | W::BOTTOM_TILED
                | W::LEFT_TILED,
        )
}

/// The input region while floating: the painted surface plus the resize band around it, so
/// the outer `WIN_PAD - RESIZE_BAND` ring of the margin stays click-through.
fn input_rect(width: i32, height: i32) -> (i32, i32, i32, i32) {
    let inset = WIN_PAD - RESIZE_BAND;
    (
        inset,
        inset,
        (width - 2 * inset).max(0),
        (height - 2 * inset).max(0),
    )
}

/// Hooks the main window's GTK signals so `FLOATING`, the shadow margin and the input shape
/// follow the window state. Called once from `create_main_window`, on the main thread.
pub fn attach(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let gtk_window = window.gtk_window()?;
    let app = window.app_handle().clone();

    // The window is not mapped yet, so its state is empty and compositing decides alone.
    FLOATING.store(
        is_floating(
            gtk::prelude::GtkWindowExt::screen(&gtk_window).is_some_and(|s| s.is_composited()),
            gdk::WindowState::empty(),
        ),
        Ordering::Relaxed,
    );

    // The last shadow width applied, so an unchanged value doesn't reconfigure the window.
    let last_shadow = Cell::new(-1);
    let update = {
        let gtk_window = gtk_window.clone();
        Rc::new(move |state: gdk::WindowState| {
            let composited =
                gtk::prelude::GtkWindowExt::screen(&gtk_window).is_some_and(|s| s.is_composited());
            let floating = is_floating(composited, state);

            let pad = if floating { WIN_PAD } else { 0 };
            if pad != last_shadow.get() {
                // An unrealized GDK window takes no shadow width; leave `last_shadow` so the
                // value is applied once the window maps.
                if let Some(gdk_window) = gtk_window.window() {
                    gdk_window.set_shadow_width(pad, pad, pad, pad);
                    last_shadow.set(pad);
                }
            }

            let region = floating.then(|| {
                let (x, y, w, h) =
                    input_rect(gtk_window.allocated_width(), gtk_window.allocated_height());
                gtk::cairo::Region::create_rectangle(&gtk::cairo::RectangleInt::new(x, y, w, h))
            });
            gtk_window.input_shape_combine_region(region.as_ref());

            if FLOATING.swap(floating, Ordering::Relaxed) != floating {
                let _ = WindowChromeEvent { floating }.emit(&app);
            }
        })
    };

    gtk_window.connect_window_state_event({
        let update = update.clone();
        move |_, event| {
            update(event.new_window_state());
            Propagation::Proceed
        }
    });

    // A resize changes the input region's size; window-state-event may not have fired yet for
    // a tiled state that arrived with the resize, so read the live state.
    gtk_window.connect_size_allocate({
        let update = update.clone();
        move |widget, _| {
            update(
                widget
                    .window()
                    .map(|w| w.state())
                    .unwrap_or_else(gdk::WindowState::empty),
            )
        }
    });

    if let Some(screen) = gtk::prelude::GtkWindowExt::screen(&gtk_window) {
        let update = update.clone();
        let gtk_window = gtk_window.clone();
        screen.connect_composited_changed(move |_| {
            update(
                gtk_window
                    .window()
                    .map(|w| w.state())
                    .unwrap_or_else(gdk::WindowState::empty),
            )
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floating_requires_a_compositor() {
        assert!(!is_floating(false, gdk::WindowState::empty()));
        assert!(is_floating(true, gdk::WindowState::empty()));
    }

    #[test]
    fn wm_states_turn_the_chrome_off() {
        use gdk::WindowState as W;
        for state in [
            W::MAXIMIZED,
            W::FULLSCREEN,
            W::TILED,
            W::TOP_TILED,
            W::RIGHT_TILED,
            W::BOTTOM_TILED,
            W::LEFT_TILED,
        ] {
            assert!(!is_floating(true, state), "{state:?}");
        }
        // States that leave the window floating keep the chrome on.
        for state in [W::STICKY, W::ABOVE, W::FOCUSED, W::WITHDRAWN] {
            assert!(is_floating(true, state), "{state:?}");
        }
    }

    #[test]
    fn input_region_is_the_surface_plus_resize_band() {
        let inset = WIN_PAD - RESIZE_BAND;
        assert_eq!(
            input_rect(1244, 804),
            (inset, inset, 1244 - 2 * inset, 804 - 2 * inset)
        );
    }

    #[test]
    fn input_region_of_a_tiny_window_is_not_negative() {
        assert_eq!(
            input_rect(10, 10),
            (WIN_PAD - RESIZE_BAND, WIN_PAD - RESIZE_BAND, 0, 0)
        );
    }
}
