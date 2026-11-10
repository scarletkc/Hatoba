//! The `hatoba` command's own commands (HOST-15): `hatoba hosts`, `hatoba exec` and `hatoba cp`
//! let other programs, such as coding agents and scripts, use the hosts saved in Hatoba, and
//! `hatoba memory` lets them read and change the AI assistant's memories (AI-53). The command
//! asks the running app over a local endpoint ([`endpoint`], [`protocol`]); the app signs in with
//! the vault's credentials and sends back only what the command printed or the file copied. Any
//! other command line opens Hatoba with its arguments (HOST-13).
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

/// What the macOS launcher puts before the first word of its command line to ask the app's
/// executable whether that word is one of this command's own ([`answer`]). That launcher is
/// written once, so a list of the words in it would miss the commands a later version adds.
pub const ASK: &str = "--own-command";

/// The exit status that answers a launcher, for a command line, given without the program, that
/// starts with [`ASK`]: 0 when the word after it is one of this command's own, 1 when it is not.
pub fn answer(args: impl IntoIterator<Item = OsString>) -> Option<i32> {
    let mut args = args.into_iter();
    (args.next()? == ASK).then(|| i32::from(!own_command(args.next().as_deref())))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// HOST-13: the macOS launcher asks which words are the command's own.
    #[test]
    fn answers_whether_a_word_is_one_of_the_commands_own() {
        let answer = |words: &[&str]| answer(words.iter().map(OsString::from));
        for command in COMMANDS {
            assert_eq!(answer(&[ASK, command]), Some(0), "{command}");
        }
        for word in ["db", "Exec", "", ASK] {
            assert_eq!(answer(&[ASK, word]), Some(1), "{word}");
        }
        assert_eq!(answer(&[ASK]), Some(1));
        // Any other command line asks nothing, and the question is not a command.
        for words in [&[][..], &["exec", "db", "ls"], &["db", ASK]] {
            assert_eq!(answer(words), None, "{words:?}");
        }
        assert!(!own_command(Some(OsStr::new(ASK))));
    }
}
