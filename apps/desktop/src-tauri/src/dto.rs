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
    /// SEC-11: a PIN is set on this device.
    pub pin_enabled: bool,
    /// The PIN also works after the app restarts.
    pub pin_persist: bool,
    /// The PIN can unlock now (the lock screen asks for it instead of the master password).
    pub pin_unlock: bool,
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

/// Which proxy a host's connection goes through (SSH-13).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    /// This device's default proxy, if it has one.
    DeviceDefault,
    /// No proxy.
    Direct,
    /// The saved proxy in `proxy_id`.
    Proxy,
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
    /// SSH-13. With a jump host, the jump host's choice applies instead.
    pub proxy_mode: ProxyMode,
    /// The saved proxy, when `proxy_mode` is `proxy`.
    pub proxy_id: Option<String>,
    pub note: String,
    /// What the AI assistant is told about the host (AI-37).
    pub ai_notes: String,
    pub updated_at: i64,
    pub last_connected_at: Option<i64>,
    /// The server's operating system from its SSH identification string on the last connection
    /// from this device, such as `ubuntu` (HOST-11). Device-local, like `last_connected_at`.
    pub os: Option<String>,
    /// The host's terminals show the server's resource usage on this device (TERM-12).
    /// Device-local, like `last_connected_at`, and off until turned on.
    pub show_stats: bool,
    /// The dashboard shows the host on this device (HOST-14). Device-local, like
    /// `last_connected_at`, and on until turned off.
    pub dashboard: bool,
    /// Environment variables the terminal asks the server to set (SSH-14).
    pub env: Vec<HostEnvVar>,
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
    pub proxy_mode: ProxyMode,
    /// Required when `proxy_mode` is `proxy`, ignored otherwise.
    pub proxy_id: Option<String>,
    pub note: String,
    /// At most 16 KB of UTF-8 (AI-37).
    pub ai_notes: String,
    /// Names are trimmed; see `hatoba_core::model::check_host_env` for the rules (SSH-14).
    pub env: Vec<HostEnvVar>,
}

/// One of a host's environment variables (SSH-14).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct HostEnvVar {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct GroupView {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub sort: i32,
    /// The dashboard shows the group's hosts on this device (HOST-14). Device-local, and on
    /// until turned off.
    pub dashboard: bool,
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
    /// The `IdentityFile` an import with keys reads: the first one that exists, otherwise the
    /// first one listed.
    pub identity_file: Option<String>,
    /// Whether `identity_file` exists, so that importing keys would read it.
    pub identity_file_found: bool,
    pub proxy_jump: Option<String>,
    /// `ProxyCommand`, which is not imported: the host connects without it (SSH-11).
    pub proxy_command: Option<String>,
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

// ───────────────────────── Proxies (SSH-13) ─────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProxyKind {
    Socks5,
    Http,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ProxyView {
    pub id: String,
    pub name: String,
    pub kind: ProxyKind,
    pub address: String,
    pub port: u16,
    /// Empty when the proxy needs no sign-in.
    pub username: String,
    pub has_password: bool,
    /// Hosts that name this proxy (not those that use it as the device default).
    pub host_ids: Vec<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct ProxyInput {
    pub id: Option<String>,
    pub name: String,
    pub kind: ProxyKind,
    pub address: String,
    pub port: u16,
    pub username: String,
    /// `None` keeps the saved password, an empty string removes it (as HOST-08).
    pub password: Option<String>,
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

/// A target typed into the hosts search field (quick connect, HOST-12). It is never saved as a
/// host; once connected it joins the device-local recent list.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct QuickTarget {
    /// DNS name or IP address, IPv6 without brackets.
    pub address: String,
    pub port: u16,
    pub username: String,
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
    /// `None` for a quick-connect session (HOST-12).
    pub host_id: Option<String>,
    pub state: SessionState,
    pub latency_ms: Option<u32>,
    pub error: Option<crate::error::AppError>,
    pub exit_status: Option<u32>,
}

// ───────────────────────── Local terminals ─────────────────────────

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

/// A shell this computer can run in a local tab (LOCAL-02).
#[derive(Debug, Clone, Serialize, Type)]
pub struct LocalShellView {
    /// Stable on this device: `pwsh`, `powershell`, `cmd`, `git-bash` or `wsl:<distribution>` on
    /// Windows, the shell's path elsewhere.
    pub id: String,
    /// In English; the app translates the names of the Windows shells.
    pub name: String,
    pub kind: LocalShellKind,
    /// The system default: PowerShell 7, else Windows PowerShell, on Windows; `$SHELL` elsewhere.
    pub default: bool,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct LocalOpenOptions {
    pub cols: u32,
    pub rows: u32,
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
    /// The login password of a quick connection (HOST-12), asked like an "ask every time"
    /// password (SSH-03) rather than as a server prompt; answered with one value.
    pub password: bool,
    /// `user@host:port` of the hop that asks.
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct TestResult {
    pub ok: bool,
    pub latency_ms: Option<u32>,
    pub host_key_verified: bool,
    pub error: Option<crate::error::AppError>,
}

/// One reading of a server's resource usage (TERM-12). Sizes are in bytes and rates in bytes
/// per second; what the server did not report is `None`.
#[derive(Debug, Clone, Serialize, Type, PartialEq)]
pub struct ServerStatsView {
    /// Busy share of all CPUs since the previous reading, 0 to 100; `None` in the first one.
    pub cpu_percent: Option<f64>,
    pub cpus: Option<u32>,
    /// Load averages over 1, 5 and 15 minutes.
    pub load: Option<[f64; 3]>,
    pub mem_total: Option<u64>,
    /// The total less what the kernel counts as available.
    pub mem_used: Option<u64>,
    /// Zero when the server has no swap.
    pub swap_total: Option<u64>,
    pub swap_used: Option<u64>,
    pub net_rx_rate: Option<u64>,
    pub net_tx_rate: Option<u64>,
    /// The interfaces the rates count: those of the default routes, or else all but loopback.
    pub net_interfaces: Vec<String>,
    /// The root filesystem.
    pub disk_total: Option<u64>,
    pub disk_used: Option<u64>,
    /// Space left for unprivileged users, as `df` counts it.
    pub disk_available: Option<u64>,
    pub uptime_secs: Option<u64>,
    /// Bytes received and sent over `net_interfaces` since each came up, usually at boot.
    pub net_rx_total: Option<u64>,
    pub net_tx_total: Option<u64>,
    /// Every mounted filesystem that stores files, each device once, at most 32; only on the
    /// dashboard (HOST-14), refreshed every fifth reading.
    pub mounts: Vec<MountView>,
}

/// One mounted filesystem's space, in bytes, as `df -P` reports it.
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct MountView {
    /// What is mounted, such as `/dev/sda1` or `nas:/export`.
    pub source: String,
    pub mount_point: String,
    pub total: u64,
    pub used: u64,
    /// Space left for unprivileged users.
    pub available: u64,
}

/// Streamed on the channel of `ssh_stats_start` (TERM-12).
#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StatsEvent {
    /// A reading, one per interval.
    Stats { stats: Box<ServerStatsView> },
    /// The server does not run Linux; the last event. `system` is its name, such as `FreeBSD`,
    /// or empty when unknown.
    Unsupported { system: String },
    /// Sampling stopped on its own (the script failed or the connection ended); the last event.
    Ended { error: crate::error::AppError },
}

// ───────────────────────── Dashboard (HOST-14) ─────────────────────────

/// Why the dashboard does not sign in to a host on its own: the connection needs the user.
#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DashboardHold {
    /// The host or a jump host asks for its password on every connection (SSH-03).
    Password,
    /// The key's passphrase is not saved, or the saved one is wrong.
    Passphrase,
    /// The server wants keyboard-interactive answers, such as a verification code (SSH-08).
    Prompt,
    /// The server's host key is not trusted yet (SSH-04).
    HostKeyNew,
    /// The server's host key differs from the trusted one (SSH-04).
    HostKeyChanged,
    /// The server rejected the saved credentials. Not tried again until **Retry**.
    SignInFailed,
}

/// Streamed on the channel of `dashboard_watch` (HOST-14), each naming its host.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DashboardEvent {
    /// Whether the host's first hop answered the HOST-10 probe: the host itself, or its
    /// outermost jump host.
    Probe { host_id: String, online: bool },
    /// Waiting for one of the connection slots, or signing in.
    Connecting { host_id: String },
    /// The host's connection settings changed since its readings were made, so they may be of
    /// another server: they are dropped.
    Reset { host_id: String },
    /// Connected and reading, over the dashboard's own connection or a tab's (`tab`).
    Reading {
        host_id: String,
        os: Option<String>,
        tab: bool,
    },
    /// A reading, one per interval.
    Stats {
        host_id: String,
        stats: Box<ServerStatsView>,
    },
    /// The server does not run Linux, so only its online status shows. `system` is its name,
    /// or empty when unknown.
    Unsupported { host_id: String, system: String },
    /// The connection needs the user; the dashboard waits for **Retry**, a change to the host,
    /// or a tab on it. `error` is the server's answer, when there was one.
    Held {
        host_id: String,
        hold: DashboardHold,
        error: Option<crate::error::AppError>,
    },
    /// Connecting or reading failed; tried again after a minute.
    Failed {
        host_id: String,
        error: crate::error::AppError,
    },
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
    /// The items the last sync round could not upload because they are larger than sync accepts
    /// (§6.2). Empty while locked and until a round of this connection has finished.
    pub too_large: Vec<TooLargeItem>,
}

/// An item larger than sync accepts. It stays pending, and every round tries it again until it is
/// shortened or deleted.
#[derive(Debug, Clone, Serialize, Type)]
pub struct TooLargeItem {
    pub item_id: String,
    pub item_type: ItemType,
    pub item_name: String,
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
    Proxy,
    AiProvider,
    SearchProvider,
    AiConversation,
    AiMessage,
    Skill,
    SkillFile,
    McpServer,
    AiMemory,
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
    /// For `has_vault`: the workers.dev URL to join it on (Flow C), or `None` when no workers.dev
    /// URL reaches the Worker.
    pub vault_url: Option<String>,
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
    pub right_click: RightClick,
    pub confirm_multiline_paste: bool,
}

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
    /// SEC-02: minutes without input before the vault locks; 0 never locks.
    pub auto_lock_minutes: u32,
    /// SEC-02: lock the vault when the computer sleeps.
    pub lock_on_sleep: bool,
    /// SEC-03: disconnect the SSH sessions, and end the local shells, when the vault locks.
    pub lock_disconnects_sessions: bool,
    /// SEC-08: seconds until a copied password is cleared from the clipboard; 0 never clears it.
    pub clipboard_clear_secs: u32,
    /// HOST-14: the dashboard shows cards or a table.
    pub dashboard_layout: DashboardLayout,
    /// HOST-14: the table columns turned off.
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
}

impl Default for LocalPrefs {
    fn default() -> Self {
        Self {
            language: Language::System,
            appearance: Appearance::System,
            density: Density::Regular,
            host_probe: true,
            auto_update_check: true,
            ai_permission_mode: AiPermissionMode::Manual,
            ai_bypass_confirmed: false,
            ai_notifications: true,
            ai_panel_open: false,
            ai_panel_width: 380,
            default_proxy_id: None,
            auto_lock_minutes: 15,
            lock_on_sleep: true,
            lock_disconnects_sessions: false,
            clipboard_clear_secs: 90,
            dashboard_layout: DashboardLayout::Cards,
            dashboard_hidden_columns: Vec::new(),
            dashboard_interval_secs: 3,
            dashboard_cpu_alert: 90,
            dashboard_mem_alert: 90,
            dashboard_disk_alert: 90,
            close_action: CloseAction::Ask,
            local_shell: None,
        }
    }
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
pub enum DashboardLayout {
    Cards,
    Table,
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
}

/// A system notification (AI-07). The WebView sends one only while the window is not focused.
#[derive(Debug, Clone, Deserialize, Type)]
pub struct NotificationInput {
    pub title: String,
    pub body: String,
    pub target: NotificationTarget,
}

/// What a notification is about, which a click on it shows: a tab, and in it a conversation
/// (AI-07). The click brings the window back and sends this to the WebView.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "app://notification-click")]
pub struct NotificationTarget {
    pub tab: Option<String>,
    pub conversation_id: Option<String>,
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

/// A provider's protocol (spec §13.2). A `system_one` provider answers decision questions for
/// the judge of Auto mode (AI-47) and cannot chat.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiProtocol {
    ChatCompletions,
    Anthropic,
    Responses,
    SystemOne,
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

/// A model of a provider (AI-03). Token limits are `None` when unknown. A provider's view fills
/// what a row leaves unknown from models.dev, as requests do.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct AiModel {
    pub id: String,
    pub name: String,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    /// The thinking levels the model accepts, lowest first (AI-05); empty for none, `None` when
    /// unknown (the panel then offers Low to High).
    pub efforts: Option<Vec<AiEffort>>,
    /// Whether the model supports adaptive thinking (Anthropic's model list or models.dev),
    /// `None` when unknown.
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
    /// Set for a provider that signs in with a ChatGPT account instead of a key (AI-52).
    pub chatgpt: Option<AiChatgptView>,
    pub updated_at: i64,
}

/// AI-52: where a ChatGPT provider's sign-in stands on this device. Its tokens never reach the
/// WebView.
#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiChatgptStatus {
    /// Signed in, with plan use allowed.
    SignedIn,
    /// Signed in, but plan use was not allowed.
    PlanDisabled,
    /// The session ended (expired, revoked, or signed out in ChatGPT): sign in again.
    Ended,
    /// Not signed in on this device (signed out here, or signed in on another device only).
    SignedOut,
}

/// AI-52: the ChatGPT account of a provider.
#[derive(Debug, Clone, Serialize, Type)]
pub struct AiChatgptView {
    pub email: String,
    pub status: AiChatgptStatus,
}

/// AI-52: a finished sign-in.
#[derive(Debug, Clone, Serialize, Type)]
pub struct AiChatgptSignIn {
    pub provider: AiProviderView,
    /// The sign-in registered Hatoba for a new account, so the panel says once that requests
    /// now use the ChatGPT plan.
    pub first_sign_in: bool,
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
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiSettingsView {
    pub default_model: Option<AiModelRef>,
    /// The thinking level new conversations start with (AI-05); `None` is Default.
    pub default_effort: Option<AiEffort>,
    pub search_provider_id: Option<String>,
    /// While no search provider is chosen, `web_search` uses the chat provider's own search
    /// (AI-14).
    pub native_search: bool,
    /// The built-in `hatoba` skill is offered (AI-34).
    pub builtin_skill_enabled: bool,
    /// Sent with every request, at most 32 KB of UTF-8 (AI-36).
    pub custom_instructions: String,
    /// Messages go with the attached terminal's screen unless the panel's + menu turns it off for
    /// the conversation (AI-46).
    pub send_terminal_screen: bool,
    /// The judge of Auto mode (AI-47): a model of a System One provider or of a provider that
    /// chats. Auto is unavailable until one is chosen.
    pub judge_model: Option<AiModelRef>,
    /// Conversations start with the memories, and the assistant saves them (AI-53).
    pub memory: bool,
}

/// What `ai_settings_save` changes in `Settings.ai`, like [`SettingsPatch`]: a field left out keeps
/// its stored value. `null` clears the default model, the search provider and the judge, and sets
/// the default thinking level to Default.
#[derive(Debug, Clone, Default, Deserialize, Type)]
#[serde(default)]
pub struct AiSettingsPatch {
    #[specta(type = Option<AiModelRef>)]
    pub default_model: Clearable<AiModelRef>,
    #[specta(type = Option<AiEffort>)]
    pub default_effort: Clearable<AiEffort>,
    #[specta(type = Option<String>)]
    pub search_provider_id: Clearable<String>,
    pub native_search: Option<bool>,
    pub builtin_skill_enabled: Option<bool>,
    pub custom_instructions: Option<String>,
    pub send_terminal_screen: Option<bool>,
    #[specta(type = Option<AiModelRef>)]
    pub judge_model: Clearable<AiModelRef>,
    pub memory: Option<bool>,
}

/// A patch field that `null` clears.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Clearable<T> {
    /// Left out: the stored value stays.
    #[default]
    Keep,
    Clear,
    Set(T),
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Clearable<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Option::<T>::deserialize(deserializer)?.map_or(Self::Clear, Self::Set))
    }
}

/// A conversation's permission mode on this device (AI-16, spec §13.5), the strictest first.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiPermissionMode {
    Manual,
    /// AI-47: a call that manual approval would ask about goes to the judge first
    /// (`ai_judge`), which runs it without the approval card or lets the card ask.
    Auto,
    Bypass,
}

/// What the judge of Auto mode decided about a call (AI-47). It never rejects a call.
#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiJudgeDecision {
    /// The call runs without the approval card.
    Run,
    /// The approval card asks, as in manual approval.
    Ask,
}

/// The judge's answer to one of its questions about a call (AI-47).
#[derive(Debug, Clone, Serialize, Type, PartialEq)]
pub struct AiJudgeAnswer {
    /// The question's id, such as `requested` or `destroys_data`.
    pub question: String,
    /// The probability of yes, from 0 to 1. Always finite, so never `null` in JSON, which the
    /// generated type of a float would allow.
    #[specta(type = specta_typescript::Number)]
    pub probability: f64,
}

/// The verdict of the judge of Auto mode on a call (AI-47): from `ai_judge`, and stored with the
/// result of a call Auto decided, so history shows why it ran without the card or what concerned
/// the judge. It never reaches the model.
#[derive(Debug, Clone, Serialize, Type, PartialEq)]
pub struct AiAutoVerdict {
    /// The provider item id of the judge; empty when no judge was chosen.
    pub provider_id: String,
    /// The judge's model id; empty when no judge was chosen.
    pub model_id: String,
    pub decision: AiJudgeDecision,
    /// The answers in the order the judge asks its questions, then any others by id; empty when
    /// the judge failed.
    pub answers: Vec<AiJudgeAnswer>,
    /// The ids of the questions whose answers kept the call from running, in the same order.
    pub concerns: Vec<String>,
    /// Why the judge gave no usable answer: `unavailable` (no judge, or its provider or model is
    /// gone), `too_long`, `timeout`, `unreachable`, `refused`, `malformed` or `cancelled`, or a
    /// kind a newer version added. Then the decision is to ask.
    pub failure: Option<String>,
}

/// The judge's verdict on one sample call of **Test** beside the judge in Settings → AI
/// (`ai_judge_test`, AI-47).
#[derive(Debug, Clone, Serialize, Type, PartialEq)]
pub struct AiJudgeSampleOutcome {
    /// The sample's stable name, such as `restart_requested`; a newer version may add others.
    pub name: String,
    /// The decision a good judge gives the sample.
    pub expected: AiJudgeDecision,
    /// What the judge decided, with its answers in the order it asks its questions.
    pub verdict: AiAutoVerdict,
}

/// What an AI approval saved on this device lets run without asking (AI-19).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiAllowanceKind {
    /// `run_command` calls whose command starts with `value`, on one host.
    Command,
    /// Every call of the tool `value` (`edit_file`, `write_file` or `send_input`), on one host.
    Tool,
    /// `fetch_url` calls of pages on the host name `value`, on any tab.
    Domain,
}

/// An AI approval saved on this device (AI-19): never synced, kept until the user removes it.
/// A command or tool approval names its host by `host_id`, or by `target` for a quick-connect tab
/// or this computer's local tabs.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct AiAllowance {
    pub kind: AiAllowanceKind,
    /// The saved host the approval is for.
    pub host_id: Option<String>,
    /// The quick-connect target (`user@host:port`) the approval is for, without a saved host, or
    /// `local` for this computer's local tabs (§7.5). Approvals stay on the device, so `local`
    /// always means the computer that keeps them.
    pub target: Option<String>,
    /// The command prefix, the tool's name, or the domain.
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiConversationView {
    pub id: String,
    pub title: String,
    /// The host the conversation last worked on.
    pub host_id: Option<String>,
    /// The quick-connect target (HOST-12) it last worked on while `host_id` is `None` (AI-09),
    /// as the conversation records it: the address in lowercase and without brackets.
    pub quick_target: Option<QuickTarget>,
    /// AI-43: with neither, the name of the server a fork's copies came from, when the fork could
    /// not tell which it is; its next message on a tab of a host or target notes the move from it.
    pub origin: Option<String>,
    /// §7.5: with neither a host nor a target, the computer whose local terminal tab it last
    /// worked on, by its name (`AppInfo.device_name` on that computer).
    pub local: Option<String>,
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

/// What `ai_tool_run` stored: the call's result entry.
#[derive(Debug, Clone, Serialize, Type, PartialEq)]
pub struct AiToolRun {
    pub entry: AiEntryView,
    /// AI-41: keeping the snapshot of this call's change dropped older ones over the size limit,
    /// of this or another conversation, so the panel reads the file changes of its other
    /// conversations again.
    pub dropped_file_changes: bool,
}

/// A stored conversation entry (§13.7), without the provider's raw message.
#[derive(Debug, Clone, Serialize, Type, PartialEq)]
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
        /// What the judge of Auto mode decided about the call (AI-47), when Auto decided it.
        auto: Option<AiAutoVerdict>,
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
    /// The changes of its `edit_file` and `write_file` calls that this device can revert, or
    /// has reverted (AI-41).
    pub file_changes: Vec<AiFileChange>,
}

/// A change an `edit_file` or `write_file` call made, kept on this device so the user can revert
/// it (AI-41).
#[derive(Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiFileChange {
    /// The entry id of the response that made the call.
    pub response_id: String,
    pub tool_call_id: String,
    /// The server the call wrote on, as `user@host:port`.
    pub server: String,
    /// The file's path as the call resolved it.
    pub path: String,
    /// The call created the file, so a revert deletes it.
    pub created: bool,
    pub state: AiFileChangeState,
}

/// Where a file tool call's change stands on this device (AI-41).
#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiFileChangeState {
    /// The call wrote the file.
    Written,
    /// The write failed after it may have begun, or its end was never recorded: the file may be
    /// cut short. It can be reverted like a written one.
    Unfinished,
    /// The user reverted the change.
    Reverted,
    /// The 32 MB limit dropped what the file held before, so the change can no longer be
    /// reverted.
    Dropped,
}

impl fmt::Debug for AiFileChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiFileChange")
            .field("response_id", &self.response_id)
            .field("tool_call_id", &self.tool_call_id)
            .field("created", &self.created)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// Why a revert did not happen (AI-41), for the panel to say in the user's language.
#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiRevertFailure {
    /// The conversation's tab is not connected.
    Disconnected,
    /// The tab is connected to another server than the one the call wrote on; `detail` names it.
    OtherServer,
    /// The user the tab is logged in as cannot write or delete the file.
    PermissionDenied,
    /// The file changed since the call wrote it into something that is not a text file Hatoba
    /// can read (a directory, a binary file, a file over 1 MB): there is no diff to show.
    NotText,
    /// A symbolic link is in the place of the file the call created, so neither the link nor its
    /// target was deleted.
    SymbolicLink,
    /// Reading the file did not finish within 30 seconds; nothing was written.
    TimedOut,
    /// Writing or deleting failed after it may have begun, a timeout included: the file may be
    /// cut short, or deleted. `detail` says why, when there was an error.
    Unfinished,
    /// Anything else; `detail` says what.
    Failed,
}

/// The outcome of a revert (AI-41).
#[derive(Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AiFileRevert {
    /// The file holds what it held before the call again, or no longer exists when the call
    /// created it. `wrote`: the revert wrote or deleted it; otherwise it was so already, as after
    /// a revert whose result was lost.
    Reverted { wrote: bool },
    /// The file holds neither what the call wrote nor what it held before, so nothing was
    /// written: the diff from `current` to `before` is what Restore anyway would do. The texts
    /// show CRLF line breaks as `\n`.
    Changed {
        path: String,
        /// What the file holds now; `None` when it does not exist.
        current: Option<String>,
        /// What it held before the call; `None` when the call created it, so Restore anyway
        /// deletes it.
        before: Option<String>,
        /// A later call of this conversation wrote the file too, so reverting the later changes
        /// first, newest first, may bring it back to what this call wrote.
        later: bool,
    },
    Failed {
        reason: AiRevertFailure,
        detail: Option<String>,
    },
}

impl fmt::Debug for AiFileRevert {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reverted { wrote } => f.debug_struct("Reverted").field("wrote", wrote).finish(),
            Self::Changed { current, later, .. } => f
                .debug_struct("Changed")
                .field("exists", &current.is_some())
                .field("later", later)
                .finish_non_exhaustive(),
            Self::Failed { reason, .. } => f
                .debug_struct("Failed")
                .field("reason", reason)
                .finish_non_exhaustive(),
        }
    }
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
    /// The tab's quick-connect target when it has no saved host (HOST-12): the next message
    /// moves the conversation off its saved host, and the request names the target instead.
    pub target: Option<QuickTarget>,
    /// A local terminal tab's computer and shell (§7.5), or on the home tab the computer a
    /// conversation recorded, while there is neither a host nor a target. The next message moves
    /// the conversation to that computer.
    pub local: Option<AiLocalTab>,
    /// A connected terminal tab is attached; `false` offers no terminal tools (AI-09).
    pub tab: bool,
    /// The tab's SSH session while it is connected, whose server's identification string the
    /// system prompt states (§13.1).
    pub session_id: Option<String>,
    /// MCP servers switched off for this conversation (AI-30, P2).
    pub disabled_mcp_servers: Vec<String>,
    /// The conversation's permission mode on this device (AI-16). The frontend applies it to each
    /// call; a message sent in another mode than the model was last told of starts with a
    /// `permission_mode` note (§13.5).
    pub permission_mode: AiPermissionMode,
    /// The attached terminal's screen as the panel read it for this message (AI-46), while the
    /// tab is connected and **Send Terminal Screen** is on for the conversation. Only a stored
    /// message (and an idle side question) shows it, in a `terminal_state` note.
    pub screen: Option<AiScreen>,
}

/// A local terminal tab (§7.5) in a turn's context.
#[derive(Debug, Clone, Deserialize, Type, PartialEq, Eq)]
pub struct AiLocalTab {
    /// The computer's name: this one's (`AppInfo.device_name`) for a local tab, or the one a home
    /// tab conversation recorded.
    pub device: String,
    /// The tab's shell, for a local tab: its name as the shell picker shows it in English, and
    /// its kind, which decides how the tools run and what the system prompt says of them.
    pub shell: Option<String>,
    pub kind: Option<LocalShellKind>,
}

/// What a `terminal_state` note shows (AI-46, §13.3). Its `Debug` leaves the text out, like
/// every log (§13.1).
#[derive(Clone, Deserialize, Type, PartialEq, Eq)]
pub struct AiScreen {
    /// The screen's last lines, or the whole screen while `alternate`.
    pub text: String,
    /// A full-screen program (vim, less, …) uses the alternate screen.
    pub alternate: bool,
}

impl fmt::Debug for AiScreen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiScreen")
            .field(
                "text",
                &format_args!("<{} chars>", self.text.chars().count()),
            )
            .field("alternate", &self.alternate)
            .finish()
    }
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
    /// The response had neither text nor tool calls, such as a reply of reasoning only (§13.1).
    Empty,
    Stopped,
    Error,
}

/// Streamed on the channel of `ai_send` / `ai_retry` for the whole turn (§13.1).
#[derive(Debug, Clone, Serialize, Type, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AiTurnEvent {
    /// The turn of a new conversation is registered under this id, which the conversation is
    /// stored under with its message. Sent before the message waits for the MCP servers (AI-45),
    /// so a stop can name the turn before `ai_send` returns.
    Opened {
        conversation_id: String,
    },
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
    /// The tool of an `ai_tool_run` call finished, and closed its exec channel if it had one; its
    /// result is stored once every earlier call of the response has one (§13.1). The panel counts
    /// the channel as free from now on.
    ToolRan {
        tool_call_id: String,
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

/// AI-42: a side question about a conversation, asked without stopping its turn.
#[derive(Clone, Deserialize, Type)]
pub struct AiSideQuestionInput {
    pub conversation_id: String,
    /// The panel's number for the question, which `ai_side_stop` names, so that a stop meant for
    /// it never stops a newer one.
    pub question_id: u32,
    /// The question, after the attachment blocks the input area adds (spec §13.3).
    pub text: String,
    /// What the next message would be sent with; a running turn's own context wins.
    pub context: AiTurnContext,
    /// The call whose approval card waits (AI-17), so its result in the request says so.
    pub approval_call_id: Option<String>,
}

impl fmt::Debug for AiSideQuestionInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiSideQuestionInput")
            .field("conversation_id", &self.conversation_id)
            .field("question_id", &self.question_id)
            .field(
                "text",
                &format_args!("<{} chars>", self.text.chars().count()),
            )
            .field("context", &self.context)
            .field("approval_call_id", &self.approval_call_id)
            .finish()
    }
}

/// Streamed on the channel of `ai_side_question` while the answer comes (AI-42).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AiSideEvent {
    Reasoning {
        delta: String,
    },
    Text {
        delta: String,
    },
    /// The answer called tools, so it was dropped and the question went again without tools:
    /// what streamed before is discarded.
    Restarted,
}

/// The answer to a side question (AI-42), which nothing stores.
#[derive(Debug, Clone, Serialize, Type)]
pub struct AiSideAnswer {
    pub text: String,
    pub finish: AiFinish,
}

/// A result the frontend produced: `read_terminal`, `send_input`, or a rejection (AI-17).
#[derive(Clone, Deserialize, Type)]
pub struct AiToolResultInput {
    pub status: AiToolStatus,
    pub content: String,
    /// AI-17 Edit: the arguments the user changed the call to, so the result tells the model.
    pub edited_arguments: Option<String>,
    /// AI-47: Auto decided the call (it ran on the judge's verdict, or its approval card showed
    /// one), so the result keeps the verdict.
    #[serde(default)]
    pub judged: bool,
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
            .field("judged", &self.judged)
            .finish()
    }
}

/// The change an `edit_file` or `write_file` call would make, for its approval card (AI-39,
/// AI-40). The texts show CRLF line breaks as `\n`; `crlf` says the write keeps them as CRLF.
#[derive(Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiFilePreview {
    /// The path the call names.
    pub path: String,
    /// The file as Hatoba read it, or `None` when it does not exist (the call creates it) or
    /// `error` is set.
    pub before: Option<String>,
    /// The file after the call, or `None` when `error` is set.
    pub after: Option<String>,
    /// Why the call cannot apply: the error result running it would give the model.
    pub error: Option<String>,
    /// Most of the file's line breaks are CRLF, and the write keeps them.
    pub crlf: bool,
}

impl fmt::Debug for AiFilePreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let size = |t: &Option<String>| t.as_ref().map(|t| t.chars().count());
        f.debug_struct("AiFilePreview")
            .field("before_chars", &size(&self.before))
            .field("after_chars", &size(&self.after))
            .field("error", &self.error.is_some())
            .field("crlf", &self.crlf)
            .finish_non_exhaustive()
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

/// A memory of the AI assistant (AI-53): a global one, with neither `host_id` nor `local`, or a
/// host memory of a saved host or of a computer's local terminal.
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiMemoryView {
    pub id: String,
    pub name: String,
    pub content: String,
    pub host_id: Option<String>,
    /// The computer's name, while `host_id` is `None`.
    pub local: Option<String>,
    pub updated_at: i64,
}

/// A memory to save from Settings → AI (AI-53), in the scope `host_id` and `local` name, as in
/// [`AiMemoryView`].
#[derive(Debug, Clone, Deserialize, Type)]
pub struct AiMemoryInput {
    /// `None` creates a memory.
    pub id: Option<String>,
    pub name: String,
    pub content: String,
    pub host_id: Option<String>,
    pub local: Option<String>,
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

/// One entry of a directory on the tab's host, for the panel's `@` menu (AI-51).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiHostEntry {
    pub name: String,
    /// A directory, or a symbolic link to one.
    pub dir: bool,
}

/// Why a path on the tab's host cannot be listed or attached (AI-51).
#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiHostRefusal {
    /// The tab is not connected.
    Disconnected,
    NotFound,
    /// Read: a directory. List: not a directory.
    WrongKind,
    /// A device, FIFO, socket or a symbolic link to nothing.
    NotRegular,
    PermissionDenied,
    /// Over 256 KB.
    TooLarge,
    /// NUL bytes or invalid UTF-8.
    Binary,
    /// The command failed or did not finish within 30 seconds.
    Failed,
}

/// A directory on the tab's host (AI-51): its entries, or why it cannot be listed.
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AiHostListing {
    Ok {
        /// The path as the host resolved it: absolute, with `~/` and a relative path taken from
        /// the home directory.
        path: String,
        /// Sorted by name.
        entries: Vec<AiHostEntry>,
        /// Entries were left out: the listing passed its size limit.
        cut: bool,
    },
    Refused {
        reason: AiHostRefusal,
    },
}

/// A text file on the tab's host for an `@` mention (AI-51), or why it cannot be attached.
#[derive(Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AiHostFile {
    Ok {
        /// The path as the host resolved it.
        path: String,
        text: String,
    },
    Refused {
        reason: AiHostRefusal,
    },
}

impl fmt::Debug for AiHostFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok { text, .. } => f
                .debug_struct("Ok")
                .field("text", &format_args!("<{} bytes>", text.len()))
                .finish_non_exhaustive(),
            Self::Refused { reason } => f.debug_struct("Refused").field("reason", reason).finish(),
        }
    }
}

/// An enabled skill the panel's `/` menu offers as a command (AI-50), the built-in one included.
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiSkillCommand {
    pub name: String,
    pub description: String,
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
    /// The prompts from the last successful listing, which the panel's `/` menu offers (AI-50).
    pub prompts: Vec<McpPromptView>,
    /// The resources from the last successful listing, which the panel's `@` menu offers (AI-51).
    pub resources: Vec<McpResourceView>,
}

impl fmt::Debug for McpServerStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpServerStatus")
            .field("server_id", &self.server_id)
            .field("state", &self.state)
            .field("error", &self.error.is_some())
            .field("stderr", &format_args!("<{} lines>", self.stderr.len()))
            .field("tools", &self.tools.len())
            .field("prompts", &self.prompts.len())
            .field("resources", &self.resources.len())
            .finish()
    }
}

/// A server's prompt (AI-50): the `/` menu offers it as `/mcp__<server>__<prompt>`, which the
/// panel names across the servers a conversation uses, as tool names are (AI-30).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpPromptView {
    /// The prompt's name on its server.
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    pub arguments: Vec<McpPromptArgumentView>,
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpPromptArgumentView {
    pub name: String,
    pub description: String,
    pub required: bool,
}

/// A server's resource (AI-51), which the `@` menu offers.
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpResourceView {
    pub uri: String,
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    pub mime_type: Option<String>,
}

/// A value for one argument of an MCP prompt (AI-50).
#[derive(Clone, Deserialize, Type)]
pub struct McpPromptArgumentInput {
    pub name: String,
    pub value: String,
}

impl fmt::Debug for McpPromptArgumentInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpPromptArgumentInput")
            .field("name", &self.name)
            .field(
                "value",
                &format_args!("<{} chars>", self.value.chars().count()),
            )
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

/// What the approval card of a `call_mcp_tool` call needs (AI-45).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct McpProxyInfo {
    /// The tool the call names, as `mcp_tool_info` gives a direct call's; null when the
    /// conversation has no tool of that name or its server was deleted.
    pub tool: Option<McpToolInfo>,
    /// False when the call cannot run: it then gets its error result without asking.
    pub runs: bool,
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
    fn the_update_check_is_on_unless_turned_off() {
        let prefs = LocalPrefs::from_stored(r#"{"language":"ja","host_probe":false}"#);
        assert!(prefs.auto_update_check);
        assert!(!prefs.host_probe);
        let prefs = LocalPrefs::from_stored(r#"{"auto_update_check":false}"#);
        assert!(!prefs.auto_update_check);
    }

    #[test]
    fn device_security_prefs_have_defaults() {
        let prefs = LocalPrefs::from_stored(r#"{"language":"ja"}"#);
        assert_eq!(prefs.auto_lock_minutes, 15);
        assert!(prefs.lock_on_sleep && !prefs.lock_disconnects_sessions);
        assert_eq!(prefs.clipboard_clear_secs, 90);
        let prefs = LocalPrefs::from_stored(
            r#"{"auto_lock_minutes":0,"lock_on_sleep":false,"clipboard_clear_secs":0}"#,
        );
        assert_eq!(
            (prefs.auto_lock_minutes, prefs.clipboard_clear_secs),
            (0, 0)
        );
        assert!(!prefs.lock_on_sleep);
    }

    #[test]
    fn ai_prefs_default_to_manual_and_unknown_values_fall_back() {
        let prefs = LocalPrefs::from_stored(r#"{"language":"ja"}"#);
        assert_eq!(prefs.ai_permission_mode, AiPermissionMode::Manual);
        assert!(!prefs.ai_bypass_confirmed && !prefs.ai_panel_open);
        assert!(prefs.ai_notifications);
        assert_eq!(prefs.ai_panel_width, 380);

        // The tool call limit an older version stored is ignored.
        let prefs = LocalPrefs::from_stored(
            r#"{"ai_permission_mode":"bypass","ai_bypass_confirmed":true,"ai_tool_call_limit":40,
                "ai_panel_open":true,"ai_panel_width":520,"ai_notifications":false}"#,
        );
        assert_eq!(prefs.ai_permission_mode, AiPermissionMode::Bypass);
        assert!(prefs.ai_bypass_confirmed && prefs.ai_panel_open);
        assert!(!prefs.ai_notifications);
        assert_eq!(prefs.ai_panel_width, 520);

        // AI-47: Auto is a mode of its own.
        let prefs = LocalPrefs::from_stored(r#"{"ai_permission_mode":"auto"}"#);
        assert_eq!(prefs.ai_permission_mode, AiPermissionMode::Auto);

        // A mode a newer build added, or a broken value, is read as the default.
        let prefs = LocalPrefs::from_stored(
            r#"{"ai_permission_mode":"ask_twice","ai_panel_width":-1,"host_probe":false}"#,
        );
        assert_eq!(prefs.ai_permission_mode, AiPermissionMode::Manual);
        assert_eq!(prefs.ai_panel_width, 380);
        assert!(!prefs.host_probe);
    }
}
