use thiserror::Error;

/// Why a Telnet tab's input could not be sent.
#[derive(Debug, Error)]
pub enum TelnetError {
    /// The connection has ended.
    #[error("the connection has ended")]
    Closed,
}
