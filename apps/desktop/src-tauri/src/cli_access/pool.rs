//! The connections `hatoba exec` and `hatoba cp` run over (HOST-15): one per saved host, kept
//! between commands so that a program running many of them signs in once, and closed after
//! [`IDLE`] unused, when the host connects another way and the commands on it are done, or with
//! the lock.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hatoba_ssh::SshSession;
use tokio_util::sync::CancellationToken;

use crate::error::{AppError, AppResult};

/// How long a connection stays open without a command or a copy on it.
pub const IDLE: Duration = Duration::from_secs(10 * 60);

#[derive(Default)]
pub struct Pool {
    /// By host id. A connection is made under its slot's lock, so commands that come together for
    /// one host share it.
    slots: Mutex<HashMap<String, Arc<tokio::sync::Mutex<Option<Pooled>>>>>,
}

struct Pooled {
    session: SshSession,
    /// How the host connected (`dashboard::config_version`).
    config: String,
    use_: Arc<Use>,
}

#[derive(Default)]
struct Use {
    state: Mutex<UseState>,
}

#[derive(Default)]
struct UseState {
    leases: usize,
    last: Option<Instant>,
    /// The pool let go of the connection while commands ran on it; the last one closes it.
    retired: bool,
}

/// A connection for one command or copy. The pool's stays open while one is out.
pub struct Lease {
    pub session: SshSession,
    use_: Option<Arc<Use>>,
}

impl Lease {
    /// A connection the pool does not keep: a terminal's (HOST-14 does the same).
    pub fn borrowed(session: SshSession) -> Self {
        Self {
            session,
            use_: None,
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(use_) = &self.use_ {
            let mut state = lock(&use_.state);
            state.last = Some(Instant::now());
            state.leases -= 1;
            if state.retired && state.leases == 0 {
                close(self.session.clone());
            }
        }
    }
}

impl Pooled {
    fn lease(&self) -> Lease {
        lock(&self.use_.state).leases += 1;
        Lease {
            session: self.session.clone(),
            use_: Some(Arc::clone(&self.use_)),
        }
    }

    fn idle_for(&self) -> Duration {
        let state = lock(&self.use_.state);
        if state.leases > 0 {
            return Duration::ZERO;
        }
        state.last.map_or(Duration::ZERO, |last| last.elapsed())
    }

    /// Closes the connection once no command or copy runs on it.
    fn retire(self) {
        let mut state = lock(&self.use_.state);
        if state.leases == 0 {
            close(self.session);
        } else {
            state.retired = true;
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn close(session: SshSession) {
    tauri::async_runtime::spawn(async move { session.disconnect().await });
}

impl Pool {
    /// The host's connection, made with `connect` when there is none yet, or when the one there
    /// was made another way than `config` or has closed. One made after `epoch` ended, which the
    /// lock ends, is closed again.
    pub async fn lease<F, Fut>(
        &self,
        host_id: &str,
        config: &str,
        epoch: &CancellationToken,
        connect: F,
    ) -> AppResult<Lease>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = AppResult<SshSession>>,
    {
        let slot = Arc::clone(lock(&self.slots).entry(host_id.to_owned()).or_default());
        let mut slot = slot.lock().await;
        if let Some(pooled) = slot
            .as_ref()
            .filter(|p| p.config == config && !p.session.is_closed())
        {
            return Ok(pooled.lease());
        }
        if let Some(old) = slot.take() {
            old.retire();
        }
        let session = connect().await?;
        if epoch.is_cancelled() {
            close(session);
            return Err(AppError::locked());
        }
        let pooled = Pooled {
            session,
            config: config.to_owned(),
            use_: Arc::default(),
        };
        let lease = pooled.lease();
        *slot = Some(pooled);
        Ok(lease)
    }

    /// Lets go of the host's connection, which closes once no command or copy runs on it.
    pub fn release(&self, host_id: &str) {
        let slots = lock(&self.slots);
        let Some(slot) = slots.get(host_id) else {
            return;
        };
        // A slot in use is making its connection, which the next lease replaces.
        if let Ok(mut slot) = slot.try_lock()
            && let Some(pooled) = slot.take()
        {
            pooled.retire();
        }
    }

    /// Closes the connections unused for `idle`, and those that closed on their own.
    pub fn close_idle(&self, idle: Duration) {
        let mut slots = lock(&self.slots);
        slots.retain(|_, slot| {
            // A slot in use is making its connection.
            let Ok(mut slot) = slot.try_lock() else {
                return true;
            };
            if slot
                .as_ref()
                .is_some_and(|p| p.session.is_closed() || p.idle_for() >= idle)
                && let Some(pooled) = slot.take()
            {
                close(pooled.session);
            }
            slot.is_some()
        });
    }

    /// Closes every connection. One being made now is closed once it is, by its epoch.
    pub fn close_all(&self) {
        let slots: Vec<_> = lock(&self.slots).drain().map(|(_, slot)| slot).collect();
        for slot in slots {
            if let Ok(mut slot) = slot.try_lock()
                && let Some(pooled) = slot.take()
            {
                close(pooled.session);
            }
        }
    }
}
