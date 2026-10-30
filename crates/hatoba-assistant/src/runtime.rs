//! The app's async runtime (Tauri's in the desktop app), where the assistant's background work
//! runs. That work may start outside any runtime, in a command that runs on the main thread.

use std::future::Future;
use std::sync::OnceLock;

use tokio::runtime::Handle;
use tokio::task::JoinHandle;

static APP: OnceLock<Handle> = OnceLock::new();

/// Sets the app's runtime. The app calls it once at startup, before anything here spawns.
pub fn set(handle: Handle) {
    let _ = APP.set(handle);
}

/// The app's runtime: the one [`set`] named, else the current one (tests).
fn app() -> Handle {
    APP.get().cloned().unwrap_or_else(Handle::current)
}

/// Runs a task on the app's runtime.
pub(crate) fn spawn<F>(task: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    app().spawn(task)
}

/// Runs blocking work on the app's runtime's blocking threads.
pub(crate) fn spawn_blocking<F, R>(work: F) -> JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    app().spawn_blocking(work)
}
