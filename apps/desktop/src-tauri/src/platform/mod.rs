//! Platform integration: window chrome, power events, idle time, OS credential store, biometrics,
//! the `hatoba` command, and the editors that open remote files.

pub mod biometric;
pub mod cli;
pub mod editor;
pub mod idle;
#[cfg(target_os = "linux")]
pub mod linux_chrome;
pub mod login_shell;
pub mod power;
pub mod secrets;
pub mod system;
pub mod window;

pub use hatoba_local::system::os_label;

/// Platform name reported to the WebView (`AppInfo.platform`).
pub fn platform_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

pub fn platform() -> crate::dto::Platform {
    match platform_name() {
        "windows" => crate::dto::Platform::Windows,
        "macos" => crate::dto::Platform::Macos,
        _ => crate::dto::Platform::Linux,
    }
}
