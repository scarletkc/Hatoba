//! SFTP file panel (spec §7.3) on top of a live terminal session.

use std::path::PathBuf;

use hatoba_ssh::{SftpClient, TransferProgress};
use tauri::{AppHandle, State};
use tauri_specta::Event;
use tokio_util::sync::CancellationToken;

use crate::dto::{FileEntry, TransferDirection, TransferProgressEvent, TransferState};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, state as app_state};

async fn client(state: &AppState, session_id: &str) -> AppResult<SftpClient> {
    let live = state.ssh.get(session_id)?;
    let client = live.sftp.get_or_try_init(|| live.session.sftp()).await?;
    Ok(client.clone())
}

fn entry(e: hatoba_ssh::FileEntry) -> FileEntry {
    FileEntry {
        name: e.name,
        path: e.path,
        is_dir: e.is_dir,
        is_symlink: e.is_symlink,
        size: e.size,
        modified: e.modified,
        permissions: e.permissions,
    }
}

fn join_remote(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Rejects names that would escape the target directory or are meaningless on the server.
fn validate_name(name: &str) -> AppResult<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(AppError::invalid("name", "invalid file name"));
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_home(state: State<'_, AppState>, session_id: String) -> AppResult<String> {
    Ok(client(&state, &session_id).await?.home().await?)
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_list(
    state: State<'_, AppState>,
    session_id: String,
    path: String,
) -> AppResult<Vec<FileEntry>> {
    let entries = client(&state, &session_id).await?.list(&path).await?;
    Ok(entries.into_iter().map(entry).collect())
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_rename(
    state: State<'_, AppState>,
    session_id: String,
    from: String,
    to: String,
) -> AppResult<()> {
    if let Some(name) = to.rsplit('/').next() {
        validate_name(name)?;
    }
    Ok(client(&state, &session_id)
        .await?
        .rename(&from, &to)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_remove(
    state: State<'_, AppState>,
    session_id: String,
    path: String,
    is_dir: bool,
) -> AppResult<()> {
    if path.trim_end_matches('/').is_empty() {
        return Err(AppError::invalid(
            "path",
            "refusing to delete the root directory",
        ));
    }
    let sftp = client(&state, &session_id).await?;
    if is_dir {
        sftp.remove_recursive(&path).await?
    } else {
        sftp.remove_file(&path).await?
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_mkdir(
    state: State<'_, AppState>,
    session_id: String,
    path: String,
) -> AppResult<()> {
    if let Some(name) = path.trim_end_matches('/').rsplit('/').next() {
        validate_name(name)?;
    }
    Ok(client(&state, &session_id).await?.mkdir(&path).await?)
}

struct Transfer {
    app: AppHandle,
    id: String,
    session_id: String,
    direction: TransferDirection,
    name: String,
    last: std::sync::Mutex<TransferProgress>,
}

impl Transfer {
    fn progress(&self, p: TransferProgress) {
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = p;
        self.emit(p, TransferState::Running, None);
    }

    fn finish(&self, state: TransferState, error: Option<String>) {
        let p = *self.last.lock().unwrap_or_else(|e| e.into_inner());
        self.emit(p, state, error);
    }

    fn emit(&self, p: TransferProgress, state: TransferState, error: Option<String>) {
        let _ = TransferProgressEvent {
            transfer_id: self.id.clone(),
            session_id: self.session_id.clone(),
            direction: self.direction,
            name: self.name.clone(),
            bytes: p.bytes,
            total: p.total,
            bytes_per_sec: p.bytes_per_sec,
            state,
            error,
        }
        .emit(&self.app);
    }
}

/// Starts a transfer in the background and returns its id; progress arrives as
/// `transfer://progress`.
fn spawn_transfer(
    app: AppHandle,
    session_id: String,
    direction: TransferDirection,
    name: String,
    run: impl FnOnce(
        std::sync::Arc<Transfer>,
        CancellationToken,
    ) -> futures::future::BoxFuture<'static, Result<(), hatoba_ssh::SshError>>
    + Send
    + 'static,
) -> String {
    let id = uuid::Uuid::now_v7().to_string();
    let token = CancellationToken::new();
    app_state(&app).ssh.add_transfer(id.clone(), token.clone());
    let zero = TransferProgress {
        bytes: 0,
        total: 0,
        bytes_per_sec: 0,
    };
    let transfer = std::sync::Arc::new(Transfer {
        app: app.clone(),
        id: id.clone(),
        session_id,
        direction,
        name,
        last: std::sync::Mutex::new(zero),
    });
    tauri::async_runtime::spawn(async move {
        transfer.progress(zero);
        let result = run(transfer.clone(), token.clone()).await;
        match result {
            Ok(()) => transfer.finish(TransferState::Done, None),
            Err(_) if token.is_cancelled() => transfer.finish(TransferState::Cancelled, None),
            Err(e) => transfer.finish(TransferState::Failed, Some(e.message)),
        }
        app_state(&transfer.app).ssh.finish_transfer(&transfer.id);
    });
    id
}

/// SFTP-02 download. `local_path` comes from the native save dialog.
#[tauri::command]
#[specta::specta]
pub async fn sftp_download(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    remote_path: String,
    local_path: String,
) -> AppResult<String> {
    let sftp = client(&state, &session_id).await?;
    let name = remote_path
        .rsplit('/')
        .next()
        .unwrap_or(&remote_path)
        .to_owned();
    let local = PathBuf::from(local_path);
    Ok(spawn_transfer(
        app,
        session_id,
        TransferDirection::Download,
        name,
        move |t, token| {
            Box::pin(async move {
                let progress = {
                    let t = t.clone();
                    move |p: TransferProgress| t.progress(p)
                };
                sftp.download(&remote_path, &local, progress, token).await
            })
        },
    ))
}

/// SFTP-02 upload into `remote_dir` (file picker or drag & drop).
#[tauri::command]
#[specta::specta]
pub async fn sftp_upload(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    local_path: String,
    remote_dir: String,
) -> AppResult<String> {
    let sftp = client(&state, &session_id).await?;
    let local = PathBuf::from(&local_path);
    let name = local
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| AppError::invalid("local_path", "not a file"))?;
    validate_name(&name)?;
    let remote = join_remote(&remote_dir, &name);
    Ok(spawn_transfer(
        app,
        session_id,
        TransferDirection::Upload,
        name,
        move |t, token| {
            Box::pin(async move {
                let progress = {
                    let t = t.clone();
                    move |p: TransferProgress| t.progress(p)
                };
                sftp.upload(&local, &remote, progress, token).await
            })
        },
    ))
}

#[tauri::command]
#[specta::specta]
pub fn transfer_cancel(state: State<'_, AppState>, transfer_id: String) {
    state.ssh.cancel_transfer(&transfer_id);
}
