use serde::{Deserialize, Serialize};
use specta::Type;

use super::AiPermissionMode;

// ───────────────────────── Settings ─────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CursorChoice {
    Block,
    Bar,
    Underline,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RightClick {
    CopyPaste,
    Menu,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct TerminalSettings {
    pub font_family: String,
    pub font_size: u32,
    pub theme: ThemeChoice,
    /// Colour scheme ids for a dark and a light terminal; `None` until one is picked.
    pub dark_scheme: Option<String>,
    pub light_scheme: Option<String>,
    pub cursor_style: CursorChoice,
    pub scrollback: u32,
    /// WIN-05: PuTTY-style right click, or a context menu.
    pub right_click: RightClick,
    /// TERM-04: ask before pasting text with line breaks.
    pub confirm_multiline_paste: bool,
    /// TERM-14: show the rest of a matching command dimmed after the cursor at a shell prompt.
    /// The suggestion list opens with its keys either way.
    pub suggest_commands: bool,
}

/// The synced settings item (spec §5.1 `Settings`).
#[derive(Debug, Clone, Serialize, Type)]
pub struct SettingsView {
    pub terminal: TerminalSettings,
}

/// The terminal fields a [`SettingsPatch`] changes; a field left out keeps its stored value.
#[derive(Debug, Clone, Default, Deserialize, Type)]
#[serde(default)]
pub struct TerminalSettingsPatch {
    pub font_family: Option<String>,
    pub font_size: Option<u32>,
    pub theme: Option<ThemeChoice>,
    pub dark_scheme: Option<String>,
    pub light_scheme: Option<String>,
    pub cursor_style: Option<CursorChoice>,
    pub scrollback: Option<u32>,
    pub right_click: Option<RightClick>,
    pub confirm_multiline_paste: Option<bool>,
    pub suggest_commands: Option<bool>,
}

/// What `settings_save` changes. Only these fields are written, over the stored settings, so a
/// window that read the settings before a sync pull keeps what other devices changed (§6.4).
#[derive(Debug, Clone, Default, Deserialize, Type)]
#[serde(default)]
pub struct SettingsPatch {
    pub terminal: TerminalSettingsPatch,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
pub enum Language {
    #[serde(rename = "system")]
    System,
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "zh-TW")]
    ZhTw,
    #[serde(rename = "en")]
    En,
    #[serde(rename = "ja")]
    Ja,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Regular,
    Compact,
}

/// Device-local UI preferences (readable while locked; never synced).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct LocalPrefs {
    pub language: Language,
    pub appearance: Appearance,
    pub density: Density,
    /// TERM-13: the interface zoom in percent, one of [`crate::platform::window::ZOOM_STEPS`]. Kept
    /// per device, since the right size depends on the screen.
    pub ui_zoom: u32,
    /// HOST-10: the TCP reachability dots.
    pub host_probe: bool,
    /// Check GitHub for a newer release once after the first unlock of each launch (spec §11).
    /// On by default; the request carries no identifier.
    pub auto_update_check: bool,
    /// AI-16: the permission mode new conversations start in on this device.
    pub ai_permission_mode: AiPermissionMode,
    /// AI-16: the user confirmed the first switch to bypass on this device.
    pub ai_bypass_confirmed: bool,
    /// AI-07: a system notification when a conversation finishes a turn or waits for approval
    /// while the window is not focused.
    pub ai_notifications: bool,
    /// The AI panel is open (§9).
    pub ai_panel_open: bool,
    /// The AI panel's width in CSS pixels.
    pub ai_panel_width: u32,
    /// SSH-13: the proxy that hosts set to the device default connect through, on this device.
    pub default_proxy_id: Option<String>,
    /// SSH-13: the proxy that Hatoba's own connections (sync, the AI assistant and the update
    /// check) go through on this device; the system proxy while `None` or once it is deleted.
    pub app_proxy_id: Option<String>,
    /// SEC-02: minutes without input before the vault locks; 0 never locks.
    pub auto_lock_minutes: u32,
    /// SEC-02: lock the vault when the computer sleeps.
    pub lock_on_sleep: bool,
    /// SEC-03: disconnect the SSH sessions, and end the local shells, when the vault locks.
    pub lock_disconnects_sessions: bool,
    /// SEC-08: seconds until a copied password is cleared from the clipboard; 0 never clears it.
    pub clipboard_clear_secs: u32,
    /// HOST-05: the hosts page lists the hosts or shows them as cards.
    pub hosts_view: HostsView,
    /// HOST-05, HOST-16: the order of the hosts page. A resource usage order applies while
    /// resource usage shows (HOST-14); the latest connection comes first otherwise.
    pub hosts_sort: HostsSort,
    /// HOST-14: the hosts page shows resource usage, which reads the listed hosts in the
    /// background.
    pub hosts_stats: bool,
    /// HOST-14: the resource usage columns turned off.
    pub dashboard_hidden_columns: Vec<String>,
    /// HOST-14: seconds between readings, 1 to 60.
    pub dashboard_interval_secs: u32,
    /// HOST-14: the CPU, memory and disk use, in percent, past which a value stands out; 0
    /// turns one off.
    pub dashboard_cpu_alert: u32,
    pub dashboard_mem_alert: u32,
    pub dashboard_disk_alert: u32,
    /// WIN-10: what the window's close button does on Windows and Linux.
    pub close_action: CloseAction,
    /// LOCAL-02: the shell new local tabs run on this device, by its id; the system default
    /// while `None` or when the shell is gone.
    pub local_shell: Option<String>,
    /// SFTP-05: the program that edits remote files on this device; the app the system opens each
    /// kind of file with while `None`.
    pub sftp_editor: Option<String>,
}

impl Default for LocalPrefs {
    fn default() -> Self {
        Self {
            language: Language::System,
            appearance: Appearance::System,
            density: Density::Regular,
            ui_zoom: 100,
            host_probe: true,
            auto_update_check: true,
            ai_permission_mode: AiPermissionMode::Manual,
            ai_bypass_confirmed: false,
            ai_notifications: true,
            ai_panel_open: false,
            ai_panel_width: 380,
            default_proxy_id: None,
            app_proxy_id: None,
            auto_lock_minutes: 15,
            lock_on_sleep: true,
            lock_disconnects_sessions: false,
            clipboard_clear_secs: 90,
            hosts_view: HostsView::List,
            hosts_sort: HostsSort::Recent,
            hosts_stats: false,
            dashboard_hidden_columns: Vec::new(),
            dashboard_interval_secs: 3,
            dashboard_cpu_alert: 90,
            dashboard_mem_alert: 90,
            dashboard_disk_alert: 90,
            close_action: CloseAction::Ask,
            local_shell: None,
            sftp_editor: None,
        }
    }
}

/// The interface zoom in effect (TERM-13). Also sent as `window://zoom` when the window moves to a
/// screen that changes it.
#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq, tauri_specta::Event)]
#[tauri_specta(event_name = "window://zoom")]
pub struct ZoomView {
    /// The zoom applied, in percent.
    pub zoom: u32,
    /// The largest step the screen the window is on allows.
    pub max: u32,
}

/// WIN-10: the window's close button asks, hides the window to the system tray, or quits.
/// Without a tray it always quits.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CloseAction {
    Ask,
    Tray,
    Quit,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostsView {
    List,
    Cards,
}

/// The orders of the hosts page: the latest connection, the name, the address, the custom order
/// (HOST-16), and the resource usage orders (HOST-14).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostsSort {
    Recent,
    Name,
    Address,
    Custom,
    Status,
    Cpu,
    Memory,
    Disk,
    Network,
    Uptime,
}

impl LocalPrefs {
    /// Reads stored prefs leniently: missing or unknown values fall back to the defaults, so
    /// prefs written by another app version never fail to load.
    pub fn from_stored(json: &str) -> Self {
        let mut merged = serde_json::to_value(Self::default()).unwrap_or_default();
        if let (Some(base), Ok(serde_json::Value::Object(stored))) = (
            merged.as_object_mut(),
            serde_json::from_str::<serde_json::Value>(json),
        ) {
            for (key, value) in stored {
                if base.contains_key(&key) {
                    let mut candidate = base.clone();
                    candidate.insert(key.clone(), value.clone());
                    if serde_json::from_value::<Self>(serde_json::Value::Object(candidate)).is_ok()
                    {
                        base.insert(key, value);
                    }
                }
            }
        }
        serde_json::from_value(merged).unwrap_or_default()
    }
}

/// Device-local state of the home tab's GitHub star prompt (spec §9; never synced).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct StarPrompt {
    /// When this device first read the state, Unix ms. The prompt waits a day from then.
    pub first_seen_at: i64,
    /// The user starred the repository, opened the bug report form, or closed the prompt.
    pub done: bool,
}
