//! A Telnet connection behind a terminal tab (TELNET-02): one task reads the server, answers its
//! commands, types the saved credentials at the login prompts, and batches the output as SSH
//! output is batched (§10.3), every 8 ms or once 32 KB have built up, into a bounded channel. When
//! the WebView falls behind, the channel fills and the server is no longer read, so about 4 MB is
//! held at most. Writes go out on a task of their own, so a server that stops reading never stalls
//! the output.

use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::codec::Telnet;
use crate::error::TelnetError;
use crate::login::{AutoLogin, LOGIN_WINDOW};

const FLUSH_INTERVAL: Duration = Duration::from_millis(8);
const FLUSH_BYTES: usize = 32 * 1024;
const EVENT_CAPACITY: usize = 128;

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
    Close,
}

/// A running Telnet connection. Dropping it, or [`close`](Self::close), closes the connection.
pub struct TelnetSession {
    commands: mpsc::UnboundedSender<Command>,
}

impl TelnetSession {
    /// Starts the protocol on `stream`, a connection to the server, with a window of `cols` ×
    /// `rows`, and returns the session and its events.
    pub fn start<S>(
        stream: S,
        cols: u16,
        rows: u16,
        login: AutoLogin,
    ) -> (Arc<Self>, mpsc::Receiver<TelnetEvent>)
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let (commands, commands_rx) = mpsc::unbounded_channel();
        let (events_tx, events_rx) = mpsc::channel(EVENT_CAPACITY);
        tokio::spawn(run(stream, cols, rows, login, commands_rx, events_tx));
        (Arc::new(Self { commands }), events_rx)
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

async fn run<S>(
    stream: S,
    cols: u16,
    rows: u16,
    mut login: AutoLogin,
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: mpsc::Sender<TelnetEvent>,
) where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (mut reader, writer) = tokio::io::split(stream);
    let (out, out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let mut write_task = tokio::spawn(write_loop(writer, out_rx));

    let mut telnet = Telnet::new(cols, rows);
    let _ = out.send(telnet.start());
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
                    if !received.reply.is_empty() {
                        let _ = out.send(received.reply);
                    }
                    if Instant::now() >= login_until {
                        login.stop();
                    }
                    if let Some(answer) = login.output(&received.data) {
                        let _ = out.send(telnet.encode(answer.as_bytes()));
                    }
                    pending.extend_from_slice(&received.data);
                    if flush_at.is_none() || pending.len() >= FLUSH_BYTES {
                        if !send_pending(&mut pending, &events).await {
                            return;
                        }
                        flush_at = Some(Instant::now() + FLUSH_INTERVAL);
                    }
                }
                Err(e) => break Some(e.to_string()),
            },
            command = commands.recv() => match command {
                Some(Command::Input(data)) => {
                    login.typed(&data);
                    let _ = out.send(telnet.encode(&data));
                }
                Some(Command::Resize { cols, rows }) => {
                    let naws = telnet.resize(cols, rows);
                    if !naws.is_empty() {
                        let _ = out.send(naws);
                    }
                }
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

async fn write_loop<W: AsyncWrite>(
    writer: W,
    mut out: mpsc::UnboundedReceiver<Vec<u8>>,
) -> std::io::Result<()> {
    let mut writer = std::pin::pin!(writer);
    while let Some(data) = out.recv().await {
        if data.is_empty() {
            continue;
        }
        writer.write_all(&data).await?;
        writer.flush().await?;
    }
    let _ = writer.shutdown().await;
    Ok(())
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
        let (session, mut events) = TelnetSession::start(client, 80, 24, login);

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
            TelnetSession::start(client, 80, 24, AutoLogin::new(None, None));
        read_some(&mut server).await;
        session.close();
        match tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
            Ok(Some(TelnetEvent::Closed { error: None })) => {}
            other => panic!("expected Closed, got {other:?}"),
        }
        assert!(session.write(b"x".to_vec()).is_err());
    }

    #[tokio::test]
    async fn resizes_reach_the_server_once_it_asks() {
        let (client, mut server) = duplex(4096);
        let (session, _events) = TelnetSession::start(client, 80, 24, AutoLogin::new(None, None));
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
