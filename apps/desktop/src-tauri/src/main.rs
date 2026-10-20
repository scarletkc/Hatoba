// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // HOST-15: the `hatoba` command's own commands, such as `hatoba exec`, run before the app
    // would start. On Windows `hatoba.exe` runs them (`src/bin/hatoba-cli.rs`), since this
    // executable has no console to print to.
    #[cfg(unix)]
    if hatoba_cli::own_command(std::env::args_os().nth(1).as_deref()) {
        std::process::exit(hatoba_cli::run(
            &hatoba_desktop_lib::CLI_APP,
            std::env::args_os(),
        ));
    }
    hatoba_desktop_lib::run()
}
