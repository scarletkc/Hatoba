//! SFTP client (SFTP-01..04).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use russh_sftp::client::{Config as SftpConfig, SftpSession};
use russh_sftp::protocol::OpenFlags;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::error::{SshError, SshErrorKind};
use crate::session::SshSession;

/// Size of each read/write while transferring.
const CHUNK: usize = 256 * 1024;
/// Minimum time between progress callbacks.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const S_IFMT: u32 = 0o170_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFLNK: u32 = 0o120_000;
const S_IFREG: u32 = 0o100_000;
/// Attempts at finding an unused temporary file name before giving up.
const PART_ATTEMPTS: u32 = 16;

type SftpError = russh_sftp::client::error::Error;

/// A directory entry as shown in the file panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileEntry {
    /// File name without directory.
    pub name: String,
    /// Full remote path (`list` argument joined with `name`).
    pub path: String,
    /// Directory, or symlink pointing to a directory.
    pub is_dir: bool,
    /// The entry itself is a symbolic link.
    pub is_symlink: bool,
    /// Size in bytes (0 if unknown).
    pub size: u64,
    /// Modification time in Unix milliseconds.
    pub modified: Option<i64>,
    /// `ls -l` style permissions, e.g. `drwxr-xr-x`.
    pub permissions: String,
    /// Raw `st_mode` as reported by the server (type and permission bits).
    pub mode: u32,
}

/// Progress of an upload or download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TransferProgress {
    /// Bytes transferred so far.
    pub bytes: u64,
    /// Total size (0 if unknown).
    pub total: u64,
    /// Smoothed transfer rate.
    pub bytes_per_sec: u64,
}

/// Direction of a transfer (for UI events built on [`TransferProgress`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferDirection {
    /// Local file to remote host.
    Upload,
    /// Remote file to local disk.
    Download,
}

/// SFTP session on its own channel of an [`SshSession`]. Cheap to clone.
#[derive(Clone)]
pub struct SftpClient {
    sftp: Arc<SftpSession>,
    /// Keeps the underlying connection alive.
    session: SshSession,
}

impl SftpClient {
    pub(crate) async fn start<S>(session: SshSession, stream: S) -> Result<Self, SshError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let config = SftpConfig {
            request_timeout_secs: 30,
            ..SftpConfig::default()
        };
        let sftp = SftpSession::new_with_config(stream, config)
            .await
            .map_err(|e| {
                if session.is_closed() {
                    session.close_error()
                } else {
                    SshError::new(
                        SshErrorKind::Sftp,
                        format!("SFTP initialization failed: {e}"),
                    )
                }
            })?;
        Ok(Self {
            sftp: Arc::new(sftp),
            session,
        })
    }

    /// Converts a library error, preferring the connection error if the
    /// connection is gone.
    fn err(&self, op: &str, path: &str, e: SftpError) -> SshError {
        if self.session.is_closed() {
            return self.session.close_error();
        }
        let mut err = SshError::from(e);
        err.message = format!("{op} {path}: {}", err.message);
        err
    }

    /// Absolute path of the login directory.
    pub async fn home(&self) -> Result<String, SshError> {
        self.sftp
            .canonicalize(".")
            .await
            .map_err(|e| self.err("resolve", ".", e))
    }

    /// Lists a directory: directories first, then names case-insensitively;
    /// `.` and `..` are omitted. Symlinks to directories are reported with
    /// `is_dir = true` (and `is_symlink = true`).
    pub async fn list(&self, path: &str) -> Result<Vec<FileEntry>, SshError> {
        let dir = self
            .sftp
            .read_dir(path)
            .await
            .map_err(|e| self.err("list", path, e))?;

        let mut entries: Vec<FileEntry> = dir
            .map(|entry| {
                let name = entry.file_name();
                let meta = entry.metadata();
                let mode = meta.permissions.unwrap_or(0);
                let kind = mode & S_IFMT;
                FileEntry {
                    path: join_remote(path, &name),
                    name,
                    is_dir: kind == S_IFDIR,
                    is_symlink: kind == S_IFLNK,
                    size: meta.size.unwrap_or(0),
                    modified: meta.mtime.map(|t| i64::from(t) * 1000),
                    permissions: permissions_string(mode),
                    mode,
                }
            })
            .collect();

        // Resolve symlink targets concurrently so links to folders can be opened.
        let links: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.is_symlink)
            .map(|(i, _)| i)
            .collect();
        let stats = futures::future::join_all(links.iter().map(|&i| {
            let sftp = Arc::clone(&self.sftp);
            let target = entries[i].path.clone();
            async move { sftp.metadata(target).await }
        }))
        .await;
        for (i, stat) in links.into_iter().zip(stats) {
            if let Ok(meta) = stat {
                entries[i].is_dir = meta.permissions.unwrap_or(0) & S_IFMT == S_IFDIR;
                if !entries[i].is_dir {
                    entries[i].size = meta.size.unwrap_or(entries[i].size);
                }
            }
        }

        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(entries)
    }

    /// Creates a directory.
    pub async fn mkdir(&self, path: &str) -> Result<(), SshError> {
        self.sftp
            .create_dir(path)
            .await
            .map_err(|e| self.err("create directory", path, e))
    }

    /// Renames or moves a file or directory. SFTP v3 refuses to overwrite an existing target.
    pub async fn rename(&self, from: &str, to: &str) -> Result<(), SshError> {
        self.sftp
            .rename(from, to)
            .await
            .map_err(|e| self.err("rename", from, e))
    }

    /// Deletes a file (or symlink).
    pub async fn remove_file(&self, path: &str) -> Result<(), SshError> {
        self.sftp
            .remove_file(path)
            .await
            .map_err(|e| self.err("delete", path, e))
    }

    /// Deletes an empty directory.
    pub async fn remove_dir(&self, path: &str) -> Result<(), SshError> {
        self.sftp
            .remove_dir(path)
            .await
            .map_err(|e| self.err("delete directory", path, e))
    }

    /// Deletes a file, or a directory with everything inside it. Symlinks are
    /// removed, never followed.
    pub async fn remove_recursive(&self, path: &str) -> Result<(), SshError> {
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() {
            return Err(SshError::new(
                SshErrorKind::Sftp,
                "refusing to delete the root directory",
            ));
        }
        let top = self
            .sftp
            .symlink_metadata(trimmed)
            .await
            .map_err(|e| self.err("stat", trimmed, e))?;
        if top.permissions.unwrap_or(0) & S_IFMT != S_IFDIR {
            return self.remove_file(trimmed).await;
        }

        // Discovery order is parents before children; delete in reverse.
        let mut dirs = vec![trimmed.to_owned()];
        let mut next = 0;
        while next < dirs.len() {
            let dir = dirs[next].clone();
            next += 1;
            let listing = self
                .sftp
                .read_dir(dir.as_str())
                .await
                .map_err(|e| self.err("list", &dir, e))?;
            for entry in listing {
                let child = entry.path();
                if entry.metadata().permissions.unwrap_or(0) & S_IFMT == S_IFDIR {
                    dirs.push(child);
                } else {
                    self.remove_file(&child).await?;
                }
            }
        }
        for dir in dirs.iter().rev() {
            self.remove_dir(dir).await?;
        }
        Ok(())
    }

    /// Downloads `remote` to `local`.
    ///
    /// Data is written to a new `<local>.<token>.part` file and renamed on
    /// success; the partial file is deleted on failure or cancellation. The
    /// temporary file is created exclusively, so an existing file is never
    /// overwritten or deleted in its place. `progress` is called at most
    /// about ten times per second plus once at the end.
    pub async fn download(
        &self,
        remote: &str,
        local: &Path,
        progress: impl Fn(TransferProgress) + Send + Sync + 'static,
        cancel: CancellationToken,
    ) -> Result<(), SshError> {
        let meta = self
            .sftp
            .metadata(remote)
            .await
            .map_err(|e| self.err("stat", remote, e))?;
        if meta.permissions.unwrap_or(0) & S_IFMT == S_IFDIR {
            return Err(SshError::new(
                SshErrorKind::Sftp,
                format!("download {remote}: is a directory"),
            ));
        }
        if cancel.is_cancelled() {
            return Err(SshError::cancelled());
        }
        let mut source = self
            .sftp
            .open(remote)
            .await
            .map_err(|e| self.err("open", remote, e))?;
        let (part, target) = create_local_part(local).await?;
        let result = self
            .download_to(
                remote,
                &mut source,
                &part,
                target,
                meta.size.unwrap_or(0),
                &progress,
                &cancel,
            )
            .await;
        let _ = source.close().await;
        match result {
            Ok(()) => tokio::fs::rename(&part, local).await.map_err(|e| {
                let _ = std::fs::remove_file(&part);
                SshError::io_context(&format!("finalize {}", local.display()), &e)
            }),
            Err(e) => {
                let _ = tokio::fs::remove_file(&part).await;
                Err(e)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn download_to(
        &self,
        remote: &str,
        source: &mut russh_sftp::client::fs::File,
        part: &Path,
        mut target: tokio::fs::File,
        total: u64,
        progress: &(impl Fn(TransferProgress) + Send + Sync),
        cancel: &CancellationToken,
    ) -> Result<(), SshError> {
        let mut tracker = Tracker::new(total);
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(SshError::cancelled()),
                read = source.read(&mut buf) => read.map_err(|e| self.io_to_sftp("read", remote, e))?,
            };
            if n == 0 {
                break;
            }
            target
                .write_all(&buf[..n])
                .await
                .map_err(|e| SshError::io_context(&format!("write {}", part.display()), &e))?;
            tracker.advance(n as u64, progress);
        }
        target
            .flush()
            .await
            .map_err(|e| SshError::io_context(&format!("write {}", part.display()), &e))?;
        drop(target);
        tracker.finish(progress);
        Ok(())
    }

    /// Uploads `local` to `remote`.
    ///
    /// Data is written to a new `<remote>.<token>.part` file and renamed on
    /// success so an interrupted upload never clobbers an existing file; the
    /// partial file is deleted on failure or cancellation. The temporary file
    /// is created exclusively, so an existing file is never overwritten or
    /// deleted in its place.
    pub async fn upload(
        &self,
        local: &Path,
        remote: &str,
        progress: impl Fn(TransferProgress) + Send + Sync + 'static,
        cancel: CancellationToken,
    ) -> Result<(), SshError> {
        let meta = tokio::fs::metadata(local)
            .await
            .map_err(|e| SshError::io_context(&format!("stat {}", local.display()), &e))?;
        if !meta.is_file() {
            return Err(SshError::new(
                SshErrorKind::Io,
                format!("{} is not a regular file", local.display()),
            ));
        }
        let source = tokio::fs::File::open(local)
            .await
            .map_err(|e| SshError::io_context(&format!("open {}", local.display()), &e))?;
        let (part, target) = self.create_remote_part(remote).await?;
        let result = self
            .upload_to(local, source, &part, target, meta.len(), &progress, &cancel)
            .await;
        match result {
            Ok(()) => self.replace(&part, remote).await,
            Err(e) => {
                let _ = self.sftp.remove_file(part.as_str()).await;
                Err(e)
            }
        }
    }

    /// Creates a new, empty temporary file next to `remote` that no other
    /// file or transfer is using.
    async fn create_remote_part(
        &self,
        remote: &str,
    ) -> Result<(String, russh_sftp::client::fs::File), SshError> {
        let mut last = None;
        for _ in 0..PART_ATTEMPTS {
            let part = format!("{remote}.{}.part", part_token());
            // EXCLUDE (O_EXCL) fails on any existing entry, including a symlink.
            match self
                .sftp
                .open_with_flags(
                    part.as_str(),
                    OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
                )
                .await
            {
                Ok(file) => return Ok((part, file)),
                // SFTP v3 has no "already exists" status, so look for the entry.
                Err(e) if self.sftp.symlink_metadata(part.as_str()).await.is_ok() => {
                    last = Some(self.err("create", &part, e));
                }
                Err(e) => return Err(self.err("create", &part, e)),
            }
        }
        Err(last.expect("at least one attempt"))
    }

    #[allow(clippy::too_many_arguments)]
    async fn upload_to(
        &self,
        local: &Path,
        mut source: tokio::fs::File,
        part: &str,
        mut target: russh_sftp::client::fs::File,
        total: u64,
        progress: &(impl Fn(TransferProgress) + Send + Sync),
        cancel: &CancellationToken,
    ) -> Result<(), SshError> {
        let mut tracker = Tracker::new(total);
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(SshError::cancelled()),
                read = source.read(&mut buf) => read
                    .map_err(|e| SshError::io_context(&format!("read {}", local.display()), &e))?,
            };
            if n == 0 {
                break;
            }
            tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(SshError::cancelled()),
                written = target.write_all(&buf[..n]) => {
                    written.map_err(|e| self.io_to_sftp("write", part, e))?;
                }
            }
            tracker.advance(n as u64, progress);
        }
        // `shutdown` waits for every pending write to be acknowledged and closes the handle.
        target
            .shutdown()
            .await
            .map_err(|e| self.io_to_sftp("write", part, e))?;
        tracker.finish(progress);
        Ok(())
    }

    /// Moves the finished `.part` file over `remote`, keeping the old file
    /// until the new one is in place (SFTP v3 rename does not overwrite).
    async fn replace(&self, part: &str, remote: &str) -> Result<(), SshError> {
        if self.sftp.rename(part, remote).await.is_ok() {
            return Ok(());
        }
        let backup = format!("{remote}.hatoba-old");
        let exists = self.sftp.try_exists(remote).await.unwrap_or(false);
        let result = if exists {
            match self.sftp.rename(remote, backup.as_str()).await {
                Ok(()) => match self.sftp.rename(part, remote).await {
                    Ok(()) => {
                        let _ = self.sftp.remove_file(backup.as_str()).await;
                        Ok(())
                    }
                    Err(e) => {
                        let _ = self.sftp.rename(backup.as_str(), remote).await;
                        Err(self.err("finalize upload", remote, e))
                    }
                },
                Err(e) => Err(self.err("replace", remote, e)),
            }
        } else {
            self.sftp
                .rename(part, remote)
                .await
                .map_err(|e| self.err("finalize upload", remote, e))
        };
        if result.is_err() {
            let _ = self.sftp.remove_file(part).await;
        }
        result
    }

    /// `russh-sftp` file I/O reports errors as `io::Error`.
    fn io_to_sftp(&self, op: &str, path: &str, e: std::io::Error) -> SshError {
        if self.session.is_closed() {
            return self.session.close_error();
        }
        SshError::new(SshErrorKind::Sftp, format!("{op} {path}: {e}"))
    }

    /// Closes the SFTP channel (the SSH connection stays up).
    pub async fn close(&self) {
        let _ = self.sftp.close().await;
    }
}

/// Throttled progress reporting with a smoothed transfer rate.
struct Tracker {
    started: Instant,
    total: u64,
    bytes: u64,
    last_report: Instant,
    last_bytes: u64,
    rate: f64,
}

impl Tracker {
    fn new(total: u64) -> Self {
        let now = Instant::now();
        Self {
            started: now,
            total,
            bytes: 0,
            last_report: now,
            last_bytes: 0,
            rate: 0.0,
        }
    }

    fn advance(&mut self, n: u64, progress: &impl Fn(TransferProgress)) {
        self.bytes += n;
        let dt = self.last_report.elapsed();
        if dt < PROGRESS_INTERVAL {
            return;
        }
        let instant = (self.bytes - self.last_bytes) as f64 / dt.as_secs_f64();
        self.rate = if self.rate == 0.0 {
            instant
        } else {
            0.3 * instant + 0.7 * self.rate
        };
        self.last_report = Instant::now();
        self.last_bytes = self.bytes;
        progress(TransferProgress {
            bytes: self.bytes,
            total: self.total.max(self.bytes),
            bytes_per_sec: self.rate as u64,
        });
    }

    /// Final event with the average rate over the whole transfer.
    fn finish(&mut self, progress: &impl Fn(TransferProgress)) {
        let secs = self.started.elapsed().as_secs_f64().max(0.001);
        progress(TransferProgress {
            bytes: self.bytes,
            total: self.total.max(self.bytes),
            bytes_per_sec: (self.bytes as f64 / secs) as u64,
        });
    }
}

/// Creates a new, empty temporary file next to `local` that no other file or
/// transfer is using.
async fn create_local_part(local: &Path) -> Result<(PathBuf, tokio::fs::File), SshError> {
    let mut last = None;
    for _ in 0..PART_ATTEMPTS {
        let part = part_path(local, &part_token());
        // `create_new` (O_EXCL) fails on any existing entry, including a symlink.
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
            .await
        {
            Ok(file) => return Ok((part, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some((part, e)),
            Err(e) => {
                return Err(SshError::io_context(
                    &format!("create {}", part.display()),
                    &e,
                ));
            }
        }
    }
    let (part, e) = last.expect("at least one attempt");
    Err(SshError::io_context(
        &format!("create {}", part.display()),
        &e,
    ))
}

fn part_path(local: &Path, token: &str) -> PathBuf {
    let mut name = local.as_os_str().to_owned();
    name.push(format!(".{token}.part"));
    PathBuf::from(name)
}

/// A short token that differs between transfers, so that transfers to the same
/// destination use different temporary files. Exclusive creation, not this
/// token, is what keeps existing files safe.
fn part_token() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    // SplitMix64 finalizer over the time, process id and counter.
    let mut x =
        nanos ^ (u64::from(std::process::id()) << 32) ^ count.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    format!("{:08x}", x as u32)
}

/// Joins a remote directory and a file name with exactly one `/`.
fn join_remote(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// `ls -l` style mode string such as `drwxr-xr-x` or `-rwsr-xr-x`.
fn permissions_string(mode: u32) -> String {
    let kind = match mode & S_IFMT {
        S_IFDIR => 'd',
        S_IFLNK => 'l',
        S_IFREG => '-',
        0o020_000 => 'c',
        0o060_000 => 'b',
        0o010_000 => 'p',
        0o140_000 => 's',
        _ => '?',
    };
    let bit = |mask: u32, ch: char| if mode & mask != 0 { ch } else { '-' };
    // Execute slot, combined with setuid / setgid / sticky.
    let exec =
        |x: u32, special: u32, lower: char, upper: char| match (mode & x != 0, mode & special != 0)
        {
            (true, true) => lower,
            (false, true) => upper,
            (true, false) => 'x',
            (false, false) => '-',
        };
    let mut s = String::with_capacity(10);
    s.push(kind);
    s.push(bit(0o400, 'r'));
    s.push(bit(0o200, 'w'));
    s.push(exec(0o100, 0o4000, 's', 'S'));
    s.push(bit(0o040, 'r'));
    s.push(bit(0o020, 'w'));
    s.push(exec(0o010, 0o2000, 's', 'S'));
    s.push(bit(0o004, 'r'));
    s.push(bit(0o002, 'w'));
    s.push(exec(0o001, 0o1000, 't', 'T'));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_strings() {
        assert_eq!(permissions_string(0o040_755), "drwxr-xr-x");
        assert_eq!(permissions_string(0o100_644), "-rw-r--r--");
        assert_eq!(permissions_string(0o120_777), "lrwxrwxrwx");
        assert_eq!(permissions_string(0o104_755), "-rwsr-xr-x");
        assert_eq!(permissions_string(0o041_777), "drwxrwxrwt");
        assert_eq!(permissions_string(0o102_745), "-rwxr-Sr-x");
        assert_eq!(permissions_string(0), "?---------");
    }

    #[test]
    fn remote_paths_join_with_single_slash() {
        assert_eq!(join_remote("/", "a"), "/a");
        assert_eq!(join_remote("/home/u", "a"), "/home/u/a");
        assert_eq!(join_remote("/home/u/", "a"), "/home/u/a");
        assert_eq!(join_remote("", "a"), "a");
    }

    #[test]
    fn part_file_name_appends_suffix() {
        assert_eq!(
            part_path(Path::new("/tmp/file.txt"), "0123abcd"),
            PathBuf::from("/tmp/file.txt.0123abcd.part")
        );
    }

    #[test]
    fn part_tokens_differ() {
        let tokens: std::collections::HashSet<_> = (0..1000).map(|_| part_token()).collect();
        assert_eq!(tokens.len(), 1000);
        assert!(tokens.iter().all(|t| t.len() == 8));
    }

    #[tokio::test]
    async fn local_part_never_reuses_an_existing_path() {
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("report.txt");
        let (a, _fa) = create_local_part(&local).await.unwrap();
        let (b, _fb) = create_local_part(&local).await.unwrap();
        assert_ne!(a, b);
        assert!(a.exists() && b.exists());
    }
}
