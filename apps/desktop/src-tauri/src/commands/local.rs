//! Local terminal tabs (spec §7.5): list this computer's shells, start one in a PTY, and relay its
//! output over the tab's channel in the frames SSH tabs use (§10.3).

use std::sync::Arc;

use hatoba_local::{LocalPty, PtyEvent, PtySize, Shell, ShellKind};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tokio::sync::mpsc;

use crate::commands::ssh::{FRAME_CLOSED, FRAME_DATA, TermFrame, emit_state};
use crate::dto::{LocalOpenOptions, LocalShellKind, LocalShellView, SessionState};
use crate::error::{AppError, AppResult};
use crate::local::{LocalSession, shell_env, tools_env};
use crate::state::AppState;

pub(crate) fn shell_view(shell: &Shell, default: bool) -> LocalShellView {
    LocalShellView {
        id: shell.id.clone(),
        name: shell.name.clone(),
        kind: match shell.kind {
            ShellKind::Unix => LocalShellKind::Unix,
            ShellKind::PowerShell => LocalShellKind::Powershell,
            ShellKind::Cmd => LocalShellKind::Cmd,
            ShellKind::GitBash => LocalShellKind::GitBash,
            ShellKind::Wsl { .. } => LocalShellKind::Wsl,
        },
        default,
    }
}

/// The shells a local tab can run (LOCAL-02), the default one first.
#[tauri::command]
#[specta::specta]
pub async fn local_shells() -> AppResult<Vec<LocalShellView>> {
    let shells = tokio::task::spawn_blocking(hatoba_local::shells::detect)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(shells
        .iter()
        .enumerate()
        .map(|(i, shell)| shell_view(shell, i == 0))
        .collect())
}

/// Starts the shell `shell_id` in a new PTY for a local tab (LOCAL-01) and returns the session
/// id. Its output streams on `channel` until it exits or `local_close`.
#[tauri::command]
#[specta::specta]
pub async fn local_open(
    app: AppHandle,
    state: State<'_, AppState>,
    shell_id: String,
    options: LocalOpenOptions,
    channel: Channel<TermFrame>,
) -> AppResult<String> {
    if !state.vault().is_unlocked() {
        return Err(AppError::locked());
    }
    let id = shell_id.clone();
    let shell = tokio::task::spawn_blocking(move || hatoba_local::shells::find(Some(&id)))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("shell"))?;
    let size = PtySize {
        cols: u16::try_from(options.cols.clamp(10, 1000)).unwrap_or(80),
        rows: u16::try_from(options.rows.clamp(2, 500)).unwrap_or(24),
    };
    let env = shell_env(&app.package_info().version.to_string());
    let home = dirs::home_dir();
    let (pty, events) =
        LocalPty::spawn(&shell, size, &env, home.as_deref()).map_err(AppError::local)?;
    let session_id = uuid::Uuid::now_v7().to_string();
    let tools = hatoba_local::LocalTools::for_shell(&shell, tools_env());
    let live = Arc::new(LocalSession::new(pty, shell, tools));
    state.local.insert(session_id.clone(), live.clone());
    tracing::info!(shell = %live.shell.name, "local terminal started");
    emit_state(&app, &session_id, None, SessionState::Connected, |_| {});
    pump(app, session_id.clone(), live, events, channel);
    Ok(session_id)
}

/// Relays the shell's output to the WebView until it exits, then drops the session.
fn pump(
    app: AppHandle,
    session_id: String,
    live: Arc<LocalSession>,
    mut events: mpsc::Receiver<PtyEvent>,
    channel: Channel<TermFrame>,
) {
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                PtyEvent::Data(bytes) => {
                    if channel.send(TermFrame::new(FRAME_DATA, &bytes)).is_err() {
                        // The WebView went away (reload / window closed): the shell goes too.
                        live.close();
                        break;
                    }
                }
                PtyEvent::Exited { exit_status } => {
                    // A shell Hatoba ended, on lock or quit, has no status of its own: its tab
                    // stays, to restart it, even if the shell took the hangup as a clean exit.
                    let exit_status = exit_status.filter(|_| !live.is_closed());
                    let _ = channel.send(TermFrame::new(FRAME_CLOSED, b""));
                    emit_state(&app, &session_id, None, SessionState::Disconnected, |e| {
                        e.exit_status = exit_status
                    });
                    break;
                }
            }
        }
        crate::state::state(&app)
            .local
            .remove_if_same(&session_id, &live);
        live.close();
    });
}

#[tauri::command]
#[specta::specta]
pub async fn local_write(
    state: State<'_, AppState>,
    session_id: String,
    data: String,
) -> AppResult<()> {
    write(&state, &session_id, data.into_bytes())
}

/// Writes bytes as they are, for xterm's binary mouse reports (see `ssh_write_bytes`).
#[tauri::command]
#[specta::specta]
pub async fn local_write_bytes(
    state: State<'_, AppState>,
    session_id: String,
    data: Vec<u8>,
) -> AppResult<()> {
    write(&state, &session_id, data)
}

fn write(state: &AppState, session_id: &str, data: Vec<u8>) -> AppResult<()> {
    // SEC-03: like SSH sessions, local shells take no input behind the lock screen.
    if !state.vault().is_unlocked() {
        return Err(AppError::locked());
    }
    state
        .local
        .get(session_id)?
        .pty
        .write(data)
        .map_err(AppError::local)
}

#[tauri::command]
#[specta::specta]
pub async fn local_resize(
    state: State<'_, AppState>,
    session_id: String,
    cols: u32,
    rows: u32,
) -> AppResult<()> {
    let live = state.local.get(&session_id)?;
    live.pty.resize(PtySize {
        cols: u16::try_from(cols.clamp(10, 1000)).unwrap_or(80),
        rows: u16::try_from(rows.clamp(2, 500)).unwrap_or(24),
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn local_close(state: State<'_, AppState>, session_id: String) -> AppResult<()> {
    if let Some(live) = state.local.take(&session_id) {
        live.close();
    }
    Ok(())
}
