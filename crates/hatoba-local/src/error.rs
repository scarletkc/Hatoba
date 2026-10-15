use thiserror::Error;

/// Why a local terminal or one of the assistant's tools on this computer failed.
#[derive(Debug, Error)]
pub enum LocalError {
    /// The shell named is not one this computer has (any more).
    #[error("this computer has no shell \"{0}\"")]
    UnknownShell(String),
    /// The shell could not be started.
    #[error("the shell could not be started: {0}")]
    Spawn(String),
    /// The terminal has ended.
    #[error("the terminal has ended")]
    Closed,
    #[error("{0}")]
    Io(String),
}
