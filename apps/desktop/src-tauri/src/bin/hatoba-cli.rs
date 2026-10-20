//! `hatoba.exe` on Windows (HOST-15): the console program behind the `hatoba` command. It runs the
//! command's own commands, such as `hatoba exec`, which print what they get back, and opens the app
//! with any other command line (HOST-13). The app's executable has no console, so it cannot.

const APP: hatoba_cli::App = hatoba_cli::App {
    identifier: env!("HATOBA_IDENTIFIER"),
    main_binary: env!("HATOBA_MAIN_BINARY"),
    version: env!("CARGO_PKG_VERSION"),
};

fn main() {
    std::process::exit(hatoba_cli::run(&APP, std::env::args_os()));
}
