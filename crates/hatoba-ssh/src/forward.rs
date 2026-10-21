//! Port forwarding: local (`ssh -L`, FWD-01), remote (`ssh -R`, FWD-03) and dynamic SOCKS
//! (`ssh -D`, FWD-04).

use std::net::SocketAddr;
use std::sync::Arc;

use russh::ChannelOpenFailure;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::error::{SshError, SshErrorKind};
use crate::handler::{Forwarded, SessionShared};
use crate::session::SshSession;
use crate::socks::{self, Failure};

/// A `-L bind_address:bind_port:dest_host:dest_port` rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalForward {
    /// Local interface to listen on; empty means `127.0.0.1`.
    pub bind_address: String,
    /// Local port; `0` picks a free port (see [`ForwardHandle::port`]).
    pub bind_port: u16,
    /// Destination as seen from the SSH server.
    pub dest_host: String,
    /// Destination port.
    pub dest_port: u16,
}

/// A `-R bind_address:bind_port:dest_host:dest_port` rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteForward {
    /// Interface on the server to listen on; empty means `localhost`. The server's
    /// `GatewayPorts` decides whether any other address is honoured.
    pub bind_address: String,
    /// Port on the server; `0` lets the server pick one (see [`ForwardHandle::port`]).
    pub bind_port: u16,
    /// Destination as seen from this computer.
    pub dest_host: String,
    /// Destination port.
    pub dest_port: u16,
}

/// A `-D bind_address:bind_port` rule: a SOCKS 4, 4a and 5 proxy whose connections the SSH
/// server opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicForward {
    /// Local interface to listen on; empty means `127.0.0.1`.
    pub bind_address: String,
    /// Local port; `0` picks a free port (see [`ForwardHandle::port`]).
    pub bind_port: u16,
}

struct ForwardInner {
    port: u16,
    local_addr: Option<SocketAddr>,
    cancel: CancellationToken,
}

/// A running forward of any kind. Cheap to clone; dropping a handle does not stop it.
#[derive(Clone)]
pub struct ForwardHandle {
    inner: Arc<ForwardInner>,
}

impl ForwardHandle {
    fn new(port: u16, local_addr: Option<SocketAddr>, cancel: CancellationToken) -> Self {
        Self {
            inner: Arc::new(ForwardInner {
                port,
                local_addr,
                cancel,
            }),
        }
    }

    /// The port the forward actually listens on: on this computer for local and dynamic
    /// forwards, on the server for remote ones.
    pub fn port(&self) -> u16 {
        self.inner.port
    }

    /// The address on this computer the forward listens on; `None` for a remote forward.
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.inner.local_addr
    }

    /// `true` until [`stop`](Self::stop) is called or the SSH connection ends.
    pub fn is_running(&self) -> bool {
        !self.inner.cancel.is_cancelled()
    }

    /// Stops listening and closes every forwarded connection. A remote forward also asks the
    /// server to stop listening.
    pub fn stop(&self) {
        self.inner.cancel.cancel();
    }
}

/// Listens on `bind_address:bind_port` on this computer, loopback when the address is empty.
async fn listen(bind_address: &str, bind_port: u16) -> Result<TcpListener, SshError> {
    let host = match bind_address.trim() {
        "" => "127.0.0.1",
        a => a,
    };
    TcpListener::bind((host, bind_port))
        .await
        .map_err(|e| SshError::io_context(&format!("listen on {host}:{bind_port}"), &e))
}

/// What a local listener does with each connection.
#[derive(Clone)]
enum Serve {
    /// Opens a `direct-tcpip` channel to this destination.
    To(String, u16),
    /// Asks the connection where to, in SOCKS.
    Socks,
}

pub(crate) async fn start_local(
    session: SshSession,
    fwd: LocalForward,
) -> Result<ForwardHandle, SshError> {
    let listener = listen(&fwd.bind_address, fwd.bind_port).await?;
    start_listener(session, listener, Serve::To(fwd.dest_host, fwd.dest_port))
}

pub(crate) async fn start_dynamic(
    session: SshSession,
    fwd: DynamicForward,
) -> Result<ForwardHandle, SshError> {
    let listener = listen(&fwd.bind_address, fwd.bind_port).await?;
    start_listener(session, listener, Serve::Socks)
}

fn start_listener(
    session: SshSession,
    listener: TcpListener,
    serve: Serve,
) -> Result<ForwardHandle, SshError> {
    let local_addr = listener
        .local_addr()
        .map_err(|e| SshError::io_context("local address", &e))?;
    let cancel = CancellationToken::new();
    tokio::spawn(accept_loop(listener, session, serve, cancel.clone()));
    Ok(ForwardHandle::new(
        local_addr.port(),
        Some(local_addr),
        cancel,
    ))
}

async fn accept_loop(
    listener: TcpListener,
    session: SshSession,
    serve: Serve,
    cancel: CancellationToken,
) {
    let mut closed = session.shared().subscribe();
    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            () = async { let _ = closed.wait_for(|c| *c).await; } => {
                cancel.cancel();
                break;
            }
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    let session = session.clone();
                    let serve = serve.clone();
                    let cancel = cancel.child_token();
                    tokio::spawn(async move {
                        let ended = match serve {
                            Serve::To(host, port) => {
                                pipe(session, stream, peer, &host, port, cancel).await
                            }
                            Serve::Socks => serve_socks(session, stream, peer, cancel).await,
                        };
                        if let Err(e) = ended {
                            tracing::debug!("forwarded connection ended: {e}");
                        }
                    });
                }
                Err(e) => {
                    tracing::debug!("accept failed: {e}");
                    // Avoid a hot loop on persistent errors such as EMFILE.
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            },
        }
    }
}

async fn open_direct(
    session: &SshSession,
    peer: SocketAddr,
    dest_host: &str,
    dest_port: u16,
) -> Result<russh::Channel<russh::client::Msg>, russh::Error> {
    session
        .handle()
        .channel_open_direct_tcpip(
            dest_host,
            u32::from(dest_port),
            peer.ip().to_string(),
            u32::from(peer.port()),
        )
        .await
}

async fn pipe(
    session: SshSession,
    tcp: TcpStream,
    peer: SocketAddr,
    dest_host: &str,
    dest_port: u16,
    cancel: CancellationToken,
) -> Result<(), SshError> {
    let _ = tcp.set_nodelay(true);
    let channel = open_direct(&session, peer, dest_host, dest_port)
        .await
        .map_err(SshError::from)?;
    copy(tcp, channel.into_stream(), cancel).await
}

/// Copies both ways until either side ends or `cancel` fires.
async fn copy<S>(
    mut tcp: TcpStream,
    mut stream: S,
    cancel: CancellationToken,
) -> Result<(), SshError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    tokio::select! {
        () = cancel.cancelled() => Ok(()),
        copied = tokio::io::copy_bidirectional(&mut tcp, &mut stream) => copied
            .map(|_| ())
            .map_err(|e| SshError::new(SshErrorKind::Io, e.to_string())),
    }
}

async fn serve_socks(
    session: SshSession,
    mut tcp: TcpStream,
    peer: SocketAddr,
    cancel: CancellationToken,
) -> Result<(), SshError> {
    let _ = tcp.set_nodelay(true);
    let request = tokio::select! {
        () = cancel.cancelled() => return Ok(()),
        request = socks::accept(&mut tcp) => request
            .map_err(|e| SshError::new(SshErrorKind::Io, format!("SOCKS: {e}")))?,
    };
    let opened = tokio::select! {
        () = cancel.cancelled() => return Ok(()),
        opened = open_direct(&session, peer, &request.host, request.port) => opened,
    };
    match opened {
        Ok(channel) => {
            request
                .succeeded(&mut tcp)
                .await
                .map_err(|e| SshError::io_context("SOCKS reply", &e))?;
            copy(tcp, channel.into_stream(), cancel).await
        }
        Err(e) => {
            let failure = match &e {
                russh::Error::ChannelOpenFailure(ChannelOpenFailure::ConnectFailed) => {
                    Failure::Refused
                }
                russh::Error::ChannelOpenFailure(
                    ChannelOpenFailure::AdministrativelyProhibited,
                ) => Failure::NotAllowed,
                _ => Failure::General,
            };
            let _ = request.failed(&mut tcp, failure).await;
            let _ = tcp.shutdown().await;
            Err(SshError::from(e).with_context(&format!(
                "SOCKS connection to {}",
                host_port(&request.host, request.port)
            )))
        }
    }
}

/// `host:port`, with an IPv6 address in brackets.
fn host_port(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Removes the route when the forward ends, so that the server's later connections are refused.
struct Route {
    shared: Arc<SessionShared>,
    id: u64,
}

impl Drop for Route {
    fn drop(&mut self) {
        self.shared.remove_remote_route(self.id);
    }
}

pub(crate) async fn start_remote(
    session: SshSession,
    fwd: RemoteForward,
) -> Result<ForwardHandle, SshError> {
    let address = match fwd.bind_address.trim() {
        "" => "localhost",
        a => a,
    }
    .to_owned();
    let (tx, rx) = mpsc::unbounded_channel();
    let shared = Arc::clone(session.shared());
    let route = Route {
        id: shared.add_remote_route(&address, u32::from(fwd.bind_port), tx),
        shared,
    };
    let picked = session
        .handle()
        .tcpip_forward(address.clone(), u32::from(fwd.bind_port))
        .await
        .map_err(|e| match e {
            russh::Error::RequestDenied => SshError::new(
                SshErrorKind::Channel,
                format!(
                    "the server did not listen on {}: the port may be in use or need root, or the \
                     server does not allow remote forwarding",
                    host_port(&address, fwd.bind_port)
                ),
            ),
            e => SshError::from(e),
        })?;
    let port = match fwd.bind_port {
        0 => u16::try_from(picked).unwrap_or(0),
        port => port,
    };
    route.shared.set_remote_port(route.id, u32::from(port));

    let cancel = CancellationToken::new();
    tokio::spawn(remote_loop(
        session,
        route,
        rx,
        (address, port),
        (fwd.dest_host, fwd.dest_port),
        cancel.clone(),
    ));
    Ok(ForwardHandle::new(port, None, cancel))
}

async fn remote_loop(
    session: SshSession,
    route: Route,
    mut rx: mpsc::UnboundedReceiver<Forwarded>,
    (address, port): (String, u16),
    (dest_host, dest_port): (String, u16),
    cancel: CancellationToken,
) {
    let mut closed = session.shared().subscribe();
    let stopped = loop {
        tokio::select! {
            () = cancel.cancelled() => break true,
            () = async { let _ = closed.wait_for(|c| *c).await; } => {
                cancel.cancel();
                break false;
            }
            forwarded = rx.recv() => match forwarded {
                Some(forwarded) => {
                    let dest_host = dest_host.clone();
                    let cancel = cancel.child_token();
                    tokio::spawn(async move {
                        if let Err(e) = serve_forwarded(forwarded, &dest_host, dest_port, cancel).await {
                            tracing::debug!("forwarded connection ended: {e}");
                        }
                    });
                }
                None => break false,
            },
        }
    };
    drop(route);
    if stopped
        && !session.is_closed()
        && let Err(e) = session
            .handle()
            .cancel_tcpip_forward(address, u32::from(port))
            .await
    {
        tracing::debug!("the server did not stop listening for a remote forward: {e}");
    }
}

/// Connects to the destination, then confirms the channel; refuses it when the destination does
/// not answer.
async fn serve_forwarded(
    forwarded: Forwarded,
    dest_host: &str,
    dest_port: u16,
    cancel: CancellationToken,
) -> Result<(), SshError> {
    let Forwarded { channel, reply } = forwarded;
    let connected = tokio::select! {
        () = cancel.cancelled() => return Ok(()),
        connected = TcpStream::connect((dest_host, dest_port)) => connected,
    };
    match connected {
        Ok(tcp) => {
            let _ = tcp.set_nodelay(true);
            reply.accept().await;
            copy(tcp, channel.into_stream(), cancel).await
        }
        Err(e) => {
            reply.reject(ChannelOpenFailure::ConnectFailed).await;
            Err(SshError::io_context(
                &format!("connect to {}", host_port(dest_host, dest_port)),
                &e,
            ))
        }
    }
}
