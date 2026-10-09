//! Interactive shell channel with output batching (§10.3).

use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use russh::client::Msg;
use russh::{Channel, ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Pty};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::error::SshError;
use crate::handler::CloseReason;
use crate::session::{SshSession, wait_request_reply};

/// Output is flushed at least this often while data keeps arriving.
const FLUSH_INTERVAL: Duration = Duration::from_millis(8);
/// ... or as soon as this much is buffered.
const FLUSH_BYTES: usize = 32 * 1024;
/// 128 events of up to 32 KB: at most ~4 MB in flight before reading stops.
const EVENT_CAPACITY: usize = 128;

/// Parameters of the PTY and shell.
#[derive(Debug, Clone)]
pub struct ShellOptions {
    /// `TERM` value, e.g. `xterm-256color`.
    pub term: String,
    /// Initial width in character cells.
    pub cols: u32,
    /// Initial height in character cells.
    pub rows: u32,
    /// Environment variables to request before the shell starts. Servers
    /// accept only variables listed in their `AcceptEnv`; refusals are
    /// ignored. Defaults to `LANG=C.UTF-8` so UTF-8 input works on servers
    /// that default to the POSIX locale.
    pub env: Vec<(String, String)>,
}

impl Default for ShellOptions {
    fn default() -> Self {
        Self {
            term: "xterm-256color".to_owned(),
            cols: 80,
            rows: 24,
            env: vec![("LANG".to_owned(), "C.UTF-8".to_owned())],
        }
    }
}

impl ShellOptions {
    /// Adds a host's variables after the ones already set, like OpenSSH's `SetEnv` (SSH-14). A
    /// variable with the name of one already set replaces its value, so a host's `LANG` replaces
    /// the default. As with OpenSSH, `TERM` sets the terminal type instead of being requested
    /// (an empty one keeps the type).
    #[must_use]
    pub fn with_env(mut self, vars: impl IntoIterator<Item = (String, String)>) -> Self {
        for (name, value) in vars {
            if name == "TERM" {
                if !value.is_empty() {
                    self.term = value;
                }
            } else if let Some(slot) = self.env.iter_mut().find(|(n, _)| *n == name) {
                slot.1 = value;
            } else {
                self.env.push((name, value));
            }
        }
        self
    }
}

/// What happened on a shell channel.
#[derive(Debug, Clone)]
pub enum ShellEvent {
    /// A batch of terminal output (stdout and stderr merged).
    Data(Bytes),
    /// The shell ended in an orderly way (exit, logout, `close()`,
    /// or the server sent a disconnect). Sent exactly once, last.
    Closed {
        /// Human readable reason.
        reason: String,
        /// Exit status of the shell if the server reported one.
        exit_status: Option<u32>,
    },
    /// The connection broke (reset, keepalive timeout, ...). Sent exactly
    /// once, last, instead of `Closed`.
    Error(SshError),
}

struct ShellInner {
    writer: ChannelWriteHalf<Msg>,
    cancel: CancellationToken,
    /// Keeps keystrokes ordered when several tasks call `write`.
    write_lock: tokio::sync::Mutex<()>,
}

/// Input side of a shell. Cheap to clone.
#[derive(Clone)]
pub struct ShellHandle {
    inner: Arc<ShellInner>,
}

impl ShellHandle {
    /// Sends keyboard input (or pasted text) to the remote shell.
    pub async fn write(&self, data: Vec<u8>) -> Result<(), SshError> {
        if self.inner.cancel.is_cancelled() {
            return Err(SshError::channel("the shell is closed"));
        }
        let _order = self.inner.write_lock.lock().await;
        self.inner
            .writer
            .data_bytes(Bytes::from(data))
            .await
            .map_err(SshError::from)
    }

    /// Tells the remote PTY about a new window size. Failures are ignored
    /// (the shell ending is reported through the event channel).
    pub async fn resize(&self, cols: u32, rows: u32) {
        if let Err(e) = self
            .inner
            .writer
            .window_change(cols.max(1), rows.max(1), 0, 0)
            .await
        {
            tracing::debug!("window change failed: {e}");
        }
    }

    /// Closes the shell channel; a `Closed` event follows.
    pub async fn close(&self) {
        if self.inner.cancel.is_cancelled() {
            return;
        }
        self.inner.cancel.cancel();
        let _ = self.inner.writer.eof().await;
        let _ = self.inner.writer.close().await;
    }
}

/// Terminal modes: sane cooked-mode defaults plus UTF-8 line editing.
fn terminal_modes() -> Vec<(Pty, u32)> {
    vec![
        (Pty::VINTR, 3),
        (Pty::VQUIT, 28),
        (Pty::VERASE, 127),
        (Pty::VKILL, 21),
        (Pty::VEOF, 4),
        (Pty::VSTART, 17),
        (Pty::VSTOP, 19),
        (Pty::VSUSP, 26),
        (Pty::VREPRINT, 18),
        (Pty::VWERASE, 23),
        (Pty::VLNEXT, 22),
        (Pty::VDISCARD, 15),
        (Pty::ICRNL, 1),
        (Pty::IXON, 1),
        (Pty::IXANY, 1),
        (Pty::IMAXBEL, 1),
        (Pty::IUTF8, 1),
        (Pty::ISIG, 1),
        (Pty::ICANON, 1),
        (Pty::ECHO, 1),
        (Pty::ECHOE, 1),
        (Pty::ECHOK, 1),
        (Pty::ECHOCTL, 1),
        (Pty::ECHOKE, 1),
        (Pty::IEXTEN, 1),
        (Pty::OPOST, 1),
        (Pty::ONLCR, 1),
        (Pty::CS8, 1),
        (Pty::TTY_OP_ISPEED, 38400),
        (Pty::TTY_OP_OSPEED, 38400),
    ]
}

pub(crate) async fn start(
    session: &SshSession,
    mut channel: Channel<Msg>,
    opts: ShellOptions,
) -> Result<(ShellHandle, mpsc::Receiver<ShellEvent>), SshError> {
    channel
        .request_pty(
            true,
            &opts.term,
            opts.cols.max(1),
            opts.rows.max(1),
            0,
            0,
            &terminal_modes(),
        )
        .await
        .map_err(SshError::from)?;
    wait_request_reply(&mut channel, "pty").await?;

    for (name, value) in &opts.env {
        // No reply requested: a refusal is not an error.
        if let Err(e) = channel.set_env(false, name.as_str(), value.as_str()).await {
            tracing::debug!("env request failed: {e}");
        }
    }

    channel.request_shell(true).await.map_err(SshError::from)?;
    wait_request_reply(&mut channel, "shell").await?;

    let (reader, writer) = channel.split();
    let (tx, rx) = mpsc::channel(EVENT_CAPACITY);
    let inner = Arc::new(ShellInner {
        writer,
        cancel: CancellationToken::new(),
        write_lock: tokio::sync::Mutex::new(()),
    });
    tokio::spawn(pump(reader, tx, Arc::clone(&inner), session.clone()));
    Ok((ShellHandle { inner }, rx))
}

/// Reads the channel, coalesces output and emits exactly one terminal event.
async fn pump(
    mut reader: ChannelReadHalf,
    tx: mpsc::Sender<ShellEvent>,
    inner: Arc<ShellInner>,
    session: SshSession,
) {
    let mut buf = BytesMut::with_capacity(FLUSH_BYTES);
    // Start "idle" so the very first output is not delayed.
    let mut last_flush = Instant::now()
        .checked_sub(FLUSH_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut exit_status: Option<u32> = None;
    let mut exit_signal: Option<String> = None;
    let mut channel_closed = false;
    let mut local_close = false;
    let mut receiver_gone = false;

    loop {
        let flush_at = (!buf.is_empty()).then(|| last_flush + FLUSH_INTERVAL);
        tokio::select! {
            biased;
            () = inner.cancel.cancelled() => {
                local_close = true;
                break;
            }
            () = tx.closed() => {
                receiver_gone = true;
                break;
            }
            () = async { tokio::time::sleep_until(flush_at.unwrap_or_else(Instant::now)).await },
                if flush_at.is_some() =>
            {
                if !flush(&tx, &mut buf, &mut last_flush).await {
                    receiver_gone = true;
                    break;
                }
            }
            msg = reader.wait() => match msg {
                Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                    buf.extend_from_slice(&data);
                    let idle = last_flush.elapsed() >= FLUSH_INTERVAL;
                    if (idle || buf.len() >= FLUSH_BYTES) && !flush(&tx, &mut buf, &mut last_flush).await {
                        receiver_gone = true;
                        break;
                    }
                }
                Some(ChannelMsg::ExitStatus { exit_status: s }) => exit_status = Some(s),
                Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                    exit_signal = Some(format!("terminated by signal {signal_name:?}"));
                }
                Some(ChannelMsg::Close) => {
                    channel_closed = true;
                    break;
                }
                Some(_) => {}
                None => break,
            },
        }
    }

    if receiver_gone {
        // Nobody is listening any more; make sure the remote shell goes away.
        inner.cancel.cancel();
        let _ = inner.writer.eof().await;
        let _ = inner.writer.close().await;
        return;
    }
    // Further writes should fail fast with "shell is closed".
    inner.cancel.cancel();
    // Deliver whatever is still buffered, then the final event.
    if !buf.is_empty() {
        flush_all(&tx, &mut buf).await;
    }

    let event = if local_close {
        ShellEvent::Closed {
            reason: "closed by the client".to_owned(),
            exit_status,
        }
    } else if channel_closed || exit_status.is_some() || exit_signal.is_some() {
        let reason = match (&exit_signal, exit_status) {
            (Some(s), _) => s.clone(),
            (None, Some(code)) => format!("the shell exited with status {code}"),
            (None, None) => "the server closed the channel".to_owned(),
        };
        ShellEvent::Closed {
            reason,
            exit_status,
        }
    } else {
        // The channel vanished without a close: the transport is gone.
        match session.shared().reason() {
            Some(CloseReason::Local) => ShellEvent::Closed {
                reason: "disconnected by the client".to_owned(),
                exit_status: None,
            },
            Some(CloseReason::Remote(reason)) => ShellEvent::Closed {
                reason,
                exit_status: None,
            },
            Some(CloseReason::Error(e)) => ShellEvent::Error(e),
            None => ShellEvent::Error(SshError::disconnected("connection lost")),
        }
    };
    let _ = tx.send(event).await;
}

/// Sends the buffer in events of at most [`FLUSH_BYTES`]. Returns `false` if
/// the receiver is gone.
async fn flush(
    tx: &mpsc::Sender<ShellEvent>,
    buf: &mut BytesMut,
    last_flush: &mut Instant,
) -> bool {
    let ok = flush_all(tx, buf).await;
    *last_flush = Instant::now();
    ok
}

async fn flush_all(tx: &mpsc::Sender<ShellEvent>, buf: &mut BytesMut) -> bool {
    while !buf.is_empty() {
        let take = buf.len().min(FLUSH_BYTES);
        let chunk = buf.split_to(take).freeze();
        if tx.send(ShellEvent::Data(chunk)).await.is_err() {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn host_env_follows_the_default_lang() {
        let opts = ShellOptions::default().with_env(pairs(&[("TZ", "Asia/Tokyo"), ("A", "")]));
        assert_eq!(
            opts.env,
            pairs(&[("LANG", "C.UTF-8"), ("TZ", "Asia/Tokyo"), ("A", "")])
        );
        assert_eq!(opts.term, "xterm-256color");
    }

    #[test]
    fn host_lang_replaces_the_default() {
        let opts =
            ShellOptions::default().with_env(pairs(&[("LC_ALL", "C"), ("LANG", "ja_JP.UTF-8")]));
        assert_eq!(opts.env, pairs(&[("LANG", "ja_JP.UTF-8"), ("LC_ALL", "C")]));
        // Names are case-sensitive, as on the server.
        let opts = ShellOptions::default().with_env(pairs(&[("lang", "x")]));
        assert_eq!(opts.env, pairs(&[("LANG", "C.UTF-8"), ("lang", "x")]));
    }

    #[test]
    fn host_term_sets_the_terminal_type() {
        let opts = ShellOptions::default().with_env(pairs(&[("TERM", "vt100"), ("TZ", "UTC")]));
        assert_eq!(opts.term, "vt100");
        assert_eq!(opts.env, pairs(&[("LANG", "C.UTF-8"), ("TZ", "UTC")]));
        let opts = ShellOptions::default().with_env(pairs(&[("TERM", "")]));
        assert_eq!(opts.term, "xterm-256color");
        assert_eq!(opts.env, pairs(&[("LANG", "C.UTF-8")]));
    }
}
