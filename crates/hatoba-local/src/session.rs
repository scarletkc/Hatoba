//! A local terminal tab's shell, which the app keeps by session id and the AI assistant's tools
//! run on.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{LocalPty, LocalTools, Shell};

/// A local terminal tab's shell.
pub struct LocalSession {
    pub pty: Arc<LocalPty>,
    pub shell: Shell,
    /// How the AI assistant's tools run on the tab (AI-12, AI-38…41, AI-48, AI-49).
    pub tools: LocalTools,
    closed: AtomicBool,
}

impl LocalSession {
    pub fn new(pty: Arc<LocalPty>, shell: Shell, tools: LocalTools) -> Self {
        Self {
            pty,
            shell,
            tools,
            closed: AtomicBool::new(false),
        }
    }

    /// Ends the shell and the console programs it started.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.pty.close();
    }

    /// `true` once the shell has ended, so the tab's tools return the disconnected result.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}
