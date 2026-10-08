//! The crate-wide error type.
//!
//! Error messages never contain secrets, item contents, envelopes or tokens (SEC-04): at most an
//! item id or a short, server-supplied error code.

use crate::sync::deploy::Permission;

/// Convenience alias used throughout the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Everything that can go wrong in `hatoba-core`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    // ---- vault state -----------------------------------------------------------------------
    /// The operation needs the vault key but the vault is locked.
    #[error("the vault is locked")]
    Locked,
    /// The local vault has not been created yet.
    #[error("the vault has not been created yet")]
    VaultNotInitialized,
    /// `create` / `restore` was called on a vault that already exists.
    #[error("the vault has already been created")]
    VaultAlreadyInitialized,
    /// The master password is wrong (the protected vault key failed to authenticate).
    #[error("wrong master password")]
    WrongPassword,
    /// The recovery code is malformed, has a bad checksum, or does not belong to this vault.
    #[error("wrong recovery code")]
    WrongRecoveryCode,
    /// Too many consecutive unlock failures (SEC-06); try again at `retry_at` (Unix ms).
    #[error("too many failed attempts, retry at {retry_at}")]
    Throttled {
        /// Unix epoch milliseconds after which another attempt is allowed.
        retry_at: i64,
    },
    /// The item id does not exist (or is deleted).
    #[error("item not found: {0}")]
    ItemNotFound(String),
    /// The item is invalid in a way the caller can fix (e.g. a fixed id used for the wrong type).
    #[error("invalid item: {0}")]
    InvalidItem(String),
    /// An empty password was supplied where a password is being set.
    #[error("the password must not be empty")]
    EmptyPassword,
    /// The vault key supplied for a biometric unlock does not match this vault.
    #[error("the supplied vault key does not match this vault")]
    WrongVaultKey,
    /// The remote vault is not the vault stored on this device.
    #[error("the remote vault is not the vault stored on this device")]
    VaultMismatch,
    /// The local vault has no recovery material to upload (it was restored from the cloud).
    #[error("recovery material is not available on this device")]
    RecoveryMaterialMissing,

    // ---- crypto & formats ------------------------------------------------------------------
    /// An envelope or key blob failed to authenticate / decrypt.
    #[error("decryption failed")]
    Decrypt,
    /// Malformed envelope, key blob, KDF parameters or similar persisted/wire format.
    #[error("invalid data: {0}")]
    Format(String),
    /// A newer on-disk / wire version than this build understands.
    #[error("unsupported version: {0}")]
    UnsupportedVersion(String),
    /// KDF parameters outside the range this build accepts.
    #[error("unacceptable KDF parameters: {0}")]
    WeakKdfParams(String),
    /// The OS random number generator failed.
    #[error("random number generator failure")]
    Random,

    // ---- local persistence -----------------------------------------------------------------
    /// SQLite error.
    #[error("database error: {0}")]
    Store(#[from] rusqlite::Error),
    /// JSON (de)serialisation error. Never includes the offending value.
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    /// Filesystem error.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// The shared vault mutex was poisoned by a panic on another thread.
    #[error("internal state poisoned")]
    Poisoned,

    // ---- sync ------------------------------------------------------------------------------
    /// Network unreachable, DNS failure or timeout. The UI shows "offline".
    #[error("offline: the sync server could not be reached")]
    Offline,
    /// The session token was rejected (401). The UI shows "authentication expired".
    #[error("sync authentication expired")]
    Unauthorized,
    /// The Setup Token was rejected by `POST /v1/setup`.
    #[error("invalid setup token")]
    InvalidSetupToken,
    /// The remote has no vault yet (`/v1/prelogin` answered 404).
    #[error("the remote has not been initialised")]
    RemoteNotInitialized,
    /// The remote already holds a vault (health says so, or setup answered 409). MVP flow C:
    /// the user should use "restore from cloud" on a new device instead.
    #[error("the remote already holds a vault")]
    RemoteInitialized,
    /// 429 from the Worker.
    #[error("rate limited by the sync server")]
    RateLimited {
        /// Seconds suggested by the `Retry-After` header, if any.
        retry_after_secs: Option<u64>,
    },
    /// Sync is not configured on this device.
    #[error("sync is not configured")]
    SyncNotConfigured,
    /// Any other server-side failure; the string is a short code, never user data.
    #[error("sync server error: {0}")]
    Server(String),
    /// The server answered with something that is not valid for the protocol.
    #[error("protocol error: {0}")]
    Protocol(String),
    /// The Worker URL / Cloudflare identifiers are malformed.
    #[error("invalid URL or identifier: {0}")]
    InvalidUrl(String),
    /// The Cloudflare API token cannot read/edit D1 (direct mode).
    #[error("the Cloudflare API token lacks D1 permissions")]
    D1Permission,
    /// The backend does not implement this operation (e.g. device list in D1 direct mode).
    #[error("not supported by this sync backend")]
    Unsupported,

    // ---- in-app deployment (§6.7) ----------------------------------------------------------
    /// Cloudflare rejected the API token: invalid, expired, or disabled.
    #[error("Cloudflare rejected the API token")]
    CloudflareToken,
    /// The API token lacks a permission the deployment needs.
    #[error("the Cloudflare API token lacks the {0} permission")]
    CloudflarePermission(Permission),
    /// Any other failed Cloudflare API call, with Cloudflare's first numeric error code.
    #[error("Cloudflare API error: HTTP {status}{}", code.map(|c| format!(", code {c}")).unwrap_or_default())]
    Cloudflare {
        /// HTTP status.
        status: u16,
        /// Cloudflare's numeric error code, if the response had one.
        code: Option<u32>,
    },
    /// The account has no workers.dev subdomain and none was chosen.
    #[error("the account has no workers.dev subdomain")]
    SubdomainRequired,
    /// Another account already has the chosen workers.dev subdomain.
    #[error("the workers.dev subdomain is not available")]
    SubdomainUnavailable,
    /// A Worker the app does not recognize already has the chosen name.
    #[error("another Worker already has this name")]
    WorkerNameTaken,
}

impl Error {
    /// A stable machine-readable code the UI layer can map to localized text.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Locked => "locked",
            Self::VaultNotInitialized => "vault_not_initialized",
            Self::VaultAlreadyInitialized => "vault_already_initialized",
            Self::WrongPassword => "wrong_password",
            Self::WrongRecoveryCode => "wrong_recovery_code",
            Self::Throttled { .. } => "throttled",
            Self::ItemNotFound(_) => "item_not_found",
            Self::InvalidItem(_) => "invalid_item",
            Self::EmptyPassword => "empty_password",
            Self::WrongVaultKey => "wrong_vault_key",
            Self::VaultMismatch => "vault_mismatch",
            Self::RecoveryMaterialMissing => "recovery_material_missing",
            Self::Decrypt => "decrypt_failed",
            Self::Format(_) => "invalid_format",
            Self::UnsupportedVersion(_) => "unsupported_version",
            Self::WeakKdfParams(_) => "weak_kdf_params",
            Self::Random => "random_failure",
            Self::Store(_) => "store",
            Self::Json(_) => "json",
            Self::Io(_) => "io",
            Self::Poisoned => "poisoned",
            Self::Offline => "offline",
            Self::Unauthorized => "unauthorized",
            Self::InvalidSetupToken => "invalid_setup_token",
            Self::RemoteNotInitialized => "remote_not_initialized",
            Self::RemoteInitialized => "remote_initialized",
            Self::RateLimited { .. } => "rate_limited",
            Self::SyncNotConfigured => "sync_not_configured",
            Self::Server(_) => "server",
            Self::Protocol(_) => "protocol",
            Self::InvalidUrl(_) => "invalid_url",
            Self::D1Permission => "d1_permission",
            Self::Unsupported => "unsupported",
            Self::CloudflareToken => "cloudflare_token",
            Self::CloudflarePermission(_) => "cloudflare_permission",
            Self::Cloudflare { .. } => "cloudflare",
            Self::SubdomainRequired => "subdomain_required",
            Self::SubdomainUnavailable => "subdomain_unavailable",
            Self::WorkerNameTaken => "worker_name_taken",
        }
    }

    /// Whether retrying later (with backoff) can succeed without user action.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::Offline | Self::RateLimited { .. } | Self::Server(_)
        )
    }
}
