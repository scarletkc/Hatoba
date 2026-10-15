//! Local terminal tabs for Hatoba (spec §7.5): a shell of this computer in a PTY, and the AI
//! assistant's tools on this computer.
//!
//! [`shells::detect`] lists the shells a tab can run, and [`pty::LocalPty`] runs one in a
//! pseudo-terminal: ConPTY on Windows, a Unix PTY elsewhere. The crate is independent of Tauri,
//! like `hatoba-ssh`, whose result types the assistant's tools share so the same code formats
//! their results for both kinds of tab.

pub mod error;
pub mod pty;
pub mod shells;
pub mod tools;

pub use error::LocalError;
pub use pty::{LocalPty, PtyEvent, PtySize};
pub use shells::{Shell, ShellKind};
pub use tools::LocalTools;
