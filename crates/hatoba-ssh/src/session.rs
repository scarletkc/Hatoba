//! Connecting (including ProxyJump chains) and the [`SshSession`] handle.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use russh::client::{self, Handle, Msg};
use russh::{Channel, ChannelMsg, Disconnect};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use crate::auth::{self, AuthContext};
use crate::error::{SshError, SshErrorKind};
use crate::forward::{self, ForwardHandle, LocalForward};
use crate::handler::{ClientHandler, CloseReason, SessionShared};
use crate::hostkey::HostKeyVerifier;
use crate::interactive::KeyboardInteractive;
use crate::net::{ConnectClock, Phase, ceil_ms, tcp_connect};
use crate::sftp::SftpClient;
use crate::shell;
pub use crate::shell::{ShellEvent, ShellHandle, ShellOptions};

/// Upper bound for output collected by [`SshSession::exec_output`].
const MAX_EXEC_OUTPUT: usize = 64 * 1024 * 1024;

/// How to prove who we are to a server.
#[derive(Clone)]
pub enum AuthMethod {
    /// Password; also answers a lone password prompt of keyboard-interactive.
    Password(Zeroizing<String>),
    /// Private key. `openssh` is normally an OpenSSH private key PEM (as
    /// produced by [`crate::parse_private_key`]), `passphrase` decrypts it.
    PrivateKey {
        /// Key text.
        openssh: Zeroizing<String>,
        /// Passphrase of an encrypted key.
        passphrase: Option<Zeroizing<String>>,
    },
    /// Keys held by ssh-agent (`SSH_AUTH_SOCK` / Windows OpenSSH pipe).
    Agent,
    /// No credentials (only useful for servers that allow it).
    None,
}

impl fmt::Debug for AuthMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Password(_) => "AuthMethod::Password(<redacted>)",
            Self::PrivateKey { .. } => "AuthMethod::PrivateKey { <redacted> }",
            Self::Agent => "AuthMethod::Agent",
            Self::None => "AuthMethod::None",
        })
    }
}

/// One intermediate host of a ProxyJump chain.
#[derive(Debug, Clone)]
pub struct JumpHop {
    /// Address of the jump host.
    pub host: String,
    /// SSH port of the jump host.
    pub port: u16,
    /// Login name on the jump host.
    pub username: String,
    /// Credentials for the jump host.
    pub auth: AuthMethod,
}

/// Everything needed to open a session.
#[derive(Clone)]
pub struct ConnectConfig {
    /// Target address (DNS name or IP).
    pub host: String,
    /// Target SSH port.
    pub port: u16,
    /// Login name on the target.
    pub username: String,
    /// Credentials for the target.
    pub auth: AuthMethod,
    /// ProxyJump chain, outermost (first to connect) first.
    pub jump: Vec<JumpHop>,
    /// Budget for DNS + TCP + handshake + authentication of each hop.
    /// Time spent waiting for the host key verifier or the user is not counted.
    pub connect_timeout: Duration,
    /// Send an SSH keepalive after this much silence (zero disables).
    pub keepalive_interval: Duration,
    /// Unanswered keepalives before the connection is declared dead (0 = never).
    pub keepalive_max: u32,
    /// Answers keyboard-interactive prompts (2FA / OTP).
    pub keyboard_interactive: Option<Arc<dyn KeyboardInteractive>>,
}

impl ConnectConfig {
    /// A config with the defaults: 15 s connect timeout, 30 s keepalive, 3 misses.
    pub fn new(
        host: impl Into<String>,
        port: u16,
        username: impl Into<String>,
        auth: AuthMethod,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            username: username.into(),
            auth,
            jump: Vec::new(),
            connect_timeout: Duration::from_secs(15),
            keepalive_interval: Duration::from_secs(30),
            keepalive_max: 3,
            keyboard_interactive: None,
        }
    }
}

impl fmt::Debug for ConnectConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("auth", &self.auth)
            .field("jump", &self.jump)
            .field("connect_timeout", &self.connect_timeout)
            .field("keepalive_interval", &self.keepalive_interval)
            .field("keepalive_max", &self.keepalive_max)
            .finish_non_exhaustive()
    }
}

fn russh_config(cfg: &ConnectConfig) -> Arc<client::Config> {
    let keepalive = (!cfg.keepalive_interval.is_zero()).then_some(cfg.keepalive_interval);
    // Backstop for writes that stall on a dead peer (keepalives cannot help
    // while the event loop is blocked in `write`), and for keepalive replies
    // that never arrive.
    let inactivity = keepalive
        .filter(|_| cfg.keepalive_max > 0)
        .map(|i| i.saturating_mul(cfg.keepalive_max.saturating_add(2)));
    Arc::new(client::Config {
        keepalive_interval: keepalive,
        keepalive_max: cfg.keepalive_max as usize,
        inactivity_timeout: inactivity,
        nodelay: true,
        ..client::Config::default()
    })
}

/// A connected, authenticated hop.
struct Hop {
    handle: Handle<ClientHandler>,
    shared: Arc<SessionShared>,
    /// TCP connect time (only for hops reached directly over TCP).
    rtt: Option<Duration>,
}

struct HopSpec<'a> {
    host: &'a str,
    port: u16,
    username: &'a str,
    auth: &'a AuthMethod,
}

async fn connect_hop(
    config: &Arc<client::Config>,
    spec: &HopSpec<'_>,
    via: Option<&Handle<ClientHandler>>,
    verifier: &Arc<dyn HostKeyVerifier>,
    interactive: Option<&Arc<dyn KeyboardInteractive>>,
    budget: Duration,
) -> Result<Hop, SshError> {
    let clock = ConnectClock::new();
    let shared = SessionShared::new();
    let handler = ClientHandler::new(
        spec.host.to_owned(),
        spec.port,
        Arc::clone(verifier),
        Arc::clone(&shared),
        Arc::clone(&clock),
    );

    let work = async {
        let (mut handle, rtt) = match via {
            None => {
                let (stream, rtt) = tcp_connect(spec.host, spec.port, &clock).await?;
                clock.set_phase(Phase::Handshake);
                let handle = client::connect_stream(Arc::clone(config), stream, handler).await?;
                (handle, Some(rtt))
            }
            Some(previous) => {
                clock.set_phase(Phase::Channel);
                let channel = previous
                    .channel_open_direct_tcpip(spec.host, u32::from(spec.port), "127.0.0.1", 0)
                    .await
                    .map_err(|e| {
                        SshError::from(e).with_context(&format!(
                            "jump host could not open a tunnel to {}:{}",
                            spec.host, spec.port
                        ))
                    })?;
                clock.set_phase(Phase::Handshake);
                let handle =
                    client::connect_stream(Arc::clone(config), channel.into_stream(), handler)
                        .await?;
                (handle, None)
            }
        };
        clock.set_phase(Phase::Auth);
        let ctx = AuthContext {
            host: spec.host,
            username: spec.username,
            interactive,
            clock: &clock,
            shared: &shared,
        };
        auth::authenticate(&mut handle, &ctx, spec.auth).await?;
        Ok::<_, SshError>((handle, rtt))
    };

    let (handle, rtt) = clock.run(budget, work).await??;
    Ok(Hop {
        handle,
        shared: Arc::clone(&shared),
        rtt,
    })
}

/// Opens an authenticated SSH session to `cfg.host`, going through the
/// `cfg.jump` chain if present.
///
/// `cfg.connect_timeout` bounds DNS + TCP + handshake + authentication of each
/// hop; time spent in the `verifier` or the keyboard-interactive callback does
/// not count. Failures are classified (see [`SshErrorKind`]).
pub async fn connect(
    cfg: ConnectConfig,
    verifier: Arc<dyn HostKeyVerifier>,
) -> Result<SshSession, SshError> {
    let config = russh_config(&cfg);
    let interactive = cfg.keyboard_interactive.as_ref();

    let mut specs: Vec<HopSpec<'_>> = cfg
        .jump
        .iter()
        .map(|j| HopSpec {
            host: &j.host,
            port: j.port,
            username: &j.username,
            auth: &j.auth,
        })
        .collect();
    specs.push(HopSpec {
        host: &cfg.host,
        port: cfg.port,
        username: &cfg.username,
        auth: &cfg.auth,
    });
    let total = specs.len();

    let mut jumps: Vec<JumpConn> = Vec::new();
    let mut first_rtt = None;
    let mut last: Option<Hop> = None;
    for (i, spec) in specs.iter().enumerate() {
        let via = jumps.last().map(|j| &j.handle);
        let hop = connect_hop(
            &config,
            spec,
            via,
            &verifier,
            interactive,
            cfg.connect_timeout,
        )
        .await
        .map_err(|e| {
            if total > 1 {
                e.with_context(&format!(
                    "hop {}/{} ({}:{})",
                    i + 1,
                    total,
                    spec.host,
                    spec.port
                ))
            } else {
                e
            }
        })?;
        if first_rtt.is_none() {
            first_rtt = hop.rtt;
        }
        if i + 1 < total {
            jumps.push(JumpConn {
                label: format!("{}:{}", spec.host, spec.port),
                handle: hop.handle,
                shared: hop.shared,
            });
        } else {
            last = Some(hop);
        }
    }
    let hop = last.ok_or_else(|| SshError::other("no hop to connect to"))?;

    let inner = Arc::new(Inner {
        handle: hop.handle,
        jumps,
        shared: hop.shared,
        latency_ms: first_rtt.map(ceil_ms).unwrap_or(0),
        host: cfg.host,
        port: cfg.port,
        username: cfg.username,
    });
    spawn_monitor(&inner);
    Ok(SshSession { inner })
}

/// An authenticated earlier hop of a ProxyJump chain.
struct JumpConn {
    label: String,
    /// Must stay alive for as long as the tunnel through it is used.
    handle: Handle<ClientHandler>,
    shared: Arc<SessionShared>,
}

/// russh does not always call the handler when a connection dies (for
/// example when shutting down the transport fails first), and a connection
/// tunnelled through a jump host dies silently with the jump host. This task
/// notices both and records a close reason so `closed()`, shells and forwards
/// are notified. It ends once the connection is marked closed or the session
/// is dropped.
fn spawn_monitor(inner: &Arc<Inner>) {
    const TICK: Duration = Duration::from_millis(250);
    let weak = Arc::downgrade(inner);
    tokio::spawn(async move {
        let mut suspected = false;
        loop {
            tokio::time::sleep(TICK).await;
            let Some(inner) = weak.upgrade() else { return };
            if inner.shared.is_closed() {
                return;
            }
            let jump_down = inner.jumps.iter().find(|j| j.handle.is_closed());
            if jump_down.is_none() && !inner.handle.is_closed() {
                suspected = false;
                continue;
            }
            // Give the handler one tick to report the precise reason first.
            if !suspected {
                suspected = true;
                continue;
            }
            let err = match jump_down {
                Some(j) => {
                    let cause = match j.shared.reason() {
                        Some(CloseReason::Error(e)) => e.message,
                        Some(CloseReason::Remote(m)) => m,
                        _ => "connection closed".to_owned(),
                    };
                    SshError::disconnected(format!(
                        "the connection through jump host {} was lost: {cause}",
                        j.label
                    ))
                }
                None => SshError::disconnected("connection closed"),
            };
            inner.shared.mark_closed(CloseReason::Error(err));
            return;
        }
    });
}

struct Inner {
    handle: Handle<ClientHandler>,
    jumps: Vec<JumpConn>,
    shared: Arc<SessionShared>,
    latency_ms: u32,
    host: String,
    port: u16,
    username: String,
}

/// Result of [`SshSession::exec_output`].
#[derive(Debug, Clone, Default)]
pub struct ExecOutput {
    /// Exit status, `None` if the command was killed by a signal.
    pub exit_status: Option<u32>,
    /// Everything the command wrote to stdout.
    pub stdout: Vec<u8>,
    /// Everything the command wrote to stderr.
    pub stderr: Vec<u8>,
}

/// An authenticated SSH connection. Cheap to clone (reference counted);
/// the connection closes when the last clone, shell, SFTP client and
/// forward are gone, or on [`disconnect`](Self::disconnect).
#[derive(Clone)]
pub struct SshSession {
    inner: Arc<Inner>,
}

impl fmt::Debug for SshSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshSession")
            .field("host", &self.inner.host)
            .field("port", &self.inner.port)
            .field("username", &self.inner.username)
            .field("closed", &self.is_closed())
            .finish()
    }
}

impl SshSession {
    /// TCP connect round-trip time of the first hop, in milliseconds
    /// (at least 1).
    pub fn latency_ms(&self) -> u32 {
        self.inner.latency_ms
    }

    /// Target host as configured.
    pub fn host(&self) -> &str {
        &self.inner.host
    }

    /// Target port.
    pub fn port(&self) -> u16 {
        self.inner.port
    }

    /// Login name used on the target.
    pub fn username(&self) -> &str {
        &self.inner.username
    }

    /// The identification string the target server sent in the version exchange, such as
    /// `SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13.5`, with control characters dropped and at most
    /// 255 characters. `None` when nothing printable was sent.
    pub fn server_id(&self) -> Option<&str> {
        self.inner.shared.server_id()
    }

    /// `true` once the connection has ended for any reason.
    pub fn is_closed(&self) -> bool {
        self.inner.shared.is_closed() || self.inner.handle.is_closed()
    }

    /// Completes when the connection ends and returns why.
    pub async fn closed(&self) -> SshError {
        let mut rx = self.inner.shared.subscribe();
        let _ = rx.wait_for(|closed| *closed).await;
        self.close_error()
    }

    pub(crate) fn shared(&self) -> &Arc<SessionShared> {
        &self.inner.shared
    }

    pub(crate) fn handle(&self) -> &Handle<ClientHandler> {
        &self.inner.handle
    }

    /// Describes why the connection ended as an error.
    pub(crate) fn close_error(&self) -> SshError {
        match self.inner.shared.reason() {
            Some(CloseReason::Error(e)) => e,
            Some(CloseReason::Remote(msg)) => SshError::disconnected(msg),
            Some(CloseReason::Local) => SshError::disconnected("disconnected by the client"),
            None => SshError::disconnected("connection closed"),
        }
    }

    /// Sends a disconnect message and tears the connection down (including
    /// any jump hops).
    pub async fn disconnect(&self) {
        self.inner.shared.mark_closed(CloseReason::Local);
        let _ = self
            .inner
            .handle
            .disconnect(Disconnect::ByApplication, "", "en")
            .await;
        for jump in self.inner.jumps.iter().rev() {
            let _ = jump
                .handle
                .disconnect(Disconnect::ByApplication, "", "en")
                .await;
        }
    }

    async fn open_channel(&self) -> Result<Channel<Msg>, SshError> {
        if self.is_closed() {
            return Err(self.close_error());
        }
        self.inner
            .handle
            .channel_open_session()
            .await
            .map_err(SshError::from)
    }

    /// Opens an interactive login shell on a PTY.
    ///
    /// Output arrives batched (every 8 ms or 32 KB) on the returned receiver;
    /// the receiver is bounded (about 4 MB in flight) and stops reading from
    /// the SSH channel when it is full.
    pub async fn open_shell(
        &self,
        opts: ShellOptions,
    ) -> Result<(ShellHandle, mpsc::Receiver<ShellEvent>), SshError> {
        let channel = self.open_channel().await?;
        shell::start(self, channel, opts).await
    }

    /// Starts the `sftp` subsystem on a new channel of this connection.
    pub async fn sftp(&self) -> Result<SftpClient, SshError> {
        let mut channel = self.open_channel().await?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(SshError::from)?;
        wait_request_reply(&mut channel, "sftp subsystem")
            .await
            .map_err(|e| SshError::new(SshErrorKind::Sftp, e.message))?;
        SftpClient::start(self.clone(), channel.into_stream()).await
    }

    /// Listens on a local TCP port and forwards every connection through the
    /// SSH server to `fwd.dest_host:fwd.dest_port` (`ssh -L`).
    pub async fn local_forward(&self, fwd: LocalForward) -> Result<ForwardHandle, SshError> {
        forward::start(self.clone(), fwd).await
    }

    /// Runs `command` and returns `(exit status, stdout)`. If the command is
    /// killed by a signal the status is 255. `stdin`, if given, is sent to the
    /// command; stdin is always closed afterwards.
    pub async fn exec(
        &self,
        command: &str,
        stdin: Option<Vec<u8>>,
    ) -> Result<(u32, Vec<u8>), SshError> {
        let out = self.exec_output(command, stdin).await?;
        Ok((out.exit_status.unwrap_or(255), out.stdout))
    }

    /// Like [`exec`](Self::exec) but also returns stderr and distinguishes
    /// "killed by a signal" (`exit_status == None`).
    pub async fn exec_output(
        &self,
        command: &str,
        stdin: Option<Vec<u8>>,
    ) -> Result<ExecOutput, SshError> {
        let channel = self.open_channel().await?;
        channel
            .exec(true, command.as_bytes().to_vec())
            .await
            .map_err(SshError::from)?;
        let (mut reader, writer) = channel.split();

        let feed = async {
            if let Some(data) = stdin {
                writer.data_bytes(Bytes::from(data)).await?;
            }
            writer.eof().await
        };
        tokio::pin!(feed);
        let mut feed_done = false;

        let mut out = ExecOutput::default();
        let mut accepted = false;
        loop {
            tokio::select! {
                result = &mut feed, if !feed_done => {
                    feed_done = true;
                    if let Err(e) = result {
                        // The command may legitimately exit without reading stdin.
                        tracing::debug!("exec: writing stdin ended early: {e}");
                    }
                }
                msg = reader.wait() => match msg {
                    Some(ChannelMsg::Success) => accepted = true,
                    Some(ChannelMsg::Failure) => {
                        return Err(SshError::channel("the server refused to run the command"));
                    }
                    Some(ChannelMsg::Data { data }) => {
                        if out.stdout.len() + data.len() > MAX_EXEC_OUTPUT {
                            return Err(SshError::other("command output is too large"));
                        }
                        out.stdout.extend_from_slice(&data);
                    }
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        if out.stderr.len() + data.len() > MAX_EXEC_OUTPUT {
                            return Err(SshError::other("command output is too large"));
                        }
                        out.stderr.extend_from_slice(&data);
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => out.exit_status = Some(exit_status),
                    Some(ChannelMsg::Close) => break,
                    Some(_) => {}
                    None => {
                        if out.exit_status.is_some() || accepted {
                            break;
                        }
                        return Err(self.close_error());
                    }
                },
            }
        }
        Ok(out)
    }
}

/// Waits for the server's reply to a `want_reply` channel request.
pub(crate) async fn wait_request_reply(
    channel: &mut Channel<Msg>,
    what: &str,
) -> Result<(), SshError> {
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Success) => return Ok(()),
            Some(ChannelMsg::Failure) => {
                return Err(SshError::channel(format!(
                    "the server refused the {what} request"
                )));
            }
            Some(ChannelMsg::Close) | None => {
                return Err(SshError::channel(format!(
                    "the channel closed while waiting for the {what} request"
                )));
            }
            Some(_) => {}
        }
    }
}
