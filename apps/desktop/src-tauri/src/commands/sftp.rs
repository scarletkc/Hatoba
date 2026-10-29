//! SFTP file panel (spec §7.3) on top of a live terminal session, and the remote files it edits
//! in a local editor (SFTP-05).

use std::path::PathBuf;

use hatoba_ssh::{SftpClient, TransferProgress};
use tauri::{AppHandle, State};
use tauri_specta::Event;
use tokio_util::sync::CancellationToken;

use crate::dto::{
    FileEntry, LocalPrefs, RemoteEditView, TransferDirection, TransferProgressEvent, TransferState,
};
use crate::error::{AppError, AppResult};
use crate::remote_edit;
use crate::state::{AppState, state as app_state};

/// The session's SFTP client. SEC-03: sessions stay connected behind the lock screen, but take no
/// input until unlocked, so every SFTP command fails while locked.
pub(crate) async fn client(state: &AppState, session_id: &str) -> AppResult<SftpClient> {
    state.with_unlocked(|_| Ok(()))?;
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

pub(crate) struct Transfer {
    app: AppHandle,
    id: String,
    session_id: String,
    direction: TransferDirection,
    name: String,
    last: std::sync::Mutex<TransferProgress>,
}

const NO_PROGRESS: TransferProgress = TransferProgress {
    bytes: 0,
    total: 0,
    bytes_per_sec: 0,
};

impl Transfer {
    /// Registers a transfer, whose cancel token `transfer_cancel` finds by its id, and shows it.
    fn begin(
        app: AppHandle,
        session_id: String,
        direction: TransferDirection,
        name: String,
    ) -> (std::sync::Arc<Self>, CancellationToken) {
        let id = uuid::Uuid::now_v7().to_string();
        let token = CancellationToken::new();
        app_state(&app).ssh.add_transfer(id.clone(), token.clone());
        let transfer = std::sync::Arc::new(Self {
            app,
            id,
            session_id,
            direction,
            name,
            last: std::sync::Mutex::new(NO_PROGRESS),
        });
        transfer.progress(NO_PROGRESS);
        (transfer, token)
    }

    /// Reports `result` as the transfer's end and forgets its cancel token.
    fn end(&self, token: &CancellationToken, result: &Result<(), hatoba_ssh::SshError>) {
        match result {
            Ok(()) => self.finish(TransferState::Done, None),
            Err(_) if token.is_cancelled() => self.finish(TransferState::Cancelled, None),
            Err(e) => self.finish(TransferState::Failed, Some(e.message.clone())),
        }
        app_state(&self.app).ssh.finish_transfer(&self.id);
    }

    /// The progress callback a [`SftpClient`] transfer takes.
    fn reporter(self: &std::sync::Arc<Self>) -> impl Fn(TransferProgress) + Send + Sync + 'static {
        let t = self.clone();
        move |p: TransferProgress| t.progress(p)
    }

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

type TransferRun = Box<
    dyn FnOnce(
            Box<dyn Fn(TransferProgress) + Send + Sync>,
            CancellationToken,
        ) -> futures::future::BoxFuture<'static, Result<(), hatoba_ssh::SshError>>
        + Send,
>;

/// Starts a transfer in the background and returns its id; progress arrives as
/// `transfer://progress`.
fn spawn_transfer(
    app: AppHandle,
    session_id: String,
    direction: TransferDirection,
    name: String,
    run: TransferRun,
) -> String {
    let (transfer, token) = Transfer::begin(app, session_id, direction, name);
    let id = transfer.id.clone();
    tauri::async_runtime::spawn(async move {
        let result = run(Box::new(transfer.reporter()), token.clone()).await;
        transfer.end(&token, &result);
    });
    id
}

/// A transfer the caller waits for, shown in the transfer list like any other (SFTP-05).
pub(crate) async fn run_transfer(
    app: AppHandle,
    session_id: String,
    direction: TransferDirection,
    name: String,
    run: TransferRun,
) -> Result<(), hatoba_ssh::SshError> {
    let (transfer, token) = Transfer::begin(app, session_id, direction, name);
    let result = run(Box::new(transfer.reporter()), token.clone()).await;
    transfer.end(&token, &result);
    result
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
        Box::new(move |progress, token| {
            Box::pin(async move { sftp.download(&remote_path, &local, progress, token).await })
        }),
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
        Box::new(move |progress, token| {
            Box::pin(async move { sftp.upload(&local, &remote, progress, token).await })
        }),
    ))
}

#[tauri::command]
#[specta::specta]
pub fn transfer_cancel(state: State<'_, AppState>, transfer_id: String) {
    state.ssh.cancel_transfer(&transfer_id);
}

/// SFTP-05: opens `path` in the editor set in Settings, downloading it into a copy that uploads
/// with every save, over the tab's connection.
#[tauri::command]
#[specta::specta]
pub async fn sftp_edit_open(
    app: AppHandle,
    state: State<'_, AppState>,
    tab_id: String,
    session_id: String,
    path: String,
) -> AppResult<RemoteEditView> {
    let editor = state.with_unlocked(|v| Ok(editor(v)))?;
    remote_edit::open(&app, tab_id, session_id, path, editor).await
}

/// SFTP-05: opens an edited file's copy in the editor again.
#[tauri::command]
#[specta::specta]
pub async fn sftp_edit_reopen(state: State<'_, AppState>, edit_id: String) -> AppResult<()> {
    let editor = state.with_unlocked(|v| Ok(editor(v)))?;
    let copy = state.edits.reopen_path(&edit_id)?;
    remote_edit::open_editor(copy, editor).await
}

/// SFTP-05: the session the tab is connected with, or `None`; saves that waited for it upload.
#[tauri::command]
#[specta::specta]
pub fn sftp_edit_session(state: State<'_, AppState>, tab_id: String, session_id: Option<String>) {
    state.edits.set_session(&tab_id, session_id);
}

/// SFTP-05: uploads an edited file now, as a retry, or over a change made on the server when
/// `overwrite`.
#[tauri::command]
#[specta::specta]
pub async fn sftp_edit_upload(
    state: State<'_, AppState>,
    edit_id: String,
    overwrite: bool,
) -> AppResult<()> {
    state.with_unlocked(|_| Ok(()))?;
    state.edits.upload(&edit_id, overwrite).await
}

/// SFTP-05: replaces an edited file's copy with the server's file.
#[tauri::command]
#[specta::specta]
pub async fn sftp_edit_discard(state: State<'_, AppState>, edit_id: String) -> AppResult<()> {
    state.with_unlocked(|_| Ok(()))?;
    state.edits.discard(&edit_id).await
}

/// SFTP-05: stops editing a file, uploading its last save first if `upload`. Returns whether
/// editing ended; a save that cannot upload yet, as behind the lock screen, keeps it going until
/// it does.
#[tauri::command]
#[specta::specta]
pub async fn sftp_edit_stop(
    state: State<'_, AppState>,
    edit_id: String,
    upload: bool,
) -> AppResult<bool> {
    state.edits.stop(&edit_id, upload).await
}

/// SFTP-05: whether an edited file's copy holds a save the server does not have, also one not
/// noticed yet.
#[tauri::command]
#[specta::specta]
pub async fn sftp_edit_changed(state: State<'_, AppState>, edit_id: String) -> AppResult<bool> {
    state.edits.changed(&edit_id).await
}

/// SFTP-05: the files being edited.
#[tauri::command]
#[specta::specta]
pub fn sftp_edit_list(state: State<'_, AppState>) -> Vec<RemoteEditView> {
    state.edits.list()
}

/// SFTP-05: quits once the user has answered for the edits' saves that did not upload.
#[tauri::command]
#[specta::specta]
pub fn sftp_edit_quit(app: AppHandle, state: State<'_, AppState>) {
    state.edits.allow_quit();
    app.exit(0);
}

/// The program set to edit remote files on this device.
fn editor(vault: &hatoba_core::vault::Vault) -> Option<String> {
    let json = vault.local_prefs().ok().flatten()?;
    LocalPrefs::from_stored(&json).sftp_editor
}

#[cfg(test)]
mod tests {
    use hatoba_core::{KdfParams, Vault};

    use super::client;
    use crate::error::ErrorCode;
    use crate::state::AppState;

    #[tokio::test]
    async fn sftp_commands_fail_behind_the_lock_screen() {
        let mut vault = Vault::open_in_memory().unwrap();
        vault
            .create_with_params("correct horse battery staple", KdfParams::for_tests())
            .unwrap();
        let state = AppState::new(vault, false);
        let unlocked = client(&state, "s1").await.err().unwrap();
        assert_ne!(unlocked.code, ErrorCode::Locked);

        state.vault().lock();
        let locked = client(&state, "s1").await.err().unwrap();
        assert_eq!(locked.code, ErrorCode::Locked);
    }
}
