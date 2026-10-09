//! Data transfer objects exchanged with the WebView (spec §10).
//!
//! These mirror `apps/desktop/src/ipc/types.ts` and are exported to TypeScript with tauri-specta.
//! **No DTO carries a secret**: hosts expose `has_password`, keys expose only public material.
//! Inputs may carry secrets one way (WebView → Rust), e.g. a new host password; they are never
//! echoed back.

use std::fmt;

use serde::{Deserialize, Serialize};
use specta::Type;
use zeroize::Zeroize;

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
    /// What the AI assistant is told about the host (AI-37).
    pub ai_notes: String,
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
    /// At most 2,000 characters (AI-37).
    pub ai_notes: String,
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
    /// The Worker's version stops sync until it or the app is updated; `worker_update` says which.
    Paused,
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
    /// Worker mode: how the Worker's `/v1/health` compares with this build (§6.7, Upgrades).
    /// `None` when there is nothing to show, including a dismissed "update available".
    pub worker_update: Option<WorkerUpdate>,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerUpdateKind {
    /// Older than the bundled version: sync continues, and the update can be dismissed.
    Available,
    /// Older than the minimum version, or a lower `api`: sync is paused.
    Required,
    /// A newer `api` than this app speaks: sync is paused until Hatoba is updated.
    AppRequired,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct WorkerUpdate {
    pub kind: WorkerUpdateKind,
    /// The version the Worker reports.
    pub version: String,
    /// The version "Update Worker" deploys; `None` in builds without the Worker bundle.
    pub bundled: Option<String>,
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
    AiProvider,
    SearchProvider,
    AiConversation,
    AiMessage,
    Skill,
    SkillFile,
    McpServer,
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

// ───────────────────────── In-app deployment (§6.7) ─────────────────────────

#[derive(Debug, Clone, Serialize, Type)]
pub struct CloudflareAccount {
    pub id: String,
    pub name: String,
}

/// Step 1 passed. Later calls name the deployment by `handle`; the API token stays in Rust.
#[derive(Debug, Clone, Serialize, Type)]
pub struct DeployStart {
    pub handle: String,
    /// The token belongs to the account given with it rather than to a user.
    pub account_owned: bool,
    /// The accounts a user token reaches; empty when the user has to enter the account ID.
    pub accounts: Vec<CloudflareAccount>,
    /// The default Worker and database names.
    pub worker_name: String,
    pub database_name: String,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct DeployTarget {
    pub account_id: String,
    pub worker_name: String,
    pub database_name: String,
    /// The workers.dev subdomain to create when the account has none.
    pub subdomain: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeployWorkerAction {
    Create,
    Update,
    /// A Hatoba Worker whose database holds a vault: the deployment stops.
    HasVault,
    /// A Worker the app does not recognize: the deployment stops.
    Foreign,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeployDatabaseAction {
    Create,
    /// An existing database with nothing but Hatoba's migrations.
    Use,
    /// The database already bound to the Hatoba Worker.
    Bound,
}

/// What step 2 found, shown before anything is written.
#[derive(Debug, Clone, Serialize, Type)]
pub struct DeployPlan {
    /// The account's workers.dev subdomain; `None` means the user has to choose one.
    pub subdomain: Option<String>,
    pub worker: DeployWorkerAction,
    pub database: Option<DeployDatabaseAction>,
    pub database_name: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeployStep {
    Verify,
    Inspect,
    CreateDatabase,
    Migrate,
    Upload,
    SetupToken,
    Route,
    Wait,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeployStepStatus {
    Running,
    Done,
    Skipped,
}

/// Sent on the `deploy_run` channel as each step starts and ends.
#[derive(Debug, Clone, Serialize, Type)]
pub struct DeployProgress {
    pub step: DeployStep,
    pub status: DeployStepStatus,
}

/// What the "Update Worker" form starts from (§6.7, Upgrades).
#[derive(Debug, Clone, Serialize, Type)]
pub struct UpgradeDefaults {
    /// The Worker URL in the sync settings.
    pub url: String,
    /// The account ID, when the app deployed the Worker.
    pub account_id: Option<String>,
    /// The Worker name: recorded when the app deployed it, or the first label of a workers.dev
    /// URL. `None` for a custom domain, where the user enters it.
    pub worker_name: Option<String>,
    /// The app deployed the Worker. Otherwise it may have come from the Deploy to Cloudflare
    /// button, whose next push from the user's repository replaces the upgrade.
    pub deployed_by_app: bool,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct UpgradeTarget {
    pub account_id: String,
    pub worker_name: String,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpgradeWorkerAction {
    Upgrade,
    /// No Worker has the name: the upgrade stops.
    Missing,
    /// A Worker the app does not recognize: the upgrade stops.
    Foreign,
    /// A Hatoba Worker without a vault: the upgrade stops.
    NoVault,
    /// The Worker is newer than the bundled version: the upgrade stops.
    Newer,
}

/// What step 2 of an upgrade found, shown before anything is written.
#[derive(Debug, Clone, Serialize, Type)]
pub struct UpgradePlan {
    pub worker: UpgradeWorkerAction,
    /// The database bound to the Worker.
    pub database_name: Option<String>,
    /// Migrations step 4 applies.
    pub migrations: u32,
    /// Step 7 runs: the Worker URL is its workers.dev URL.
    pub route: bool,
    /// The version the Worker URL reports, when it answers.
    pub version: Option<String>,
    /// The version the upgrade deploys.
    pub bundled: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct DeployOutcome {
    pub url: String,
    /// `false`: the Worker has not answered yet (step 8 is waiting); `deploy_check` asks again.
    pub ready: bool,
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
    /// AI-16: the permission mode new conversations start in on this device.
    pub ai_permission_mode: AiPermissionMode,
    /// AI-16: the user confirmed the first switch to bypass on this device.
    pub ai_bypass_confirmed: bool,
    /// AI-18: a turn pauses after this many tool calls.
    pub ai_tool_call_limit: u32,
    /// The AI panel is open (§9).
    pub ai_panel_open: bool,
    /// The AI panel's width in CSS pixels.
    pub ai_panel_width: u32,
}

impl Default for LocalPrefs {
    fn default() -> Self {
        Self {
            language: Language::System,
            appearance: Appearance::System,
            density: Density::Regular,
            host_probe: true,
            auto_update_check: false,
            ai_permission_mode: AiPermissionMode::Manual,
            ai_bypass_confirmed: false,
            ai_tool_call_limit: 25,
            ai_panel_open: false,
            ai_panel_width: 380,
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

/// Device-local state of the sidebar's GitHub star prompt (spec §9; never synced).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct StarPrompt {
    /// When this device first read the state, Unix ms. The prompt waits a day from then.
    pub first_seen_at: i64,
    /// The user starred the repository, opened the bug report form, or closed the prompt.
    pub done: bool,
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

// ───────────────────────── AI assistant (§13) ─────────────────────────
//
// API keys travel one way, WebView → Rust: views say only `has_api_key`, and the inputs that
// carry a key print it redacted and wipe it when dropped. Conversation content (messages, tool
// inputs and results) is never logged (SEC-04), so the inputs that carry it print only lengths.

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiProtocol {
    ChatCompletions,
    Anthropic,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
pub enum AiAuthHeader {
    #[serde(rename = "x-api-key")]
    XApiKey,
    #[serde(rename = "authorization")]
    Authorization,
}

/// A thinking level (AI-05), lowest first. Where one is optional, `None` is Default.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AiEffort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

/// A model of a provider (AI-03). Token limits are `None` when unknown.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct AiModel {
    pub id: String,
    pub name: String,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    /// The thinking levels the model accepts, lowest first (AI-05); empty for none, `None` when
    /// unknown (the panel then offers Low to High).
    pub efforts: Option<Vec<AiEffort>>,
    /// Whether the model supports adaptive thinking (Anthropic's model list), `None` when unknown.
    pub adaptive_thinking: Option<bool>,
}

/// AI-01. The API key never reaches the WebView: only whether one is saved.
#[derive(Debug, Clone, Serialize, Type)]
pub struct AiProviderView {
    pub id: String,
    pub name: String,
    pub protocol: AiProtocol,
    pub base_url: String,
    pub has_api_key: bool,
    /// Only used by `anthropic`.
    pub auth_header: AiAuthHeader,
    pub models: Vec<AiModel>,
    pub updated_at: i64,
}

#[derive(Clone, Deserialize, Type)]
pub struct AiProviderInput {
    /// `None` creates a provider. With an id, a `None` key keeps (or tests with) the saved one.
    pub id: Option<String>,
    pub name: String,
    pub protocol: AiProtocol,
    pub base_url: String,
    /// `None` keeps the saved key (as HOST-08 does for passwords); `""` clears it.
    pub api_key: Option<String>,
    pub auth_header: AiAuthHeader,
    pub models: Vec<AiModel>,
}

impl fmt::Debug for AiProviderInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiProviderInput")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("protocol", &self.protocol)
            .field("base_url", &self.base_url)
            .field("api_key", &redacted(self.api_key.as_deref()))
            .field("auth_header", &self.auth_header)
            .field("models", &self.models)
            .finish()
    }
}

impl Drop for AiProviderInput {
    fn drop(&mut self) {
        self.api_key.zeroize();
    }
}

/// AI-04 Test Connection, also used for the search provider test.
#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiTestFailure {
    Auth,
    Network,
    UnknownModel,
    InvalidUrl,
    Other,
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiTestResult {
    pub ok: bool,
    pub failure: Option<AiTestFailure>,
    pub status: Option<u16>,
    /// The provider's own message, when it sent one.
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    Brave,
    Tavily,
    Searxng,
}

/// The backend of `web_search` (AI-14).
#[derive(Debug, Clone, Serialize, Type)]
pub struct SearchProviderView {
    pub id: String,
    pub kind: SearchKind,
    /// The instance URL for SearXNG, `None` otherwise.
    pub base_url: Option<String>,
    pub has_api_key: bool,
    pub updated_at: i64,
}

#[derive(Clone, Deserialize, Type)]
pub struct SearchProviderInput {
    pub id: Option<String>,
    pub kind: SearchKind,
    pub base_url: Option<String>,
    /// `None` keeps the saved key; `""` clears it.
    pub api_key: Option<String>,
}

impl fmt::Debug for SearchProviderInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SearchProviderInput")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("base_url", &self.base_url)
            .field("api_key", &redacted(self.api_key.as_deref()))
            .finish()
    }
}

impl Drop for SearchProviderInput {
    fn drop(&mut self) {
        self.api_key.zeroize();
    }
}

/// How an optional secret prints: whether it is there, never what it is.
fn redacted(secret: Option<&str>) -> &'static str {
    match secret {
        None => "<keep>",
        Some("") => "<empty>",
        Some(_) => "<redacted>",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct AiModelRef {
    pub provider_id: String,
    pub model_id: String,
}

/// The synced `Settings.ai` (§5.1).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct AiSettingsView {
    pub default_model: Option<AiModelRef>,
    /// The thinking level new conversations start with (AI-05); `None` is Default.
    pub default_effort: Option<AiEffort>,
    pub search_provider_id: Option<String>,
    /// The built-in `hatoba` skill is offered (AI-34).
    pub builtin_skill_enabled: bool,
    /// Sent with every request, at most 4,000 characters (AI-36).
    pub custom_instructions: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiPermissionMode {
    Manual,
    Bypass,
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiConversationView {
    pub id: String,
    pub title: String,
    /// The host the conversation last worked on.
    pub host_id: Option<String>,
    pub pinned: bool,
    /// `entry_id` where the context sent to the model starts (AI-21).
    pub context_start: Option<String>,
    /// The thinking level its last message was sent with (AI-05); `None` is Default.
    pub effort: Option<AiEffort>,
    pub created_at: i64,
    pub updated_at: i64,
    /// The newest entry or conversation change, for sorting history (AI-23).
    pub last_activity: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiFinish {
    Stop,
    ToolCalls,
    Length,
    Refused,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiToolStatus {
    Ok,
    Error,
    Rejected,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiToolCall {
    pub id: String,
    pub name: String,
    /// JSON text.
    pub arguments: String,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
pub struct AiUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated: bool,
}

/// A stored conversation entry (§13.7), without the provider's raw message.
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum AiEntryView {
    User {
        entry_id: String,
        created_at: i64,
        text: String,
    },
    Assistant {
        entry_id: String,
        created_at: i64,
        provider_id: String,
        model_id: String,
        text: String,
        reasoning: Option<String>,
        tool_calls: Vec<AiToolCall>,
        finish: AiFinish,
        usage: Option<AiUsage>,
    },
    Tool {
        entry_id: String,
        created_at: i64,
        tool_call_id: String,
        status: AiToolStatus,
        content: String,
    },
    Summary {
        entry_id: String,
        created_at: i64,
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct AiConversationDetail {
    pub conversation: AiConversationView,
    pub entries: Vec<AiEntryView>,
    /// A turn of this conversation is running (its events go to the channel that started it).
    pub running: bool,
}

/// What a request is made with. The frontend owns the tab, so it says what the turn may act on.
#[derive(Debug, Clone, Deserialize, Type, PartialEq, Eq)]
pub struct AiTurnContext {
    pub provider_id: String,
    pub model_id: String,
    /// The thinking level (AI-05); `None` is Default. Requests send the highest level the model
    /// offers that is not above it, and the conversation keeps it for its next message.
    pub effort: Option<AiEffort>,
    /// The tab's host; the next message moves the conversation to it (AI-09).
    pub host_id: Option<String>,
    /// A connected terminal tab is attached; `false` offers no terminal tools (AI-09).
    pub tab: bool,
    /// The tab's SSH session while it is connected, whose server's identification string the
    /// system prompt states (§13.1).
    pub session_id: Option<String>,
    /// MCP servers switched off for this conversation (AI-30, P2).
    pub disabled_mcp_servers: Vec<String>,
}

#[derive(Clone, Deserialize, Type)]
pub struct AiSendInput {
    /// `None` starts a new conversation, stored with this first message (AI-07).
    pub conversation_id: Option<String>,
    pub text: String,
    pub context: AiTurnContext,
}

impl fmt::Debug for AiSendInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiSendInput")
            .field("conversation_id", &self.conversation_id)
            .field(
                "text",
                &format_args!("<{} chars>", self.text.chars().count()),
            )
            .field("context", &self.context)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct AiSendStarted {
    pub conversation: AiConversationView,
    pub user_entry: AiEntryView,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiTurnEndReason {
    Completed,
    Length,
    Refused,
    Stopped,
    Error,
}

/// Streamed on the channel of `ai_send` / `ai_retry` for the whole turn (§13.1).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AiTurnEvent {
    RequestStarted,
    Text {
        delta: String,
    },
    Reasoning {
        delta: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        estimated: bool,
    },
    /// An entry was stored: the assistant response, a tool result, or a cancelled result.
    Entry {
        entry: AiEntryView,
    },
    /// One response finished. With `tool_calls`, Rust waits for every call's result.
    Done {
        finish: AiFinish,
    },
    Error {
        status: Option<u16>,
        message: String,
    },
    /// The provider refused the thinking level, so the request was sent again without it and the
    /// model answers at its default depth (AI-05).
    EffortIgnored,
    TurnEnded {
        reason: AiTurnEndReason,
    },
}

/// A result the frontend produced: `read_terminal`, `send_input`, or a rejection (AI-17).
#[derive(Clone, Deserialize, Type)]
pub struct AiToolResultInput {
    pub status: AiToolStatus,
    pub content: String,
    /// AI-17 Edit: the arguments the user changed the call to, so the result tells the model.
    pub edited_arguments: Option<String>,
}

impl fmt::Debug for AiToolResultInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiToolResultInput")
            .field("status", &self.status)
            .field(
                "content",
                &format_args!("<{} chars>", self.content.chars().count()),
            )
            .field("edited", &self.edited_arguments.is_some())
            .finish()
    }
}

// ───────────────────────── AI assistant: skills (§13.8) ─────────────────────────
//
// Skill text reaches the model as instructions; it is never logged (SEC-04), so the types that
// carry it print only sizes.

#[derive(Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct SkillFileView {
    /// Relative path with forward slashes, such as `references/nginx.md`. Never `SKILL.md`.
    pub path: String,
    pub content: String,
}

impl fmt::Debug for SkillFileView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SkillFileView")
            .field("path", &self.path)
            .field("content", &format_args!("<{} bytes>", self.content.len()))
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct SkillView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    /// Paths of the files besides `SKILL.md`, sorted.
    pub files: Vec<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SkillDetail {
    pub skill: SkillView,
    /// `SKILL.md` without its frontmatter.
    pub body: String,
    pub files: Vec<SkillFileView>,
    /// Other frontmatter fields, kept for export. They change nothing (AI-27).
    pub frontmatter_keys: Vec<String>,
}

/// The built-in `hatoba` skill (AI-34), read-only.
#[derive(Debug, Clone, Serialize, Type)]
pub struct BuiltinSkillView {
    pub name: String,
    pub description: String,
    /// `Settings.ai.builtin_skill_enabled`.
    pub enabled: bool,
    /// `SKILL.md` without its frontmatter, with the app's version filled in.
    pub body: String,
    /// The files besides `SKILL.md`, sorted by path.
    pub files: Vec<SkillFileView>,
}

#[derive(Clone, Deserialize, Type)]
pub struct SkillInput {
    /// `None` creates a skill.
    pub id: Option<String>,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub body: String,
    /// Every file besides `SKILL.md`; a saved file left out is deleted.
    pub files: Vec<SkillFileView>,
}

impl fmt::Debug for SkillInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SkillInput")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("enabled", &self.enabled)
            .field("body", &format_args!("<{} bytes>", self.body.len()))
            .field("files", &self.files)
            .finish()
    }
}

/// Why a skill cannot be imported or saved (AI-27).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SkillIssue {
    MissingSkillMd,
    InvalidFrontmatter {
        detail: String,
    },
    InvalidName {
        name: String,
    },
    MissingDescription,
    DescriptionTooLong {
        chars: u64,
    },
    FileTooLarge {
        path: String,
        size: u64,
    },
    UnsafePath {
        path: String,
    },
    TooManyFiles {
        count: u64,
    },
    /// More than 5 MB in total.
    TooLarge {
        bytes: u64,
    },
}

/// Why a file dropped on the AI panel is not attached (AI-35): the panel's own reasons.
#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DroppedFileRefusal {
    /// An image, which attachments do not take yet.
    Image,
    /// Over 256 KB.
    TooLarge,
    /// NUL bytes or invalid UTF-8.
    Binary,
    /// Not a file, gone, or not readable.
    Unreadable,
}

/// A text file dropped on the AI panel (AI-35), named by its base name only, never its path (a
/// path can name the local user).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DroppedFile {
    Ok {
        name: String,
        text: String,
    },
    Refused {
        name: String,
        reason: DroppedFileRefusal,
    },
}

/// What an import would save, shown before saving (AI-27). Importable when `issues` is empty.
#[derive(Debug, Clone, Serialize, Type)]
pub struct SkillImportPreview {
    /// `None` when `SKILL.md` is missing or unreadable.
    pub name: Option<String>,
    pub description: Option<String>,
    pub body: Option<String>,
    pub files: Vec<SkillFileView>,
    pub frontmatter_keys: Vec<String>,
    /// Files that are not UTF-8 text, which the import skips.
    pub skipped: Vec<String>,
    pub issues: Vec<SkillIssue>,
    /// A saved skill with the same name, which the user may replace (or rename the new one).
    pub existing_id: Option<String>,
    /// The name is the built-in skill's (AI-34): the skill can be imported only under another.
    pub reserved_name: bool,
    /// A digest of what was read, which `skill_import` takes back: it imports the source only if
    /// it still reads the same, so what is saved is what this preview showed.
    pub token: String,
}

// ───────────────────────── AI assistant: MCP servers (§13.9) ─────────────────────────
//
// Environment and header values travel one way, WebView → Rust, like API keys: views carry
// only their names, and the inputs that carry them print them redacted and wipe them when
// dropped (AI-29).

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpTransportView {
    Stdio {
        command: String,
        args: Vec<String>,
        env_keys: Vec<String>,
    },
    Http {
        url: String,
        header_keys: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpServerView {
    pub id: String,
    pub name: String,
    pub transport: McpTransportView,
    /// Ask even in bypass mode. Synced (AI-31).
    pub always_ask: bool,
    /// Enabled on this device; a server from another device starts enabled only for `http`.
    pub enabled: bool,
    /// On this device, every tool of the server runs without asking in manual mode (AI-31).
    pub always_allow: bool,
    /// On this device, these tools (the server's own names) run without asking in manual mode.
    pub always_allow_tools: Vec<String>,
    pub updated_at: i64,
}

#[derive(Clone, Deserialize, Type)]
pub struct McpSecretInput {
    pub key: String,
    /// `None` keeps the saved value for this key.
    pub value: Option<String>,
}

impl fmt::Debug for McpSecretInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpSecretInput")
            .field("key", &self.key)
            .field("value", &redacted(self.value.as_deref()))
            .finish()
    }
}

impl Drop for McpSecretInput {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpTransportInput {
    Stdio {
        command: String,
        args: Vec<String>,
        env: Vec<McpSecretInput>,
    },
    Http {
        url: String,
        headers: Vec<McpSecretInput>,
    },
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct McpServerInput {
    /// `None` creates a server.
    pub id: Option<String>,
    pub name: String,
    pub transport: McpTransportInput,
    pub always_ask: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpServerState {
    Stopped,
    Starting,
    Running,
    Failed,
}

/// Shown on the approval card; never changes whether a call asks (AI-31).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq, Default)]
pub struct McpToolAnnotations {
    pub title: Option<String>,
    pub read_only_hint: Option<bool>,
    pub destructive_hint: Option<bool>,
    pub idempotent_hint: Option<bool>,
    pub open_world_hint: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpToolView {
    /// The name offered to the model, `mcp__<server>__<tool>` after cleaning (AI-30).
    pub name: String,
    /// The server's own tool name.
    pub tool: String,
    pub description: String,
    pub annotations: McpToolAnnotations,
    /// Always allow on this device (per tool, or because the whole server is).
    pub always_allow: bool,
}

/// A server's live state (AI-32), also pushed as `ai://mcp-status` on every change. The stderr
/// lines stay in memory and are never logged.
#[derive(Clone, Serialize, Type, PartialEq, Eq, tauri_specta::Event)]
#[tauri_specta(event_name = "ai://mcp-status")]
pub struct McpServerStatus {
    pub server_id: String,
    pub state: McpServerState,
    pub error: Option<String>,
    /// The last stderr lines of a `stdio` server, kept in memory only (AI-32).
    pub stderr: Vec<String>,
    /// The tools from the last successful listing.
    pub tools: Vec<McpToolView>,
}

impl fmt::Debug for McpServerStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpServerStatus")
            .field("server_id", &self.server_id)
            .field("state", &self.state)
            .field("error", &self.error.is_some())
            .field("stderr", &format_args!("<{} lines>", self.stderr.len()))
            .field("tools", &self.tools.len())
            .finish()
    }
}

/// The MCP tool behind a name the model called, for the approval card (AI-31).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpToolInfo {
    pub server_id: String,
    pub server_name: String,
    pub always_ask: bool,
    pub tool: McpToolView,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct McpImportServer {
    pub name: String,
    pub transport: McpTransportView,
    /// A saved server has this name; the import adds a numeric suffix.
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct McpImportSkipped {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct McpImportPreview {
    pub servers: Vec<McpImportServer>,
    /// Entries that cannot be imported, such as an `sse` server, with the reason.
    pub skipped: Vec<McpImportSkipped>,
}

// ───────────────────────── AI assistant: history search (AI-24) ─────────────────────────

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiSearchHit {
    pub conversation_id: String,
    /// The first matching entry, or `None` when only the title matched.
    pub entry_id: Option<String>,
    /// Text around the first match.
    pub snippet: String,
}

#[cfg(test)]
mod tests {
    use super::{AiPermissionMode, LocalPrefs};

    #[test]
    fn prefs_from_an_older_version_leave_the_update_check_off() {
        let prefs = LocalPrefs::from_stored(r#"{"language":"ja","host_probe":false}"#);
        assert!(!prefs.auto_update_check);
        assert!(!prefs.host_probe);
        let prefs = LocalPrefs::from_stored(r#"{"auto_update_check":true}"#);
        assert!(prefs.auto_update_check);
    }

    #[test]
    fn ai_prefs_default_to_manual_and_unknown_values_fall_back() {
        let prefs = LocalPrefs::from_stored(r#"{"language":"ja"}"#);
        assert_eq!(prefs.ai_permission_mode, AiPermissionMode::Manual);
        assert!(!prefs.ai_bypass_confirmed && !prefs.ai_panel_open);
        assert_eq!((prefs.ai_tool_call_limit, prefs.ai_panel_width), (25, 380));

        let prefs = LocalPrefs::from_stored(
            r#"{"ai_permission_mode":"bypass","ai_bypass_confirmed":true,"ai_tool_call_limit":40,
                "ai_panel_open":true,"ai_panel_width":520}"#,
        );
        assert_eq!(prefs.ai_permission_mode, AiPermissionMode::Bypass);
        assert!(prefs.ai_bypass_confirmed && prefs.ai_panel_open);
        assert_eq!((prefs.ai_tool_call_limit, prefs.ai_panel_width), (40, 520));

        // A mode a newer build added, or a broken value, is read as the default.
        let prefs = LocalPrefs::from_stored(
            r#"{"ai_permission_mode":"auto","ai_tool_call_limit":-1,"host_probe":false}"#,
        );
        assert_eq!(prefs.ai_permission_mode, AiPermissionMode::Manual);
        assert_eq!(prefs.ai_tool_call_limit, 25);
        assert!(!prefs.host_probe);
    }
}
