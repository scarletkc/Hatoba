use serde::{Deserialize, Serialize};
use specta::Type;

use super::McpServerStatus;

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Windows,
    Macos,
    Linux,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct AppInfo {
    pub version: String,
    pub platform: Platform,
    /// Windows 11 Mica is active (WIN-07).
    pub mica: bool,
    /// The local user name, which quick connect (HOST-12) uses when the target names no user;
    /// empty when unknown.
    pub local_user: String,
    /// This computer's name, as the sync device list shows it, which a conversation on a local
    /// tab records (§7.5).
    pub device_name: String,
    /// The package manager that installed this copy and updates it, which turns the in-app
    /// update off (spec §11); `None` for the installers from the GitHub release.
    pub package_manager: Option<PackageManager>,
}

/// A package manager that installs Hatoba from the GitHub release and updates it itself.
#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PackageManager {
    Scoop,
    /// The `hatoba-bin` package in the Arch User Repository.
    Aur,
    /// One that Hatoba has no instructions for.
    Other,
}

/// Whether the Linux main window shows its painted chrome right now: off while maximized,
/// fullscreen or tiled, and off without a compositor. The WebView mirrors it to
/// `:root[data-floating]`; `window_floating` reports the same value.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "window://chrome")]
pub struct WindowChromeEvent {
    pub floating: bool,
}

/// WIN-10: the close button asks whether to hide the window to the tray or quit.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "window://close-prompt")]
pub struct ClosePromptEvent;

/// WIN-10: the tray menu's **Update to vX.Y.Z** was clicked.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "tray://update")]
pub struct TrayUpdateEvent;

/// WIN-10: the tray menu's labels in the app's language, and whether the vault can be locked.
#[derive(Debug, Clone, Deserialize, Type)]
// macOS has no tray, so nothing reads them there.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub struct TrayMenuInput {
    pub open: String,
    pub lock: String,
    pub quit: String,
    pub can_lock: bool,
    /// **Update to vX.Y.Z** while a check has found a newer release; without it the menu has no
    /// such item.
    pub update: Option<String>,
    pub can_update: bool,
    /// FWD-05: the heading over the running forwards, and each one's items.
    pub forwards: String,
    pub copy_address: String,
    pub stop_forward: String,
    /// Where a remote forward listens, with `{addr}` for its address on the server (FWD-03).
    pub on_server: String,
    /// What a dynamic forward is (FWD-04).
    pub socks_proxy: String,
}

/// A system notification (AI-07). The WebView sends one only while the window is not focused.
#[derive(Debug, Clone, Deserialize, Type)]
pub struct NotificationInput {
    pub title: String,
    pub body: String,
    pub target: NotificationTarget,
}

/// What a notification is about, which a click on it shows: a tab, and in it a conversation
/// (AI-07), or a saved host whose forwards stopped (FWD-05). The click brings the window back and
/// sends this to the WebView.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "app://notification-click")]
pub struct NotificationTarget {
    pub tab: Option<String>,
    pub conversation_id: Option<String>,
    pub host_id: Option<String>,
}

/// A second launch handed its arguments to this window (HOST-13); `launch_take` returns them.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "app://launch")]
pub struct LaunchEvent;

/// The `hatoba` command on this device (HOST-13).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct CliCommandView {
    /// Settings → General has it on.
    pub enabled: bool,
    /// Where the command lives: the folder added to the user's `PATH` on Windows, the launcher
    /// itself on Linux and macOS.
    pub location: String,
    /// Whether that folder is on the `PATH` Hatoba started with. Only Linux reports false: a new
    /// `~/.local/bin` joins the `PATH` at the next sign-in.
    pub on_path: bool,
    /// HOST-15: other programs may use the saved hosts through the command. Only counts while the
    /// command is on.
    pub access: bool,
}

/// What a program did through the `hatoba` command (HOST-15), newest first in Settings → General.
/// Kept in memory until the vault locks.
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct CliActivity {
    pub id: String,
    pub kind: CliActivityKind,
    pub host_id: String,
    pub host_name: String,
    /// The command, or the file's source and target.
    pub detail: String,
    pub started_at: i64,
    pub state: CliActivityState,
    /// The exit status of a command that ended.
    pub status: Option<i32>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CliActivityKind {
    Exec,
    Upload,
    Download,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CliActivityState {
    Running,
    Done,
    Failed,
}

/// The list in Settings → General changed (HOST-15).
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "cli://activity")]
pub struct CliActivityEvent {
    pub entries: Vec<CliActivity>,
}

/// What the update check found (spec §11).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct UpdateCheck {
    pub current_version: String,
    /// A release newer than the running version, or `None` when there is none.
    pub update: Option<AvailableUpdate>,
}

/// A newer release that `update_install` can download and install.
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AvailableUpdate {
    pub version: String,
    /// The release notes in Markdown.
    pub notes: Option<String>,
    /// When the release was published, Unix ms.
    pub published_at: Option<i64>,
    /// The release page on GitHub.
    pub release_url: String,
}

/// Sent on the `update_install` channel.
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UpdateProgress {
    /// Bytes downloaded so far, and the installer's size when the server sent it.
    Downloading { downloaded: u64, total: Option<u64> },
    /// The download passed the signature check. Hatoba closes, and the installer starts it again.
    Installing,
}

#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "vault://locked")]
pub struct VaultLockedEvent {
    pub reason: LockReason,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LockReason {
    Manual,
    Idle,
    Sleep,
}

// ───────────────────────── AI assistant (§13) ─────────────────────────
//
// The assistant's DTOs are `hatoba_assistant::dto`'s, re-exported above.

/// [`McpServerStatus`] as the `ai://mcp-status` event, which the assistant's crate cannot derive
/// without Tauri.
#[derive(Clone, Serialize, tauri_specta::Event)]
#[serde(transparent)]
#[tauri_specta(event_name = "ai://mcp-status")]
pub struct McpStatusEvent(pub McpServerStatus);

/// The event's type in TypeScript is `McpServerStatus` itself.
impl Type for McpStatusEvent {
    fn definition(types: &mut specta::Types) -> specta::datatype::DataType {
        McpServerStatus::definition(types)
    }
}
