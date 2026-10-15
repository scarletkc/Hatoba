//! Where the exec scripts of the AI assistant's file and search tools run (spec §13.4): an exec
//! channel of an SSH connection, or the `sh` of this computer for a local tab (§7.5,
//! [`crate::local_sh::LocalSh`]).

use std::future::Future;

use crate::error::SshError;
use crate::session::{ExecOutput, SshSession};

/// Runs a command line as a login shell would, with `stdin`, and returns what it printed.
pub trait Exec: Sync {
    /// Like [`SshSession::exec_output`]: the exit status is `None` when the command was ended by a
    /// signal.
    fn exec_output(
        &self,
        command: &str,
        stdin: Option<Vec<u8>>,
    ) -> impl Future<Output = Result<ExecOutput, SshError>> + Send;
}

impl Exec for SshSession {
    fn exec_output(
        &self,
        command: &str,
        stdin: Option<Vec<u8>>,
    ) -> impl Future<Output = Result<ExecOutput, SshError>> + Send {
        SshSession::exec_output(self, command, stdin)
    }
}
