//! Cloud sync (spec §6): backends, the engine and the user-facing flows.
//!
//! * [`backend`]: the `SyncBackend` trait and its wire types.
//! * [`worker`]: HTTPS to the user's Cloudflare Worker (recommended).
//! * [`d1`]: Cloudflare D1 REST API directly (P1).
//! * [`deploy`]: deploying the Worker to the user's account through the Cloudflare API (P1).
//! * [`compat`]: which Worker versions this build syncs with, and when it offers an upgrade.
//! * [`engine`]: one pull/push round, conflict handling, tombstones.
//! * [`conflict`]: the §6.4 rules as pure functions.
//! * [`flows`]: enable sync, restore on a new device, sign in again, change password, recovery,
//!   device management.
//!
//! Timers, triggers and retry scheduling belong to the shell; [`Backoff`] is provided for it.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::platform::{SecretStore, secret_keys};
use crate::vault::Vault;

pub mod backend;
pub mod compat;
pub mod conflict;
pub mod d1;
pub mod deploy;
pub mod engine;
pub mod flows;
mod http;
pub mod worker;

#[cfg(test)]
mod d1_tests;
#[cfg(test)]
mod fake;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod worker_tests;

pub use backend::{
    Change, DeviceLogin, KdfInfo, PullPage, PushResult, Recovered, RecoveryUpdate, RemoteDevice,
    RemoteItem, ServerInfo, Session, SyncBackend, VaultInit, VaultMeta, VaultMetaUpdate,
};
pub use compat::{MIN_WORKER_VERSION, WORKER_API, WorkerCompat, worker_compat};
pub use conflict::{ConflictEntry, Resolution};
pub use d1::D1Backend;
pub use engine::{SyncEngine, SyncOptions, SyncReport};
pub use flows::{
    DeviceEntry, change_password_remote, devices, enable_sync, recover_remote, restore_from_cloud,
    revoke_device, rotate_recovery_remote, sign_in,
};
pub use worker::WorkerBackend;

/// The vault as the shell shares it between commands and the sync engine.
///
/// The engine locks it for short local steps only and never across an `.await`.
pub type SharedVault = Arc<Mutex<Vault>>;

/// Wraps a vault for sharing.
#[must_use]
pub fn share(vault: Vault) -> SharedVault {
    Arc::new(Mutex::new(vault))
}

/// Locks the shared vault.
///
/// # Errors
/// [`Error::Poisoned`] if another thread panicked while holding the lock.
pub fn lock_vault(vault: &SharedVault) -> Result<MutexGuard<'_, Vault>> {
    vault.lock().map_err(|_| Error::Poisoned)
}

/// Stops the sync rounds of one connection (spec §6.3). The shell gives each connection its own
/// token and cancels it when it disconnects sync, replaces the backend or locks the vault.
///
/// A cancelled round abandons the request in flight, sends no further request and returns
/// [`Error::Cancelled`] at its next vault step without touching the vault. The engine checks
/// the token while it holds the vault lock, so a shell that cancels first and then changes the
/// sync state under that lock never has it overwritten by the old round. A request already sent
/// may still reach the server; its result is dropped, as if the response had been lost.
#[derive(Clone, Debug)]
pub struct CancelToken(Arc<tokio::sync::watch::Sender<bool>>);

impl Default for CancelToken {
    fn default() -> Self {
        Self(Arc::new(tokio::sync::watch::Sender::new(false)))
    }
}

impl CancelToken {
    /// A token that is not cancelled.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancels every round that holds this token. Calling it again does nothing.
    pub fn cancel(&self) {
        self.0.send_replace(true);
    }

    /// Whether [`cancel`](Self::cancel) has been called.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }

    /// Resolves once the token is cancelled.
    pub async fn cancelled(&self) {
        let mut rx = self.0.subscribe();
        // `self` keeps the sender alive, so this returns only once the token is cancelled.
        let _ = rx.wait_for(|cancelled| *cancelled).await;
    }

    /// Runs `call`, or drops it with [`Error::Cancelled`] once the token is cancelled.
    ///
    /// # Errors
    /// [`Error::Cancelled`], or the error of `call`.
    pub async fn guard<T>(&self, call: impl Future<Output = Result<T>>) -> Result<T> {
        tokio::select! {
            biased;
            () = self.cancelled() => Err(Error::Cancelled),
            result = call => result,
        }
    }
}

/// Which backend this device syncs with. Contains no secrets (the session token and the
/// Cloudflare API token live in the OS credential store).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum SyncConfig {
    /// A deployed Hatoba Worker.
    Worker {
        /// Normalised Worker base URL.
        url: String,
        /// Set when the app deployed the Worker (spec §6.7); it fills in the upgrade form.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deployment: Option<WorkerDeployment>,
    },
    /// Direct Cloudflare D1 access.
    D1 {
        /// Cloudflare account id.
        account_id: String,
        /// D1 database UUID.
        database_id: String,
    },
}

/// Where the app deployed a Worker. Not secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerDeployment {
    /// Cloudflare account ID.
    pub account_id: String,
    /// Worker name.
    pub worker_name: String,
}

/// Saves a session in the OS credential store.
///
/// # Errors
/// Credential store errors.
pub fn save_session(store: &dyn SecretStore, session: &Session) -> Result<()> {
    let json = zeroize::Zeroizing::new(serde_json::to_string(session)?);
    store.set(secret_keys::SYNC_SESSION, &json)
}

/// Loads the saved session, if any. An unreadable entry is treated as absent.
///
/// # Errors
/// Credential store errors.
pub fn load_session(store: &dyn SecretStore) -> Result<Option<Session>> {
    Ok(store
        .get(secret_keys::SYNC_SESSION)?
        .and_then(|json| serde_json::from_str(&json).ok()))
}

/// Forgets the saved session.
///
/// # Errors
/// Credential store errors.
pub fn clear_session(store: &dyn SecretStore) -> Result<()> {
    store.delete(secret_keys::SYNC_SESSION)
}

/// Exponential retry delay with jitter, for the shell's sync scheduler: 2 s, 4 s, 8 s, …
/// capped at 5 minutes, each randomised by ±20 %. Call [`reset`](Self::reset) after a success.
#[derive(Clone, Debug)]
pub struct Backoff {
    base: Duration,
    max: Duration,
    attempt: u32,
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new()
    }
}

impl Backoff {
    /// 2 s growing to 5 min.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            base: Duration::from_secs(2),
            max: Duration::from_secs(300),
            attempt: 0,
        }
    }

    /// Custom bounds.
    #[must_use]
    pub const fn with_bounds(base: Duration, max: Duration) -> Self {
        Self {
            base,
            max,
            attempt: 0,
        }
    }

    /// The delay before the next retry; advances the schedule.
    pub fn next_delay(&mut self) -> Duration {
        let unit = getrandom::u32().map_or(0.5, |n| f64::from(n) / f64::from(u32::MAX));
        self.next_delay_with(unit)
    }

    /// Deterministic variant: `unit` in `[0, 1]` selects the jitter (0 → −20 %, 1 → +20 %).
    pub fn next_delay_with(&mut self, unit: f64) -> Duration {
        let exponent = self.attempt.min(30);
        self.attempt = self.attempt.saturating_add(1);
        let raw = self.base.as_secs_f64() * f64::from(1u32 << exponent.min(20));
        let jittered = raw * (0.8 + 0.4 * unit.clamp(0.0, 1.0));
        Duration::from_secs_f64(jittered.min(self.max.as_secs_f64()))
    }

    /// Starts over from the shortest delay.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    /// Failed attempts since the last reset.
    #[must_use]
    pub fn attempts(&self) -> u32 {
        self.attempt
    }
}

#[cfg(test)]
mod backoff_tests {
    use super::*;

    #[test]
    fn grows_exponentially_and_caps_at_five_minutes() {
        let mut b = Backoff::new();
        let secs: Vec<f64> = (0..10)
            .map(|_| b.next_delay_with(0.5).as_secs_f64())
            .collect();
        assert_eq!(
            secs,
            [2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0, 300.0, 300.0]
        );
        assert_eq!(b.attempts(), 10);
        b.reset();
        assert_eq!(b.next_delay_with(0.5), Duration::from_secs(2));
    }

    #[test]
    fn jitter_stays_within_twenty_percent() {
        for attempt in 0..12 {
            let mut lo = Backoff::new();
            let mut hi = Backoff::new();
            for _ in 0..attempt {
                lo.next_delay_with(0.5);
                hi.next_delay_with(0.5);
            }
            let raw = (2.0 * f64::from(1u32 << attempt)).min(300.0);
            let (l, h) = (
                lo.next_delay_with(0.0).as_secs_f64(),
                hi.next_delay_with(1.0).as_secs_f64(),
            );
            assert!(l >= raw * 0.8 - 1e-9 && l <= 300.0, "{l} vs {raw}");
            assert!(h <= 300.0 + 1e-9 && h >= l, "{h}");
        }
        // Random variant always lands in range.
        let mut b = Backoff::new();
        for _ in 0..50 {
            let d = b.next_delay();
            assert!(d <= Duration::from_secs(300));
        }
    }

    #[test]
    fn sync_config_json_shape() {
        let w = SyncConfig::Worker {
            url: "https://x.example".into(),
            deployment: None,
        };
        assert_eq!(
            serde_json::to_string(&w).unwrap(),
            r#"{"mode":"worker","url":"https://x.example"}"#
        );
        let deployed = SyncConfig::Worker {
            url: "https://hatoba-sync.kc.workers.dev".into(),
            deployment: Some(WorkerDeployment {
                account_id: "a".into(),
                worker_name: "hatoba-sync".into(),
            }),
        };
        assert_eq!(
            serde_json::to_string(&deployed).unwrap(),
            r#"{"mode":"worker","url":"https://hatoba-sync.kc.workers.dev","deployment":{"account_id":"a","worker_name":"hatoba-sync"}}"#
        );
        assert_eq!(
            serde_json::from_str::<SyncConfig>(&serde_json::to_string(&deployed).unwrap()).unwrap(),
            deployed
        );
        let d = SyncConfig::D1 {
            account_id: "a".into(),
            database_id: "d".into(),
        };
        assert_eq!(
            serde_json::from_str::<SyncConfig>(&serde_json::to_string(&d).unwrap()).unwrap(),
            d
        );
    }

    #[test]
    fn session_persistence_round_trip() {
        let store = crate::platform::MemorySecretStore::new();
        assert!(load_session(&store).unwrap().is_none());
        let s = Session {
            token: zeroize::Zeroizing::new("tok".into()),
            expires_at: 99,
        };
        save_session(&store, &s).unwrap();
        let back = load_session(&store).unwrap().unwrap();
        assert_eq!((back.token.as_str(), back.expires_at), ("tok", 99));
        assert!(!format!("{back:?}").contains("tok\""));
        clear_session(&store).unwrap();
        assert!(load_session(&store).unwrap().is_none());
    }
}
