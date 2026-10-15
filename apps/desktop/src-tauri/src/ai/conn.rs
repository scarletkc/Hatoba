//! What a terminal tab's tools run over (AI-08): the tab's SSH connection, or a local tab's shell
//! on this computer (§7.5). The file and search tools take either; the scripts for a host are in
//! `hatoba_ssh`, those for this computer in `hatoba_local`.

use std::sync::Arc;
use std::time::Duration;

use hatoba_ssh::SshSession;
use hatoba_ssh::remote_file::{self, Expect, RemoteDir, RemoteFile, RemoteFileError, RemoteLines};
use hatoba_ssh::remote_search::{self, GrepQuery, RemoteSearchError, Scope, SearchOutput};

use crate::local::LocalSession;

#[derive(Clone)]
pub enum Conn {
    Ssh(SshSession),
    Local(Arc<LocalSession>),
}

impl Conn {
    /// `true` once the connection, or the local shell, has ended.
    pub fn is_closed(&self) -> bool {
        match self {
            Self::Ssh(session) => session.is_closed(),
            Self::Local(local) => local.is_closed(),
        }
    }

    /// Whether the tools take Windows paths: a local tab of PowerShell, Command Prompt or Git
    /// Bash (§7.5).
    pub fn windows_paths(&self) -> bool {
        match self {
            Self::Ssh(_) => false,
            Self::Local(local) => local.tools.windows_paths(),
        }
    }

    /// The server, as the record of what the model has seen and the file snapshots name it:
    /// `user@host:port`, or for a local tab `local` (this computer's files) or
    /// `wsl:<distribution>`.
    pub fn server(&self) -> String {
        match self {
            Self::Ssh(session) => format!(
                "{}@{}:{}",
                session.username(),
                session.host(),
                session.port()
            ),
            Self::Local(local) => local.tools.server().to_owned(),
        }
    }

    pub async fn read(&self, path: &str, max: u64) -> Result<RemoteFile, RemoteFileError> {
        match self {
            Self::Ssh(session) => remote_file::read(session, path, max).await,
            Self::Local(local) => local.tools.read(path, max).await,
        }
    }

    pub async fn read_lines(
        &self,
        path: &str,
        from: u64,
        max: u64,
    ) -> Result<RemoteLines, RemoteFileError> {
        match self {
            Self::Ssh(session) => remote_file::read_lines(session, path, from, max).await,
            Self::Local(local) => local.tools.read_lines(path, from, max).await,
        }
    }

    pub async fn list_dir(&self, path: &str, max: u64) -> Result<RemoteDir, RemoteFileError> {
        match self {
            Self::Ssh(session) => remote_file::list_dir(session, path, max).await,
            Self::Local(local) => local.tools.list_dir(path, max).await,
        }
    }

    pub async fn write(
        &self,
        path: &str,
        content: &[u8],
        expect: Expect<'_>,
        mode: Option<u32>,
    ) -> Result<String, RemoteFileError> {
        match self {
            Self::Ssh(session) => remote_file::write(session, path, content, expect, mode).await,
            Self::Local(local) => local.tools.write(path, content, expect, mode).await,
        }
    }

    pub async fn delete(&self, path: &str, expect: &[u8]) -> Result<String, RemoteFileError> {
        match self {
            Self::Ssh(session) => remote_file::delete(session, path, expect).await,
            Self::Local(local) => local.tools.delete(path, expect).await,
        }
    }

    pub async fn glob(
        &self,
        scope: &Scope<'_>,
        max: u64,
        time_limit: Duration,
    ) -> Result<SearchOutput, RemoteSearchError> {
        match self {
            Self::Ssh(session) => remote_search::glob(session, scope, max, time_limit).await,
            Self::Local(local) => local.tools.glob(scope, max, time_limit).await,
        }
    }

    pub async fn grep(
        &self,
        query: &GrepQuery<'_>,
        max: u64,
        time_limit: Duration,
    ) -> Result<SearchOutput, RemoteSearchError> {
        match self {
            Self::Ssh(session) => remote_search::grep(session, query, max, time_limit).await,
            Self::Local(local) => local.tools.grep(query, max, time_limit).await,
        }
    }
}
