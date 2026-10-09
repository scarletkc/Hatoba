//! The russh client handler: host key verification, the server's identification string and
//! disconnect tracking.

use std::sync::{Arc, Mutex, OnceLock};

use russh::Names;
use russh::client::{self, DisconnectReason};
use russh::keys::PublicKeyOrCertificate;
use tokio::sync::watch;

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
}

impl SessionShared {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            closed_tx: watch::Sender::new(false),
            reason: Mutex::new(None),
            server_id: OnceLock::new(),
        })
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
