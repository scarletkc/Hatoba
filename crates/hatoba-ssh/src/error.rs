//! Error types shared by the whole crate.
//!
//! Every fallible public API returns [`SshError`]: a coarse, serializable
//! [`SshErrorKind`] that the UI can map to a localized message (SSH-05), plus a
//! short English technical detail in `message`. Messages never contain secrets
//! or terminal content.

use std::fmt;
use std::io;

use serde::{Deserialize, Serialize};

/// Coarse classification of an SSH failure, serialized as `snake_case`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SshErrorKind {
    /// Host name could not be resolved.
    Dns,
    /// The peer actively refused the TCP connection (ECONNREFUSED).
    Refused,
    /// A connect / handshake / authentication phase did not finish in time.
    Timeout,
    /// No route to the host or network.
    Unreachable,
    /// The server rejected every offered credential.
    AuthFailed,
    /// The host key verifier returned `false`.
    HostKeyRejected,
    /// A private key could not be parsed or decrypted.
    KeyParse,
    /// The connection was lost (reset, keepalive timeout, server disconnect).
    Disconnected,
    /// SSH protocol violation or no common algorithms.
    Protocol,
    /// Local or generic I/O error.
    Io,
    /// A channel could not be opened or a channel request was refused.
    Channel,
    /// SFTP subsystem error (status codes, missing files, ...).
    Sftp,
    /// The operation was cancelled by the caller.
    Cancelled,
    /// The proxy could not be reached (DNS, refused, unreachable or no answer in time).
    ProxyUnreachable,
    /// The proxy wants a username and password, or did not accept them.
    ProxyAuth,
    /// The proxy answered but did not open the connection to the server (its rules, an
    /// unexpected answer, or a closed connection during the handshake).
    Proxy,
    /// Anything else.
    Other,
}

impl SshErrorKind {
    /// Stable `snake_case` identifier (same as the serde representation).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dns => "dns",
            Self::Refused => "refused",
            Self::Timeout => "timeout",
            Self::Unreachable => "unreachable",
            Self::AuthFailed => "auth_failed",
            Self::HostKeyRejected => "host_key_rejected",
            Self::KeyParse => "key_parse",
            Self::Disconnected => "disconnected",
            Self::Protocol => "protocol",
            Self::Io => "io",
            Self::Channel => "channel",
            Self::Sftp => "sftp",
            Self::Cancelled => "cancelled",
            Self::ProxyUnreachable => "proxy_unreachable",
            Self::ProxyAuth => "proxy_auth",
            Self::Proxy => "proxy",
            Self::Other => "other",
        }
    }

    /// Short human readable description (English).
    pub fn description(self) -> &'static str {
        match self {
            Self::Dns => "host name could not be resolved",
            Self::Refused => "connection refused",
            Self::Timeout => "connection timed out",
            Self::Unreachable => "host or network unreachable",
            Self::AuthFailed => "authentication failed",
            Self::HostKeyRejected => "host key rejected",
            Self::KeyParse => "private key could not be used",
            Self::Disconnected => "connection lost",
            Self::Protocol => "SSH protocol error",
            Self::Io => "I/O error",
            Self::Channel => "SSH channel error",
            Self::Sftp => "SFTP error",
            Self::Cancelled => "cancelled",
            Self::ProxyUnreachable => "proxy unreachable",
            Self::ProxyAuth => "proxy authentication failed",
            Self::Proxy => "proxy error",
            Self::Other => "error",
        }
    }
}

impl fmt::Display for SshErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

/// Error returned by all fallible public APIs of this crate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshError {
    /// Classification, stable across releases.
    pub kind: SshErrorKind,
    /// Short technical detail (English, secret-free).
    pub message: String,
}

impl SshError {
    /// Creates an error of the given kind.
    pub fn new(kind: SshErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub(crate) fn other(message: impl Into<String>) -> Self {
        Self::new(SshErrorKind::Other, message)
    }

    pub(crate) fn channel(message: impl Into<String>) -> Self {
        Self::new(SshErrorKind::Channel, message)
    }

    pub(crate) fn disconnected(message: impl Into<String>) -> Self {
        Self::new(SshErrorKind::Disconnected, message)
    }

    pub(crate) fn cancelled() -> Self {
        Self::new(SshErrorKind::Cancelled, "operation cancelled")
    }

    /// Builds an error from an I/O error with a context prefix such as
    /// `"connect to example.com:22"`.
    pub(crate) fn io_context(context: &str, e: &io::Error) -> Self {
        Self::new(classify_io(e), format!("{context}: {e}"))
    }

    /// Prefixes the message with `context`, keeping the kind.
    pub(crate) fn with_context(mut self, context: &str) -> Self {
        self.message = format!("{context}: {}", self.message);
        self
    }
}

impl fmt::Display for SshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for SshError {}

/// Maps an I/O error kind onto an [`SshErrorKind`].
pub(crate) fn classify_io(e: &io::Error) -> SshErrorKind {
    use io::ErrorKind as K;
    match e.kind() {
        K::ConnectionRefused => SshErrorKind::Refused,
        K::TimedOut => SshErrorKind::Timeout,
        K::NetworkUnreachable | K::HostUnreachable | K::NetworkDown => SshErrorKind::Unreachable,
        K::ConnectionReset
        | K::ConnectionAborted
        | K::BrokenPipe
        | K::UnexpectedEof
        | K::NotConnected => SshErrorKind::Disconnected,
        _ => SshErrorKind::Io,
    }
}

impl From<io::Error> for SshError {
    fn from(e: io::Error) -> Self {
        Self::new(classify_io(&e), e.to_string())
    }
}

impl From<russh::Error> for SshError {
    fn from(e: russh::Error) -> Self {
        Self::from(&e)
    }
}

impl From<&russh::Error> for SshError {
    fn from(e: &russh::Error) -> Self {
        use russh::Error as E;
        match e {
            E::IO(io) => Self::new(classify_io(io), io.to_string()),
            E::Disconnect | E::HUP => Self::disconnected("connection closed by the remote side"),
            E::KeepaliveTimeout => {
                Self::disconnected("keepalive timeout: the server stopped responding")
            }
            E::InactivityTimeout => Self::disconnected("inactivity timeout: no data from server"),
            E::ConnectionTimeout => Self::new(SshErrorKind::Timeout, "connection timeout"),
            E::SendError | E::RecvError | E::Join(_) => {
                Self::disconnected("connection closed before the operation completed")
            }
            E::UnknownKey | E::KeyChanged { .. } => Self::new(
                SshErrorKind::HostKeyRejected,
                "server host key was not accepted",
            ),
            E::NoAuthMethod => {
                Self::new(SshErrorKind::AuthFailed, "no usable authentication method")
            }
            E::Keys(k) => Self::new(SshErrorKind::KeyParse, k.to_string()),
            E::ChannelOpenFailure(reason) => Self::channel(format!(
                "server refused to open the channel: {}",
                reason.description()
            )),
            E::WrongChannel | E::RequestDenied => Self::channel(e.to_string()),
            E::NoCommonAlgo { .. }
            | E::Kex
            | E::KexInit
            | E::UnknownAlgo
            | E::Version
            | E::PacketAuth
            | E::DecryptionError
            | E::Inconsistent
            | E::PacketSize(_)
            | E::WrongServerSig
            | E::StrictKeyExchangeViolation { .. } => {
                Self::new(SshErrorKind::Protocol, e.to_string())
            }
            other => Self::other(other.to_string()),
        }
    }
}

impl From<russh_sftp::client::error::Error> for SshError {
    fn from(e: russh_sftp::client::error::Error) -> Self {
        use russh_sftp::client::error::Error as E;
        match e {
            E::Timeout => Self::new(SshErrorKind::Sftp, "SFTP request timed out"),
            other => Self::new(SshErrorKind::Sftp, other.to_string()),
        }
    }
}

impl From<crate::keys::KeyError> for SshError {
    fn from(e: crate::keys::KeyError) -> Self {
        Self::new(SshErrorKind::KeyParse, e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use io::ErrorKind as K;

    fn kind_of(k: K) -> SshErrorKind {
        classify_io(&io::Error::from(k))
    }

    #[test]
    fn io_errors_are_classified() {
        assert_eq!(kind_of(K::ConnectionRefused), SshErrorKind::Refused);
        assert_eq!(kind_of(K::TimedOut), SshErrorKind::Timeout);
        assert_eq!(kind_of(K::HostUnreachable), SshErrorKind::Unreachable);
        assert_eq!(kind_of(K::NetworkUnreachable), SshErrorKind::Unreachable);
        assert_eq!(kind_of(K::ConnectionReset), SshErrorKind::Disconnected);
        assert_eq!(kind_of(K::BrokenPipe), SshErrorKind::Disconnected);
        assert_eq!(kind_of(K::UnexpectedEof), SshErrorKind::Disconnected);
        assert_eq!(kind_of(K::PermissionDenied), SshErrorKind::Io);
    }

    #[test]
    fn russh_errors_are_classified() {
        let kind = |e: russh::Error| SshError::from(e).kind;
        assert_eq!(kind(russh::Error::Disconnect), SshErrorKind::Disconnected);
        assert_eq!(kind(russh::Error::HUP), SshErrorKind::Disconnected);
        assert_eq!(
            kind(russh::Error::KeepaliveTimeout),
            SshErrorKind::Disconnected
        );
        assert_eq!(
            kind(russh::Error::UnknownKey),
            SshErrorKind::HostKeyRejected
        );
        assert_eq!(kind(russh::Error::Kex), SshErrorKind::Protocol);
        assert_eq!(kind(russh::Error::Version), SshErrorKind::Protocol);
        assert_eq!(kind(russh::Error::NoAuthMethod), SshErrorKind::AuthFailed);
        assert_eq!(kind(russh::Error::RequestDenied), SshErrorKind::Channel);
        assert_eq!(
            kind(russh::Error::IO(io::Error::from(K::ConnectionRefused))),
            SshErrorKind::Refused
        );
    }

    #[test]
    fn display_and_context() {
        let e = SshError::new(SshErrorKind::Refused, "connect to 1.2.3.4:22: nope");
        assert_eq!(
            e.to_string(),
            "connection refused: connect to 1.2.3.4:22: nope"
        );
        let e = e.with_context("hop 1/2");
        assert_eq!(e.kind, SshErrorKind::Refused);
        assert!(e.message.starts_with("hop 1/2: connect to"));
    }
}
