//! Sync scheduling (spec §6.3): after unlock, 2 s after local edits, every 60 s, on window focus
//! and on demand. Failures never affect local use; transient ones retry with exponential backoff.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use hatoba_core::model::Item;
use hatoba_core::platform::{SecretStore, secret_keys};
use hatoba_core::sync::{
    Backoff, D1Backend, SyncBackend, SyncConfig, SyncEngine, WorkerBackend, load_session,
};
use tauri::AppHandle;
use tauri_specta::Event;
use tokio::sync::Notify;

use crate::dto::{SyncCounts, SyncKind, SyncState, SyncStatus};
use crate::error::{AppError, AppResult};
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

struct Inner {
    backend: Option<Arc<dyn SyncBackend>>,
    state: SyncState,
    message: Option<String>,
    auto: bool,
    pending_trigger: Option<Trigger>,
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
                state: SyncState::Off,
                message: None,
                auto: true,
                pending_trigger: None,
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

    pub fn set_backend(&self, backend: Option<Arc<dyn SyncBackend>>) {
        let mut inner = self.inner();
        inner.state = if backend.is_some() {
            SyncState::Idle
        } else {
            SyncState::Off
        };
        inner.message = None;
        inner.backend = backend;
    }

    pub fn set_auto(&self, auto: bool) {
        self.inner().auto = auto;
    }

    pub fn auto(&self) -> bool {
        self.inner().auto
    }

    fn set_state(&self, state: SyncState, message: Option<String>) {
        let mut inner = self.inner();
        inner.state = state;
        inner.message = message;
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
        st.sync.set_state(SyncState::AuthFailed, None);
    }
    emit_status(app);
    trigger(app, Trigger::Unlock);
}

/// Stops syncing (on lock). The backend is rebuilt on the next unlock.
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
    let (state, message, auto) = {
        let inner = st.sync.inner();
        (inner.state, inner.message.clone(), inner.auto)
    };
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

/// Runs one sync round now. Errors are reflected in the status and returned.
pub async fn run_round(app: &AppHandle) -> AppResult<()> {
    let st = state(app);
    let Some(backend) = st.sync.backend() else {
        return Ok(());
    };
    if !st.vault().is_unlocked() {
        return Ok(());
    }
    let _guard = st.sync.round.lock().await;
    st.sync.set_state(SyncState::Syncing, None);
    emit_status(app);
    let engine =
        SyncEngine::new(st.vault.clone(), backend).with_conflict_suffix(conflict_suffix(app));
    let result = engine.sync().await;
    let outcome = match result {
        Ok(report) => {
            tracing::info!(
                pulled = report.pulled,
                pushed = report.pushed,
                conflicts = report.conflicts_resolved,
                "sync round done"
            );
            st.sync.set_state(SyncState::Idle, None);
            adopt_legacy_prefs(app);
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
            st.sync.set_state(state, Some(err.detail.clone()));
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
