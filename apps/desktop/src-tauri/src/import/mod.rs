//! Hosts saved by other SSH clients, read for import (SSH-11, SSH-12, SSH-15): `~/.ssh/config`,
//! Xshell, FinalShell, MobaXterm, PuTTY and Tabby. Each reader turns its client's sessions into
//! [`Entry`] values; `commands::import` previews them and creates the hosts.

mod finalshell;
mod mobaxterm;
mod putty;
mod ssh_config;
mod tabby;
mod text;
mod xshell;

use std::path::Path;

use hatoba_core::model::ProxyKind;
use zeroize::Zeroizing;

use crate::dto::ImportSource;
use crate::error::AppResult;

/// A host as another client saved it.
#[derive(Debug, Clone, Default)]
pub struct Entry {
    /// Identifies the entry within its source: an alias, a session's path, or a profile id.
    pub id: String,
    pub name: String,
    pub address: String,
    pub port: u16,
    /// The login user; `None` signs in as the local user, as OpenSSH does.
    pub username: Option<String>,
    /// The folder it is in, outermost first.
    pub folder: Vec<String>,
    /// Private key files, in order of precedence.
    pub identity_files: Vec<String>,
    /// The client keeps the private key itself, so it can't be imported; the key's name there,
    /// when the session names it.
    pub kept_key: Option<String>,
    /// Signs in with the SSH agent.
    pub agent: bool,
    pub jump: Option<Jump>,
    pub proxy: Option<Proxy>,
    /// A proxy setting Hatoba can't use, such as `ProxyCommand nc %h %p`, for the preview and the
    /// warning.
    pub skipped_proxy: Option<String>,
    /// `SetEnv` variables (SSH-14), as name and value.
    pub set_env: Vec<(String, String)>,
    /// The session's description or comment.
    pub note: String,
}

/// How an [`Entry`] reaches its server through other SSH servers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Jump {
    /// A `ProxyJump` value, whose first hop names a host.
    Named(String),
    /// Another entry of the same source, by [`Entry::id`].
    Entry(String),
    /// SSH gateways given by address, outermost first.
    Gateways(Vec<Gateway>),
}

/// An SSH gateway (jump host) a session gives by address rather than by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gateway {
    pub address: String,
    pub port: u16,
    pub username: Option<String>,
}

impl Gateway {
    /// `user@address:port`, as the preview shows it.
    pub fn label(&self) -> String {
        match &self.username {
            Some(user) => format!("{user}@{}:{}", self.address, self.port),
            None => format!("{}:{}", self.address, self.port),
        }
    }
}

/// A SOCKS5 or HTTP proxy a session uses (SSH-13).
#[derive(Debug, Clone)]
pub struct Proxy {
    pub kind: ProxyKind,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub password: Zeroizing<String>,
}

impl Proxy {
    /// `SOCKS5 address:port`, as the preview shows it.
    pub fn label(&self) -> String {
        let kind = match self.kind {
            ProxyKind::Socks5 => "SOCKS5",
            ProxyKind::Http => "HTTP",
        };
        format!("{kind} {}:{}", self.address, self.port)
    }
}

/// What an import found.
#[derive(Debug, Default)]
pub struct Source {
    /// Where the hosts were read from; `None` when nothing was found in the usual places.
    pub location: Option<String>,
    /// The client encrypted the file, so nothing could be read from it.
    pub encrypted: bool,
    pub entries: Vec<Entry>,
}

/// Reads `source` from `path`, a file or folder the user chose, or else from where the client
/// keeps its sessions on this device. Nothing there is not an error: the source is empty.
pub fn read(source: ImportSource, path: Option<&Path>) -> AppResult<Source> {
    match source {
        ImportSource::SshConfig => ssh_config::read(),
        ImportSource::Xshell => xshell::read(path),
        ImportSource::Finalshell => finalshell::read(path),
        ImportSource::Mobaxterm => mobaxterm::read(path),
        ImportSource::Putty => putty::read(path),
        ImportSource::Tabby => tabby::read(path),
    }
}

/// `address`, without a `user@` before it, with the user.
fn split_user(address: &str) -> (Option<&str>, &str) {
    match address.rsplit_once('@') {
        Some((user, host)) => (Some(user).filter(|u| !u.is_empty()), host),
        None => (None, address),
    }
}

/// A port number from a session file, or `default` when it is missing or not a port.
fn port_or(value: Option<&str>, default: u16) -> u16 {
    value
        .and_then(|v| v.trim().parse::<u16>().ok())
        .filter(|p| *p != 0)
        .unwrap_or(default)
}

/// `value` trimmed, or `None` when that leaves nothing.
fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}
