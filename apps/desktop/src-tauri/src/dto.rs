//! Data transfer objects exchanged with the WebView (spec §10).
//!
//! These mirror `apps/desktop/src/ipc/types.ts` and are exported to TypeScript with tauri-specta.
//! **No DTO carries a secret**: hosts expose `has_password`, keys expose only public material.
//! Inputs may carry secrets one way (WebView → Rust), e.g. a new host password; they are never
//! echoed back.

use serde::{Deserialize, Serialize};
use specta::Type;

// ───────────────────────── Vault ─────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VaultState {
    Uninitialized,
    Locked,
    Unlocked,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct VaultStatus {
    pub state: VaultState,
    pub failed_attempts: u32,
    pub retry_at: Option<i64>,
    pub sync_kind: SyncKind,
    pub biometric_available: bool,
    pub biometric_enabled: bool,
}

// ───────────────────────── Hosts ─────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    Password,
    Key,
    Agent,
    Ask,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct HostView {
    pub id: String,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub auth_kind: AuthKind,
    pub has_password: bool,
    pub key_id: Option<String>,
    pub group_id: Option<String>,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub jump_host_id: Option<String>,
    pub note: String,
    pub updated_at: i64,
    pub last_connected_at: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct HostInput {
    pub id: Option<String>,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub auth_kind: AuthKind,
    /// `None` keeps the saved password (HOST-08).
    pub password: Option<String>,
    pub key_id: Option<String>,
    pub group_id: Option<String>,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub jump_host_id: Option<String>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct GroupView {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub sort: i32,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct GroupInput {
    pub id: Option<String>,
    pub name: String,
    pub parent_id: Option<String>,
    pub sort: i32,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct TagCount {
    pub name: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SshConfigCandidate {
    pub alias: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub identity_file: Option<String>,
    pub proxy_jump: Option<String>,
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ImportResult {
    pub hosts_created: u32,
    pub keys_imported: u32,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ProbeResult {
    pub id: String,
    pub online: bool,
    pub latency_ms: Option<u32>,
}

// ───────────────────────── Keys ─────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeyAlgorithm {
    Ed25519,
    Ecdsa,
    Rsa,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct KeyView {
    pub id: String,
    pub name: String,
    pub algorithm: KeyAlgorithm,
    pub bits: u32,
    pub public_key: String,
    pub fingerprint: String,
    pub comment: String,
    pub has_passphrase: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub used_by: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct KeyImportInput {
    pub name: String,
    pub private_key: Option<String>,
    pub path: Option<String>,
    pub passphrase: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GenerateAlgorithm {
    Ed25519,
    Rsa,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct KeyGenerateInput {
    pub name: String,
    pub algorithm: GenerateAlgorithm,
    pub comment: String,
    pub passphrase: Option<String>,
}

// ───────────────────────── SSH ─────────────────────────

#[derive(Debug, Clone, Deserialize, Type)]
pub struct ConnectOptions {
    pub cols: u32,
    pub rows: u32,
    /// `ask` auth (SSH-03): used once, never stored.
    pub password: Option<String>,
    pub passphrase: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Connecting,
    Connected,
    Disconnected,
    Failed,
}

#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "ssh://state")]
pub struct SessionStateEvent {
    pub session_id: String,
    pub host_id: String,
    pub state: SessionState,
    pub latency_ms: Option<u32>,
    pub error: Option<crate::error::AppError>,
    pub exit_status: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostKeyPromptKind {
    New,
    Changed,
}

#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "ssh://hostkey-prompt")]
pub struct HostKeyPrompt {
    pub request_id: String,
    pub session_id: Option<String>,
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint: String,
    pub kind: HostKeyPromptKind,
    pub known_fingerprint: Option<String>,
    pub known_key_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct AuthPromptField {
    pub prompt: String,
    pub echo: bool,
}

#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "ssh://auth-prompt")]
pub struct AuthPrompt {
    pub request_id: String,
    pub session_id: Option<String>,
    pub name: String,
    pub instructions: String,
    pub prompts: Vec<AuthPromptField>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct TestResult {
    pub ok: bool,
    pub latency_ms: Option<u32>,
    pub host_key_verified: bool,
    pub error: Option<crate::error::AppError>,
}

// ───────────────────────── SFTP ─────────────────────────

#[derive(Debug, Clone, Serialize, Type)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<i64>,
    pub permissions: String,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransferDirection {
    Upload,
    Download,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransferState {
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "transfer://progress")]
pub struct TransferProgressEvent {
    pub transfer_id: String,
    pub session_id: String,
    pub direction: TransferDirection,
    pub name: String,
    pub bytes: u64,
    pub total: u64,
    pub bytes_per_sec: u64,
    pub state: TransferState,
    pub error: Option<String>,
}

// ───────────────────────── Sync ─────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncKind {
    None,
    Worker,
    D1,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    Off,
    Idle,
    Syncing,
    Offline,
    AuthFailed,
    Error,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SyncCounts {
    pub hosts: u32,
    pub keys: u32,
    pub groups: u32,
}

#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "sync://status")]
pub struct SyncStatus {
    pub kind: SyncKind,
    pub endpoint: Option<String>,
    pub database: Option<String>,
    pub state: SyncState,
    pub last_synced_at: Option<i64>,
    pub pending: u32,
    pub conflicts: u32,
    pub auto_sync: bool,
    pub message: Option<String>,
    pub counts: Option<SyncCounts>,
}

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyncConfigInput {
    Worker {
        url: String,
        setup_token: Option<String>,
    },
    D1 {
        account_id: String,
        database_id: String,
        api_token: String,
    },
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct D1Database {
    pub id: String,
    pub name: String,
    pub region: Option<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SyncTestResult {
    pub ok: bool,
    pub initialized: bool,
    pub version: Option<String>,
    pub latency_ms: Option<u32>,
    pub databases: Option<Vec<D1Database>>,
    pub error: Option<crate::error::AppError>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct DeviceView {
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub created_at: i64,
    pub last_seen: i64,
    pub current: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolution {
    LocalWon,
    RemoteWon,
    KeptBoth,
    ModifiedWon,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ConflictField {
    pub field: String,
    pub local: Option<String>,
    pub remote: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ItemType {
    Host,
    Group,
    Key,
    KnownHost,
    Forward,
    Snippet,
    Settings,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ConflictView {
    pub id: i64,
    pub item_id: String,
    pub item_type: ItemType,
    pub item_name: String,
    pub resolution: ConflictResolution,
    pub local_updated_at: Option<i64>,
    pub remote_updated_at: Option<i64>,
    pub local_deleted: bool,
    pub remote_deleted: bool,
    pub fields: Vec<ConflictField>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictAction {
    Keep,
    Restore,
}

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

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TerminalSettings {
    pub font_family: String,
    pub font_size: u32,
    pub theme: ThemeChoice,
    pub cursor_style: CursorChoice,
    pub scrollback: u32,
    pub right_click: RightClick,
    pub confirm_multiline_paste: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SettingsView {
    pub terminal: TerminalSettings,
    pub auto_lock_minutes: u32,
    pub lock_disconnects_sessions: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
pub enum Language {
    #[serde(rename = "system")]
    System,
    #[serde(rename = "zh-CN")]
    ZhCn,
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
    pub host_probe: bool,
    /// Check GitHub for a newer release once after unlock. Off by default: no request leaves
    /// the device unless the user asks for it (spec §11, no telemetry).
    pub auto_update_check: bool,
}

impl Default for LocalPrefs {
    fn default() -> Self {
        Self {
            language: Language::System,
            appearance: Appearance::System,
            density: Density::Regular,
            host_probe: true,
            auto_update_check: false,
        }
    }
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
    pub mica: bool,
}

/// What the update check found on GitHub Releases (spec §11).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct UpdateCheck {
    pub current_version: String,
    /// The newest stable release, or `None` when none has been published yet.
    pub latest_version: Option<String>,
    /// The release page to download the installer from.
    pub release_url: Option<String>,
    pub update_available: bool,
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

// ───────────────────────── Port forwarding (FWD-01/02) ─────────────────────────

/// A saved local (`-L`) forward. Remote and dynamic forwards are P2.
#[derive(Debug, Clone, Serialize, Type)]
pub struct ForwardView {
    pub id: String,
    pub host_id: String,
    pub bind_address: String,
    pub bind_port: u16,
    pub dest_host: String,
    pub dest_port: u16,
    pub auto_start: bool,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct ForwardInput {
    pub id: Option<String>,
    pub host_id: String,
    pub bind_address: String,
    pub bind_port: u16,
    pub dest_host: String,
    pub dest_port: u16,
    pub auto_start: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ForwardState {
    Running,
    Stopped,
    Failed,
}

/// A forward's state on one live session.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "ssh://forward")]
pub struct ForwardStatusEvent {
    pub session_id: String,
    pub forward_id: String,
    pub state: ForwardState,
    /// The actually bound local port (differs from `bind_port` when it was 0).
    pub local_port: Option<u16>,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::LocalPrefs;

    #[test]
    fn prefs_from_an_older_version_leave_the_update_check_off() {
        let prefs = LocalPrefs::from_stored(r#"{"language":"ja","host_probe":false}"#);
        assert!(!prefs.auto_update_check);
        assert!(!prefs.host_probe);
        let prefs = LocalPrefs::from_stored(r#"{"auto_update_check":true}"#);
        assert!(prefs.auto_update_check);
    }
}
