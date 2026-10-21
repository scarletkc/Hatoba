//! The russh client handler: host key verification, the server's identification string,
//! disconnect tracking, and the connections the server opens for remote forwards.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use russh::client::{self, ChannelOpenHandle, DisconnectReason, Msg};
use russh::keys::PublicKeyOrCertificate;
use russh::{Channel, Names};
use tokio::sync::{mpsc, watch};

use crate::error::{SshError, SshErrorKind};
use crate::hostkey::{HostKeyInfo, HostKeyVerifier};
use crate::net::ConnectClock;

/// Error type of [`ClientHandler`].
#[derive(Debug)]
pub(crate) enum HandlerError {
    /// A russh protocol / transport error.
    Russh(russh::Error),
    /// The [`HostKeyVerifier`] refused the server key.
    HostKeyRejected,
}

impl From<russh::Error> for HandlerError {
    fn from(e: russh::Error) -> Self {
        Self::Russh(e)
    }
}

impl From<HandlerError> for SshError {
    fn from(e: HandlerError) -> Self {
        match e {
            HandlerError::Russh(e) => e.into(),
            HandlerError::HostKeyRejected => SshError::new(
                SshErrorKind::HostKeyRejected,
                "server host key was not accepted",
            ),
        }
    }
}

/// Why a connection ended.
#[derive(Debug, Clone)]
pub(crate) enum CloseReason {
    /// We called `disconnect()`.
    Local,
    /// The server sent an orderly SSH_MSG_DISCONNECT.
    Remote(String),
    /// Transport or protocol failure.
    Error(SshError),
}

/// State shared between the handler (running inside russh's event loop) and
/// the session / shell / forward tasks.
pub(crate) struct SessionShared {
    closed_tx: watch::Sender<bool>,
    reason: Mutex<Option<CloseReason>>,
    /// The identification string the server sent in the version exchange, cleaned.
    server_id: OnceLock<String>,
    /// The host could not run `remote_command`'s script, so commands run as they are.
    plain_commands: AtomicBool,
    /// The remote forwards listening on the server (FWD-03).
    remote: Mutex<RemoteRoutes>,
}

/// A connection the server accepted on a remote forward's port, not yet confirmed: the forward
/// confirms it once its destination answers, as `ssh -R` does.
pub(crate) struct Forwarded {
    pub(crate) channel: Channel<Msg>,
    pub(crate) reply: ChannelOpenHandle,
}

#[derive(Default)]
struct RemoteRoutes {
    next_id: u64,
    routes: Vec<RemoteRoute>,
}

struct RemoteRoute {
    id: u64,
    /// The address as the `tcpip-forward` request named it, which the server sends back.
    address: String,
    /// The port the server listens on; 0 until the server names the one it picked.
    port: u32,
    tx: mpsc::UnboundedSender<Forwarded>,
}

impl SessionShared {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            closed_tx: watch::Sender::new(false),
            reason: Mutex::new(None),
            server_id: OnceLock::new(),
            plain_commands: AtomicBool::new(false),
            remote: Mutex::new(RemoteRoutes::default()),
        })
    }

    fn routes(&self) -> std::sync::MutexGuard<'_, RemoteRoutes> {
        self.remote.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Sends the connections the server accepts on `address:port` to `tx` from now on, before
    /// the server is asked to listen there, so that none arrives unrouted. Returns the route's id.
    pub(crate) fn add_remote_route(
        &self,
        address: &str,
        port: u32,
        tx: mpsc::UnboundedSender<Forwarded>,
    ) -> u64 {
        let mut routes = self.routes();
        routes.next_id += 1;
        let id = routes.next_id;
        routes.routes.push(RemoteRoute {
            id,
            address: address.to_owned(),
            port,
            tx,
        });
        id
    }

    /// The port the server picked for a route asked with port 0.
    pub(crate) fn set_remote_port(&self, id: u64, port: u32) {
        if let Some(route) = self.routes().routes.iter_mut().find(|r| r.id == id) {
            route.port = port;
        }
    }

    pub(crate) fn remove_remote_route(&self, id: u64) {
        self.routes().routes.retain(|r| r.id != id);
    }

    /// Hands a connection the server accepted on `address:port` to its forward. The address the
    /// request named comes back, but not from every server, so the port alone decides when it
    /// does not match; a route still waiting for the port the server picks comes last. A
    /// connection no forward takes is dropped, which refuses it, and `false` returned.
    fn route_forwarded(&self, address: &str, port: u32, forwarded: Forwarded) -> bool {
        let routes = self.routes();
        let route = routes
            .routes
            .iter()
            .find(|r| r.port == port && r.address == address)
            .or_else(|| routes.routes.iter().find(|r| r.port == port))
            .or_else(|| {
                routes
                    .routes
                    .iter()
                    .find(|r| r.port == 0 && r.address == address)
            });
        route.is_some_and(|route| route.tx.send(forwarded).is_ok())
    }

    /// Records the server's identification string. It never changes, so the first one stays.
    pub(crate) fn set_server_id(&self, raw: &[u8]) {
        let id = clean_server_id(raw);
        if !id.is_empty() {
            let _ = self.server_id.set(id);
        }
    }

    pub(crate) fn server_id(&self) -> Option<&str> {
        self.server_id.get().map(String::as_str)
    }

    /// Commands run as they are on this connection: the host could not run `remote_command`'s
    /// script.
    pub(crate) fn plain_commands(&self) -> bool {
        self.plain_commands.load(Ordering::Relaxed)
    }

    pub(crate) fn set_plain_commands(&self) {
        self.plain_commands.store(true, Ordering::Relaxed);
    }

    /// Records why the connection ended (first reason wins) and wakes watchers.
    pub(crate) fn mark_closed(&self, reason: CloseReason) {
        {
            let mut slot = self.reason.lock().unwrap_or_else(|e| e.into_inner());
            if slot.is_none() {
                *slot = Some(reason);
            }
        }
        self.closed_tx.send_replace(true);
    }

    pub(crate) fn is_closed(&self) -> bool {
        *self.closed_tx.borrow()
    }

    pub(crate) fn reason(&self) -> Option<CloseReason> {
        self.reason
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<bool> {
        self.closed_tx.subscribe()
    }
}

/// Strips control characters and caps the length of server-provided text.
fn sanitize(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(200).collect()
}

/// Longest identification string kept: RFC 4253 §4.2 allows 255 characters with the CR LF.
pub(crate) const MAX_SERVER_ID_CHARS: usize = 255;

/// The server's identification string as text: invalid UTF-8 replaced, control characters
/// dropped, at most [`MAX_SERVER_ID_CHARS`] characters, and surrounding whitespace trimmed.
fn clean_server_id(raw: &[u8]) -> String {
    let id: String = String::from_utf8_lossy(raw)
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_SERVER_ID_CHARS)
        .collect();
    id.trim().to_owned()
}

pub(crate) struct ClientHandler {
    host: String,
    port: u16,
    verifier: Arc<dyn HostKeyVerifier>,
    shared: Arc<SessionShared>,
    clock: Arc<ConnectClock>,
}

impl ClientHandler {
    pub(crate) fn new(
        host: String,
        port: u16,
        verifier: Arc<dyn HostKeyVerifier>,
        shared: Arc<SessionShared>,
        clock: Arc<ConnectClock>,
    ) -> Self {
        Self {
            host,
            port,
            verifier,
            shared,
            clock,
        }
    }
}

impl client::Handler for ClientHandler {
    type Error = HandlerError;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let info = HostKeyInfo::from_public_key(&server_public_key.public_key());
        // The user may take as long as they like to confirm a fingerprint.
        let _pause = self.clock.pause();
        if self.verifier.verify(&self.host, self.port, &info).await {
            Ok(true)
        } else {
            Err(HandlerError::HostKeyRejected)
        }
    }

    /// Runs after every key exchange, the first one right after the version exchange, so the
    /// session knows the server's identification string before authentication starts.
    async fn kex_done(
        &mut self,
        _shared_secret: Option<&[u8]>,
        _names: &Names,
        session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        self.shared.set_server_id(session.remote_sshid());
        Ok(())
    }

    /// A connection to a remote forward's port (FWD-03).
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<Msg>,
        connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let forwarded = Forwarded { channel, reply };
        if !self
            .shared
            .route_forwarded(connected_address, connected_port, forwarded)
        {
            tracing::debug!(
                "refused a connection to {connected_address}:{connected_port}: no forward there"
            );
        }
        Ok(())
    }

    async fn disconnected(
        &mut self,
        reason: DisconnectReason<Self::Error>,
    ) -> Result<(), Self::Error> {
        match reason {
            DisconnectReason::ReceivedDisconnect(info) => {
                let message = sanitize(&info.message);
                self.shared
                    .mark_closed(CloseReason::Remote(if message.is_empty() {
                        format!("server closed the connection ({:?})", info.reason_code)
                    } else {
                        format!("server closed the connection: {message}")
                    }));
                Ok(())
            }
            DisconnectReason::Error(e) => {
                let err = match &e {
                    HandlerError::Russh(r) => SshError::from(r),
                    HandlerError::HostKeyRejected => SshError::new(
                        SshErrorKind::HostKeyRejected,
                        "server host key was not accepted",
                    ),
                };
                self.shared.mark_closed(CloseReason::Error(err));
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_ids_are_cleaned_and_capped() {
        assert_eq!(
            clean_server_id(b"SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13.5"),
            "SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13.5"
        );
        assert_eq!(clean_server_id(b" SSH-2.0-x\r\n"), "SSH-2.0-x");
        assert_eq!(
            clean_server_id(b"SSH-2.0-\x1b[31mred\xff"),
            "SSH-2.0-[31mred\u{fffd}"
        );
        let long = format!("SSH-2.0-{}", "a".repeat(400));
        assert_eq!(
            clean_server_id(long.as_bytes()).chars().count(),
            MAX_SERVER_ID_CHARS
        );

        let shared = SessionShared::new();
        assert_eq!(shared.server_id(), None);
        shared.set_server_id(b"\r\n");
        assert_eq!(shared.server_id(), None, "nothing printable is no id");
        shared.set_server_id(b"SSH-2.0-first");
        shared.set_server_id(b"SSH-2.0-second");
        assert_eq!(shared.server_id(), Some("SSH-2.0-first"));
    }
}
