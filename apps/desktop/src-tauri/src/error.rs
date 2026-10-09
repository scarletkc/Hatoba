//! The single error type every command returns. Serialized to the WebView as `AppError`
//! (see `src/ipc/types.ts`). `detail` is short technical English and never contains secrets.

use serde::Serialize;
use specta::Type;

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Locked,
    NotInitialized,
    AlreadyInitialized,
    WrongPassword,
    WrongRecoveryCode,
    Throttled,
    NotFound,
    InvalidInput,
    KeyParse,
    Ssh,
    Sftp,
    Sync,
    SyncAuth,
    SyncOffline,
    RemoteInitialized,
    RemoteNotInitialized,
    /// In-app deployment (§6.7): Cloudflare rejected the API token.
    CloudflareToken,
    /// The API token lacks the permission in `permission`.
    CloudflarePermission,
    /// Any other Cloudflare API failure; `cf_code` has Cloudflare's code when it sent one.
    Cloudflare,
    SubdomainRequired,
    SubdomainUnavailable,
    WorkerNameTaken,
    /// An upgrade found no Hatoba Worker with a vault under the name.
    WorkerNotFound,
    /// An upgrade found a Worker newer than the one this build deploys.
    WorkerNewer,
    /// This build does not embed the Worker, so it cannot deploy it.
    NoWorkerBundle,
    /// AI assistant (§13): a model provider or search provider failed. `http_status` has the
    /// status it answered with, when it answered; `detail` is its own message.
    Ai,
    /// A downloaded update does not carry a valid signature from the release key (spec §11).
    UpdateSignature,
    Cancelled,
    Io,
    Internal,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SshErrorKind {
    Dns,
    Refused,
    Timeout,
    Unreachable,
    AuthFailed,
    HostKeyRejected,
    KeyParse,
    Disconnected,
    Protocol,
    Io,
    Channel,
    Sftp,
    Cancelled,
    /// SSH-13: the proxy could not be reached.
    ProxyUnreachable,
    /// SSH-13: the proxy wants a username and password, or did not accept them.
    ProxyAuth,
    /// SSH-13: the proxy did not open the connection to the server.
    Proxy,
    /// SSH-13: the host or the device default names a proxy that was deleted.
    ProxyMissing,
    Other,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CloudflarePermission {
    WorkersScripts,
    D1,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeyParseErrorKind {
    UnsupportedFormat,
    PassphraseRequired,
    WrongPassphrase,
    UnsupportedAlgorithm,
    Invalid,
}

#[derive(Debug, Clone, Serialize, Type, thiserror::Error)]
#[error("{code:?}: {detail}")]
pub struct AppError {
    pub code: ErrorCode,
    pub detail: String,
    pub retry_at: Option<i64>,
    pub field: Option<String>,
    pub ssh_kind: Option<SshErrorKind>,
    pub key_kind: Option<KeyParseErrorKind>,
    pub permission: Option<CloudflarePermission>,
    pub cf_code: Option<u32>,
    /// `ai`: the HTTP status the provider answered with.
    pub http_status: Option<u16>,
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn new(code: ErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
            retry_at: None,
            field: None,
            ssh_kind: None,
            key_kind: None,
            permission: None,
            cf_code: None,
            http_status: None,
        }
    }

    pub fn locked() -> Self {
        Self::new(ErrorCode::Locked, "vault is locked")
    }

    pub fn not_found(what: &str) -> Self {
        Self::new(ErrorCode::NotFound, format!("{what} not found"))
    }

    pub fn invalid(field: &str, detail: impl Into<String>) -> Self {
        Self {
            field: Some(field.to_owned()),
            ..Self::new(ErrorCode::InvalidInput, detail)
        }
    }

    pub fn internal(detail: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, detail)
    }

    pub fn io(err: impl std::fmt::Display) -> Self {
        Self::new(ErrorCode::Io, err.to_string())
    }

    pub fn ssh(kind: SshErrorKind, detail: impl Into<String>) -> Self {
        Self {
            ssh_kind: Some(kind),
            ..Self::new(ErrorCode::Ssh, detail)
        }
    }

    pub fn key(kind: KeyParseErrorKind, detail: impl Into<String>) -> Self {
        Self {
            key_kind: Some(kind),
            ..Self::new(ErrorCode::KeyParse, detail)
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::io(e)
    }
}

impl From<tauri::Error> for AppError {
    fn from(e: tauri::Error) -> Self {
        Self::internal(e.to_string())
    }
}

impl From<hatoba_core::Error> for AppError {
    fn from(e: hatoba_core::Error) -> Self {
        use hatoba_core::Error as E;
        let detail = e.to_string();
        match e {
            E::Locked => Self::locked(),
            E::VaultNotInitialized => Self::new(ErrorCode::NotInitialized, detail),
            E::VaultAlreadyInitialized => Self::new(ErrorCode::AlreadyInitialized, detail),
            E::WrongPassword => Self::new(ErrorCode::WrongPassword, detail),
            E::WrongRecoveryCode => Self::new(ErrorCode::WrongRecoveryCode, detail),
            E::Throttled { retry_at } => Self {
                retry_at: Some(retry_at),
                ..Self::new(ErrorCode::Throttled, detail)
            },
            E::ItemNotFound(_) => Self::new(ErrorCode::NotFound, detail),
            E::InvalidItem(_) => Self::new(ErrorCode::InvalidInput, detail),
            E::EmptyPassword => Self::invalid("password", detail),
            E::Io(_) => Self::new(ErrorCode::Io, detail),
            E::Offline | E::RateLimited { .. } => Self::new(ErrorCode::SyncOffline, detail),
            E::Unauthorized | E::InvalidSetupToken | E::D1Permission => {
                Self::new(ErrorCode::SyncAuth, detail)
            }
            E::RemoteNotInitialized => Self::new(ErrorCode::RemoteNotInitialized, detail),
            E::RemoteInitialized => Self::new(ErrorCode::RemoteInitialized, detail),
            E::InvalidUrl(_) => Self::invalid("url", detail),
            E::CloudflareToken => Self::new(ErrorCode::CloudflareToken, detail),
            E::CloudflarePermission(p) => Self {
                permission: Some(match p {
                    hatoba_core::sync::deploy::Permission::WorkersScripts => {
                        CloudflarePermission::WorkersScripts
                    }
                    hatoba_core::sync::deploy::Permission::D1 => CloudflarePermission::D1,
                }),
                ..Self::new(ErrorCode::CloudflarePermission, detail)
            },
            E::Cloudflare { code, .. } => Self {
                cf_code: code,
                ..Self::new(ErrorCode::Cloudflare, detail)
            },
            E::SubdomainRequired => Self::new(ErrorCode::SubdomainRequired, detail),
            E::SubdomainUnavailable => Self::new(ErrorCode::SubdomainUnavailable, detail),
            E::WorkerNameTaken => Self::new(ErrorCode::WorkerNameTaken, detail),
            E::WorkerNotFound => Self::new(ErrorCode::WorkerNotFound, detail),
            E::WorkerNewer => Self::new(ErrorCode::WorkerNewer, detail),
            E::SyncNotConfigured
            | E::Server(_)
            | E::Protocol(_)
            | E::Unsupported
            | E::VaultMismatch => Self::new(ErrorCode::Sync, detail),
            _ => Self::internal(detail),
        }
    }
}

/// A provider or search provider failure (§13). The adapters' messages never carry a key, a URL
/// or request content: an HTTP error keeps the provider's own message in `detail`.
impl From<hatoba_ai::AiError> for AppError {
    fn from(e: hatoba_ai::AiError) -> Self {
        use hatoba_ai::AiError as E;
        match e {
            E::Cancelled => Self::new(ErrorCode::Cancelled, "cancelled"),
            E::InvalidUrl(message) => Self::invalid("base_url", message),
            E::Http { status, message } => Self {
                http_status: Some(status),
                ..Self::new(ErrorCode::Ai, message)
            },
            E::Config(message)
            | E::Network(message)
            | E::Protocol(message)
            | E::Blocked(message)
            | E::UnsupportedContentType(message) => Self::new(ErrorCode::Ai, message),
        }
    }
}

impl From<hatoba_ssh::SshError> for AppError {
    fn from(e: hatoba_ssh::SshError) -> Self {
        use hatoba_ssh::SshErrorKind as K;
        let kind = match e.kind {
            K::Dns => SshErrorKind::Dns,
            K::Refused => SshErrorKind::Refused,
            K::Timeout => SshErrorKind::Timeout,
            K::Unreachable => SshErrorKind::Unreachable,
            K::AuthFailed => SshErrorKind::AuthFailed,
            K::HostKeyRejected => SshErrorKind::HostKeyRejected,
            K::KeyParse => SshErrorKind::KeyParse,
            K::Disconnected => SshErrorKind::Disconnected,
            K::Protocol => SshErrorKind::Protocol,
            K::Io => SshErrorKind::Io,
            K::Channel => SshErrorKind::Channel,
            K::Sftp => SshErrorKind::Sftp,
            K::Cancelled => SshErrorKind::Cancelled,
            K::ProxyUnreachable => SshErrorKind::ProxyUnreachable,
            K::ProxyAuth => SshErrorKind::ProxyAuth,
            K::Proxy => SshErrorKind::Proxy,
            _ => SshErrorKind::Other,
        };
        let code = match kind {
            SshErrorKind::Sftp => ErrorCode::Sftp,
            SshErrorKind::Cancelled => ErrorCode::Cancelled,
            _ => ErrorCode::Ssh,
        };
        Self {
            ssh_kind: Some(kind),
            ..Self::new(code, e.message)
        }
    }
}

impl From<hatoba_ssh::KeyError> for AppError {
    fn from(e: hatoba_ssh::KeyError) -> Self {
        use hatoba_ssh::KeyError as K;
        let kind = match &e {
            K::UnsupportedFormat => KeyParseErrorKind::UnsupportedFormat,
            K::PassphraseRequired => KeyParseErrorKind::PassphraseRequired,
            K::WrongPassphrase => KeyParseErrorKind::WrongPassphrase,
            K::UnsupportedAlgorithm(_) => KeyParseErrorKind::UnsupportedAlgorithm,
            K::Invalid(_) => KeyParseErrorKind::Invalid,
        };
        Self::key(kind, e.to_string())
    }
}
