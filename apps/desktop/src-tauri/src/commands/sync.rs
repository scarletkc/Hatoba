//! Cloud sync commands: wizard, status, devices, conflicts (spec §6, §9 Cloud Sync).

use std::sync::Arc;
use std::time::Instant;

use hatoba_core::model::{HostAuth, Item};
use hatoba_core::platform::{SecretStore, secret_keys};
use hatoba_core::sync::{
    ConflictEntry, D1Backend, Resolution, SyncBackend, SyncConfig, WorkerBackend, clear_session,
    d1, flows, save_session,
};
use tauri::{AppHandle, State};

use crate::dto::{
    ConflictAction, ConflictField, ConflictResolution, ConflictView, D1Database, DeviceView,
    ItemType, SyncConfigInput, SyncStatus, SyncTestResult,
};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::state::AppState;
use crate::sync::{self, Trigger, device_info};

/// Turns the wizard input into a config + backend. The D1 API token goes to the credential store
/// only once the setup succeeds.
pub(crate) fn backend_from_input(
    input: &SyncConfigInput,
) -> AppResult<(SyncConfig, Arc<dyn SyncBackend>)> {
    Ok(match input {
        SyncConfigInput::Worker { url, .. } => {
            let backend = WorkerBackend::new(url)?;
            (
                SyncConfig::Worker {
                    url: backend.base_url().to_owned(),
                    deployment: None,
                },
                Arc::new(backend),
            )
        }
        SyncConfigInput::D1 {
            account_id,
            database_id,
            api_token,
        } => {
            if database_id.trim().is_empty() {
                return Err(AppError::invalid("database_id", "choose a database"));
            }
            let backend = D1Backend::new(account_id.trim(), database_id.trim(), api_token.trim())?;
            (
                SyncConfig::D1 {
                    account_id: account_id.trim().to_owned(),
                    database_id: database_id.trim().to_owned(),
                },
                Arc::new(backend),
            )
        }
    })
}

pub(crate) fn persist(
    state: &AppState,
    config: &SyncConfig,
    input: &SyncConfigInput,
    session: &hatoba_core::sync::Session,
) -> AppResult<()> {
    save_session(state.secrets.as_ref(), session)?;
    if let SyncConfigInput::D1 { api_token, .. } = input {
        state
            .secrets
            .set(secret_keys::D1_API_TOKEN, api_token.trim())?;
    }
    state.vault().set_sync_config(Some(config))?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn sync_status(app: AppHandle) -> SyncStatus {
    sync::status(&app)
}

/// Wizard "Test connection": Worker `/v1/health`, or (D1 mode) token check + database list.
#[tauri::command]
#[specta::specta]
pub async fn sync_test(config: SyncConfigInput) -> AppResult<SyncTestResult> {
    let started = Instant::now();
    let result = match &config {
        SyncConfigInput::Worker { url, .. } => match WorkerBackend::new(url) {
            Ok(backend) => backend
                .health()
                .await
                .map(|info| (info.initialized, Some(info.version), None)),
            Err(e) => Err(e),
        },
        SyncConfigInput::D1 {
            account_id,
            database_id,
            api_token,
        } => {
            async {
                let databases = d1::list_databases(account_id.trim(), api_token.trim()).await?;
                let initialized = if database_id.trim().is_empty() {
                    false
                } else {
                    D1Backend::new(account_id.trim(), database_id.trim(), api_token.trim())?
                        .health()
                        .await?
                        .initialized
                };
                let list = databases
                    .into_iter()
                    .map(|d| D1Database {
                        id: d.id,
                        name: d.name,
                        region: d.region,
                    })
                    .collect();
                Ok((initialized, None, Some(list)))
            }
            .await
        }
    };
    Ok(match result {
        Ok((initialized, version, databases)) => SyncTestResult {
            ok: true,
            initialized,
            version,
            latency_ms: Some(started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32),
            databases,
            error: None,
        },
        Err(e) => SyncTestResult {
            ok: false,
            initialized: false,
            version: None,
            latency_ms: None,
            databases: None,
            error: Some(e.into()),
        },
    })
}

/// Flow A (§6.6): initialise the remote with this vault, sign in and push everything.
#[tauri::command]
#[specta::specta]
pub async fn sync_configure(
    app: AppHandle,
    state: State<'_, AppState>,
    config: SyncConfigInput,
    password: String,
) -> AppResult<()> {
    let (cfg, backend) = backend_from_input(&config)?;
    let setup_token = match &config {
        SyncConfigInput::Worker { setup_token, .. } => setup_token
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty()),
        SyncConfigInput::D1 { .. } => None,
    };
    let session = flows::enable_sync(
        &state.vault,
        backend.as_ref(),
        &password,
        setup_token,
        device_info(),
    )
    .await?;
    persist(&state, &cfg, &config, &session)?;
    state.sync.set_backend(Some(backend));
    sync::emit_status(&app);
    sync::trigger(&app, Trigger::Manual);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn sync_now(app: AppHandle) -> AppResult<()> {
    sync::run_round(&app).await
}

/// Re-authenticate after "认证失效" (session expired or revoked).
#[tauri::command]
#[specta::specta]
pub async fn sync_login(
    app: AppHandle,
    state: State<'_, AppState>,
    password: String,
) -> AppResult<()> {
    let config = state
        .vault()
        .sync_config()?
        .ok_or_else(|| AppError::new(ErrorCode::Sync, "sync is not configured"))?;
    let backend = match state.sync.backend() {
        Some(b) => b,
        None => sync::backend_for(&config, state.secrets.as_ref())?,
    };
    let session = flows::sign_in(&state.vault, backend.as_ref(), &password, device_info()).await?;
    save_session(state.secrets.as_ref(), &session)?;
    backend.set_session(Some(session));
    state.sync.set_backend(Some(backend));
    sync::emit_status(&app);
    sync::trigger(&app, Trigger::Manual);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn sync_set_auto(app: AppHandle, state: State<'_, AppState>, enabled: bool) {
    state.sync.set_auto(enabled);
    sync::emit_status(&app);
    if enabled {
        sync::trigger(&app, Trigger::Manual);
    }
}

/// Stops syncing on this device. Local data and the remote copy are both kept.
#[tauri::command]
#[specta::specta]
pub fn sync_disconnect(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    clear_session(state.secrets.as_ref())?;
    state.secrets.delete(secret_keys::D1_API_TOKEN)?;
    state.vault().set_sync_config(None)?;
    state.sync.set_backend(None);
    sync::emit_status(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn sync_devices(state: State<'_, AppState>) -> AppResult<Vec<DeviceView>> {
    let backend = state.sync.require_backend()?;
    let devices = flows::devices(&state.vault, backend.as_ref()).await?;
    Ok(devices
        .into_iter()
        .map(|d| DeviceView {
            name: d
                .name
                .unwrap_or_else(|| d.device_id.chars().take(8).collect()),
            platform: d.platform.unwrap_or_default(),
            device_id: d.device_id,
            created_at: d.created_at,
            last_seen: d.last_seen,
            current: d.current,
        })
        .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn sync_revoke_device(
    app: AppHandle,
    state: State<'_, AppState>,
    device_id: String,
) -> AppResult<()> {
    let backend = state.sync.require_backend()?;
    let current = state.vault().device_id() == device_id;
    flows::revoke_device(backend.as_ref(), &device_id).await?;
    if current {
        clear_session(state.secrets.as_ref())?;
        backend.set_session(None);
        sync::emit_status(&app);
    }
    Ok(())
}

fn item_type(item: Option<&Item>) -> ItemType {
    match item {
        Some(Item::Host(_)) | None => ItemType::Host,
        Some(Item::Group(_)) => ItemType::Group,
        Some(Item::Key(_)) => ItemType::Key,
        Some(Item::KnownHost(_)) => ItemType::KnownHost,
        Some(Item::Forward(_)) => ItemType::Forward,
        Some(Item::Snippet(_)) => ItemType::Snippet,
        Some(Item::Settings(_)) => ItemType::Settings,
    }
}

/// Non-secret fields shown side by side in the conflict view.
fn summary(item: &Item, hosts: &dyn Fn(&str) -> Option<String>) -> Vec<(&'static str, String)> {
    match item {
        Item::Host(h) => vec![
            ("name", h.name.clone()),
            ("address", h.address.clone()),
            ("port", h.port.to_string()),
            ("username", h.username.clone()),
            (
                "auth",
                match &h.auth {
                    HostAuth::Password { .. } => "password",
                    HostAuth::Key { .. } => "key",
                    HostAuth::Agent => "agent",
                    HostAuth::Ask => "ask",
                }
                .to_owned(),
            ),
            (
                "jump_host",
                h.jump_host_id
                    .as_deref()
                    .and_then(hosts)
                    .unwrap_or_default(),
            ),
            ("tags", h.tags.join(", ")),
            ("note", h.note.clone()),
        ],
        Item::Group(g) => vec![("name", g.name.clone())],
        Item::Key(k) => vec![
            ("name", k.name.clone()),
            ("fingerprint", k.fingerprint.clone()),
        ],
        Item::KnownHost(k) => vec![("fingerprint", k.fingerprint.clone())],
        Item::Forward(f) => vec![("bind_port", f.bind_port.to_string())],
        Item::Snippet(s) => vec![("name", s.name.clone())],
        Item::Settings(_) => vec![],
    }
}

fn conflict_view(c: &ConflictEntry, hosts: &dyn Fn(&str) -> Option<String>) -> ConflictView {
    let local = c
        .local
        .as_ref()
        .map(|i| summary(i, hosts))
        .unwrap_or_default();
    let remote = c
        .remote
        .as_ref()
        .map(|i| summary(i, hosts))
        .unwrap_or_default();
    let mut fields: Vec<ConflictField> = Vec::new();
    if c.local_deleted != c.remote_deleted {
        let state = |deleted: bool| Some(if deleted { "deleted" } else { "modified" }.to_owned());
        fields.push(ConflictField {
            field: "state".into(),
            local: state(c.local_deleted),
            remote: state(c.remote_deleted),
        });
    }
    let keys: Vec<&str> = local.iter().chain(remote.iter()).map(|(k, _)| *k).collect();
    let mut seen = std::collections::HashSet::new();
    for key in keys.into_iter().filter(|k| seen.insert(*k)) {
        let l = local
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty());
        let r = remote
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty());
        if l != r {
            fields.push(ConflictField {
                field: key.to_owned(),
                local: l,
                remote: r,
            });
        }
    }
    let name = c
        .local
        .as_ref()
        .or(c.remote.as_ref())
        .map(Item::display_name)
        .unwrap_or_else(|| c.item_id.clone());
    ConflictView {
        id: c.id,
        item_id: c.item_id.clone(),
        item_type: item_type(c.local.as_ref().or(c.remote.as_ref())),
        item_name: name,
        resolution: match c.resolution {
            Resolution::LocalWins if c.remote_deleted => ConflictResolution::ModifiedWon,
            Resolution::RemoteWins if c.local_deleted => ConflictResolution::ModifiedWon,
            Resolution::LocalWins => ConflictResolution::LocalWon,
            Resolution::RemoteWins => ConflictResolution::RemoteWon,
            Resolution::LocalWinsRemoteCopied | Resolution::RemoteWinsLocalCopied => {
                ConflictResolution::KeptBoth
            }
        },
        local_updated_at: c.local_updated_at,
        remote_updated_at: c.remote_updated_at,
        local_deleted: c.local_deleted,
        remote_deleted: c.remote_deleted,
        fields,
        created_at: c.created_at,
    }
}

/// §6.4: conflicts are resolved automatically; this lists the unreviewed ones (P1).
#[tauri::command]
#[specta::specta]
pub fn sync_conflicts(state: State<'_, AppState>) -> AppResult<Vec<ConflictView>> {
    state.with_unlocked(|v| {
        let names: std::collections::HashMap<String, String> =
            v.hosts().into_iter().map(|(id, h)| (id, h.name)).collect();
        let lookup = |id: &str| names.get(id).cloned();
        Ok(v.conflicts(true)?
            .iter()
            .map(|c| conflict_view(c, &lookup))
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub fn sync_conflict_resolve(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    action: ConflictAction,
) -> AppResult<()> {
    state.with_unlocked(|v| {
        if action == ConflictAction::Restore {
            v.restore_conflict_loser(id)?;
        }
        v.mark_conflict_reviewed(id)?;
        Ok(())
    })?;
    if action == ConflictAction::Restore {
        sync::local_change(&app);
    } else {
        sync::emit_status(&app);
    }
    Ok(())
}
