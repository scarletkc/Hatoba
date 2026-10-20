//! The `hatoba` command's own commands (HOST-15): `hatoba hosts`, `hatoba exec` and `hatoba cp`
//! let other programs, such as coding agents and scripts, use the hosts saved in Hatoba. The
//! command asks the running app over a local endpoint ([`endpoint`], [`protocol`]); the app signs
//! in with the vault's credentials and sends back only what the command printed or the file
//! copied. Any other command line opens Hatoba with its arguments (HOST-13).
//!
//! On Windows this runs as `hatoba.exe`, a console program beside the app, whose own executable
//! has no console. Elsewhere the app's executable runs it before anything else ([`own_command`]).

mod args;
mod client;
pub mod endpoint;
pub mod protocol;

use std::ffi::{OsStr, OsString};

pub use args::{COMMANDS, SWITCH};

/// The app this command belongs to, from the build's Tauri configuration.
#[derive(Debug, Clone, Copy)]
pub struct App {
    /// Tauri's `identifier`, which names the endpoint.
    pub identifier: &'static str,
    /// Tauri's `mainBinaryName`, the app's executable without `.exe`.
    pub main_binary: &'static str,
    pub version: &'static str,
}

/// Whether a command line, given without the program, is one of this command's own.
pub fn own_command(first: Option<&OsStr>) -> bool {
    first
        .and_then(OsStr::to_str)
        .is_some_and(|word| COMMANDS.contains(&word))
}

/// Runs a command line, given with the program first as `std::env::args_os` has it, and returns
/// the exit status.
pub fn run(app: &App, argv: impl IntoIterator<Item = OsString>) -> i32 {
    match args::parse(argv.into_iter().skip(1).collect()) {
        Ok(command) => client::run(app, command),
        Err(message) => {
            eprintln!("hatoba: {message}\nRun hatoba --help to see how to use it.");
            2
        }
    }
}
