//! Data transfer objects exchanged with the WebView (spec §10).
//!
//! tauri-specta exports them, with their doc comments, to `apps/desktop/src/ipc/bindings.ts`, the
//! frontend's only definition of them.
//! **No DTO carries a secret**: hosts expose `has_password`, keys expose only public material.
//! Inputs may carry secrets one way (WebView → Rust), e.g. a new host password; they are never
//! echoed back.

use serde::{Deserialize, Serialize};
use specta::Type;

pub use hatoba_assistant::dto::*;

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
    /// Consecutive failed unlock attempts (SEC-06).
    pub failed_attempts: u32,
    /// While throttled, the next attempt is allowed at this time.
    pub retry_at: Option<i64>,
    pub sync_kind: SyncKind,
    /// Windows Hello or Touch ID is available on this device.
    pub biometric_available: bool,
    /// Biometric unlock is enrolled for this vault.
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
    /// A password is saved (HOST-08: it can be replaced but never read back).
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
    /// Device-local, never synced (HOST-06).
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
    /// `None` creates a new host.
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

/// Where a host import reads from (SSH-11, SSH-12, SSH-15).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportSource {
    SshConfig,
    Xshell,
    Finalshell,
    Mobaxterm,
    Putty,
    Securecrt,
    Tabby,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ImportPreview {
    /// The file, folder or registry key the hosts were read from; `None` when nothing was found
    /// where the client keeps its sessions.
    pub location: Option<String>,
    /// The client encrypted the file, so nothing could be read from it.
    pub encrypted: bool,
    pub candidates: Vec<ImportCandidate>,
}

/// A host an import can create.
#[derive(Debug, Clone, Serialize, Type)]
pub struct ImportCandidate {
    /// Identifies it within the source, for `hosts_import`.
    pub id: String,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    /// The folder it is in, its names joined by ` / `.
    pub folder: Option<String>,
    /// The private key file an import with keys reads: the first one that exists, otherwise the
    /// first one listed.
    pub identity_file: Option<String>,
    /// Whether `identity_file` exists, so that importing keys would read it.
    pub identity_file_found: bool,
    /// The client keeps the private key itself, so it is not imported; the key's name there, or
    /// empty when the session does not name it.
    pub kept_key: Option<String>,
    /// The jump host: a `ProxyJump` value, another session's name, or gateways.
    pub jump: Option<String>,
    /// The proxy it imports, such as `SOCKS5 127.0.0.1:1080`.
    pub proxy: Option<String>,
    /// A proxy setting that is not imported, such as `ProxyCommand …`: the host connects
    /// without it.
    pub skipped_proxy: Option<String>,
    /// A host with the same name exists.
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
    /// A password is saved (as HOST-08: it can be replaced but never read back).
    pub has_password: bool,
    /// Hosts that name this proxy (not those that use it as the device default).
    pub host_ids: Vec<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct ProxyInput {
    /// `None` creates a new proxy.
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
    /// IDs of the hosts that use this key (KEY-03, KEY-05).
    pub used_by: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct KeyImportInput {
    pub name: String,
    /// Pasted private key text; ignored when `path` is set.
    pub private_key: Option<String>,
    /// A file chosen in the native dialog, which Rust reads.
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
    /// For a passphrase-protected key whose passphrase isn't saved.
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
    /// `None` for a quick-connect session (HOST-12).
    pub host_id: Option<String>,
    pub state: SessionState,
    /// The connection's round-trip time, once connected.
    pub latency_ms: Option<u32>,
    pub error: Option<crate::error::AppError>,
    /// The remote exit status of a clean `disconnected`.
    pub exit_status: Option<u32>,
}

// ───────────────────────── Local terminals ─────────────────────────

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
    /// `new`: the first connection (TOFU). `changed`: the fingerprint differs from the saved one,
    /// and the connection is blocked (SSH-04).
    pub kind: HostKeyPromptKind,
    pub known_fingerprint: Option<String>,
    pub known_key_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct AuthPromptField {
    pub prompt: String,
    pub echo: bool,
}

/// A keyboard-interactive (2FA) prompt (SSH-08).
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
    /// The server runs neither Linux nor macOS; the last event. `system` is its name, such as
    /// `FreeBSD`, or empty when unknown.
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
    /// The server runs neither Linux nor macOS, so only its online status shows. `system` is
    /// its name, or empty when unknown.
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
    /// Such as `drwxr-xr-x`.
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

/// SFTP-05: where the copy of a file edited in a local editor stands.
#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemoteEditState {
    /// The server has the copy's content.
    Saved,
    /// A save waits to upload: until the file settles, or until the tab connects again.
    Pending,
    Uploading,
    /// The last upload failed (`error`), or was cancelled (no `error`).
    Failed,
    /// The file changed on the server since it was downloaded or uploaded.
    Conflict,
}

/// SFTP-05: a remote file being edited in a local editor.
#[derive(Debug, Clone, Serialize, Type)]
pub struct RemoteEditView {
    pub edit_id: String,
    /// The terminal tab it was opened from, whose connection it uploads over.
    pub tab_id: String,
    /// The name it was opened as, which the copy keeps.
    pub name: String,
    /// The file written back: the path opened, or a symlink's target.
    pub remote_path: String,
    pub state: RemoteEditState,
    pub error: Option<String>,
    /// The vault locked before the last save uploaded: editing ended, and the copy waits until
    /// the user uploads or discards it.
    pub held: bool,
}

/// `edit` is `None` once editing has ended and the copy is gone.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "sftp://edit")]
pub struct RemoteEditEvent {
    pub edit_id: String,
    pub edit: Option<RemoteEditView>,
}

/// Hatoba is quitting with files being edited: the WebView asks about saves that did not upload,
/// then quits with `sftp_edit_quit`.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "sftp://quit")]
pub struct RemoteEditQuitEvent;

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
    /// The Worker's host, or `api.cloudflare.com` in D1 direct mode.
    pub endpoint: Option<String>,
    /// D1 direct mode: the database's ID.
    pub database: Option<String>,
    pub state: SyncState,
    pub last_synced_at: Option<i64>,
    /// Local changes not pushed yet.
    pub pending: u32,
    /// Automatically resolved conflicts not reviewed yet (§6.4).
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
    /// The remote already holds a vault.
    pub initialized: bool,
    pub version: Option<String>,
    pub latency_ms: Option<u32>,
    /// D1 direct mode: the databases the token can see, to pick from once the token is verified.
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
    /// A field key the interface translates, such as `port`, `jump_host`, `tags`, `name` or
    /// `state`.
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
    /// Only the fields that differ; secrets are never included.
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
    /// WIN-05: PuTTY-style right click, or a context menu.
    pub right_click: RightClick,
    /// TERM-04: ask before pasting text with line breaks.
    pub confirm_multiline_paste: bool,
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
            auto_lock_minutes: 15,
            lock_on_sleep: true,
            lock_disconnects_sessions: false,
            clipboard_clear_secs: 90,
            hosts_view: HostsView::List,
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

// ───────────────────────── Port forwarding (FWD-01..05) ─────────────────────────

/// Which way a forward goes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ForwardKind {
    /// `ssh -L`: listens on this computer, and the server connects to the destination.
    Local,
    /// `ssh -R`: listens on the server, and this computer connects to the destination.
    Remote,
    /// `ssh -D`: a SOCKS proxy on this computer, whose connections the server opens.
    Dynamic,
}

impl From<hatoba_core::model::ForwardKind> for ForwardKind {
    fn from(kind: hatoba_core::model::ForwardKind) -> Self {
        use hatoba_core::model::ForwardKind as K;
        match kind {
            K::Local => Self::Local,
            K::Remote => Self::Remote,
            K::Dynamic => Self::Dynamic,
        }
    }
}

impl From<ForwardKind> for hatoba_core::model::ForwardKind {
    fn from(kind: ForwardKind) -> Self {
        match kind {
            ForwardKind::Local => Self::Local,
            ForwardKind::Remote => Self::Remote,
            ForwardKind::Dynamic => Self::Dynamic,
        }
    }
}

/// A saved forward. The bind address and port are on this computer, or on the server for a
/// remote forward.
#[derive(Debug, Clone, Serialize, Type)]
pub struct ForwardView {
    pub id: String,
    pub host_id: String,
    /// Empty for a forward named by its rule.
    pub name: String,
    pub kind: ForwardKind,
    pub bind_address: String,
    /// 0 picks a free port (on the server for a remote forward).
    pub bind_port: u16,
    /// As the server reaches it, or as this computer does for a remote forward; empty and 0 for
    /// a dynamic one.
    pub dest_host: String,
    pub dest_port: u16,
    /// FWD-02: starts whenever Hatoba connects to the host.
    pub auto_start: bool,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct ForwardInput {
    pub id: Option<String>,
    pub host_id: String,
    /// Trimmed; empty for none.
    pub name: String,
    pub kind: ForwardKind,
    pub bind_address: String,
    pub bind_port: u16,
    /// Ignored for a dynamic forward.
    pub dest_host: String,
    pub dest_port: u16,
    pub auto_start: bool,
}

/// One-off credentials for the connection a forward's start opens (SSH-03): used once, never
/// stored.
#[derive(Clone, Default, Deserialize, Type)]
pub struct ForwardOptions {
    pub password: Option<String>,
    pub passphrase: Option<String>,
}

/// A forward running on its host's connection.
#[derive(Debug, Clone, Serialize, Type)]
pub struct RunningForwardView {
    pub forward_id: String,
    /// Its rule's name, kept current while it runs; empty for none.
    pub name: String,
    pub kind: ForwardKind,
    /// The port it actually listens on (differs from `bind_port` when that was 0), on the server
    /// for a remote forward.
    pub port: u16,
    /// Where it is reached: from this computer, `localhost:<port>` or the address it listens on
    /// when that is one interface's; on the server for a remote forward, the address it listens
    /// on there.
    pub address: String,
    /// `host:port` it leads to, as the server reaches it, or as this computer does for a remote
    /// forward; empty for a dynamic one.
    pub dest: String,
}

/// A forward that failed to start on its host's connection, until it starts or stops: a port in
/// use, say.
#[derive(Debug, Clone, Serialize, Type)]
pub struct FailedForwardView {
    pub forward_id: String,
    pub error: crate::error::AppError,
}

/// SSH-16: a saved host's open connection, which its terminal tabs and forwards share.
#[derive(Debug, Clone, Serialize, Type)]
pub struct HostConnectionView {
    pub host_id: String,
    /// The host's name when it connected.
    pub host_name: String,
    /// By port.
    pub forwards: Vec<RunningForwardView>,
    pub failed: Vec<FailedForwardView>,
}

/// SSH-16: every saved host's open connection, sent whenever one opens or closes or its forwards
/// change. `version` grows with each change, so a list read earlier never replaces a newer one.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "ssh://connections")]
pub struct ConnectionsEvent {
    pub version: u64,
    pub connections: Vec<HostConnectionView>,
}

/// FWD-05: a host's connection ended on its own with forwards on it, which stopped with it.
/// Closing it with nothing left on it, the lock (SEC-03) and quitting send nothing.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "ssh://forwards-lost")]
pub struct ForwardsLostEvent {
    pub host_id: String,
    pub host_name: String,
    pub error: crate::error::AppError,
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
