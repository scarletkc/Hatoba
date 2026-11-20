use serde::{Deserialize, Serialize};
use specta::Type;

use super::SyncKind;

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
    /// The place in the hosts page's custom order (HOST-16), shared with Telnet hosts: lower comes
    /// first, and 0 (no place yet) after every place.
    pub sort: i64,
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

/// A Telnet host (TELNET-01). Its password never leaves Rust: only whether one is saved.
#[derive(Debug, Clone, Serialize, Type)]
pub struct TelnetHostView {
    pub id: String,
    pub name: String,
    pub address: String,
    pub port: u16,
    /// Typed at the login prompt; empty to type it by hand (TELNET-03).
    pub username: String,
    pub has_password: bool,
    pub group_id: Option<String>,
    pub tags: Vec<String>,
    pub favorite: bool,
    /// The place in the hosts page's custom order, as for SSH hosts (HOST-16).
    pub sort: i64,
    pub proxy_mode: ProxyMode,
    pub proxy_id: Option<String>,
    pub note: String,
    pub ai_notes: String,
    pub updated_at: i64,
    /// Device-local, as for SSH hosts (HOST-06).
    pub last_connected_at: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct TelnetHostInput {
    pub id: Option<String>,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    /// `None` keeps the saved password (HOST-08); empty removes it.
    pub password: Option<String>,
    pub group_id: Option<String>,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub proxy_mode: ProxyMode,
    /// Required when `proxy_mode` is `proxy`, ignored otherwise.
    pub proxy_id: Option<String>,
    pub note: String,
    /// At most 16 KB of UTF-8 (AI-37).
    pub ai_notes: String,
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
