//! Host key verification hook (SSH-04).
//!
//! This crate only reports the key a server presented; TOFU, known-host
//! storage and mismatch handling live in the caller.

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use russh::keys::ssh_key::{HashAlg, PublicKey};

/// A server host key as presented during the key exchange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyInfo {
    /// Algorithm name, e.g. `ssh-ed25519`, `ecdsa-sha2-nistp256`, `ssh-rsa`.
    pub key_type: String,
    /// OpenSSH base64 key blob without the type prefix (as in `known_hosts`).
    pub public_key: String,
    /// `SHA256:...` fingerprint, unpadded base64 (same as OpenSSH prints).
    pub fingerprint: String,
}

impl HostKeyInfo {
    pub(crate) fn from_public_key(key: &PublicKey) -> Self {
        let blob = key.to_bytes().unwrap_or_default();
        Self {
            key_type: key.algorithm().to_string(),
            public_key: STANDARD.encode(blob),
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
        }
    }
}

/// Decides whether a presented host key is trusted.
///
/// Called from the SSH handshake for every hop of a connection (including
/// ProxyJump hops). Returning `false` aborts the connection with
/// [`SshErrorKind::HostKeyRejected`](crate::SshErrorKind::HostKeyRejected).
/// The connect timeout is paused while `verify` is running, so the
/// implementation may wait for user input.
#[async_trait]
pub trait HostKeyVerifier: Send + Sync {
    async fn known_key_types(&self, _host: &str, _port: u16) -> Vec<String> {
        Vec::new()
    }

    /// `host` and `port` are the address configured for the hop being
    /// verified (not the resolved IP).
    async fn verify(&self, host: &str, port: u16, key: &HostKeyInfo) -> bool;
}
