//! SSH sessions, PTY, SFTP, port forwarding and key handling for Hatoba.
//!
//! The crate is platform independent and has no dependency on Tauri. The
//! entry point is [`connect`], which returns an [`SshSession`] from which
//! shells ([`SshSession::open_shell`]), SFTP clients ([`SshSession::sftp`]),
//! local port forwards ([`SshSession::local_forward`]) and one-off commands
//! ([`SshSession::exec`]) are opened.
//!
//! Secrets (passwords, private keys, passphrases) are wrapped in
//! [`zeroize::Zeroizing`] and are never logged; neither is terminal content.

mod agent;
mod auth;
pub mod config;
pub mod error;
pub mod forward;
mod handler;
pub mod hostkey;
pub mod interactive;
pub mod keys;
mod net;
mod ppk;
pub mod probe;
mod server_os;
pub mod session;
pub mod sftp;
pub mod shell;

pub use config::{SshConfigHost, parse_ssh_config, parse_ssh_config_with_home};
pub use error::{SshError, SshErrorKind};
pub use forward::{ForwardHandle, LocalForward};
pub use hostkey::{HostKeyInfo, HostKeyVerifier};
pub use interactive::{KeyboardInteractive, Prompt, PromptRequest};
pub use keys::{
    GenerateKind, KeyAlgorithm, KeyError, ParsedKey, fingerprint_sha256, generate_key,
    parse_private_key, try_fingerprint_sha256,
};
pub use probe::tcp_probe;
pub use server_os::{ServerOs, server_os};
pub use session::{
    AuthMethod, ConnectConfig, ExecOutput, JumpHop, ShellEvent, ShellHandle, ShellOptions,
    SshSession, connect,
};
pub use sftp::{FileEntry, SftpClient, TransferDirection, TransferProgress};
