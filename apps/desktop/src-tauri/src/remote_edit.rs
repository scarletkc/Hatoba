//! SFTP-05: remote files edited in a local editor.
//!
//! Opening a file downloads it into a folder of its own under `edits` in the app's data folder,
//! keeping its name, and opens the copy in the editor. A worker per file checks the copy four
//! times a second, and once a save has not changed for half a second, uploads it over the
//! connection of the tab the file was opened from. Before each upload it compares the remote
//! file's size and modification time with what it last downloaded or uploaded, so a change made
//! on the server meanwhile is never overwritten without asking. Editing ends with Stop Editing,
//! with the tab, or when Hatoba quits, and the copy is deleted then; the lock only pauses it. The
//! copies a crash leaves are deleted at the next start.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use hatoba_ssh::{RemoteStat, SftpClient};
use sha2::{Digest, Sha256};
use tauri::AppHandle;
use tauri_specta::Event;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, MissedTickBehavior};

use crate::commands::sftp::{client, run_transfer};
use crate::dto::{RemoteEditEvent, RemoteEditState, RemoteEditView, TransferDirection};
use crate::error::{AppError, AppResult, ErrorCode, SshErrorKind};
use crate::state::state as app_state;

/// How often each copy is checked for saves.
const POLL: Duration = Duration::from_millis(250);
/// How long a save must stay unchanged before it uploads, since editors write in steps.
const SETTLE: Duration = Duration::from_millis(500);

#[derive(Default)]
pub struct RemoteEdits {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// `<app data>/edits`, set at startup.
    root: Option<PathBuf>,
    edits: HashMap<String, Arc<Edit>>,
    /// The session of each tab with edits, while it is connected.
    sessions: HashMap<String, String>,
    /// Hatoba may quit: the user answered for the saves that did not upload.
    quit: bool,
}

struct Edit {
    id: String,
    tab_id: String,
    remote: String,
    /// The folder of this file's copy, deleted when editing ends.
    dir: PathBuf,
    copy: PathBuf,
    commands: mpsc::UnboundedSender<Command>,
    view: Mutex<RemoteEditView>,
}

enum Command {
    /// The tab connected again: a save that waited for it uploads.
    Connected,
    /// Uploads the copy now; `overwrite` skips the check for a change on the server.
    Upload {
        overwrite: bool,
        reply: oneshot::Sender<AppResult<()>>,
    },
    /// Downloads the server's file into the copy again.
    Discard {
        reply: oneshot::Sender<AppResult<()>>,
    },
    /// Ends editing, after uploading the copy if `upload` and it changed. Replies whether editing
    /// ended: a save that cannot upload yet keeps it going until it does or the user discards it.
    Stop {
        upload: bool,
        reply: oneshot::Sender<bool>,
    },
    /// Replies whether the copy differs from what the server has, or cannot be read.
    Changed { reply: oneshot::Sender<bool> },
    /// The vault locked: nothing uploads until it unlocks.
    Lock,
    /// The vault unlocked: editing goes on, after asking about saves that did not upload.
    Unlock,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Edit {
    fn view(&self) -> RemoteEditView {
        lock(&self.view).clone()
    }
}

impl RemoteEdits {
    /// Sets the folder the copies go into, and deletes what a crash left there.
    pub fn init(&self, data_dir: &Path) {
        let root = data_dir.join("edits");
        if let Err(e) = std::fs::remove_dir_all(&root)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("could not delete the copies of edited files: {e}");
        }
        lock(&self.inner).root = Some(root);
    }

    /// Deletes every copy as the app quits.
    pub fn shutdown(&self) {
        let root = {
            let mut inner = lock(&self.inner);
            inner.edits.clear();
            inner.root.clone()
        };
        if let Some(root) = root {
            let _ = std::fs::remove_dir_all(root);
        }
    }

    pub fn list(&self) -> Vec<RemoteEditView> {
        let inner = lock(&self.inner);
        let mut views: Vec<_> = inner.edits.values().map(|e| e.view()).collect();
        views.sort_by(|a, b| a.edit_id.cmp(&b.edit_id));
        views
    }

    /// Records the session `tab_id` is connected with, or that it is not; saves that waited for
    /// the connection upload.
    pub fn set_session(&self, tab_id: &str, session_id: Option<String>) {
        let mut inner = lock(&self.inner);
        match session_id {
            Some(id) => inner.sessions.insert(tab_id.to_owned(), id),
            None => inner.sessions.remove(tab_id),
        };
        for edit in inner.edits.values().filter(|e| e.tab_id == tab_id) {
            let _ = edit.commands.send(Command::Connected);
        }
    }

    /// The vault locked: editing pauses, so nothing uploads until the unlock.
    pub fn lock(&self) {
        for edit in lock(&self.inner).edits.values() {
            let _ = edit.commands.send(Command::Lock);
        }
    }

    /// The vault unlocked: editing goes on, and copies with saves that did not upload ask first.
    pub fn unlock(&self) {
        for edit in lock(&self.inner).edits.values() {
            let _ = edit.commands.send(Command::Unlock);
        }
    }

    /// Whether quitting waits for the user to answer for the edits first.
    pub fn holds_quit(&self) -> bool {
        let inner = lock(&self.inner);
        !inner.quit && !inner.edits.is_empty()
    }

    /// The user answered for the edits: Hatoba quits, deleting the copies.
    pub fn allow_quit(&self) {
        lock(&self.inner).quit = true;
    }

    fn get(&self, edit_id: &str) -> AppResult<Arc<Edit>> {
        lock(&self.inner)
            .edits
            .get(edit_id)
            .cloned()
            .ok_or_else(|| AppError::not_found("edit"))
    }

    async fn ask<T>(
        &self,
        edit_id: &str,
        command: impl FnOnce(oneshot::Sender<T>) -> Command,
    ) -> AppResult<T> {
        let (tx, rx) = oneshot::channel();
        self.get(edit_id)?
            .commands
            .send(command(tx))
            .map_err(|_| AppError::not_found("edit"))?;
        rx.await.map_err(|_| AppError::not_found("edit"))
    }

    pub async fn upload(&self, edit_id: &str, overwrite: bool) -> AppResult<()> {
        self.ask(edit_id, |reply| Command::Upload { overwrite, reply })
            .await?
    }

    pub async fn discard(&self, edit_id: &str) -> AppResult<()> {
        self.ask(edit_id, |reply| Command::Discard { reply })
            .await?
    }

    pub async fn stop(&self, edit_id: &str, upload: bool) -> AppResult<bool> {
        match self
            .ask(edit_id, |reply| Command::Stop { upload, reply })
            .await
        {
            // It ended meanwhile.
            Err(e) if e.code == ErrorCode::NotFound => Ok(true),
            other => other,
        }
    }

    /// Whether the copy holds a save the server does not have, also one the worker has not
    /// noticed yet.
    pub async fn changed(&self, edit_id: &str) -> AppResult<bool> {
        match self.ask(edit_id, |reply| Command::Changed { reply }).await {
            Err(e) if e.code == ErrorCode::NotFound => Ok(false),
            other => other,
        }
    }

    pub fn reopen_path(&self, edit_id: &str) -> AppResult<PathBuf> {
        Ok(self.get(edit_id)?.copy.clone())
    }

    fn session_of(&self, tab_id: &str) -> Option<String> {
        lock(&self.inner).sessions.get(tab_id).cloned()
    }

    fn find(&self, tab_id: &str, remote: &str) -> Option<Arc<Edit>> {
        lock(&self.inner)
            .edits
            .values()
            .find(|e| e.tab_id == tab_id && e.remote == remote)
            .cloned()
    }

    fn root(&self) -> AppResult<PathBuf> {
        lock(&self.inner)
            .root
            .clone()
            .ok_or_else(|| AppError::internal("the folder for edited files is not set"))
    }

    fn remove(&self, edit: &Edit) {
        let mut inner = lock(&self.inner);
        inner.edits.remove(&edit.id);
        if !inner.edits.values().any(|e| e.tab_id == edit.tab_id) {
            inner.sessions.remove(&edit.tab_id);
        }
    }
}

/// Opens `path` of the tab's session in the editor, downloading it first unless the tab already
/// edits it. A symlink is resolved, so the file it points to is written back.
pub async fn open(
    app: &AppHandle,
    tab_id: String,
    session_id: String,
    path: String,
    editor: Option<String>,
) -> AppResult<RemoteEditView> {
    let state = app_state(app);
    let edits = &state.edits;
    let sftp = client(&state, &session_id).await?;
    edits.set_session(&tab_id, Some(session_id.clone()));
    let remote = sftp.resolve(&path).await?;
    if let Some(edit) = edits.find(&tab_id, &remote) {
        open_editor(edit.copy.clone(), editor).await?;
        return Ok(edit.view());
    }
    let stat = sftp
        .stat(&remote)
        .await?
        .ok_or_else(|| AppError::not_found("file"))?;
    if stat.is_dir() {
        return Err(AppError::invalid("path", "a folder cannot be edited"));
    }
    if stat.is_special() {
        return Err(AppError::invalid(
            "path",
            "only regular files can be edited",
        ));
    }
    let name = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or("file")
        .to_owned();
    let id = uuid::Uuid::now_v7().simple().to_string();
    let root = edits.root()?;
    let dir = root.join(folder_name(&tab_id)).join(&id[id.len() - 12..]);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(AppError::io)?;
    restrict(&root);
    let copy = dir.join(local_name(&name));
    let downloaded = run_transfer(
        app.clone(),
        session_id.clone(),
        TransferDirection::Download,
        name.clone(),
        Box::new({
            let (sftp, remote, copy) = (sftp.clone(), remote.clone(), copy.clone());
            move |progress, token| {
                Box::pin(async move { sftp.download(&remote, &copy, progress, token).await })
            }
        }),
    )
    .await;
    let content = match downloaded {
        Ok(()) => tokio::fs::read(&copy).await.map_err(AppError::io),
        Err(e) => Err(e.into()),
    };
    let content = match content {
        Ok(content) if state.vault().is_unlocked() => content,
        Ok(_) => {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            return Err(AppError::locked());
        }
        Err(e) => {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            return Err(e);
        }
    };

    let (commands, rx) = mpsc::unbounded_channel();
    let view = RemoteEditView {
        edit_id: id.clone(),
        tab_id: tab_id.clone(),
        name,
        remote_path: remote.clone(),
        state: RemoteEditState::Saved,
        error: None,
        held: false,
    };
    let edit = Arc::new(Edit {
        id: id.clone(),
        tab_id,
        remote,
        dir,
        copy: copy.clone(),
        commands,
        view: Mutex::new(view.clone()),
    });
    {
        let mut inner = lock(&edits.inner);
        // Ending the tab's other edits during the download forgot its session.
        inner
            .sessions
            .entry(edit.tab_id.clone())
            .or_insert(session_id);
        inner.edits.insert(id, edit.clone());
    }
    let worker = Worker {
        app: app.clone(),
        seen: seen(&copy).await,
        edit: edit.clone(),
        stat,
        synced: digest(&content),
        changed_at: None,
        state: RemoteEditState::Saved,
        error: None,
        finishing: false,
        locked: false,
        held: false,
        partial: false,
    };
    tauri::async_runtime::spawn(worker.run(rx));
    if let Err(e) = open_editor(copy, editor).await {
        // An edit with no editor showing it would only wait.
        let _ = edits.stop(&edit.id, false).await;
        return Err(e);
    }
    Ok(view)
}

/// Opens a copy in the editor set in Settings, or the system's app for its kind of file.
pub async fn open_editor(copy: PathBuf, editor: Option<String>) -> AppResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::platform::editor::open(&copy, editor.as_deref())
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
    .map_err(|e| {
        tracing::warn!("no editor opened an edited file: {e}");
        AppError::new(ErrorCode::Editor, e.to_string())
    })
}

/// Keeps the copies out of other local users' reach.
fn restrict(root: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    let _ = root;
}

/// A tab's folder: its id, with anything a file name cannot hold left out.
fn folder_name(tab_id: &str) -> String {
    let name: String = tab_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(64)
        .collect();
    if name.is_empty() { "tab".into() } else { name }
}

/// The remote file's name as a local file name on every platform, so the editor still picks the
/// syntax by its extension: characters Windows refuses become `_`, trailing dots and spaces go,
/// and a device name such as `con` gets a `_` in front.
fn local_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    while out.ends_with(['.', ' ']) {
        out.pop();
    }
    let stem = out.split('.').next().unwrap_or("").to_ascii_lowercase();
    let reserved = matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.as_bytes()[3].is_ascii_digit());
    if out.is_empty() {
        "file".into()
    } else if reserved {
        format!("_{out}")
    } else {
        out
    }
}

fn digest(content: &[u8]) -> [u8; 32] {
    Sha256::digest(content).into()
}

type Seen = Option<(u64, Option<SystemTime>)>;

/// The copy's size and modification time, which change with each save.
async fn seen(copy: &Path) -> Seen {
    let meta = tokio::fs::metadata(copy).await.ok()?;
    Some((meta.len(), meta.modified().ok()))
}

/// Whether the server's file is still the one last downloaded or uploaded.
fn unchanged(now: &RemoteStat, then: &RemoteStat) -> bool {
    now.size == then.size && now.mtime == then.mtime
}

enum Flow {
    Go,
    End,
}

/// One edited file: watches its copy and writes it back, one step at a time.
struct Worker {
    app: AppHandle,
    edit: Arc<Edit>,
    /// The server's file as last downloaded or uploaded, whose size and modification time tell
    /// whether it changed since.
    stat: RemoteStat,
    /// The content the server has.
    synced: [u8; 32],
    /// The copy as last checked.
    seen: Seen,
    /// When the copy last changed, until its save is checked.
    changed_at: Option<Instant>,
    state: RemoteEditState,
    error: Option<String>,
    /// Stop Editing waits for the copy to upload.
    finishing: bool,
    /// The vault is locked: nothing uploads.
    locked: bool,
    /// After the unlock, a save that did not upload waits for the user to upload or discard it.
    held: bool,
    /// A failed upload left the server's file partly written in place: the next one writes it
    /// again without asking about the change.
    partial: bool,
}

impl Worker {
    async fn run(mut self, mut commands: mpsc::UnboundedReceiver<Command>) {
        self.publish();
        let mut tick = tokio::time::interval(POLL);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            let flow = tokio::select! {
                command = commands.recv() => match command {
                    Some(command) => self.handle(command).await,
                    None => Flow::End,
                },
                _ = tick.tick(), if !self.locked && !self.held => self.poll().await,
            };
            if let Flow::End = flow {
                break;
            }
        }
        // Commands sent meanwhile find the edit gone.
        app_state(&self.app).edits.remove(&self.edit);
        drop(commands);
        if let Err(e) = tokio::fs::remove_dir_all(&self.edit.dir).await {
            tracing::debug!("the copy of an edited file stays until the next start: {e}");
        }
        let _ = RemoteEditEvent {
            edit_id: self.edit.id.clone(),
            edit: None,
        }
        .emit(&self.app);
    }

    async fn handle(&mut self, command: Command) -> Flow {
        match command {
            Command::Connected => {
                // A save that failed while the connection was dying uploads too.
                if !self.locked
                    && !self.held
                    && matches!(
                        self.state,
                        RemoteEditState::Pending | RemoteEditState::Failed
                    )
                    && self.changed_at.is_none()
                {
                    return self.upload(false).await.0;
                }
                Flow::Go
            }
            Command::Upload { overwrite, reply } => {
                self.held = false;
                let (flow, result) = self.upload(overwrite).await;
                let _ = reply.send(result);
                flow
            }
            Command::Discard { reply } => {
                let result = self.discard().await;
                let ended = result.is_ok() && self.finishing;
                let _ = reply.send(result);
                if ended { Flow::End } else { Flow::Go }
            }
            Command::Stop {
                upload: false,
                reply,
            } => {
                let _ = reply.send(true);
                Flow::End
            }
            Command::Stop {
                upload: true,
                reply,
            } => {
                self.finishing = true;
                self.held = false;
                // A copy that cannot be read now (an editor replacing it) is kept, as changed.
                let flow = match self.changed().await {
                    Some(false) => Flow::End,
                    _ => self.upload(false).await.0,
                };
                let _ = reply.send(matches!(flow, Flow::End));
                if matches!(flow, Flow::Go) {
                    self.publish();
                }
                flow
            }
            Command::Changed { reply } => {
                let _ = reply.send(self.changed().await != Some(false));
                Flow::Go
            }
            Command::Lock => {
                self.locked = true;
                Flow::Go
            }
            Command::Unlock => {
                self.locked = false;
                // Also a save made while locked, which nothing checked: the user may not have made it.
                if self.changed().await != Some(false) {
                    self.held = true;
                    self.publish();
                }
                Flow::Go
            }
        }
    }

    /// Notices saves: once the copy has not changed for [`SETTLE`], a changed content uploads.
    async fn poll(&mut self) -> Flow {
        let now = seen(&self.edit.copy).await;
        // Missing while an editor replaces it with a new file.
        if now.is_none() {
            return Flow::Go;
        }
        if now != self.seen {
            self.seen = now;
            self.changed_at = Some(Instant::now());
            return Flow::Go;
        }
        if self.changed_at.is_none_or(|at| at.elapsed() < SETTLE) {
            return Flow::Go;
        }
        self.changed_at = None;
        match self.changed().await {
            Some(true) => self.upload(false).await.0,
            Some(false) => {
                // Saved back to what the server has.
                if matches!(
                    self.state,
                    RemoteEditState::Pending | RemoteEditState::Failed
                ) {
                    self.set(RemoteEditState::Saved, None);
                }
                Flow::Go
            }
            None => Flow::Go,
        }
    }

    /// Whether the copy's content differs from the server's; `None` when it cannot be read.
    async fn changed(&self) -> Option<bool> {
        let content = tokio::fs::read(&self.edit.copy).await.ok()?;
        Some(digest(&content) != self.synced)
    }

    /// The tab's SFTP client and session, while the tab is connected.
    async fn client(&self) -> Option<(String, SftpClient)> {
        let state = app_state(&self.app);
        let session_id = state.edits.session_of(&self.edit.tab_id)?;
        let sftp = client(&state, &session_id).await.ok()?;
        Some((session_id, sftp))
    }

    /// Whether the session is gone, so a failed step waits for the tab to connect again.
    fn disconnected(&self, session_id: &str) -> bool {
        match app_state(&self.app).ssh.get(session_id) {
            Ok(live) => live.session.is_closed(),
            Err(_) => true,
        }
    }

    /// Writes the copy back through a temporary file, with the mode the file had, unless the
    /// server's file changed since it was last downloaded or uploaded and `overwrite` is false.
    async fn upload(&mut self, overwrite: bool) -> (Flow, AppResult<()>) {
        // Nothing uploads behind the lock screen, also before the lock's command arrives: a save
        // made there asks after the unlock.
        if !app_state(&self.app).vault().is_unlocked() {
            self.locked = true;
            return (Flow::Go, Err(AppError::locked()));
        }
        let Some((session_id, sftp)) = self.client().await else {
            self.set(RemoteEditState::Pending, None);
            return (Flow::Go, Err(not_connected()));
        };
        // These bytes are what uploads and what is recorded as synced; a save after they were read
        // changes `seen`, and uploads next.
        self.seen = seen(&self.edit.copy).await;
        let content = match tokio::fs::read(&self.edit.copy).await {
            Ok(content) => content,
            Err(e) => {
                self.set(RemoteEditState::Failed, Some(e.to_string()));
                return (Flow::Go, Err(AppError::io(e)));
            }
        };
        self.set(RemoteEditState::Uploading, None);
        let remote = self.edit.remote.clone();
        // The file as it is now, whose mode, owner and group the upload keeps: a chmod made on the
        // server since it opened stays.
        let now = match sftp.stat(&remote).await {
            Ok(now) => now,
            Err(e) => return self.failed(&session_id, e.into()),
        };
        // Changed or deleted on the server.
        let ours = self.partial && now.is_some();
        if !overwrite && !ours && !now.is_some_and(|now| unchanged(&now, &self.stat)) {
            self.set(RemoteEditState::Conflict, None);
            return (Flow::Go, Err(conflict()));
        }
        // A file deleted on the server is made again with the mode it had, owned by the login user.
        let old = now.or(Some(RemoteStat {
            uid: None,
            gid: None,
            ..self.stat
        }));
        let sent = digest(&content);
        let in_place = Arc::new(AtomicBool::new(false));
        let written = run_transfer(
            self.app.clone(),
            session_id.clone(),
            TransferDirection::Upload,
            self.view_name(),
            Box::new({
                let (sftp, remote, in_place) = (sftp.clone(), remote.clone(), in_place.clone());
                move |progress, token| {
                    Box::pin(async move {
                        sftp.write_back(content, &remote, old, &in_place, progress, token)
                            .await
                    })
                }
            }),
        )
        .await;
        self.partial = written.is_err() && (self.partial || in_place.load(Ordering::Relaxed));
        match written {
            Ok(()) => {}
            Err(e) if e.kind == hatoba_ssh::SshErrorKind::Cancelled => {
                self.set(RemoteEditState::Failed, None);
                return (
                    Flow::Go,
                    Err(AppError::new(ErrorCode::Cancelled, "cancelled")),
                );
            }
            Err(e) => return self.failed(&session_id, e.into()),
        }
        self.synced = sent;
        match sftp.stat(&remote).await {
            Ok(Some(now)) => self.stat = now,
            // Read again before the next upload, which then asks.
            _ => self.stat.mtime = None,
        }
        self.set(RemoteEditState::Saved, None);
        if !self.finishing {
            return (Flow::Go, Ok(()));
        }
        // A save made while this upload ran goes up too, before the copy is deleted.
        match self.changed().await {
            Some(false) => (Flow::End, Ok(())),
            _ => Box::pin(self.upload(false)).await,
        }
    }

    fn failed(&mut self, session_id: &str, e: AppError) -> (Flow, AppResult<()>) {
        if self.disconnected(session_id) {
            self.set(RemoteEditState::Pending, None);
        } else {
            self.set(RemoteEditState::Failed, Some(e.detail.clone()));
        }
        (Flow::Go, Err(e))
    }

    /// Downloads the server's file into the copy again, which an editor that watches its files
    /// reloads.
    async fn discard(&mut self) -> AppResult<()> {
        let Some((session_id, sftp)) = self.client().await else {
            return Err(not_connected());
        };
        let remote = self.edit.remote.clone();
        let stat = match sftp.stat(&remote).await {
            Ok(Some(stat)) => stat,
            Ok(None) => return Err(AppError::not_found("file")),
            Err(e) => return Err(e.into()),
        };
        run_transfer(
            self.app.clone(),
            session_id,
            TransferDirection::Download,
            self.view_name(),
            Box::new({
                let (sftp, copy) = (sftp.clone(), self.edit.copy.clone());
                move |progress, token| {
                    Box::pin(async move { sftp.download(&remote, &copy, progress, token).await })
                }
            }),
        )
        .await?;
        let content = tokio::fs::read(&self.edit.copy)
            .await
            .map_err(AppError::io)?;
        self.synced = digest(&content);
        self.stat = stat;
        self.seen = seen(&self.edit.copy).await;
        self.changed_at = None;
        self.held = false;
        self.partial = false;
        self.set(RemoteEditState::Saved, None);
        Ok(())
    }

    fn view_name(&self) -> String {
        lock(&self.edit.view).name.clone()
    }

    fn set(&mut self, state: RemoteEditState, error: Option<String>) {
        self.state = state;
        self.error = error;
        self.publish();
    }

    fn publish(&self) {
        let view = {
            let mut view = lock(&self.edit.view);
            view.state = self.state;
            view.error = self.error.clone();
            view.held = self.held;
            view.clone()
        };
        let _ = RemoteEditEvent {
            edit_id: self.edit.id.clone(),
            edit: Some(view),
        }
        .emit(&self.app);
    }
}

fn not_connected() -> AppError {
    AppError::ssh(SshErrorKind::Disconnected, "the tab is not connected")
}

fn conflict() -> AppError {
    AppError::new(ErrorCode::Sftp, "the file changed on the server")
}

#[cfg(test)]
mod tests {
    use super::{folder_name, local_name};

    #[test]
    fn copies_keep_the_name_where_the_platform_allows_it() {
        assert_eq!(local_name("nginx.conf"), "nginx.conf");
        assert_eq!(local_name(".bashrc"), ".bashrc");
        assert_eq!(local_name("a:b*c?.txt"), "a_b_c_.txt");
        assert_eq!(local_name("notes. "), "notes");
        assert_eq!(local_name("con.txt"), "_con.txt");
        assert_eq!(local_name("COM1"), "_COM1");
        assert_eq!(local_name("compose.yml"), "compose.yml");
        assert_eq!(local_name("..."), "file");
    }

    #[test]
    fn tab_folders_hold_only_safe_characters() {
        assert_eq!(folder_name("tab-3"), "tab-3");
        assert_eq!(folder_name("../x"), "x");
        assert_eq!(folder_name("//"), "tab");
    }
}
