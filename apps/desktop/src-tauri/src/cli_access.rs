//! Other programs use the saved hosts through the `hatoba` command (HOST-15): `hatoba hosts`
//! lists them, `hatoba exec` runs a command on one, `hatoba cp` copies a file to or from one.
//! The command asks this app over a local endpoint (`hatoba_cli::endpoint`), which only the user
//! can open; the app signs in with the vault's credentials and sends back only what the command
//! printed or the file it copied. It listens while Settings → General has both the command and
//! this setting on. The vault has to be unlocked, and the lock stops what runs.
//!
//! Questions a connection has for the user (an untrusted host key, a password that is not saved, a
//! code) appear in the window as they do for a terminal, which comes to the front, and the command
//! says what it waits for; `--no-prompt` fails instead.

mod pool;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use hatoba_cli::SWITCH;
use hatoba_cli::protocol::{
    AuthKind, End, Frame, HEADER, HostInfo, Op, Progress, Request, VERSION,
};
use hatoba_core::model::{Host, HostAuth, Item};
use hatoba_core::vault::Vault;
use hatoba_ssh::remote_command::{self, Output, Stopped};
use hatoba_ssh::{SftpClient, SshError, SshSession, TransferProgress};
use tauri::AppHandle;
use tauri_specta::Event;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::dto::{
    CliActivity, CliActivityEvent, CliActivityKind, CliActivityState, HostKeyPromptKind,
};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::ssh::{self, Asking, OnAsk, Overrides, Unattended};
use crate::state::{now_ms, state};

/// How long the command may take to send its request once connected.
const REQUEST_WAIT: Duration = Duration::from_secs(10);
/// How often idle connections are looked for.
const IDLE_CHECK: Duration = Duration::from_secs(60);
/// How many uses Settings lists.
const ACTIVITY: usize = 20;

const LOCKED: &str = "Hatoba is locked. Unlock it, then run the command again.";

#[derive(Default)]
pub struct CliAccess {
    /// Stops listening; `None` while the setting is off.
    listener: Mutex<Option<CancellationToken>>,
    /// Ends with the lock and when the setting goes off: what runs then stops.
    epoch: Mutex<CancellationToken>,
    pool: pool::Pool,
    /// Newest first.
    activity: Mutex<VecDeque<CliActivity>>,
    next_id: AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl CliAccess {
    fn epoch(&self) -> CancellationToken {
        lock(&self.epoch).clone()
    }

    /// Stops every command and copy, and closes the connections.
    fn end_epoch(&self) {
        std::mem::take(&mut *lock(&self.epoch)).cancel();
        self.pool.close_all();
    }

    /// SEC-03: nothing runs behind the lock screen, no connection stays, and the list of what ran
    /// is forgotten.
    pub fn lock(&self, app: &AppHandle) {
        self.end_epoch();
        lock(&self.activity).clear();
        self.emit(app);
    }

    pub fn activity(&self) -> Vec<CliActivity> {
        lock(&self.activity).iter().cloned().collect()
    }

    fn emit(&self, app: &AppHandle) {
        let _ = CliActivityEvent {
            entries: self.activity(),
        }
        .emit(app);
    }

    fn begin(
        &self,
        app: &AppHandle,
        kind: CliActivityKind,
        host: &Target,
        detail: String,
    ) -> String {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed).to_string();
        {
            let mut activity = lock(&self.activity);
            activity.push_front(CliActivity {
                id: id.clone(),
                kind,
                host_id: host.id.clone(),
                host_name: host.name.clone(),
                detail,
                started_at: now_ms(),
                state: CliActivityState::Running,
                status: None,
                error: None,
            });
            activity.truncate(ACTIVITY);
        }
        self.emit(app);
        id
    }

    fn finish(&self, app: &AppHandle, id: &str, end: &End) {
        {
            let mut activity = lock(&self.activity);
            // Gone after a lock.
            let Some(entry) = activity.iter_mut().find(|e| e.id == id) else {
                return;
            };
            entry.state = if end.error.is_some() {
                CliActivityState::Failed
            } else {
                CliActivityState::Done
            };
            entry.status = end.status;
            entry.error = end.error.clone();
        }
        self.emit(app);
    }
}

/// Starts or stops listening as the device's prefs say (`commands::cli::access_on`).
pub fn sync(app: &AppHandle) {
    let state = state(app);
    let on = crate::commands::cli::access_on(&state.vault());
    let mut listener = lock(&state.cli.listener);
    if on == listener.is_some() {
        return;
    }
    if !on {
        if let Some(stop) = listener.take() {
            stop.cancel();
        }
        drop(listener);
        state.cli.end_epoch();
        return;
    }
    let stop = CancellationToken::new();
    *listener = Some(stop.clone());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = listen(&app, &stop).await {
            tracing::warn!("the hatoba command cannot reach Hatoba: listening failed: {e}");
            // `sync` cancels the token under this lock before it starts another listener.
            let state = crate::state::state(&app);
            let mut listener = lock(&state.cli.listener);
            if !stop.is_cancelled() {
                listener.take();
            }
        }
    });
}

#[cfg(windows)]
async fn listen(app: &AppHandle, stop: &CancellationToken) -> std::io::Result<()> {
    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

    let name = hatoba_cli::endpoint::endpoint(crate::CLI_APP.identifier)?;
    let security = hatoba_cli::endpoint::OwnerOnly::new()?;
    let create = |first: bool| -> std::io::Result<NamedPipeServer> {
        // SAFETY: the security attributes live in `security`, past the call.
        unsafe {
            ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(&name, security.as_ptr())
        }
    };
    let mut server = create(true)?;
    let mut idle = tokio::time::interval(IDLE_CHECK);
    loop {
        tokio::select! {
            () = stop.cancelled() => return Ok(()),
            _ = idle.tick() => state(app).cli.pool.close_idle(pool::IDLE),
            connected = server.connect() => {
                let client = std::mem::replace(&mut server, create(false)?);
                match connected {
                    Ok(()) => {
                        tauri::async_runtime::spawn(serve(app.clone(), client));
                    }
                    // A command that was killed before the app took it, which leaves the
                    // instance unusable but the others fine.
                    Err(e) => tracing::debug!("a hatoba command did not connect: {e}"),
                }
            }
        }
    }
}

#[cfg(unix)]
async fn listen(app: &AppHandle, stop: &CancellationToken) -> std::io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let path = hatoba_cli::endpoint::endpoint(crate::CLI_APP.identifier)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Left by an app that did not quit cleanly; only one Hatoba runs at a time.
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let bound = std::fs::metadata(&path)?;
    let owner = bound.uid();
    let mut idle = tokio::time::interval(IDLE_CHECK);
    let result = loop {
        tokio::select! {
            () = stop.cancelled() => break Ok(()),
            _ = idle.tick() => state(app).cli.pool.close_idle(pool::IDLE),
            accepted = listener.accept() => match accepted {
                // Another user's process, which the socket's mode keeps out already.
                Ok((stream, _)) if stream.peer_cred().is_ok_and(|c| c.uid() == owner) => {
                    tauri::async_runtime::spawn(serve(app.clone(), stream));
                }
                Ok(_) => {}
                Err(e) => break Err(e),
            }
        }
    };
    // A listener that started after this one stopped has its own socket there.
    if std::fs::metadata(&path).is_ok_and(|m| m.ino() == bound.ino() && m.dev() == bound.dev()) {
        let _ = std::fs::remove_file(&path);
    }
    result
}

/// One `hatoba` command: its request, then the frames that answer it, the last one
/// [`Frame::End`]. Closing its end of the endpoint, as a command that is stopped or killed does,
/// stops what it asked for.
async fn serve<S>(app: AppHandle, stream: S)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut reader, mut writer) = tokio::io::split(stream);
    let request = match tokio::time::timeout(REQUEST_WAIT, read_frame(&mut reader)).await {
        Ok(Ok(Some(Frame::Request(request)))) => request,
        _ => return,
    };
    let gone = CancellationToken::new();
    let watch = tauri::async_runtime::spawn({
        let gone = gone.clone();
        async move {
            let mut buf = [0u8; 256];
            while matches!(reader.read(&mut buf).await, Ok(n) if n > 0) {}
            gone.cancel();
        }
    });
    let (tx, mut rx) = mpsc::channel::<Frame>(64);
    let write = {
        let gone = gone.clone();
        async move {
            while let Some(frame) = rx.recv().await {
                let end = matches!(frame, Frame::End(_));
                if writer.write_all(&frame.encode()).await.is_err() {
                    gone.cancel();
                    break;
                }
                if end {
                    break;
                }
            }
            let _ = writer.shutdown().await;
        }
    };
    let work = async {
        let end = handle(&app, request, &tx, &gone).await;
        let _ = tx.send(Frame::End(end)).await;
    };
    tokio::join!(work, write);
    watch.abort();
}

async fn read_frame(from: &mut (impl AsyncRead + Unpin)) -> std::io::Result<Option<Frame>> {
    let mut header = [0u8; HEADER];
    match from.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let (kind, len) = Frame::header(header)?;
    let mut payload = vec![0u8; len];
    from.read_exact(&mut payload).await?;
    Frame::decode(kind, payload).map(Some)
}

async fn handle(
    app: &AppHandle,
    request: Request,
    tx: &mpsc::Sender<Frame>,
    gone: &CancellationToken,
) -> End {
    if request.version != VERSION {
        return End::error(
            "this hatoba command comes from another version of Hatoba. Quit and reopen Hatoba, \
             which updates it, then run it again.",
        );
    }
    let state = state(app);
    // Taken before the vault is checked, so a lock that comes in between still stops what follows.
    let epoch = state.cli.epoch();
    if !state.vault().is_unlocked() {
        return End::error(LOCKED);
    }
    let prompt = match &request.op {
        Op::Hosts => false,
        Op::Exec { prompt, .. } | Op::Upload { prompt, .. } | Op::Download { prompt, .. } => {
            *prompt
        }
    };
    let ctx = Ctx {
        app,
        tx,
        epoch,
        gone,
        prompt,
    };
    match request.op {
        Op::Hosts => match state.with_unlocked(|v| Ok(host_list(v))) {
            Ok(hosts) => {
                let _ = tx.send(Frame::Hosts(hosts)).await;
                End::ok()
            }
            Err(_) => End::error(LOCKED),
        },
        Op::Exec {
            host, command, cwd, ..
        } => exec(&ctx, &host, command, cwd).await,
        Op::Upload {
            host,
            local,
            remote,
            ..
        } => upload(&ctx, &host, local, remote).await,
        Op::Download {
            host,
            remote,
            local,
            ..
        } => download(&ctx, &host, remote, local).await,
    }
}

/// The saved hosts, by name, without their credentials.
pub(crate) fn host_list(vault: &Vault) -> Vec<HostInfo> {
    let mut hosts: Vec<HostInfo> = vault
        .hosts()
        .into_iter()
        .map(|(id, h)| HostInfo {
            auth: match &h.auth {
                HostAuth::Password { password } if !password.is_empty() => AuthKind::Password,
                HostAuth::Password { .. } | HostAuth::Ask => AuthKind::Ask,
                HostAuth::Key { .. } => AuthKind::Key,
                HostAuth::Agent => AuthKind::Agent,
            },
            group: h
                .group_id
                .as_deref()
                .and_then(|g| vault.get(g))
                .and_then(Item::as_group)
                .map(|g| g.name.clone()),
            jump: h
                .jump_host_id
                .as_deref()
                .and_then(|j| vault.get(j))
                .and_then(Item::as_host)
                .map(|j| j.name.clone()),
            id,
            name: h.name,
            address: h.address,
            port: h.port,
            username: h.username,
            tags: h.tags,
            ai_notes: h.ai_notes,
        })
        .collect();
    hosts.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    hosts
}

/// The saved host a command names: by its id, or by its name as `hatoba <name>` finds it
/// (HOST-13), which several hosts may share.
pub(crate) fn find_host(vault: &Vault, wanted: &str) -> Result<(String, Host), String> {
    if let Some(host) = vault.get(wanted).and_then(Item::as_host) {
        return Ok((wanted.to_owned(), host.clone()));
    }
    let key = wanted.trim().to_lowercase();
    let mut named: Vec<(String, Host)> = vault
        .hosts()
        .into_iter()
        .filter(|(_, h)| h.name.trim().to_lowercase() == key)
        .collect();
    match named.len() {
        1 => Ok(named.remove(0)),
        0 => Err(format!(
            "no saved host is named {wanted}. Run hatoba hosts to see their names."
        )),
        n => {
            named.sort_by(|a, b| a.0.cmp(&b.0));
            let ids: Vec<&str> = named.iter().map(|(id, _)| id.as_str()).collect();
            Err(format!(
                "{n} saved hosts are named {wanted}. Name one by its id instead: {}.",
                ids.join(", ")
            ))
        }
    }
}

/// The host a request is for.
struct Target {
    id: String,
    name: String,
    /// How it connects now (`dashboard::config_version`).
    config: String,
}

/// What a request runs with.
struct Ctx<'a> {
    app: &'a AppHandle,
    tx: &'a mpsc::Sender<Frame>,
    epoch: CancellationToken,
    /// The command closed its end.
    gone: &'a CancellationToken,
    /// Questions for the user go to the window; otherwise a connection that has one fails.
    prompt: bool,
}

impl Ctx<'_> {
    fn target(&self, wanted: &str) -> Result<Target, String> {
        state(self.app)
            .with_unlocked(|v| {
                Ok(find_host(v, wanted).map(|(id, host)| Target {
                    config: crate::dashboard::config_version(v, &host),
                    name: host.name,
                    id,
                }))
            })
            .unwrap_or_else(|_| Err(LOCKED.to_owned()))
    }

    async fn stopped(&self) {
        tokio::select! {
            () = self.epoch.cancelled() => {}
            () = self.gone.cancelled() => {}
        }
    }

    /// Why `what` stopped before its end.
    fn stop_message(&self, what: &str) -> String {
        if !self.epoch.is_cancelled() {
            format!("the hatoba command ended, which stopped {what}.")
        } else if state(self.app).vault().is_unlocked() {
            // Turning off the command itself, the switch above, ends it too.
            format!(
                "\"{SWITCH}\" or the hatoba command was turned off in Hatoba, which stopped {what}."
            )
        } else {
            format!("Hatoba locked, which stopped {what}.")
        }
    }

    fn begin(&self, kind: CliActivityKind, target: &Target, detail: String) -> String {
        state(self.app).cli.begin(self.app, kind, target, detail)
    }

    fn finish(&self, id: &str, end: &End) {
        state(self.app).cli.finish(self.app, id, end);
    }

    /// The host's connection: a terminal's that connects the way the host does now, as the
    /// dashboard takes it (HOST-14), or the pool's.
    async fn lease(&self, target: &Target) -> Result<pool::Lease, String> {
        let state = state(self.app);
        if let Some(session) = state.connections.session_for(&target.id, &target.config) {
            // The pool's own connection to the host goes once the commands on it are done.
            state.cli.pool.release(&target.id);
            return Ok(pool::Lease::borrowed(session));
        }
        let leasing = state
            .cli
            .pool
            .lease(&target.id, &target.config, &self.epoch, || {
                self.connect(target)
            });
        tokio::select! {
            leased = leasing => leased.map_err(|e| {
                if self.epoch.is_cancelled() {
                    self.stop_message("the connection")
                } else {
                    e.detail
                }
            }),
            () = self.stopped() => Err(self.stop_message("the connection")),
        }
    }

    /// Connects to the host. The error's detail is the whole message for the command.
    async fn connect(&self, target: &Target) -> AppResult<SshSession> {
        let overrides = Overrides {
            ask_password: self.prompt,
            ..Overrides::default()
        };
        let cfg = state(self.app)
            .with_unlocked(|v| {
                let host = v
                    .get(&target.id)
                    .and_then(Item::as_host)
                    .cloned()
                    .ok_or_else(|| AppError::not_found("host"))?;
                ssh::build_config(v, &host, Some(&target.id), Some(&overrides))
            })
            .map_err(|e| AppError::internal(config_error(&e, &target.name)))?;
        if !self.prompt {
            let (result, outcome) = ssh::connect_unattended(self.app, cfg).await;
            return result
                .map_err(|e| AppError::internal(unattended_error(&e, &outcome, &target.name)));
        }
        // Weak, so a connection that keeps its prompt handlers does not keep this request open.
        let tx = self.tx.downgrade();
        let app = self.app.clone();
        let on_ask: OnAsk = Arc::new(move |asking| {
            crate::platform::window::reopen_main(&app);
            let text = match asking {
                Asking::HostKey { target } => {
                    format!("waiting for you to check the host key of {target} in Hatoba")
                }
                Asking::SignIn { target } => {
                    format!("waiting for you to sign in to {target} in Hatoba")
                }
            };
            if let Some(tx) = tx.upgrade() {
                let _ = tx.try_send(Frame::Notice(text));
            }
        });
        ssh::connect_asking(self.app, cfg, None, Some(on_ask))
            .await
            .map_err(|e| {
                AppError::internal(format!("can't connect to {}: {}", target.name, e.detail))
            })
    }

    /// Reports a copy's progress to the command.
    fn progress(&self) -> impl Fn(TransferProgress) + Send + Sync + 'static {
        let tx = self.tx.downgrade();
        move |p| {
            if let Some(tx) = tx.upgrade() {
                let _ = tx.try_send(Frame::Progress(Progress {
                    bytes: p.bytes,
                    total: p.total,
                }));
            }
        }
    }

    /// Runs a copy that `cancel` stops, until it ends or the request stops; a stopped copy still
    /// runs to its end, which removes what it left.
    async fn copy_until_stopped(
        &self,
        copying: impl Future<Output = Result<(), SshError>>,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        tokio::pin!(copying);
        let result = tokio::select! {
            result = &mut copying => result,
            () = self.stopped() => {
                cancel.cancel();
                copying.await
            }
        };
        result.map_err(|e| {
            if cancel.is_cancelled() {
                self.stop_message("the copy")
            } else {
                e.message
            }
        })
    }

    async fn sftp(&self, lease: &pool::Lease, target: &Target) -> Result<SftpClient, String> {
        lease
            .session
            .sftp()
            .await
            .map_err(|e| format!("can't copy files with {}: {}", target.name, e.message))
    }
}

/// Why a connection could not be set up from the vault.
fn config_error(e: &AppError, name: &str) -> String {
    match (&e.code, e.field.as_deref()) {
        (ErrorCode::Locked, _) => LOCKED.to_owned(),
        (_, Some("password")) => format!(
            "{name} has no saved password. Run the command without --no-prompt to enter it in \
             Hatoba."
        ),
        _ => format!("can't connect to {name}: {}", e.detail),
    }
}

/// Why a connection that may ask nothing failed.
fn unattended_error(e: &AppError, outcome: &Unattended, name: &str) -> String {
    match outcome.host_key() {
        Some(HostKeyPromptKind::New) => format!(
            "the host key of {name}, or of a jump host on the way, isn't trusted yet. Run the \
             command without --no-prompt to check it in Hatoba."
        ),
        Some(HostKeyPromptKind::Changed) => format!(
            "the host key of {name}, or of a jump host on the way, has changed. Run the command \
             without --no-prompt to check it in Hatoba."
        ),
        None if outcome.prompted() && !outcome.password_prompted() => format!(
            "{name} asks for a code or a password that isn't saved. Run the command without \
             --no-prompt to answer it in Hatoba."
        ),
        None => format!("can't connect to {name}: {}", e.detail),
    }
}

async fn exec(ctx: &Ctx<'_>, host: &str, command: String, cwd: Option<String>) -> End {
    let target = match ctx.target(host) {
        Ok(target) => target,
        Err(message) => return End::error(message),
    };
    let id = ctx.begin(CliActivityKind::Exec, &target, command.clone());
    let end = run_exec(ctx, &target, &command, cwd.as_deref()).await;
    ctx.finish(&id, &end);
    end
}

async fn run_exec(ctx: &Ctx<'_>, target: &Target, command: &str, cwd: Option<&str>) -> End {
    let lease = match ctx.lease(target).await {
        Ok(lease) => lease,
        Err(message) => return End::error(message),
    };
    if cwd.is_some() && lease.session.server_os() == Some(hatoba_ssh::ServerOs::Windows) {
        return End::error(format!(
            "{} runs commands with cmd.exe or PowerShell, so --cwd cannot start one in another \
             directory. Leave out --cwd and change the directory in the command.",
            target.name
        ));
    }
    let (output, mut pieces) = mpsc::channel(16);
    let run = remote_command::run_streaming(&lease.session, command, cwd, ctx.stopped(), output);
    let pass_on = async {
        while let Some(piece) = pieces.recv().await {
            let frame = match piece {
                Output::Stdout(bytes) => Frame::Stdout(bytes.to_vec()),
                Output::Stderr(bytes) => Frame::Stderr(bytes.to_vec()),
            };
            // Without a reader, the command is stopping already.
            let _ = ctx.tx.send(frame).await;
        }
    };
    let (ran, ()) = tokio::join!(run, pass_on);
    match ran {
        Ok(run) => match (run.stopped, run.output.exit_status) {
            (None, Some(status)) => End::status(i32::try_from(status).unwrap_or(i32::MAX)),
            (None, None) => {
                End::error("the command ended without an exit status, as when a signal ends it")
            }
            (Some(stopped), _) => {
                let mut message = ctx.stop_message("the command");
                if stopped == Stopped::MayRun {
                    message.push_str(" It may still be running on the host.");
                }
                End::error(message)
            }
        },
        Err(e) if lease.session.is_closed() => End::error(format!(
            "the connection to {} was lost: {}",
            target.name, e.message
        )),
        Err(e) => End::error(e.message),
    }
}

async fn upload(ctx: &Ctx<'_>, host: &str, local: String, remote: String) -> End {
    let target = match ctx.target(host) {
        Ok(target) => target,
        Err(message) => return End::error(message),
    };
    let id = ctx.begin(
        CliActivityKind::Upload,
        &target,
        format!("{local} → {remote}"),
    );
    let end = match run_upload(ctx, &target, Path::new(&local), &remote).await {
        Ok(()) => End::ok(),
        Err(message) => End::error(message),
    };
    ctx.finish(&id, &end);
    end
}

async fn run_upload(
    ctx: &Ctx<'_>,
    target: &Target,
    local: &Path,
    remote: &str,
) -> Result<(), String> {
    let meta = std::fs::metadata(local).map_err(|e| format!("{}: {e}", local.display()))?;
    if meta.is_dir() {
        return Err(format!(
            "{} is a folder. hatoba cp copies one file at a time.",
            local.display()
        ));
    }
    let name = local
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("{} has no file name", local.display()))?
        .to_owned();
    let lease = ctx.lease(target).await?;
    let sftp = ctx.sftp(&lease, target).await?;
    let result = async {
        let remote = remote_target(&sftp, remote, &name).await?;
        let cancel = CancellationToken::new();
        let copying = sftp.upload(local, &remote, ctx.progress(), cancel.clone());
        ctx.copy_until_stopped(copying, &cancel).await
    }
    .await;
    sftp.close().await;
    result
}

async fn download(ctx: &Ctx<'_>, host: &str, remote: String, local: String) -> End {
    let target = match ctx.target(host) {
        Ok(target) => target,
        Err(message) => return End::error(message),
    };
    let id = ctx.begin(
        CliActivityKind::Download,
        &target,
        format!("{remote} → {local}"),
    );
    let end = match run_download(ctx, &target, &remote, PathBuf::from(&local)).await {
        Ok(()) => End::ok(),
        Err(message) => End::error(message),
    };
    ctx.finish(&id, &end);
    end
}

async fn run_download(
    ctx: &Ctx<'_>,
    target: &Target,
    remote: &str,
    mut local: PathBuf,
) -> Result<(), String> {
    let lease = ctx.lease(target).await?;
    let sftp = ctx.sftp(&lease, target).await?;
    let result = async {
        let remote = expand_home(&sftp, remote).await?;
        if local.is_dir() {
            let name = remote.rsplit('/').find(|n| !n.is_empty()).ok_or_else(|| {
                format!("{remote} is a folder. hatoba cp copies one file at a time.")
            })?;
            local.push(name);
        }
        let cancel = CancellationToken::new();
        let copying = sftp.download(&remote, &local, ctx.progress(), cancel.clone());
        ctx.copy_until_stopped(copying, &cancel).await
    }
    .await;
    sftp.close().await;
    result
}

/// A remote path with `~` and `~/…` from the home directory. An empty one is the home directory,
/// and any other relative one is left for the server, which takes it from there too.
async fn expand_home(sftp: &SftpClient, path: &str) -> Result<String, String> {
    let rest = match path {
        "" | "~" => "",
        _ => match path.strip_prefix("~/") {
            Some(rest) => rest,
            None => return Ok(path.to_owned()),
        },
    };
    let home = sftp.home().await.map_err(|e| e.message)?;
    Ok(if rest.is_empty() {
        home
    } else {
        join_remote(&home, rest)
    })
}

/// Where an upload goes: into `path` when it is a folder or ends with `/`, under the file's name.
async fn remote_target(sftp: &SftpClient, path: &str, name: &str) -> Result<String, String> {
    let path = expand_home(sftp, path).await?;
    Ok(
        if path.ends_with('/') || sftp.is_dir(&path).await == Some(true) {
            join_remote(&path, name)
        } else {
            path
        },
    )
}

fn join_remote(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}
