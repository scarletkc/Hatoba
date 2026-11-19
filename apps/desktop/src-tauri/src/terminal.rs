//! Plumbing shared by SSH, Telnet and local terminal tabs (spec §7.1, §7.5, §7.6, §10.3):
//! binary output frames, state events, terminal size clamping, and the session registry.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tauri::AppHandle;
use tauri::ipc::{InvokeResponseBody, IpcResponse};
use tauri_specta::Event;

use crate::dto::{SessionState, SessionStateEvent};
use crate::error::{AppError, AppResult};
use crate::state::MutexExt;

pub(crate) const FRAME_DATA: u8 = 0;
pub(crate) const FRAME_CLOSED: u8 = 1;
pub(crate) const FRAME_ERROR: u8 = 2;
/// The session id, sent before connecting: the tab can then answer the session's host-key and
/// login prompts for itself if it closes while they are open.
pub(crate) const FRAME_SESSION: u8 = 3;

/// One terminal frame on the per-session channel: tag byte + payload. Sent through Tauri's raw
/// binary IPC path (an `ArrayBuffer` in the WebView), never as a JSON number array (§10.3).
#[derive(specta::Type)]
pub struct TermFrame(#[specta(type = Vec<u8>)] Vec<u8>);

impl TermFrame {
    pub(crate) fn new(tag: u8, payload: &[u8]) -> Self {
        let mut buf = Vec::with_capacity(payload.len() + 1);
        buf.push(tag);
        buf.extend_from_slice(payload);
        Self(buf)
    }
}

impl IpcResponse for TermFrame {
    fn body(self) -> tauri::Result<InvokeResponseBody> {
        Ok(InvokeResponseBody::Raw(self.0))
    }
}

pub(crate) fn emit_state(
    app: &AppHandle,
    session_id: &str,
    host_id: Option<&str>,
    state: SessionState,
    extra: impl FnOnce(&mut SessionStateEvent),
) {
    let mut event = SessionStateEvent {
        session_id: session_id.to_owned(),
        host_id: host_id.map(str::to_owned),
        state,
        latency_ms: None,
        error: None,
        exit_status: None,
    };
    extra(&mut event);
    let _ = event.emit(app);
}

/// Clamps terminal dimensions to safe PTY bounds (`cols` in `10..=1000`, `rows` in `2..=500`).
pub(crate) fn clamp_size(cols: u32, rows: u32) -> (u16, u16) {
    (
        u16::try_from(cols.clamp(10, 1000)).unwrap_or(80),
        u16::try_from(rows.clamp(2, 500)).unwrap_or(24),
    )
}

/// Open terminal sessions keyed by session id.
pub struct SessionRegistry<T> {
    sessions: Mutex<HashMap<String, Arc<T>>>,
    on_remove: fn(&T),
}

impl<T> Default for SessionRegistry<T> {
    fn default() -> Self {
        Self::new(|_| {})
    }
}

impl<T> SessionRegistry<T> {
    /// Creates a registry that runs `on_remove` under the lock whenever a session is removed.
    pub fn new(on_remove: fn(&T)) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            on_remove,
        }
    }

    pub fn insert(&self, id: String, session: Arc<T>) {
        self.sessions.lock_unpoisoned().insert(id, session);
    }

    pub fn get(&self, id: &str) -> AppResult<Arc<T>> {
        self.sessions
            .lock_unpoisoned()
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::not_found("session"))
    }

    /// The session with `id`, when one is open; returns `None` rather than an error if closed.
    pub fn find(&self, id: &str) -> Option<Arc<T>> {
        self.sessions.lock_unpoisoned().get(id).cloned()
    }

    /// Removes `id` only if it still points to `live`, returning whether it was removed.
    pub fn remove_if_same(&self, id: &str, live: &Arc<T>) -> bool {
        let mut sessions = self.sessions.lock_unpoisoned();
        if sessions.get(id).is_some_and(|s| Arc::ptr_eq(s, live)) {
            sessions.remove(id);
            (self.on_remove)(live);
            true
        } else {
            false
        }
    }

    pub fn take(&self, id: &str) -> Option<Arc<T>> {
        let mut sessions = self.sessions.lock_unpoisoned();
        let live = sessions.remove(id)?;
        (self.on_remove)(&live);
        Some(live)
    }

    /// Runs `f` on every currently registered session under the registry lock.
    pub fn for_each(&self, mut f: impl FnMut(&T)) {
        for live in self.sessions.lock_unpoisoned().values() {
            f(live);
        }
    }

    pub fn take_all(&self) -> Vec<(String, Arc<T>)> {
        self.sessions
            .lock_unpoisoned()
            .drain()
            .map(|(id, live)| {
                (self.on_remove)(&live);
                (id, live)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct Dummy(AtomicUsize);

    #[test]
    fn clamp_size_bounds_cols_and_rows() {
        assert_eq!(clamp_size(0, 0), (10, 2));
        assert_eq!(clamp_size(9, 1), (10, 2));
        assert_eq!(clamp_size(10, 2), (10, 2));
        assert_eq!(clamp_size(80, 24), (80, 24));
        assert_eq!(clamp_size(1000, 500), (1000, 500));
        assert_eq!(clamp_size(1001, 501), (1000, 500));
        assert_eq!(clamp_size(u32::MAX, u32::MAX), (1000, 500));
    }

    #[test]
    fn remove_if_same_checks_identity_and_runs_hook() {
        let reg = SessionRegistry::new(|d: &Dummy| {
            d.0.fetch_add(1, Ordering::Relaxed);
        });
        let first = Arc::new(Dummy(AtomicUsize::new(0)));
        let second = Arc::new(Dummy(AtomicUsize::new(0)));

        reg.insert("s1".into(), Arc::clone(&first));
        assert!(!reg.remove_if_same("missing", &first));
        assert_eq!(first.0.load(Ordering::Relaxed), 0);

        // Replacing the slot means the stale session cannot remove the newer one.
        reg.insert("s1".into(), Arc::clone(&second));
        assert!(!reg.remove_if_same("s1", &first));
        assert_eq!(first.0.load(Ordering::Relaxed), 0);
        assert_eq!(second.0.load(Ordering::Relaxed), 0);
        assert!(Arc::ptr_eq(&reg.get("s1").unwrap(), &second));

        // Removing with the current instance removes it and invokes the hook once.
        assert!(reg.remove_if_same("s1", &second));
        assert_eq!(second.0.load(Ordering::Relaxed), 1);
        assert!(reg.find("s1").is_none());
        assert!(!reg.remove_if_same("s1", &second));
        assert_eq!(second.0.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn take_and_take_all_run_on_remove_hook() {
        let reg = SessionRegistry::new(|d: &Dummy| {
            d.0.fetch_add(1, Ordering::Relaxed);
        });
        let a = Arc::new(Dummy(AtomicUsize::new(0)));
        let b = Arc::new(Dummy(AtomicUsize::new(0)));
        let c = Arc::new(Dummy(AtomicUsize::new(0)));

        reg.insert("a".into(), Arc::clone(&a));
        reg.insert("b".into(), Arc::clone(&b));
        reg.insert("c".into(), Arc::clone(&c));

        assert!(reg.take("missing").is_none());
        let taken = reg.take("a").unwrap();
        assert!(Arc::ptr_eq(&taken, &a));
        assert_eq!(a.0.load(Ordering::Relaxed), 1);

        let all = reg.take_all();
        assert_eq!(all.len(), 2);
        assert_eq!(b.0.load(Ordering::Relaxed), 1);
        assert_eq!(c.0.load(Ordering::Relaxed), 1);
        assert!(reg.take_all().is_empty());
    }
}
