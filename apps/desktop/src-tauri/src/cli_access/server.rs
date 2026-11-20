//! The local endpoint (`hatoba_cli::endpoint`) that answers `hatoba` commands (HOST-15).

use hatoba_ai::tools::MemoryAction;
use hatoba_cli::protocol::{End, Frame, HEADER, Op, Request, VERSION};
use tauri::AppHandle;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::state::{MutexExt, state};

use super::{Ctx, IDLE_CHECK, LOCKED, REQUEST_WAIT, exec, host_list, memories, pool, transfer};

/// Starts or stops listening as the device's prefs say (`commands::cli::access_on`).
pub fn sync(app: &AppHandle) {
    let state = state(app);
    let on = crate::commands::cli::access_on(&state.vault());
    let mut listener = state.cli.listener.lock_unpoisoned();
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
            let mut listener = state.cli.listener.lock_unpoisoned();
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
                Ok((stream, _)) if peer_uid(&stream).is_ok_and(|uid| uid == owner) => {
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

/// The user a `hatoba` command that connected runs as. On macOS, Tokio's `peer_cred` also asks
/// for the command's process id (`LOCAL_PEEREPID`), which the system no longer gives once the
/// command has closed its end, as `hatoba exec` does right after it sent a short stdin;
/// `getpeereid` still gives the user then.
#[cfg(target_os = "macos")]
pub(super) fn peer_uid(stream: &tokio::net::UnixStream) -> std::io::Result<u32> {
    use std::os::fd::AsRawFd;
    let (mut uid, mut gid) = (0, 0);
    // SAFETY: the descriptor is the stream's, open for the whole call, and the pointers are to
    // locals that outlive it.
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } == 0 {
        Ok(uid)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// The user a `hatoba` command that connected runs as.
#[cfg(all(unix, not(target_os = "macos")))]
pub(super) fn peer_uid(stream: &tokio::net::UnixStream) -> std::io::Result<u32> {
    stream.peer_cred().map(|c| c.uid())
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
    // The rest of this connection is the stdin of the command that asked for it.
    if let Op::Stdin { token } = &request.op {
        let waiting = state(&app).cli.stdin.lock_unpoisoned().remove(token);
        if let Some(waiting) = waiting
            && request.version == VERSION
        {
            let _ = waiting.send(Box::new(reader));
        }
        return;
    }
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
        Op::Hosts
        | Op::Stdin { .. }
        | Op::Memories { .. }
        | Op::MemorySave { .. }
        | Op::MemoryDelete { .. } => false,
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
            host,
            command,
            cwd,
            sudo,
            stdin,
            ..
        } => exec::exec(&ctx, &host, command, cwd, sudo, stdin).await,
        Op::Upload {
            host,
            local,
            remote,
            sudo,
            mode,
            ..
        } => transfer::upload(&ctx, &host, local, remote, sudo, mode).await,
        Op::Download {
            host,
            remote,
            local,
            sudo,
            ..
        } => transfer::download(&ctx, &host, remote, local, sudo).await,
        // `serve` hands it to its command.
        Op::Stdin { .. } => End::error("no command waits for this stdin"),
        Op::Memories { host, json } => {
            match state.with_unlocked(|v| Ok(memories::list(v, &host, json))) {
                Ok(Ok(listing)) => {
                    if !listing.text.is_empty() {
                        let _ = tx.send(Frame::Stdout(listing.text.into_bytes())).await;
                    }
                    if let Some(notice) = listing.notice {
                        let _ = tx.send(Frame::Notice(notice)).await;
                    }
                    End::ok()
                }
                Ok(Err(refused)) => End::error(refused),
                Err(_) => End::error(LOCKED),
            }
        }
        Op::MemorySave {
            host,
            name,
            content,
        } => change_memory(&ctx, host, MemoryAction::Save, name, Some(content)).await,
        Op::MemoryDelete { host, name } => {
            change_memory(&ctx, host, MemoryAction::Delete, name, None).await
        }
    }
}

/// `hatoba memory save` and `delete`: what was done goes to stdout, and the change syncs.
async fn change_memory(
    ctx: &Ctx<'_>,
    host: String,
    action: MemoryAction,
    name: String,
    content: Option<String>,
) -> End {
    let changed =
        state(ctx.app).with_unlocked(|v| Ok(memories::change(v, &host, action, name, content)));
    match changed {
        Ok(Ok(done)) => {
            crate::sync::local_change(ctx.app);
            let _ = ctx
                .tx
                .send(Frame::Stdout(format!("{done}\n").into_bytes()))
                .await;
            End::ok()
        }
        Ok(Err(refused)) => End::error(refused),
        Err(_) => End::error(LOCKED),
    }
}
