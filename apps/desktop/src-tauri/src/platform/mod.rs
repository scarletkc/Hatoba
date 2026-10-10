//! Platform integration: window chrome, power events, OS credential store, biometrics.

pub mod biometric;
pub mod power;
pub mod secrets;
pub mod shell_path;
pub mod system;
pub mod window;

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

/// Human-readable OS name for the device list, e.g. "Windows", "macOS", "Linux".
pub fn os_label() -> String {
    match platform_name() {
        "windows" => "Windows".into(),
        "macos" => "macOS".into(),
        _ => "Linux".into(),
    }
}
