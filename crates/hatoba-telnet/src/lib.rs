//! Telnet terminal tabs for Hatoba (spec §7.6): the Telnet protocol between a terminal and a
//! server, and the saved credentials typed at its login prompts.
//!
//! [`TelnetSession`] runs the protocol on a connection the caller opened, so the desktop app dials
//! through the host's proxy as it does for SSH (SSH-13). The crate is independent of Tauri, like
//! `hatoba-ssh` and `hatoba-local`.

pub mod codec;
pub mod error;
pub mod line;
pub mod login;
pub mod session;

pub use codec::Telnet;
pub use error::TelnetError;
pub use login::AutoLogin;
pub use session::{TelnetEvent, TelnetSession};
