//! Telnet terminal tabs (spec §7.6): each tab's connection, by session id. Their ids are UUIDv7
//! like the SSH and local sessions', so one id never names two kinds of tab.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use hatoba_telnet::TelnetSession;

use crate::error::{AppError, AppResult};

/// A Telnet tab's connection.
pub struct TelnetLive {
    pub session: Arc<TelnetSession>,
    /// The Telnet host it connects to; `None` for a quick connection (TELNET-05).
    pub host_id: Option<String>,
    closed: AtomicBool,
}

impl TelnetLive {
    pub fn new(session: Arc<TelnetSession>, host_id: Option<String>) -> Self {
        Self {
            session,
            host_id,
            closed: AtomicBool::new(false),
        }
    }

    /// Closes the connection.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.session.close();
    }

    /// `true` once Hatoba closed the connection, on lock, quit or the tab's close.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

#[derive(Default)]
pub struct TelnetManager {
    sessions: Mutex<HashMap<String, Arc<TelnetLive>>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl TelnetManager {
    pub fn insert(&self, id: String, live: Arc<TelnetLive>) {
        lock(&self.sessions).insert(id, live);
    }

    pub fn get(&self, id: &str) -> AppResult<Arc<TelnetLive>> {
        lock(&self.sessions)
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::not_found("session"))
    }

    /// Removes the session only if it is still the given instance.
    pub fn remove_if_same(&self, id: &str, live: &Arc<TelnetLive>) {
        let mut sessions = lock(&self.sessions);
        if sessions.get(id).is_some_and(|s| Arc::ptr_eq(s, live)) {
            sessions.remove(id);
        }
    }

    pub fn take(&self, id: &str) -> Option<Arc<TelnetLive>> {
        lock(&self.sessions).remove(id)
    }

    /// SEC-03 "lock disconnects sessions", and quitting: takes every session out.
    pub fn take_all(&self) -> Vec<Arc<TelnetLive>> {
        lock(&self.sessions).drain().map(|(_, live)| live).collect()
    }
}
