//! A shell of this computer in a pseudo-terminal (LOCAL-01): ConPTY on Windows, a Unix PTY
//! elsewhere, through `portable-pty`.
//!
//! The PTY's reads and writes block, so each runs on a thread of its own: the reader hands what
//! it reads to a task that batches it as SSH output is batched (§10.3), every 8 ms or once 32 KB
//! have built up, into a bounded channel. When the WebView falls behind, the channel fills, the
//! reader waits, and the shell waits on its writes, so about 4 MB is held at most.
//!
//! The tab ends when the shell does. A process the shell left in the background can hold the PTY
//! open, and ConPTY keeps its output open until the pseudo-console is closed, so once the shell
//! has exited, what it printed last is read for a moment and the PTY is closed then.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, native_pty_system};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use crate::error::LocalError;
use crate::shells::Shell;

/// Output is sent at most this often while it keeps coming, as for an SSH shell.
const FLUSH_INTERVAL: Duration = Duration::from_millis(8);
/// … or as soon as this much has built up.
const FLUSH_BYTES: usize = 32 * 1024;
/// Batches in flight to the WebView: about 4 MB before the shell waits.
const EVENT_CAPACITY: usize = 128;
/// How long what the shell printed last is still read after it exited.
const EXIT_DRAIN: Duration = Duration::from_millis(300);

/// A terminal size in character cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PtySize {
    pub cols: u16,
    pub rows: u16,
}

impl From<PtySize> for portable_pty::PtySize {
    fn from(size: PtySize) -> Self {
        Self {
            rows: size.rows,
            cols: size.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

/// What a local terminal sends.
#[derive(Debug)]
pub enum PtyEvent {
    /// Output, in order.
    Data(Bytes),
    /// The shell ended, with its exit status when known. Always the last event.
    Exited { exit_status: Option<u32> },
}

type Master = Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>;

/// A running shell in a PTY. Dropping it, or [`close`](Self::close), ends the shell.
pub struct LocalPty {
    master: Master,
    input: std::sync::mpsc::Sender<Vec<u8>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
}

impl LocalPty {
    /// Starts `shell` in a new PTY of `size`, in `cwd`, with `env` added to Hatoba's own
    /// environment, and returns the terminal and its events.
    pub fn spawn(
        shell: &Shell,
        size: PtySize,
        env: &[(String, String)],
        cwd: Option<&Path>,
    ) -> Result<(Arc<Self>, mpsc::Receiver<PtyEvent>), LocalError> {
        let pair = native_pty_system()
            .openpty(size.into())
            .map_err(|e| LocalError::Spawn(e.to_string()))?;
        let mut command = CommandBuilder::new(&shell.program);
        command.args(&shell.args);
        if let Some(cwd) = cwd {
            command.cwd(cwd);
        }
        for (name, value) in env {
            command.env(name, value);
        }
        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| LocalError::Spawn(e.to_string()))?;
        // Only the shell holds the terminal's other end, so its exit ends the reads.
        drop(pair.slave);
        let killer = child.clone_killer();
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| LocalError::Spawn(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| LocalError::Spawn(e.to_string()))?;
        let master: Master = Arc::new(Mutex::new(Some(pair.master)));

        let (raw_tx, raw_rx) = mpsc::channel::<Bytes>(32);
        spawn_thread("hatoba-pty-read", move || read_loop(reader, &raw_tx));
        let (input, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        spawn_thread("hatoba-pty-write", move || write_loop(writer, &input_rx));
        let (exit_tx, exit_rx) = oneshot::channel();
        spawn_thread("hatoba-pty-wait", move || {
            let status = child.wait().ok().map(|s| s.exit_code());
            let _ = exit_tx.send(status);
        });

        let (events_tx, events_rx) = mpsc::channel(EVENT_CAPACITY);
        tokio::spawn(pump(raw_rx, exit_rx, events_tx, master.clone()));
        Ok((
            Arc::new(Self {
                master,
                input,
                killer: Mutex::new(killer),
            }),
            events_rx,
        ))
    }

    /// Sends input to the shell, in order with earlier input.
    pub fn write(&self, data: Vec<u8>) -> Result<(), LocalError> {
        if data.is_empty() {
            return Ok(());
        }
        self.input.send(data).map_err(|_| LocalError::Closed)
    }

    /// Tells the shell the terminal's new size.
    pub fn resize(&self, size: PtySize) {
        let master = self.master.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(master) = master.as_ref()
            && let Err(e) = master.resize(size.into())
        {
            tracing::debug!("local terminal resize failed: {e}");
        }
    }

    /// Ends the shell and closes the terminal. Closing the pseudo-console ends the console
    /// programs the shell started too, and on Unix the closed PTY hangs them up, as closing a tab
    /// of another terminal does.
    pub fn close(&self) {
        {
            let mut killer = self.killer.lock().unwrap_or_else(|p| p.into_inner());
            let _ = killer.kill();
        }
        release(&self.master);
    }
}

/// Closes the PTY for good. Closing a pseudo-console waits until its host has written what it
/// still holds, so that happens on a thread of its own, never on the caller's.
fn release(master: &Master) {
    let master = master.lock().unwrap_or_else(|p| p.into_inner()).take();
    if let Some(master) = master {
        spawn_thread("hatoba-pty-close", move || drop(master));
    }
}

impl Drop for LocalPty {
    fn drop(&mut self) {
        self.close();
    }
}

fn spawn_thread(name: &str, f: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.to_owned()).spawn(f) {
        tracing::error!("cannot start the local terminal's {name} thread: {e}");
    }
}

fn read_loop(mut reader: Box<dyn Read + Send>, raw: &mpsc::Sender<Bytes>) {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if raw
                    .blocking_send(Bytes::copy_from_slice(&buf[..n]))
                    .is_err()
                {
                    break;
                }
            }
        }
    }
}

fn write_loop(mut writer: Box<dyn Write + Send>, input: &std::sync::mpsc::Receiver<Vec<u8>>) {
    while let Ok(data) = input.recv() {
        if writer
            .write_all(&data)
            .and_then(|()| writer.flush())
            .is_err()
        {
            break;
        }
    }
}

/// Batches the reader's output into events until the shell has exited and its last output is
/// read, then closes the PTY and sends [`PtyEvent::Exited`].
async fn pump(
    mut raw: mpsc::Receiver<Bytes>,
    mut exit: oneshot::Receiver<Option<u32>>,
    events: mpsc::Sender<PtyEvent>,
    master: Master,
) {
    let mut pending = BytesMut::new();
    // Armed after a send: output within the interval waits for it, so the first output after a
    // quiet spell goes at once.
    let mut flush_at: Option<Instant> = None;
    let mut exited: Option<Option<u32>> = None;
    let mut drain_until: Option<Instant> = None;
    let mut reading = true;
    loop {
        tokio::select! {
            chunk = raw.recv(), if reading => match chunk {
                Some(chunk) => {
                    pending.extend_from_slice(&chunk);
                    if flush_at.is_none() || pending.len() >= FLUSH_BYTES {
                        if !send_pending(&mut pending, &events).await {
                            return;
                        }
                        flush_at = Some(Instant::now() + FLUSH_INTERVAL);
                    }
                }
                None => {
                    reading = false;
                    if exited.is_some() {
                        break;
                    }
                    // The reads ended first: the shell is exiting, and its status follows.
                    drain_until.get_or_insert_with(|| Instant::now() + Duration::from_secs(2));
                }
            },
            status = &mut exit, if exited.is_none() => {
                exited = Some(status.ok().flatten());
                if !reading {
                    break;
                }
                drain_until = Some(Instant::now() + EXIT_DRAIN);
            }
            () = sleep_until(flush_at), if flush_at.is_some() => {
                if pending.is_empty() {
                    flush_at = None;
                } else {
                    if !send_pending(&mut pending, &events).await {
                        return;
                    }
                    flush_at = Some(Instant::now() + FLUSH_INTERVAL);
                }
            }
            () = sleep_until(drain_until), if drain_until.is_some() => break,
        }
    }
    send_pending(&mut pending, &events).await;
    release(&master);
    // What the closing pseudo-console still writes is read and left out, so its host is never
    // stuck on a full pipe.
    if reading {
        let rest = async { while raw.recv().await.is_some() {} };
        let _ = tokio::time::timeout(Duration::from_secs(2), rest).await;
    }
    drop(raw);
    let _ = events
        .send(PtyEvent::Exited {
            exit_status: exited.flatten(),
        })
        .await;
}

/// Sends what has built up, at most [`FLUSH_BYTES`] an event. `false` once nobody listens.
async fn send_pending(pending: &mut BytesMut, events: &mpsc::Sender<PtyEvent>) -> bool {
    while !pending.is_empty() {
        let n = pending.len().min(FLUSH_BYTES);
        let chunk = pending.split_to(n).freeze();
        if events.send(PtyEvent::Data(chunk)).await.is_err() {
            return false;
        }
    }
    true
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shells::ShellKind;

    fn size() -> PtySize {
        PtySize { cols: 80, rows: 24 }
    }

    async fn output_until(
        events: &mut mpsc::Receiver<PtyEvent>,
        needle: &str,
    ) -> (String, Option<Option<u32>>) {
        let mut text = String::new();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let Ok(event) = tokio::time::timeout_at(deadline, events.recv()).await else {
                panic!("the shell did not answer in time; it printed {text:?}");
            };
            match event {
                Some(PtyEvent::Data(bytes)) => {
                    text.push_str(&String::from_utf8_lossy(&bytes));
                    if !needle.is_empty() && text.contains(needle) {
                        return (text, None);
                    }
                }
                Some(PtyEvent::Exited { exit_status }) => return (text, Some(exit_status)),
                None => return (text, None),
            }
        }
    }

    fn test_shell() -> Shell {
        #[cfg(windows)]
        {
            crate::shells::detect()
                .into_iter()
                .find(|s| s.kind == ShellKind::Cmd)
                .expect("cmd.exe")
        }
        #[cfg(not(windows))]
        {
            Shell {
                id: "/bin/sh".into(),
                name: "sh".into(),
                kind: ShellKind::Unix,
                program: "/bin/sh".into(),
                args: Vec::new(),
            }
        }
    }

    #[tokio::test]
    async fn runs_input_and_reports_the_exit_status() {
        let shell = test_shell();
        let (pty, mut events) =
            LocalPty::spawn(&shell, size(), &[], std::env::temp_dir().parent()).unwrap();
        pty.resize(PtySize {
            cols: 100,
            rows: 30,
        });
        // ConPTY asks where the cursor is before it starts the shell, and xterm.js answers; so
        // does the test, which any shell takes as input it ignores.
        pty.write(b"[1;1R".to_vec()).unwrap();
        pty.write(b"echo hatoba-ok-$((40+2))%PROMPT:~0,0%\r".to_vec())
            .unwrap();
        // sh expands $((…)), cmd the empty substring; either prints a line only the command could.
        let (text, _) = output_until(&mut events, "hatoba-ok-").await;
        assert!(text.contains("hatoba-ok-"), "{text}");
        pty.write(b"exit 3\r".to_vec()).unwrap();
        let (_, exited) = output_until(&mut events, "").await;
        assert_eq!(exited, Some(Some(3)));
    }

    #[tokio::test]
    async fn close_ends_the_shell() {
        let shell = test_shell();
        let (pty, mut events) = LocalPty::spawn(&shell, size(), &[], None).unwrap();
        pty.close();
        let (_, exited) = output_until(&mut events, "").await;
        assert!(exited.is_some());
    }
}
