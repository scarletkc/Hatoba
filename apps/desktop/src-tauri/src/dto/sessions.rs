use serde::{Deserialize, Serialize};
use specta::Type;

use super::LocalShellKind;

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
    /// The sudo password of the saved host `name`, for `hatoba exec --sudo` or `hatoba cp
    /// --sudo` (HOST-15), asked like a password; answered with one value.
    pub sudo: bool,
    /// The sudo password given before was wrong.
    pub wrong: bool,
    /// `user@host:port` of the hop that asks.
    pub target: String,
}

/// A message the server sent while signing in (`SSH_MSG_USERAUTH_BANNER`). The terminal tab
/// connecting shows it in its terminal, as `ssh` prints it, and while the server holds the login,
/// as Tailscale SSH's check mode does until the user signs in at its link, it shows in a dialog.
/// `auth_prompt_respond` with no answers cancels the connection; `ssh://auth-prompt-closed` says
/// the server answered, which closes the dialog.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "ssh://auth-banner")]
pub struct AuthBanner {
    pub request_id: String,
    /// The terminal tab's session; `None` for a connection without a terminal, such as a port
    /// forward's (FWD-05).
    pub session_id: Option<String>,
    /// `user@host:port` of the hop that sent it.
    pub target: String,
    /// Lines end with `\n`; the only other control character is tab.
    pub text: String,
}

/// An `ssh://auth-prompt` that is no longer waited for, because the request that asked it ended
/// unanswered: its dialog closes.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "ssh://auth-prompt-closed")]
pub struct AuthPromptClosed {
    pub request_id: String,
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

/// What command suggestions take from the server of an SSH tab (TERM-16).
#[derive(Debug, Clone, Serialize, Type)]
pub struct ServerSuggestionsView {
    /// The shell's own history, newest first, each command once: the login shell's history first.
    pub history: Vec<String>,
    /// The names of the executables on its `PATH`, sorted, each once.
    pub commands: Vec<String>,
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
