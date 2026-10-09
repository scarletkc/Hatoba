//! Sync scheduling (spec §6.3): after unlock, 2 s after local edits, every 60 s, on window focus
//! and on demand. Failures never affect local use; transient ones retry with exponential backoff.
//! In Worker mode, the first round after unlock reads `/v1/health` and pauses sync while the
//! Worker's version needs an update (spec §6.7, Upgrades).
//!
//! Every backend the controller installs starts a new *connection* with its own generation and
//! [`CancelToken`]. Installing another backend, or none, ends the previous connection: its rounds
//! stop at their next step without touching the vault, its status updates are ignored, and a
//! replaced backend forgets its credentials. Disconnecting also waits for the running round to
//! stop before the saved settings are cleared.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use hatoba_core::model::Item;
use hatoba_core::platform::{SecretStore, secret_keys};
use hatoba_core::sync::{
    Backoff, CancelToken, D1Backend, SyncBackend, SyncConfig, SyncEngine, WorkerBackend,
    WorkerCompat, load_session, worker_compat,
};
use tauri::AppHandle;
use tauri_specta::Event;
use tokio::sync::Notify;

use crate::dto::{SyncCounts, SyncKind, SyncState, SyncStatus, WorkerUpdate, WorkerUpdateKind};
use crate::error::{AppError, AppResult};
use crate::lock;
use crate::state::{AppState, state};

const INTERVAL: Duration = Duration::from_secs(60);
const DEBOUNCE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Unlock,
    LocalChange,
    Focus,
    Manual,
}

/// What the Worker's `/v1/health` last reported, against this build.
struct WorkerCheck {
    compat: WorkerCompat,
    version: String,
}

struct Inner {
    backend: Option<Arc<dyn SyncBackend>>,
    /// Counts connections; every `set_backend` starts a new one.
    generation: u64,
    /// Cancelled when the current connection ends.
    cancel: CancelToken,
    state: SyncState,
    message: Option<String>,
    auto: bool,
    pending_trigger: Option<Trigger>,
    /// Worker mode: the last `/v1/health` reading since the backend was set.
    worker: Option<WorkerCheck>,
}

/// The backend of one connection, as a round captures it when it starts.
#[derive(Clone)]
struct Connection {
    backend: Arc<dyn SyncBackend>,
    generation: u64,
    cancel: CancelToken,
}

pub struct SyncController {
    inner: Mutex<Inner>,
    wake: Notify,
    /// Serialises sync rounds (scheduler vs. "Sync now").
    round: tokio::sync::Mutex<()>,
}

impl Default for SyncController {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner {
                backend: None,
                generation: 0,
                cancel: CancelToken::new(),
                state: SyncState::Off,
                message: None,
                auto: true,
                pending_trigger: None,
                worker: None,
            }),
            wake: Notify::new(),
            round: tokio::sync::Mutex::new(()),
        }
    }
}

impl SyncController {
    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn backend(&self) -> Option<Arc<dyn SyncBackend>> {
        self.inner().backend.clone()
    }

    pub fn require_backend(&self) -> AppResult<Arc<dyn SyncBackend>> {
        self.backend()
            .ok_or_else(|| AppError::new(crate::error::ErrorCode::Sync, "sync is not configured"))
    }

    /// The current connection, if sync has a backend.
    fn connection(&self) -> Option<Connection> {
        let inner = self.inner();
        inner.backend.as_ref().map(|backend| Connection {
            backend: Arc::clone(backend),
            generation: inner.generation,
            cancel: inner.cancel.clone(),
        })
    }

    /// Starts a new connection with `backend`, or none, and ends the previous one. A backend
    /// that is replaced rather than installed again (as signing in again does) forgets its
    /// credentials, so a round still holding it cannot authenticate.
    pub fn set_backend(&self, backend: Option<Arc<dyn SyncBackend>>) {
        let replaced = {
            let mut inner = self.inner();
            inner.cancel.cancel();
            inner.cancel = CancelToken::new();
            inner.generation += 1;
            inner.state = if backend.is_some() {
                SyncState::Idle
            } else {
                SyncState::Off
            };
            inner.message = None;
            inner.worker = None;
            let old = std::mem::replace(&mut inner.backend, backend);
            old.filter(|old| {
                !inner
                    .backend
                    .as_ref()
                    .is_some_and(|new| Arc::ptr_eq(old, new))
            })
        };
        if let Some(old) = replaced {
            old.forget_credentials();
        }
    }

    /// Ends the connection, as `set_backend(None)`, and waits until its round, if one is
    /// running, has stopped. The guard keeps the next round from starting while the caller
    /// clears the saved sync settings.
    pub async fn end_connection(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.set_backend(None);
        self.round.lock().await
    }

    /// Whether `conn` is still the current connection.
    fn is_current(&self, conn: &Connection) -> bool {
        self.inner().generation == conn.generation
    }

    /// Runs `f` on the controller state if `conn` is still the current connection. Returns
    /// whether it ran: an ended connection's rounds never change the status of the next one.
    fn update_for(&self, conn: &Connection, f: impl FnOnce(&mut Inner)) -> bool {
        let mut inner = self.inner();
        let current = inner.generation == conn.generation;
        if current {
            f(&mut inner);
        }
        current
    }

    fn set_state_for(&self, conn: &Connection, state: SyncState, message: Option<String>) -> bool {
        self.update_for(conn, |inner| {
            inner.state = state;
            inner.message = message;
        })
    }

    pub fn set_auto(&self, auto: bool) {
        self.inner().auto = auto;
    }

    pub fn auto(&self) -> bool {
        self.inner().auto
    }

    /// Whether the Worker's version pauses sync.
    fn paused(&self) -> bool {
        self.inner()
            .worker
            .as_ref()
            .is_some_and(|w| w.compat.pauses_sync())
    }
}

/// The Worker version this build deploys, when it embeds the Worker.
pub fn bundled_version() -> Option<String> {
    crate::deploy::bundle().ok().map(|b| b.version.clone())
}

fn worker_mode(st: &AppState) -> bool {
    matches!(
        st.vault().sync_config(),
        Ok(Some(SyncConfig::Worker { .. }))
    )
}

/// Reads the Worker's `/v1/health` and records how it compares with this build, unless the
/// connection ended meanwhile. A Worker that does not answer keeps the last reading; the sync
/// round reports the failure.
async fn read_worker_health(st: &AppState, conn: &Connection) {
    match conn.cancel.guard(conn.backend.health()).await {
        Ok(info) => {
            let compat = worker_compat(&info, bundled_version().as_deref());
            tracing::info!(version = %info.version, api = info.api, ?compat, "worker health");
            st.sync.update_for(conn, |inner| {
                inner.worker = Some(WorkerCheck {
                    compat,
                    version: info.version,
                });
            });
        }
        Err(e) => tracing::info!("worker health check failed: {}", e.code()),
    }
}

/// Reads the Worker's `/v1/health` now, because the sync status page opened or an upgrade
/// finished, and pauses or resumes sync to match.
pub async fn recheck_worker(app: &AppHandle) {
    let st = state(app);
    let Some(conn) = st.sync.connection() else {
        return;
    };
    if !worker_mode(&st) {
        return;
    }
    let resumed = {
        let _guard = st.sync.round.lock().await;
        let was_paused = st.sync.paused();
        read_worker_health(&st, &conn).await;
        match (was_paused, st.sync.paused()) {
            (false, true) => {
                st.sync.set_state_for(&conn, SyncState::Paused, None);
                false
            }
            (true, false) => st.sync.set_state_for(&conn, SyncState::Idle, None),
            _ => false,
        }
    };
    emit_status(app);
    if resumed {
        // Pushes what was edited while sync was paused.
        trigger(app, Trigger::Manual);
    }
}

/// Builds the configured backend with its session / API token from the OS credential store.
pub fn backend_for(
    config: &SyncConfig,
    secrets: &dyn SecretStore,
) -> AppResult<Arc<dyn SyncBackend>> {
    Ok(match config {
        SyncConfig::Worker { url, .. } => {
            let backend = WorkerBackend::new(url)?;
            backend.set_session(load_session(secrets)?);
            Arc::new(backend)
        }
        SyncConfig::D1 {
            account_id,
            database_id,
        } => {
            let token = secrets.get(secret_keys::D1_API_TOKEN)?.ok_or_else(|| {
                AppError::new(
                    crate::error::ErrorCode::SyncAuth,
                    "the Cloudflare API token is missing",
                )
            })?;
            Arc::new(D1Backend::new(account_id, database_id, &token)?)
        }
    })
}

/// Called after unlock / restore: (re)creates the backend from the saved config and syncs.
pub fn on_unlock(app: &AppHandle) {
    let st = state(app);
    let config = st.vault().sync_config().ok().flatten();
    let backend = config
        .as_ref()
        .and_then(|c| match backend_for(c, st.secrets.as_ref()) {
            Ok(b) => Some(b),
            Err(e) => {
                tracing::warn!("sync backend unavailable: {}", e.detail);
                None
            }
        });
    let configured = config.is_some();
    st.sync.set_backend(backend);
    if configured && st.sync.backend().is_none() {
        st.sync.inner().state = SyncState::AuthFailed;
    }
    emit_status(app);
    trigger(app, Trigger::Unlock);
}

/// Stops syncing (on lock), and the backend forgets its credentials. It is rebuilt from the
/// credential store on the next unlock.
pub fn stop(app: &AppHandle) {
    state(app).sync.set_backend(None);
    emit_status(app);
}

/// Called by every command that changed vault items.
pub fn local_change(app: &AppHandle) {
    trigger(app, Trigger::LocalChange);
}

pub fn trigger(app: &AppHandle, trigger: Trigger) {
    let st = state(app);
    {
        let mut inner = st.sync.inner();
        // A pending immediate trigger wins over a debounced one.
        if inner.pending_trigger.is_none() || trigger != Trigger::LocalChange {
            inner.pending_trigger = Some(trigger);
        }
    }
    st.sync.wake.notify_one();
    if trigger == Trigger::LocalChange {
        emit_status(app);
    }
}

pub fn status(app: &AppHandle) -> SyncStatus {
    let st = state(app);
    let vault = st.vault();
    let config = vault.sync_config().ok().flatten();
    let (state, message, auto, worker) = {
        let inner = st.sync.inner();
        (
            inner.state,
            inner.message.clone(),
            inner.auto,
            inner.worker.as_ref().map(|w| (w.compat, w.version.clone())),
        )
    };
    let worker_update = worker.and_then(|(compat, version)| {
        let bundled = bundled_version();
        let kind = match compat {
            WorkerCompat::Current => return None,
            // Dismissed until the next bundled version.
            WorkerCompat::UpdateAvailable if vault.worker_update_dismissed() == bundled => {
                return None;
            }
            WorkerCompat::UpdateAvailable => WorkerUpdateKind::Available,
            WorkerCompat::UpdateRequired => WorkerUpdateKind::Required,
            WorkerCompat::AppUpdateRequired => WorkerUpdateKind::AppRequired,
        };
        Some(WorkerUpdate {
            kind,
            version,
            bundled,
        })
    });
    let (kind, endpoint, database) = match &config {
        None => (SyncKind::None, None, None),
        Some(SyncConfig::Worker { url, .. }) => {
            let host = url
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_end_matches('/')
                .to_owned();
            (SyncKind::Worker, Some(host), None)
        }
        Some(SyncConfig::D1 { database_id, .. }) => (
            SyncKind::D1,
            Some("api.cloudflare.com".to_owned()),
            Some(database_id.clone()),
        ),
    };
    let unlocked = vault.is_unlocked();
    let counts = unlocked.then(|| {
        let (mut hosts, mut keys, mut groups) = (0, 0, 0);
        for (_, item) in vault.items() {
            match item {
                Item::Host(_) => hosts += 1,
                Item::Key(_) => keys += 1,
                Item::Group(_) => groups += 1,
                _ => {}
            }
        }
        SyncCounts {
            hosts,
            keys,
            groups,
        }
    });
    SyncStatus {
        kind,
        endpoint,
        database,
        state: if config.is_none() {
            SyncState::Off
        } else {
            state
        },
        last_synced_at: vault.sync_last_at(),
        pending: vault.pending_count().min(u64::from(u32::MAX)) as u32,
        conflicts: if unlocked {
            vault.unreviewed_conflict_count().min(u64::from(u32::MAX)) as u32
        } else {
            0
        },
        auto_sync: auto,
        message,
        counts,
        worker_update,
    }
}

pub fn emit_status(app: &AppHandle) {
    let _ = status(app).emit(app);
}

fn conflict_suffix(app: &AppHandle) -> &'static str {
    use crate::dto::Language;
    let lang = match crate::commands::settings::prefs_get(state(app)).map(|p| p.language) {
        Ok(Language::ZhCn) => "zh".to_owned(),
        Ok(Language::Ja) => "ja".to_owned(),
        Ok(Language::En) => "en".to_owned(),
        _ => sys_locale(),
    };
    if lang.starts_with("zh") {
        "（冲突副本）"
    } else if lang.starts_with("ja") {
        "（競合コピー）"
    } else {
        " (conflict copy)"
    }
}

fn sys_locale() -> String {
    std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .unwrap_or_default()
        .to_lowercase()
}

/// Runs one sync round now. Errors are reflected in the status and returned. A round whose
/// connection ends before or while it runs stops and returns `Ok`, leaving the status alone.
pub async fn run_round(app: &AppHandle) -> AppResult<()> {
    let st = state(app);
    let Some(conn) = st.sync.connection() else {
        return Ok(());
    };
    if !st.vault().is_unlocked() {
        return Ok(());
    }
    let _guard = st.sync.round.lock().await;
    // The connection may have ended while this round waited for the previous one.
    if !st.sync.set_state_for(&conn, SyncState::Syncing, None) {
        return Ok(());
    }
    emit_status(app);
    if worker_mode(&st) {
        // The first round after unlock reads /v1/health, and so does every round while the
        // Worker's version pauses sync, so an upgrade from another device resumes it.
        let unread = st.sync.inner().worker.is_none();
        if unread || st.sync.paused() {
            read_worker_health(&st, &conn).await;
        }
        if st.sync.paused() {
            // Local changes stay pending and go out after the update.
            st.sync.set_state_for(&conn, SyncState::Paused, None);
            emit_status(app);
            return Ok(());
        }
    }
    let engine = SyncEngine::new(st.vault.clone(), Arc::clone(&conn.backend))
        .with_conflict_suffix(conflict_suffix(app))
        .with_cancel(conn.cancel.clone());
    // Pulled settings take effect now, not at the next local save or unlock.
    let result = lock::refresh_after_sync(&st, engine.sync()).await;
    let outcome = match result {
        Ok(report) => {
            tracing::info!(
                pulled = report.pulled,
                pushed = report.pushed,
                conflicts = report.conflicts_resolved,
                "sync round done"
            );
            if st.sync.set_state_for(&conn, SyncState::Idle, None) {
                adopt_legacy_prefs(app);
            }
            Ok(())
        }
        // Whatever stopped the round belongs to the connection that ended, not to the next one.
        Err(e) if !st.sync.is_current(&conn) => {
            tracing::info!("sync round stopped, its connection ended: {}", e.code());
            Ok(())
        }
        Err(e) => {
            let err: AppError = e.into();
            let state = match err.code {
                crate::error::ErrorCode::SyncOffline => SyncState::Offline,
                crate::error::ErrorCode::SyncAuth => SyncState::AuthFailed,
                crate::error::ErrorCode::Locked => SyncState::Idle,
                _ => SyncState::Error,
            };
            tracing::warn!("sync round failed: {}", err.detail);
            st.sync
                .set_state_for(&conn, state, Some(err.detail.clone()));
            Err(err)
        }
    };
    emit_status(app);
    outcome
}

/// The prefs move that waits for sync (`settings::adopt_legacy_prefs`). The round has just
/// pulled, so this device holds the latest settings and the move cannot outdate newer edits
/// from other devices. A write goes out with the next round.
fn adopt_legacy_prefs(app: &AppHandle) {
    match state(app).with_unlocked(crate::commands::settings::adopt_legacy_prefs) {
        Ok(true) => local_change(app),
        Ok(false) => {}
        Err(e) => tracing::warn!(
            "could not move the terminal prefs into the synced settings: {}",
            e.detail
        ),
    }
}

/// The background scheduler loop.
pub fn spawn_scheduler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut backoff = Backoff::default();
        let mut retry_at: Option<tokio::time::Instant> = None;
        loop {
            let st = state(&app);
            let wait = retry_at.map_or(INTERVAL, |at| {
                at.saturating_duration_since(tokio::time::Instant::now())
                    .min(INTERVAL)
            });
            let woke = tokio::time::timeout(wait, st.sync.wake.notified())
                .await
                .is_ok();
            let trigger = st.sync.inner().pending_trigger.take();
            if woke && trigger == Some(Trigger::LocalChange) {
                // Debounce bursts of edits (§6.3: 2 s).
                tokio::time::sleep(DEBOUNCE).await;
                st.sync.inner().pending_trigger.take();
            }
            let due = trigger.is_some()
                || retry_at.is_none_or(|at| tokio::time::Instant::now() >= at)
                || !woke;
            let manual = trigger == Some(Trigger::Manual);
            if !due || st.sync.backend().is_none() || !(st.sync.auto() || manual) {
                continue;
            }
            if st.sync.inner().state == SyncState::AuthFailed && !manual {
                continue; // needs the user to sign in again
            }
            match run_round(&app).await {
                Ok(()) => {
                    backoff.reset();
                    retry_at = None;
                }
                Err(e)
                    if matches!(
                        e.code,
                        crate::error::ErrorCode::SyncOffline | crate::error::ErrorCode::Sync
                    ) =>
                {
                    retry_at = Some(tokio::time::Instant::now() + backoff.next_delay());
                }
                Err(_) => retry_at = None,
            }
        }
    });
}

/// Device name and platform shown in the device list (stored encrypted on the server).
pub fn device_info() -> hatoba_core::platform::DeviceInfo {
    let name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|n| !n.trim().is_empty())
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_owned())
        })
        .unwrap_or_else(|| "Hatoba".to_owned());
    hatoba_core::platform::DeviceInfo {
        name,
        platform: crate::platform::os_label(),
    }
}

impl AppState {
    pub fn sync_configured(&self) -> bool {
        self.vault().sync_config().ok().flatten().is_some()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use hatoba_core::sync::{Session, SyncBackend, WorkerBackend};
    use tokio::sync::Notify;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use zeroize::Zeroizing;

    use super::SyncController;
    use crate::dto::SyncState;

    /// A Worker that answers the device list, so a request shows whether a backend can still
    /// authenticate.
    async fn worker() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/devices"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "devices": [] })),
            )
            .mount(&server)
            .await;
        server
    }

    fn signed_in(server: &MockServer) -> Arc<dyn SyncBackend> {
        let backend = WorkerBackend::new(&server.uri()).unwrap();
        backend.set_session(Some(Session {
            token: Zeroizing::new("session-token-for-tests".into()),
            expires_at: i64::MAX,
        }));
        Arc::new(backend)
    }

    #[tokio::test]
    async fn a_new_connection_ends_the_previous_one() {
        let server = worker().await;
        let sync = SyncController::default();
        let first = signed_in(&server);
        sync.set_backend(Some(Arc::clone(&first)));
        let old = sync.connection().unwrap();

        // Signing in again installs the same backend, which keeps its new session.
        sync.set_backend(Some(Arc::clone(&first)));
        assert!(old.cancel.is_cancelled());
        assert!(!sync.is_current(&old));
        first.devices().await.unwrap();

        // Another backend replaces it, and it forgets its credentials.
        let current = sync.connection().unwrap();
        sync.set_backend(Some(signed_in(&server)));
        assert!(current.cancel.is_cancelled());
        assert!(matches!(
            first.devices().await,
            Err(hatoba_core::Error::Unauthorized)
        ));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn an_ended_connection_cannot_change_the_status() {
        let server = worker().await;
        let sync = SyncController::default();
        sync.set_backend(Some(signed_in(&server)));
        let old = sync.connection().unwrap();

        sync.set_backend(None);
        assert!(!sync.set_state_for(&old, SyncState::AuthFailed, Some("expired".into())));
        assert_eq!(sync.inner().state, SyncState::Off);

        sync.set_backend(Some(signed_in(&server)));
        assert!(!sync.set_state_for(&old, SyncState::Error, Some("old round".into())));
        assert_eq!(sync.inner().state, SyncState::Idle);
        assert!(sync.inner().message.is_none());
        let current = sync.connection().unwrap();
        assert!(sync.set_state_for(&current, SyncState::Syncing, None));
        assert_eq!(sync.inner().state, SyncState::Syncing);
    }

    #[tokio::test]
    async fn ending_the_connection_waits_for_its_round_to_stop() {
        let server = worker().await;
        let sync = Arc::new(SyncController::default());
        let backend = signed_in(&server);
        sync.set_backend(Some(Arc::clone(&backend)));
        let conn = sync.connection().unwrap();
        let (running, stopped) = (Arc::new(Notify::new()), Arc::new(AtomicBool::new(false)));

        // A round that holds the round lock until its connection is cancelled.
        let round = tokio::spawn({
            let (sync, running, stopped) = (
                Arc::clone(&sync),
                Arc::clone(&running),
                Arc::clone(&stopped),
            );
            async move {
                let _guard = sync.round.lock().await;
                running.notify_one();
                conn.cancel.cancelled().await;
                tokio::task::yield_now().await;
                stopped.store(true, Ordering::SeqCst);
            }
        });
        running.notified().await;

        let guard = sync.end_connection().await;
        assert!(stopped.load(Ordering::SeqCst), "the round stopped first");
        assert!(sync.connection().is_none());
        assert_eq!(sync.inner().state, SyncState::Off);
        assert!(matches!(
            backend.devices().await,
            Err(hatoba_core::Error::Unauthorized)
        ));
        drop(guard);
        round.await.unwrap();
    }
}
