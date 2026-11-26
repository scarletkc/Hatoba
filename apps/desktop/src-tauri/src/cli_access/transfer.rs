//! `hatoba cp` uploads and downloads over SFTP or sudo (HOST-15).

use std::future::Future;
use std::path::{Path, PathBuf};

use hatoba_cli::protocol::{End, Frame, Progress};
use hatoba_ssh::remote_file::RemoteFileError;
use hatoba_ssh::sudo::{self, RunAs};
use hatoba_ssh::{SftpClient, SshError, SshSession, TransferProgress};
use tokio_util::sync::CancellationToken;

use crate::dto::CliActivityKind;

use super::{Ctx, Target, detail, pool};

impl Ctx<'_> {
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

pub(super) async fn upload(
    ctx: &Ctx<'_>,
    host: &str,
    local: String,
    remote: String,
    sudo: bool,
    mode: Option<u32>,
) -> End {
    let target = match ctx.target(host) {
        Ok(target) => target,
        Err(message) => return End::error(message),
    };
    let id = ctx.begin(
        CliActivityKind::Upload,
        &target,
        detail(sudo, format!("{local} → {remote}")),
    );
    let end = match run_upload(ctx, &target, Path::new(&local), &remote, sudo, mode).await {
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
    sudo: bool,
    mode: Option<u32>,
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
    let run_as = match sudo {
        true => Some(ctx.run_as(&lease.session, target, "the copy").await?),
        false => None,
    };
    let sftp = ctx.sftp(&lease, target).await?;
    let result = async {
        if let Some(run_as) = &run_as {
            return upload_as_root(
                ctx,
                &lease.session,
                &sftp,
                run_as,
                local,
                remote,
                &name,
                mode,
            )
            .await;
        }
        let remote = remote_target(&sftp, remote, &name).await?;
        // The new file takes the place of one already there, with its permission bits, as `cp`
        // and `scp` keep them.
        let mode = match mode {
            Some(mode) => Some(mode),
            None => match sftp.stat(&remote).await {
                Ok(Some(old)) if !old.is_dir() && !old.is_special() => old.mode.map(|m| m & 0o777),
                _ => None,
            },
        };
        let cancel = CancellationToken::new();
        let copying = sftp.upload_with_mode(local, &remote, mode, ctx.progress(), cancel.clone());
        ctx.copy_until_stopped(copying, &cancel).await
    }
    .await;
    sftp.close().await;
    result
}

/// `hatoba cp --sudo` to the host: the file goes over SFTP into a folder that only the login user
/// can open, and from there, as root, to `remote` (`hatoba_ssh::sudo::install`).
#[allow(clippy::too_many_arguments)]
async fn upload_as_root(
    ctx: &Ctx<'_>,
    session: &SshSession,
    sftp: &SftpClient,
    run_as: &RunAs,
    local: &Path,
    remote: &str,
    name: &str,
    mode: Option<u32>,
) -> Result<(), String> {
    let mut to = absolute(sftp, remote).await?;
    if to.ends_with('/') {
        to = join_remote(&to, name);
    }
    let dir = sudo::temp_dir(session).await.map_err(|e| e.message)?;
    let result = async {
        let staged = sudo::temp_file(&dir);
        let cancel = CancellationToken::new();
        let copying = sftp.upload(local, &staged, ctx.progress(), cancel.clone());
        ctx.copy_until_stopped(copying, &cancel).await?;
        // Not stopped once begun: the copy writes the file in place, so stopping it would leave the
        // file cut short, and closing the channel does not end it on the host anyway.
        sudo::install(session, run_as, &staged, &to, name, mode)
            .await
            .map_err(|e| copy_error(&to, e))
    }
    .await;
    remove_temp_dir(session, &dir).await;
    result
}

pub(super) async fn download(
    ctx: &Ctx<'_>,
    host: &str,
    remote: String,
    local: String,
    sudo: bool,
) -> End {
    let target = match ctx.target(host) {
        Ok(target) => target,
        Err(message) => return End::error(message),
    };
    let id = ctx.begin(
        CliActivityKind::Download,
        &target,
        detail(sudo, format!("{remote} → {local}")),
    );
    let end = match run_download(ctx, &target, &remote, PathBuf::from(&local), sudo).await {
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
    sudo: bool,
) -> Result<(), String> {
    let lease = ctx.lease(target).await?;
    let run_as = match sudo {
        true => Some(ctx.run_as(&lease.session, target, "the copy").await?),
        false => None,
    };
    let sftp = ctx.sftp(&lease, target).await?;
    let result = async {
        let remote = match run_as {
            Some(_) => absolute(&sftp, remote).await?,
            None => expand_home(&sftp, remote).await?,
        };
        if local.is_dir() {
            let name = remote.rsplit('/').find(|n| !n.is_empty()).ok_or_else(|| {
                format!("{remote} is a folder. hatoba cp copies one file at a time.")
            })?;
            local.push(name);
        }
        if let Some(run_as) = &run_as {
            return download_as_root(ctx, &lease.session, run_as, &remote, &local).await;
        }
        let cancel = CancellationToken::new();
        let copying = sftp.download(&remote, &local, ctx.progress(), cancel.clone());
        ctx.copy_until_stopped(copying, &cancel).await
    }
    .await;
    sftp.close().await;
    result
}

/// `hatoba cp --sudo` from the host: root reads the file `remote` onto the exec channel
/// (`hatoba_ssh::sudo::download`), so no copy of it is left on the host. A stopped download still
/// runs to its end, which removes what it wrote here.
async fn download_as_root(
    ctx: &Ctx<'_>,
    session: &SshSession,
    run_as: &RunAs,
    remote: &str,
    local: &Path,
) -> Result<(), String> {
    let cancel = CancellationToken::new();
    let copying = sudo::download(
        session,
        run_as,
        remote,
        local,
        ctx.progress(),
        cancel.clone(),
    );
    tokio::pin!(copying);
    let result = tokio::select! {
        result = &mut copying => result,
        () = ctx.stopped() => {
            cancel.cancel();
            copying.await
        }
    };
    result.map_err(|e| {
        if cancel.is_cancelled() {
            ctx.stop_message("the copy")
        } else {
            copy_error(remote, e)
        }
    })
}

/// Removes the folder a `--sudo` copy went through; one left behind keeps its copy where only the
/// login user can open it.
async fn remove_temp_dir(session: &SshSession, dir: &str) {
    if let Err(e) = sudo::remove_temp_dir(session, dir).await {
        tracing::debug!(
            "hatoba cp --sudo left its folder on the host: {}",
            e.message
        );
    }
}

/// What a `--sudo` copy of `path` that failed on the host says.
fn copy_error(path: &str, e: RemoteFileError) -> String {
    match e {
        RemoteFileError::Ssh(e) => e.message,
        RemoteFileError::IsDirectory => {
            format!("{path} is a folder. hatoba cp copies one file at a time.")
        }
        e => format!("{path}: {e}"),
    }
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

/// A remote path as root's scripts take it: absolute, with `~`, `~/…` and any other relative one
/// from the login user's home directory.
async fn absolute(sftp: &SftpClient, path: &str) -> Result<String, String> {
    let path = expand_home(sftp, path).await?;
    if path.starts_with('/') {
        return Ok(path);
    }
    let home = sftp.home().await.map_err(|e| e.message)?;
    Ok(join_remote(&home, &path))
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

pub(super) fn join_remote(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}
