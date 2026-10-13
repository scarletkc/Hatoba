//! The dashboard's readings (HOST-14). `dashboard::Dashboard` holds the connections.

use std::sync::Arc;

use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use crate::dto::DashboardEvent;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Reads the resource usage of `host_ids` every `interval_secs` (1 to 60) in the background,
/// in place of the page's earlier watch, until `dashboard_unwatch` with the returned id or the
/// lock. What happens to each host streams on `channel`. Hosts both watches read keep their
/// connections.
#[tauri::command]
#[specta::specta]
pub fn dashboard_watch(
    app: AppHandle,
    state: State<'_, AppState>,
    host_ids: Vec<String>,
    interval_secs: u32,
    channel: Channel<DashboardEvent>,
) -> AppResult<u64> {
    if !state.vault().is_unlocked() {
        return Err(AppError::locked());
    }
    let sink = Arc::new(move |event| channel.send(event).is_ok());
    Ok(state.dashboard.watch(&app, &host_ids, interval_secs, sink))
}

/// Stops the watch `watch_id`; the connections close a minute later unless the page watches
/// their hosts again.
#[tauri::command]
#[specta::specta]
pub fn dashboard_unwatch(state: State<'_, AppState>, watch_id: u64) {
    state.dashboard.unwatch(watch_id);
}

/// **Retry**: signs in to a held host again, and tries an offline or failed one now.
#[tauri::command]
#[specta::specta]
pub fn dashboard_retry(state: State<'_, AppState>, host_id: String) {
    state.dashboard.retry(&host_id);
}
