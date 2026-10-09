//! The [`SyncBackend`] abstraction (spec §6.1) and the types that cross it.
//!
//! Two implementations exist: [`WorkerBackend`](super::worker::WorkerBackend) (HTTPS to the
//! user's Cloudflare Worker) and [`D1Backend`](super::d1::D1Backend) (Cloudflare's D1 REST API
//! directly). The sync engine only ever sees this trait. Everything that crosses it is already
//! encrypted except the handful of fields documented as metadata.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::crypto::Key32;
use crate::error::Result;

/// Maximum number of changes the Worker accepts in one push.
pub const MAX_CHANGES_PER_PUSH: usize = 100;

/// Answer of `GET /v1/health`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerInfo {
    /// Service name (`hatoba-sync`).
    pub service: String,
    /// Service version.
    pub version: String,
    /// Wire API version.
    pub api: u32,
    /// Whether a vault has already been set up on the server.
    pub initialized: bool,
}

/// KDF inputs the server hands out before login (`GET /v1/prelogin`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KdfInfo {
    /// Base64 salt.
    pub kdf_salt: String,
    /// The raw JSON string of the KDF parameters.
    pub kdf_params: String,
}

/// Everything the server stores at setup. The raw `auth_key` / `recovery_auth` are sent; the
/// server keeps only their SHA-256.
pub struct VaultInit {
    /// Schema version.
    pub schema_version: u32,
    /// Base64 salt.
    pub kdf_salt: String,
    /// KDF parameters JSON.
    pub kdf_params: String,
    /// Login secret derived from the master password.
    pub auth_key: Key32,
    /// Vault key wrapped by `enc_key` (envelope JSON).
    pub protected_vault_key: String,
    /// Vault key wrapped by `recovery_key` (envelope JSON).
    pub recovery_vault_key: String,
    /// Recovery secret.
    pub recovery_auth: Key32,
    /// Worker mode: the deployment's `SETUP_TOKEN`. Ignored by direct D1 mode.
    pub setup_token: Option<Zeroizing<String>>,
}

impl std::fmt::Debug for VaultInit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VaultInit(<redacted>)")
    }
}

/// A sync session. In Worker mode this is the bearer token; keep it in the OS credential store.
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    /// Bearer token.
    pub token: Zeroizing<String>,
    /// Expiry, Unix ms.
    pub expires_at: i64,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// The device identity presented at login. `sealed_name` is an envelope JSON string sealed with
/// the vault key under AAD `hatoba/device/v1/{device_id}`; the server cannot read it.
#[derive(Clone, Debug)]
pub struct DeviceLogin {
    /// This device's id.
    pub device_id: String,
    /// Sealed `{"name":…,"platform":…}`.
    pub sealed_name: String,
}

/// The vault metadata held by the server (`GET /v1/vault`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultMeta {
    /// Schema version.
    pub schema_version: u32,
    /// Base64 salt.
    pub kdf_salt: String,
    /// KDF parameters JSON.
    pub kdf_params: String,
    /// Vault key wrapped by `enc_key`.
    pub protected_vault_key: String,
    /// Vault key wrapped by `recovery_key`.
    pub recovery_vault_key: String,
    /// Highest sequence number allocated so far.
    pub seq: u64,
}

/// One item as the server stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteItem {
    /// Item id.
    pub id: String,
    /// Envelope JSON; `None` for tombstones.
    pub envelope: Option<String>,
    /// Server revision (1 for the first write).
    pub revision: u64,
    /// Global change sequence number.
    pub seq: u64,
    /// Tombstone flag.
    pub deleted: bool,
    /// Plaintext `updated_at` as supplied by the pushing client.
    pub updated_at: i64,
}

/// One page of `GET /v1/items`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullPage {
    /// Items with `seq` greater than the request's `since`, ordered by `seq`.
    pub items: Vec<RemoteItem>,
    /// Cursor to continue from.
    pub next_since: u64,
    /// Whether more pages follow.
    pub has_more: bool,
}

/// A local change to push.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// Item id.
    pub id: String,
    /// The server revision this edit is based on; 0 for an item the server has never seen.
    pub base_revision: u64,
    /// Whether this is a deletion.
    pub deleted: bool,
    /// Envelope JSON; `None` for deletions.
    pub envelope: Option<String>,
    /// Plaintext `updated_at`.
    pub updated_at: i64,
}

/// Outcome of one pushed change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushResult {
    /// Accepted.
    Ok {
        /// Item id.
        id: String,
        /// New server revision.
        revision: u64,
        /// Sequence number assigned.
        seq: u64,
    },
    /// The server's revision differs from `base_revision`; here is its version.
    Conflict {
        /// Item id.
        id: String,
        /// The current server row.
        server: RemoteItem,
    },
    /// Rejected for this item only (`too_large`, `not_found`, …).
    Error {
        /// Item id.
        id: String,
        /// Short error code.
        error: String,
    },
}

impl PushResult {
    /// The id of the item this result is about.
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Ok { id, .. } | Self::Conflict { id, .. } | Self::Error { id, .. } => id,
        }
    }
}

/// New recovery material sent together with a password change.
pub struct RecoveryUpdate {
    /// Vault key wrapped by the new recovery key.
    pub recovery_vault_key: String,
    /// The new recovery secret.
    pub recovery_auth: Key32,
}

/// `PUT /v1/vault/password`: atomically replaces the password-derived metadata (and optionally
/// the recovery material); the server revokes all other sessions.
pub struct VaultMetaUpdate {
    /// New base64 salt.
    pub kdf_salt: String,
    /// New KDF parameters JSON.
    pub kdf_params: String,
    /// New login secret.
    pub auth_key: Key32,
    /// Vault key wrapped by the new `enc_key`.
    pub protected_vault_key: String,
    /// Rotated recovery material, sent as a pair or not at all.
    pub recovery: Option<RecoveryUpdate>,
}

impl std::fmt::Debug for VaultMetaUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VaultMetaUpdate(<redacted>)")
    }
}

/// Answer of `POST /v1/recover`.
pub struct Recovered {
    /// The vault key wrapped by the recovery key.
    pub recovery_vault_key: String,
    /// KDF inputs for deriving the new password's keys.
    pub kdf: KdfInfo,
    /// A restricted session that may only call the password-change endpoint.
    pub session: Session,
}

/// One entry of the device list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteDevice {
    /// Device id.
    pub device_id: String,
    /// Sealed device name (envelope JSON); open with the vault key.
    pub device_name: String,
    /// First login, Unix ms.
    pub created_at: i64,
    /// Last request, Unix ms.
    pub last_seen: i64,
    /// Session expiry, Unix ms.
    pub expires_at: i64,
    /// Whether this is the calling device.
    pub current: bool,
}

/// A sync server. All methods are cancel-safe and never log request or response bodies.
///
/// Implementations keep the current [`Session`] internally: [`login`](Self::login) and
/// [`recover`](Self::recover) install the session they obtain, and
/// [`set_session`](Self::set_session) restores one saved earlier (for example from the keychain
/// after a restart).
#[async_trait]
pub trait SyncBackend: Send + Sync {
    /// `GET /v1/health`: connectivity test and "already initialised?" probe.
    async fn health(&self) -> Result<ServerInfo>;
    /// `GET /v1/prelogin`: salt and KDF parameters. `Error::RemoteNotInitialized` if no vault.
    async fn prelogin(&self) -> Result<KdfInfo>;
    /// `POST /v1/setup`: one-time initialisation. `Error::RemoteInitialized` if already done.
    async fn setup(&self, init: VaultInit) -> Result<()>;
    /// `POST /v1/login`: exchanges `auth_key` for a session and installs it.
    async fn login(&self, auth_key: &[u8; 32], device: &DeviceLogin) -> Result<Session>;
    /// `POST /v1/recover`: proves possession of the recovery code; installs the restricted session.
    async fn recover(&self, recovery_auth: &[u8; 32], device: &DeviceLogin) -> Result<Recovered>;
    /// `GET /v1/vault`.
    async fn fetch_vault(&self) -> Result<VaultMeta>;
    /// `GET /v1/items?since=&limit=`.
    async fn pull(&self, since_seq: u64, limit: u32) -> Result<PullPage>;
    /// `POST /v1/items`. Results are in request order. Implementations split requests larger
    /// than [`MAX_CHANGES_PER_PUSH`].
    async fn push(&self, changes: Vec<Change>) -> Result<Vec<PushResult>>;
    /// `PUT /v1/vault/password`.
    async fn update_vault_meta(&self, update: VaultMetaUpdate) -> Result<()>;
    /// `GET /v1/devices`.
    async fn devices(&self) -> Result<Vec<RemoteDevice>>;
    /// `DELETE /v1/devices/:id`.
    async fn revoke_device(&self, device_id: &str) -> Result<()>;
    /// Installs (or clears) the session used for authenticated calls.
    fn set_session(&self, session: Option<Session>);
    /// Forgets every credential held in memory: the session and, in direct D1 mode, the
    /// Cloudflare API token. Later authenticated calls fail with
    /// [`Error::Unauthorized`](crate::Error::Unauthorized) without sending a request. The shell
    /// calls this when it disconnects sync, replaces the backend or locks the vault, so that a
    /// round still holding the backend cannot authenticate.
    fn forget_credentials(&self) {
        self.set_session(None);
    }
}
