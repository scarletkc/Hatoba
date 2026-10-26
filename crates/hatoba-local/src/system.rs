//! This computer: its OS's name and the user's, which the app shows and the AI assistant is told.

/// Human-readable OS name for the device list, e.g. "Windows", "macOS", "Linux".
pub fn os_label() -> String {
    if cfg!(target_os = "windows") {
        "Windows".into()
    } else if cfg!(target_os = "macos") {
        "macOS".into()
    } else {
        "Linux".into()
    }
}

/// The local user name, which OpenSSH logs in as when a destination names no user (SSH config
/// import, quick connect); empty when unknown.
pub fn default_user() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default()
}
