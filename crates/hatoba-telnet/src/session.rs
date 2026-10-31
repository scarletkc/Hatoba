//! A Telnet connection behind a terminal tab (TELNET-02): one task reads the server, answers its
//! commands, types the saved credentials at the login prompts, and batches the output as SSH
//! output is batched (§10.3), every 8 ms or once 32 KB have built up, into a bounded channel. When
//! the WebView falls behind, the channel fills and the server is no longer read, so about 4 MB is
//! held at most. Writes go out on a task of their own, so a server that stops reading never stalls
//! the output; once 1 MB of the client's own protocol replies waits for such a server, the
//! connection closes. While the server leaves echo or line editing to the client, the input goes
//! through [`LineDiscipline`], whose echo joins the output.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::codec::Telnet;
use crate::error::TelnetError;
use crate::line::LineDiscipline;
use crate::login::{AutoLogin, LOGIN_WINDOW};

const FLUSH_INTERVAL: Duration = Duration::from_millis(8);
const FLUSH_BYTES: usize = 32 * 1024;
const EVENT_CAPACITY: usize = 128;
/// How much of the client's protocol replies may wait to go out before the server is taken to have
/// stopped reading. A server that keeps sending option offers while it reads nothing would
/// otherwise grow them without end. The user's input does not count: a large paste waits as long
/// as it needs.
const OUT_LIMIT: usize = 1024 * 1024;
/// Why the connection closed when [`OUT_LIMIT`] was reached.
const STALLED: &str = "The server stopped reading what Hatoba sends.";

/// What a Telnet tab sends.
#[derive(Debug)]
pub enum TelnetEvent {
    /// Output, in order.
    Data(Bytes),
    /// The connection ended, closed by either side when `error` is `None`. Always the last
    /// event.
    Closed { error: Option<String> },
}

enum Command {
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    StopLogin,
    Close,
}

/// A running Telnet connection. Dropping it, or [`close`](Self::close), closes the connection.
pub struct TelnetSession {
    commands: mpsc::UnboundedSender<Command>,
    /// Set by [`stop_login`](Self::stop_login). The task reads it before it types, so output
    /// it reads after the stop gets nothing typed, even before the command arrives.
    login_stopped: Arc<AtomicBool>,
}

impl TelnetSession {
    /// Starts the protocol on `stream`, a connection to the server, with a window of `cols` ×
    /// `rows`, and returns the session and its events. With `negotiate`, as on the Telnet port,
    /// the client opens the option negotiation ([`Telnet::start`]); otherwise it only answers the
    /// server's.
    pub fn start<S>(
        stream: S,
        cols: u16,
        rows: u16,
        negotiate: bool,
        login: AutoLogin,
    ) -> (Arc<Self>, mpsc::Receiver<TelnetEvent>)
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let (commands, commands_rx) = mpsc::unbounded_channel();
        let (events_tx, events_rx) = mpsc::channel(EVENT_CAPACITY);
        let login_stopped = Arc::new(AtomicBool::new(false));
        tokio::spawn(run(
            stream,
            cols,
            rows,
            negotiate,
            login,
            login_stopped.clone(),
            commands_rx,
            events_tx,
        ));
        (
            Arc::new(Self {
                commands,
                login_stopped,
            }),
            events_rx,
        )
    }

    /// Sends the terminal's input to the server, in order with earlier input.
    pub fn write(&self, data: Vec<u8>) -> Result<(), TelnetError> {
        if data.is_empty() {
            return Ok(());
        }
        self.commands
            .send(Command::Input(data))
            .map_err(|_| TelnetError::Closed)
    }

    /// Tells the server the terminal's new size, when it asked for it (NAWS).
    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = self.commands.send(Command::Resize { cols, rows });
    }

    /// Stops typing the saved login and wipes it, as when the vault locks (SEC-03); the
    /// connection stays.
    pub fn stop_login(&self) {
        self.login_stopped.store(true, Ordering::SeqCst);
        // The command wipes the credentials, also while the server sends nothing.
        let _ = self.commands.send(Command::StopLogin);
    }

    /// Closes the connection.
    pub fn close(&self) {
        let _ = self.commands.send(Command::Close);
    }
}

impl Drop for TelnetSession {
    fn drop(&mut self) {
        self.close();
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "what `TelnetSession::start` takes, with the session's channels"
)]
async fn run<S>(
    stream: S,
    cols: u16,
    rows: u16,
    negotiate: bool,
    mut login: AutoLogin,
    login_stopped: Arc<AtomicBool>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: mpsc::Sender<TelnetEvent>,
) where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (mut reader, writer) = tokio::io::split(stream);
    let (tx, out_rx) = mpsc::unbounded_channel();
    let queued = Arc::new(AtomicUsize::new(0));
    let out = Outbox {
        tx,
        queued: queued.clone(),
    };
    let mut write_task = tokio::spawn(write_loop(writer, out_rx, queued));

    let mut telnet = Telnet::new(cols, rows);
    if negotiate {
        out.send(telnet.start());
    }
    let mut line = LineDiscipline::default();
    let login_until = Instant::now() + LOGIN_WINDOW;

    let mut buf = vec![0u8; 16 * 1024];
    let mut pending = BytesMut::new();
    let mut flush_at: Option<Instant> = None;
    let error = loop {
        tokio::select! {
            read = reader.read(&mut buf) => match read {
                Ok(0) => break None,
                Ok(n) => {
                    let received = telnet.receive(&buf[..n]);
                    if !out.reply(received.reply) {
                        break Some(STALLED.to_owned());
                    }
                    if telnet.server_suppresses_go_ahead() {
                        out.send(telnet.encode(&line.flush()));
                    }
                    if Instant::now() >= login_until || login_stopped.load(Ordering::SeqCst) {
                        login.stop();
                    }
                    if let Some(answer) = login.output(&received.data) {
                        out.send(telnet.encode(answer.as_bytes()));
                    }
                    if !show(&mut pending, &mut flush_at, &events, &received.data).await {
                        write_task.abort();
                        return;
                    }
                }
                Err(e) => break Some(e.to_string()),
            },
            command = commands.recv() => match command {
                Some(Command::Input(data)) => {
                    login.typed(&data);
                    let typed = line.input(
                        &data,
                        !telnet.server_echoes(),
                        !telnet.server_suppresses_go_ahead(),
                    );
                    out.send(telnet.encode(&typed.send));
                    if !show(&mut pending, &mut flush_at, &events, &typed.echo).await {
                        write_task.abort();
                        return;
                    }
                }
                Some(Command::Resize { cols, rows }) => out.send(telnet.resize(cols, rows)),
                Some(Command::StopLogin) => login.stop(),
                Some(Command::Close) | None => break None,
            },
            written = &mut write_task => {
                break match written {
                    Ok(Err(e)) => Some(e.to_string()),
                    _ => None,
                };
            }
            () = sleep_until(flush_at), if flush_at.is_some() => {
                if pending.is_empty() {
                    flush_at = None;
                } else {
                    if !send_pending(&mut pending, &events).await {
                        write_task.abort();
                        return;
                    }
                    flush_at = Some(Instant::now() + FLUSH_INTERVAL);
                }
            }
        }
    };
    write_task.abort();
    send_pending(&mut pending, &events).await;
    let _ = events.send(TelnetEvent::Closed { error }).await;
}

/// What waits to go out to the server: bytes, and whether they are a protocol reply, which counts
/// against [`OUT_LIMIT`].
type Outgoing = (Vec<u8>, bool);

struct Outbox {
    tx: mpsc::UnboundedSender<Outgoing>,
    /// The protocol replies' bytes that wait.
    queued: Arc<AtomicUsize>,
}

impl Outbox {
    /// Queues what the user typed, or a window size or login they caused.
    fn send(&self, data: Vec<u8>) {
        if !data.is_empty() {
            let _ = self.tx.send((data, false));
        }
    }

    /// Queues a reply to the server's commands; `false` when more than [`OUT_LIMIT`] of replies
    /// would then wait.
    fn reply(&self, data: Vec<u8>) -> bool {
        if data.is_empty() {
            return true;
        }
        if self.queued.fetch_add(data.len(), Ordering::SeqCst) + data.len() > OUT_LIMIT {
            return false;
        }
        let _ = self.tx.send((data, true));
        true
    }
}

async fn write_loop<W: AsyncWrite>(
    writer: W,
    mut out: mpsc::UnboundedReceiver<Outgoing>,
    queued: Arc<AtomicUsize>,
) -> std::io::Result<()> {
    let mut writer = std::pin::pin!(writer);
    while let Some((data, reply)) = out.recv().await {
        writer.write_all(&data).await?;
        writer.flush().await?;
        if reply {
            queued.fetch_sub(data.len(), Ordering::SeqCst);
        }
    }
    let _ = writer.shutdown().await;
    Ok(())
}

/// Adds output for the terminal to what has built up, and sends it when it is due. `false` once
/// nobody listens.
async fn show(
    pending: &mut BytesMut,
    flush_at: &mut Option<Instant>,
    events: &mpsc::Sender<TelnetEvent>,
    data: &[u8],
) -> bool {
    if data.is_empty() {
        return true;
    }
    pending.extend_from_slice(data);
    if flush_at.is_none() || pending.len() >= FLUSH_BYTES {
        if !send_pending(pending, events).await {
            return false;
        }
        *flush_at = Some(Instant::now() + FLUSH_INTERVAL);
    }
    true
}

/// Sends what has built up, at most [`FLUSH_BYTES`] an event. `false` once nobody listens.
async fn send_pending(pending: &mut BytesMut, events: &mpsc::Sender<TelnetEvent>) -> bool {
    while !pending.is_empty() {
        let n = pending.len().min(FLUSH_BYTES);
        let chunk = pending.split_to(n).freeze();
        if events.send(TelnetEvent::Data(chunk)).await.is_err() {
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
    use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

    use super::*;

    async fn read_some(server: &mut tokio::io::DuplexStream) -> Vec<u8> {
        let mut buf = vec![0u8; 1024];
        let n = tokio::time::timeout(Duration::from_secs(5), server.read(&mut buf))
            .await
            .expect("the client wrote nothing")
            .unwrap();
        buf.truncate(n);
        buf
    }

    async fn next_data(events: &mut mpsc::Receiver<TelnetEvent>) -> Vec<u8> {
        match tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
            Ok(Some(TelnetEvent::Data(data))) => data.to_vec(),
            other => panic!("expected output, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn logs_in_and_passes_the_terminal_through() {
        let (client, mut server) = duplex(4096);
        let login = AutoLogin::new(Some("admin".into()), Some("pw".into()));
        let (session, mut events) = TelnetSession::start(client, 80, 24, true, login);

        // The client's offers come first.
        assert_eq!(read_some(&mut server).await[..3], [255, 251, 31]);

        server.write_all(b"login: ").await.unwrap();
        assert_eq!(next_data(&mut events).await, b"login: ");
        assert_eq!(read_some(&mut server).await, b"admin\r\n");

        server.write_all(b"\r\nPassword: ").await.unwrap();
        assert_eq!(read_some(&mut server).await, b"pw\r\n");

        session.write(b"ls\r".to_vec()).unwrap();
        assert_eq!(read_some(&mut server).await, b"ls\r\n");

        drop(server);
        loop {
            match events.recv().await {
                Some(TelnetEvent::Data(_)) => {}
                Some(TelnetEvent::Closed { error }) => {
                    assert_eq!(error, None);
                    break;
                }
                None => panic!("the session ended without Closed"),
            }
        }
    }

    #[tokio::test]
    async fn closing_the_session_ends_it_with_closed() {
        let (client, mut server) = duplex(4096);
        let (session, mut events) =
            TelnetSession::start(client, 80, 24, true, AutoLogin::new(None, None));
        read_some(&mut server).await;
        session.close();
        match tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
            Ok(Some(TelnetEvent::Closed { error: None })) => {}
            other => panic!("expected Closed, got {other:?}"),
        }
        assert!(session.write(b"x".to_vec()).is_err());
    }

    #[tokio::test]
    async fn a_stopped_login_types_nothing_at_the_prompts() {
        let (client, mut server) = duplex(4096);
        let login = AutoLogin::new(Some("admin".into()), Some("pw".into()));
        let (session, mut events) = TelnetSession::start(client, 80, 24, true, login);
        read_some(&mut server).await;
        // The prompt comes right after the stop, before the task can have read its command.
        session.stop_login();
        server.write_all(b"login: ").await.unwrap();
        assert_eq!(next_data(&mut events).await, b"login: ");
        server.write_all(b"\r\nPassword: ").await.unwrap();
        next_data(&mut events).await;
        let mut buf = [0u8; 64];
        let typed = tokio::time::timeout(Duration::from_millis(300), server.read(&mut buf)).await;
        assert!(typed.is_err(), "the client typed {:?}", typed);
    }

    #[tokio::test]
    async fn a_server_that_stops_reading_is_disconnected() {
        let (client, mut server) = duplex(4096);
        let (_session, mut events) =
            TelnetSession::start(client, 80, 24, true, AutoLogin::new(None, None));
        // The server reads nothing, and offers an option it is refused, over and over.
        tokio::spawn(async move {
            let offer = [255u8, 251, 99].repeat(4096);
            for _ in 0..200 {
                if server.write_all(&offer).await.is_err() {
                    break;
                }
            }
        });
        let error = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                match events.recv().await {
                    Some(TelnetEvent::Closed { error }) => break error,
                    Some(TelnetEvent::Data(_)) => {}
                    None => panic!("the session ended without Closed"),
                }
            }
        })
        .await
        .expect("the session stayed open");
        assert_eq!(error.as_deref(), Some(STALLED));
    }

    #[tokio::test]
    async fn a_large_paste_reaches_a_server_that_reads() {
        let (client, mut server) = duplex(4096);
        let (session, mut events) =
            TelnetSession::start(client, 80, 24, true, AutoLogin::new(None, None));
        read_some(&mut server).await;
        // The server echoes and takes characters, so the paste goes as it is typed.
        server.write_all(&[255, 251, 1, 255, 251, 3]).await.unwrap();
        server.write_all(b"$ ").await.unwrap();
        assert_eq!(next_data(&mut events).await, b"$ ");
        let paste = vec![b'a'; 3 * OUT_LIMIT];
        session.write(paste.clone()).unwrap();
        let mut received = vec![0u8; paste.len()];
        tokio::time::timeout(Duration::from_secs(30), server.read_exact(&mut received))
            .await
            .expect("the paste did not arrive")
            .unwrap();
        assert_eq!(received, paste);
        assert!(events.try_recv().is_err(), "the session ended");
    }

    #[tokio::test]
    async fn on_another_port_the_server_opens_the_negotiation() {
        let (client, mut server) = duplex(4096);
        let (_session, _events) =
            TelnetSession::start(client, 80, 24, false, AutoLogin::new(None, None));
        let mut buf = [0u8; 64];
        let sent = tokio::time::timeout(Duration::from_millis(300), server.read(&mut buf)).await;
        assert!(sent.is_err(), "the client sent {sent:?}");
        // The server's own requests are answered.
        server.write_all(&[255, 253, 31]).await.unwrap();
        assert_eq!(
            read_some(&mut server).await,
            [255, 251, 31, 255, 250, 31, 0, 80, 0, 24, 255, 240]
        );
    }

    #[tokio::test]
    async fn a_server_that_leaves_echo_and_editing_to_the_client_gets_lines_shown_here() {
        let (client, mut server) = duplex(4096);
        let (session, mut events) =
            TelnetSession::start(client, 80, 24, false, AutoLogin::new(None, None));
        server.write_all(b"220 mail ESMTP\r\n").await.unwrap();
        assert_eq!(next_data(&mut events).await, b"220 mail ESMTP\r\n");
        session.write(b"EHLX\x7fO x\r".to_vec()).unwrap();
        assert_eq!(read_some(&mut server).await, b"EHLO x\r\n");
        let echo = b"EHLX\x08 \x08O x\r\n";
        let mut shown = Vec::new();
        while shown.len() < echo.len() {
            shown.extend(next_data(&mut events).await);
        }
        assert_eq!(shown, echo);
    }

    #[tokio::test]
    async fn resizes_reach_the_server_once_it_asks() {
        let (client, mut server) = duplex(4096);
        let (session, _events) =
            TelnetSession::start(client, 80, 24, true, AutoLogin::new(None, None));
        read_some(&mut server).await;
        server.write_all(&[255, 253, 31]).await.unwrap();
        assert_eq!(
            read_some(&mut server).await,
            [255, 250, 31, 0, 80, 0, 24, 255, 240]
        );
        session.resize(120, 40);
        assert_eq!(
            read_some(&mut server).await,
            [255, 250, 31, 0, 120, 0, 40, 255, 240]
        );
    }
}
