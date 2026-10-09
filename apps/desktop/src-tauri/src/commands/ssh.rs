//! Terminal sessions (spec §7.1, §7.2, §10.3), including quick connect (HOST-12).

use std::sync::Arc;

use hatoba_core::model::Item;
use hatoba_ssh::{AuthMethod, ConnectConfig, ShellEvent, ShellOptions};
use tauri::ipc::{Channel, InvokeResponseBody, IpcResponse};
use tauri::{AppHandle, State};
use tauri_specta::Event;
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use crate::commands::hosts::host_from_input;
use crate::commands::quick;
use crate::dto::{
    ConnectOptions, HostInput, QuickTarget, SessionState, SessionStateEvent, TestResult,
};
use crate::error::{AppError, AppResult};
use crate::ssh::{LiveSession, Overrides, build_config, connect_with};
use crate::state::{AppState, now_ms};

const FRAME_DATA: u8 = 0;
const FRAME_CLOSED: u8 = 1;
const FRAME_ERROR: u8 = 2;

/// One terminal frame on the per-session channel: tag byte + payload. Sent through Tauri's raw
/// binary IPC path (an `ArrayBuffer` in the WebView), never as a JSON number array (§10.3).
#[derive(specta::Type)]
pub struct TermFrame(#[specta(type = Vec<u8>)] Vec<u8>);

impl TermFrame {
    fn new(tag: u8, payload: &[u8]) -> Self {
        let mut buf = Vec::with_capacity(payload.len() + 1);
        buf.push(tag);
        buf.extend_from_slice(payload);
        Self(buf)
    }
}

impl IpcResponse for TermFrame {
    fn body(self) -> tauri::Result<InvokeResponseBody> {
        Ok(InvokeResponseBody::Raw(self.0))
    }
}

fn emit_state(
    app: &AppHandle,
    session_id: &str,
    host_id: Option<&str>,
    state: SessionState,
    extra: impl FnOnce(&mut SessionStateEvent),
) {
    let mut event = SessionStateEvent {
        session_id: session_id.to_owned(),
        host_id: host_id.map(str::to_owned),
        state,
        latency_ms: None,
        error: None,
        exit_status: None,
    };
    extra(&mut event);
    let _ = event.emit(app);
}

fn overrides(options: &ConnectOptions) -> Overrides {
    Overrides {
        password: options
            .password
            .clone()
            .filter(|p| !p.is_empty())
            .map(Zeroizing::new),
        passphrase: options
            .passphrase
            .clone()
            .filter(|p| !p.is_empty())
            .map(Zeroizing::new),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn ssh_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    host_id: String,
    options: ConnectOptions,
    channel: Channel<TermFrame>,
) -> AppResult<String> {
    let session_id = uuid::Uuid::now_v7().to_string();
    let cfg = state.with_unlocked(|v| {
        let host = v
            .get(&host_id)
            .and_then(Item::as_host)
            .cloned()
            .ok_or_else(|| AppError::not_found("host"))?;
        build_config(v, &host, Some(&host_id), Some(&overrides(&options)))
    })?;
    let (live, events) =
        open_terminal(&app, &state, cfg, &session_id, Some(&host_id), &options).await?;
    crate::commands::forwards::start_auto(&app, &session_id, &live, &host_id).await;
    pump(
        app,
        session_id.clone(),
        Some(host_id),
        live,
        events,
        channel,
    );
    Ok(session_id)
}

/// Quick connect (HOST-12): a terminal on a target typed into the hosts search field, which is
/// not saved as a host. Authentication is the ssh-agent, then the server's keyboard-interactive
/// prompts or a password asked once (SSH-03, SSH-08, SSH-09); the host key goes through the
/// usual check and is saved like any other (SSH-04).
#[tauri::command]
#[specta::specta]
pub async fn ssh_connect_target(
    app: AppHandle,
    state: State<'_, AppState>,
    target: QuickTarget,
    options: ConnectOptions,
    channel: Channel<TermFrame>,
) -> AppResult<String> {
    let target = quick::validate(target)?;
    // Known hosts live in the vault (SSH-04).
    state.with_unlocked(|_| Ok(()))?;
    let session_id = uuid::Uuid::now_v7().to_string();
    let cfg = ConnectConfig::new(
        target.address.clone(),
        target.port,
        target.username.clone(),
        AuthMethod::AgentThenAsk,
    );
    let (live, events) = open_terminal(&app, &state, cfg, &session_id, None, &options).await?;
    if let Err(e) = state.with_unlocked(|v| quick::remember(v, &target)) {
        tracing::debug!("recent target not recorded: {}", e.detail);
    }
    pump(app, session_id.clone(), None, live, events, channel);
    Ok(session_id)
}

/// Connects, opens the shell and registers the session, reporting each step as an
/// `ssh://state` event.
async fn open_terminal(
    app: &AppHandle,
    state: &AppState,
    cfg: ConnectConfig,
    session_id: &str,
    host_id: Option<&str>,
    options: &ConnectOptions,
) -> AppResult<(Arc<LiveSession>, mpsc::Receiver<ShellEvent>)> {
    emit_state(app, session_id, host_id, SessionState::Connecting, |_| {});
    let fail = |err: AppError| {
        emit_state(app, session_id, host_id, SessionState::Failed, |e| {
            e.error = Some(err.clone())
        });
        err
    };
    let session = connect_with(app, cfg, Some(session_id.to_owned()))
        .await
        .map_err(fail)?;
    let shell_opts = ShellOptions {
        cols: options.cols.clamp(10, 1000),
        rows: options.rows.clamp(2, 500),
        ..ShellOptions::default()
    };
    let (shell, events) = match session.open_shell(shell_opts).await {
        Ok(v) => v,
        Err(e) => {
            session.disconnect().await;
            return Err(fail(e.into()));
        }
    };
    let live = Arc::new(LiveSession::new(session.clone(), shell));
    state.ssh.insert(session_id.to_owned(), live.clone());
    // HOST-06, HOST-11: device-local, never synced. A quick connection has no host to record on.
    if let Some(host_id) = host_id {
        let os = session.server_os().map(hatoba_ssh::ServerOs::as_str);
        if let Err(e) = state.with_unlocked(|v| {
            v.set_last_connected(host_id, now_ms())?;
            Ok(v.set_host_os(host_id, os)?)
        }) {
            tracing::debug!(
                "last-connected time and server OS not recorded: {}",
                e.detail
            );
        }
    }
    let latency = session.latency_ms();
    emit_state(app, session_id, host_id, SessionState::Connected, |e| {
        e.latency_ms = Some(latency)
    });
    Ok((live, events))
}

/// Relays shell output to the WebView until the shell ends, then drops the session.
fn pump(
    app: AppHandle,
    session_id: String,
    host_id: Option<String>,
    live: Arc<LiveSession>,
    mut events: mpsc::Receiver<ShellEvent>,
    channel: Channel<TermFrame>,
) {
    tauri::async_runtime::spawn(async move {
        let host_id = host_id.as_deref();
        while let Some(event) = events.recv().await {
            match event {
                ShellEvent::Data(bytes) => {
                    if channel.send(TermFrame::new(FRAME_DATA, &bytes)).is_err() {
                        // The WebView went away (reload / window closed): don't leak the session.
                        live.close().await;
                        break;
                    }
                }
                ShellEvent::Closed {
                    reason,
                    exit_status,
                } => {
                    let _ = channel.send(TermFrame::new(FRAME_CLOSED, reason.as_bytes()));
                    emit_state(
                        &app,
                        &session_id,
                        host_id,
                        SessionState::Disconnected,
                        |e| e.exit_status = exit_status,
                    );
                    break;
                }
                ShellEvent::Error(err) => {
                    let _ = channel.send(TermFrame::new(FRAME_ERROR, err.message.as_bytes()));
                    let err: AppError = err.into();
                    emit_state(
                        &app,
                        &session_id,
                        host_id,
                        SessionState::Disconnected,
                        |e| e.error = Some(err),
                    );
                    break;
                }
            }
        }
        crate::state::state(&app)
            .ssh
            .remove_if_same(&session_id, &live);
        live.close().await;
    });
}

#[tauri::command]
#[specta::specta]
pub async fn ssh_write(
    state: State<'_, AppState>,
    session_id: String,
    data: String,
) -> AppResult<()> {
    // SEC-03: sessions stay connected behind the lock screen, but take no input until unlocked.
    if !state.vault().is_unlocked() {
        return Err(AppError::locked());
    }
    let live = state.ssh.get(&session_id)?;
    live.shell.write(data.into_bytes()).await?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn ssh_resize(
    state: State<'_, AppState>,
    session_id: String,
    cols: u32,
    rows: u32,
) -> AppResult<()> {
    let live = state.ssh.get(&session_id)?;
    live.shell
        .resize(cols.clamp(10, 1000), rows.clamp(2, 500))
        .await;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn ssh_disconnect(state: State<'_, AppState>, session_id: String) -> AppResult<()> {
    if let Some(live) = state.ssh.take(&session_id) {
        live.close().await;
    }
    Ok(())
}

/// "Test connection" in the host editor: connect, authenticate, verify the host key, disconnect.
#[tauri::command]
#[specta::specta]
pub async fn ssh_test(
    app: AppHandle,
    state: State<'_, AppState>,
    input: HostInput,
) -> AppResult<TestResult> {
    let cfg = state.with_unlocked(|v| {
        let existing = match &input.id {
            Some(id) => v.get(id).and_then(Item::as_host).cloned(),
            None => None,
        };
        let host = host_from_input(&input, existing.as_ref(), v)?;
        build_config(v, &host, input.id.as_deref(), None)
    })?;
    Ok(match connect_with(&app, cfg, None).await {
        Ok(session) => {
            let latency = session.latency_ms();
            session.disconnect().await;
            TestResult {
                ok: true,
                latency_ms: Some(latency),
                host_key_verified: true,
                error: None,
            }
        }
        Err(e) => TestResult {
            ok: false,
            latency_ms: None,
            host_key_verified: false,
            error: Some(e),
        },
    })
}

#[tauri::command]
#[specta::specta]
pub fn hostkey_respond(state: State<'_, AppState>, request_id: String, accept: bool) {
    state.ssh.answer_hostkey(&request_id, accept);
}

#[tauri::command]
#[specta::specta]
pub fn auth_prompt_respond(
    state: State<'_, AppState>,
    request_id: String,
    answers: Option<Vec<String>>,
) {
    state.ssh.answer_auth(&request_id, answers);
}
