//! System-wide idle time for auto-lock (SEC-02): how long since the last key press, click or
//! mouse move anywhere on the system, not only in Hatoba's window.
//!
//! Windows reads `GetLastInputInfo` and macOS `CGEventSourceSecondsSinceLastEventType`, as
//! Electron's `powerMonitor` does. Linux has no source that works on both X11 and Wayland, so
//! there the timer counts only the activity in Hatoba's window (`activity_ping`).

use std::time::Duration;

/// Time since the last input anywhere on the system, or `None` where the platform does not
/// report it.
pub fn system_idle() -> Option<Duration> {
    imp::system_idle()
}

#[cfg(target_os = "windows")]
mod imp {
    use std::time::Duration;

    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

    pub fn system_idle() -> Option<Duration> {
        let mut info = LASTINPUTINFO {
            cbSize: size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        // SAFETY: `info` is a LASTINPUTINFO with its size set, as the call requires.
        if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
            return None;
        }
        // SAFETY: no arguments.
        let now = unsafe { GetTickCount() };
        // Both count milliseconds since boot in 32 bits, so the difference survives the wrap
        // after 49.7 days.
        Some(Duration::from_millis(u64::from(
            now.wrapping_sub(info.dwTime),
        )))
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::time::Duration;

    /// `kCGEventSourceStateCombinedSessionState`
    const COMBINED_SESSION_STATE: i32 = 0;
    /// `kCGAnyInputEventType`
    const ANY_INPUT_EVENT: u32 = !0;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
    }

    pub fn system_idle() -> Option<Duration> {
        // SAFETY: takes two plain values and returns a number.
        let secs = unsafe {
            CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION_STATE, ANY_INPUT_EVENT)
        };
        Duration::try_from_secs_f64(secs).ok()
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod imp {
    pub fn system_idle() -> Option<std::time::Duration> {
        None
    }
}
