//! The data transfer objects that more than one crate of the desktop app exchanges with the
//! WebView (spec §10). tauri-specta exports them with the app's own.

use serde::{Deserialize, Serialize};
use specta::Type;

/// A target typed into the hosts search field (quick connect, HOST-12). It is never saved as a
/// host; once connected it joins the device-local recent list.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct QuickTarget {
    /// DNS name or IP address, IPv6 without brackets.
    pub address: String,
    pub port: u16,
    pub username: String,
}

/// What kind of shell a local tab runs (§7.5).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalShellKind {
    /// A shell of macOS or Linux.
    Unix,
    /// PowerShell 7 or Windows PowerShell.
    Powershell,
    Cmd,
    GitBash,
    Wsl,
}
