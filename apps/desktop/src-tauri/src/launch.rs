//! What follows `hatoba` on the command line (HOST-13). The arguments of this launch, and those a
//! second launch hands over, wait here until the unlocked window takes them with `launch_take`.

use std::sync::{Mutex, MutexGuard};

use tauri::AppHandle;
use tauri_specta::Event;

use crate::dto::LaunchEvent;
use crate::platform;

/// The uninstaller runs Hatoba with this to take the `hatoba` command off the `PATH`.
const UNINSTALL_COMMAND: &str = "--uninstall-command";

/// Launches whose arguments the window has not taken yet, oldest first.
pub struct LaunchQueue(Mutex<Vec<Vec<String>>>);

impl LaunchQueue {
    pub const fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Vec<String>>> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Queues a launch's arguments, given with the program first as `std::env::args` has it.
    /// Returns false when it had none.
    pub fn push(&self, argv: impl IntoIterator<Item = String>) -> bool {
        let args = user_args(argv);
        if args.is_empty() {
            return false;
        }
        self.lock().push(args);
        true
    }

    pub fn take(&self) -> Vec<Vec<String>> {
        std::mem::take(&mut *self.lock())
    }
}

pub static QUEUE: LaunchQueue = LaunchQueue::new();

/// The arguments after the program. macOS gives an app opened from Finder a `-psn_…` process
/// serial number on older versions.
fn user_args(argv: impl IntoIterator<Item = String>) -> Vec<String> {
    argv.into_iter()
        .skip(1)
        .filter(|a| !a.trim().is_empty() && !a.starts_with("-psn_"))
        .collect()
}

/// A second launch handed over its arguments and quit: bring the window to the front and tell
/// it when there is something to take.
pub fn forwarded(app: &AppHandle, argv: Vec<String>) {
    platform::window::reopen_main(app);
    if QUEUE.push(argv) {
        let _ = LaunchEvent.emit(app);
    }
}

/// Runs the uninstaller's `--uninstall-command`, without starting the app. Returns whether this
/// launch was that.
pub fn uninstall_command() -> bool {
    if std::env::args().nth(1).as_deref() != Some(UNINSTALL_COMMAND) {
        return false;
    }
    if let Err(e) = platform::cli::uninstall() {
        eprintln!("removing the hatoba command failed: {e}");
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_owned()).collect()
    }

    #[test]
    fn queues_the_arguments_after_the_program() {
        let queue = LaunchQueue::new();
        assert!(!queue.push(argv(&["Hatoba.exe"])));
        assert!(!queue.push(argv(&["/Applications/Hatoba.app/Contents/MacOS/Hatoba", "-psn_0_12345"])));
        assert!(queue.push(argv(&["hatoba", "root@db", " "])));
        assert!(queue.push(argv(&["hatoba", "ssh", "-p", "2222", "My Server"])));
        assert_eq!(
            queue.take(),
            vec![argv(&["root@db"]), argv(&["ssh", "-p", "2222", "My Server"])]
        );
        assert!(queue.take().is_empty());
    }
}
