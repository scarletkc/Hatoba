//! The dashboard (HOST-13): the resource usage of every host the page shows, read in the
//! background without the user.
//!
//! Each host the page shows gets a reader task. It reads over a terminal tab's connection to the
//! host when there is one, and otherwise over a connection of its own, which it opens without ever
//! asking the user anything ([`connect_unattended`]). A host that needs a password, a passphrase,
//! a verification code or a host key decision is held until the user retries, changes the host or
//! opens a tab on it, and so is one whose sign-in failed, so that tools like fail2ban never see
//! repeated failures from this machine. At most [`MAX_SETUPS`] connections are being set up at
//! once, and a host whose first hop does not answer the HOST-10 probe is not signed in to. When
//! the page goes away the readers stop, and their connections close a minute later unless a
//! terminal tab took one over in the meantime ([`Dashboard::take_connection`]).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use hatoba_core::model::{Host, HostAuth, Item};
use hatoba_core::vault::Vault;
use hatoba_ssh::{ConnectConfig, ProxyConfig, SshSession};
use tauri::AppHandle;
use tokio::sync::{Notify, OnceCell, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::convert::stats_view;
use crate::dto::{DashboardEvent, DashboardHold, HostKeyPromptKind};
use crate::error::{AppError, KeyParseErrorKind, SshErrorKind};
use crate::ssh::{Unattended, build_config, connect_unattended, first_hop, known_host_keys};
use crate::state::state;

/// Connections being set up at the same time, from the TCP connection to the end of sign-in.
/// OpenSSH's default `MaxStartups` starts dropping unauthenticated connections past 10, which
/// matters when many hosts sit behind one jump host.
pub const MAX_SETUPS: usize = 8;
/// How long a connection stays open after the page stops reading its host.
const LINGER: Duration = Duration::from_secs(60);
/// How long a host that is offline, failed or held waits before it is looked at again.
const RECHECK: Duration = Duration::from_secs(60);
/// A probe answer is shared by every host with the same first hop for this long, so 30 hosts
/// behind one jump host probe it once.
const PROBE_FRESH: Duration = Duration::from_secs(30);
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// The most hosts one watch reads.
const MAX_HOSTS: usize = 500;
/// The systems (HOST-11 ids) whose servers have no `/proc` to read, with their names.
const NO_PROC: &[(&str, &str)] = &[
    ("freebsd", "FreeBSD"),
    ("netbsd", "NetBSD"),
    ("windows", "Windows"),
];

/// Where a watch's events go. `false` once the page can no longer receive them.
pub type Sink = Arc<dyn Fn(DashboardEvent) -> bool + Send + Sync>;

pub struct Dashboard {
    inner: Arc<Mutex<Inner>>,
    setups: Arc<Semaphore>,
}

impl Default for Dashboard {
    fn default() -> Self {
        Self {
            inner: Arc::default(),
            setups: Arc::new(Semaphore::new(MAX_SETUPS)),
        }
    }
}

#[derive(Default)]
struct Inner {
    next_watch: u64,
    /// The page's watch, while it shows.
    watch: Option<Watch>,
    hosts: HashMap<String, HostEntry>,
    probes: HashMap<String, Arc<Probe>>,
}

struct Watch {
    id: u64,
    interval: Duration,
    sink: Sink,
}

#[derive(Default)]
struct HostEntry {
    reader: Option<Reader>,
    /// The dashboard's own connection, until a tab takes it over or it closes.
    own: Option<Own>,
    /// Why the host is not signed in to on its own, until its versions change or **Retry**.
    hold: Option<Held>,
    /// A server found not to run Linux, for the host's configuration then.
    unsupported: Option<(String, String)>,
    /// Bumped by every release, so a linger that a later one replaced leaves the connection.
    released: u64,
}

struct Reader {
    cancel: CancellationToken,
    retry: Arc<Notify>,
}

struct Own {
    session: SshSession,
    /// [`Versions::config`] when it connected.
    config: String,
}

struct Held {
    hold: DashboardHold,
    error: Option<AppError>,
    /// [`Versions::all`] when it was held.
    version: String,
}

struct Probe {
    at: Instant,
    online: OnceCell<bool>,
}

fn lock(inner: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    inner.lock().unwrap_or_else(|p| p.into_inner())
}

/// Closes a connection without waiting for it.
fn close(session: SshSession) {
    tauri::async_runtime::spawn(async move { session.disconnect().await });
}

impl Dashboard {
    /// Reads `host_ids` every `interval_secs` (1 to 60) and sends what happens to `sink`, in
    /// place of an earlier watch. Hosts both watches read keep their readers and connections,
    /// unless the interval changed; the others stop. Returns the id `unwatch` takes.
    pub fn watch(
        &self,
        app: &AppHandle,
        host_ids: &[String],
        interval_secs: u32,
        sink: Sink,
    ) -> u64 {
        let interval = Duration::from_secs(u64::from(interval_secs.clamp(1, 60)));
        let wanted: Vec<&String> = {
            let mut seen = HashSet::new();
            host_ids
                .iter()
                .filter(|id| seen.insert(id.as_str()))
                .take(MAX_HOSTS)
                .collect()
        };
        let mut inner = lock(&self.inner);
        inner.next_watch += 1;
        let id = inner.next_watch;
        let restart = inner.watch.as_ref().is_some_and(|w| w.interval != interval);
        inner.watch = Some(Watch { id, interval, sink });
        let mut released = Vec::new();
        for (host_id, entry) in &mut inner.hosts {
            let kept = wanted.contains(&host_id);
            if (restart || !kept)
                && let Some(reader) = entry.reader.take()
            {
                reader.cancel.cancel();
                if !kept {
                    released.push(host_id.clone());
                }
            }
        }
        for host_id in wanted {
            let entry = inner.hosts.entry(host_id.clone()).or_default();
            if entry.reader.is_none() {
                let reader = Reader {
                    cancel: CancellationToken::new(),
                    retry: Arc::new(Notify::new()),
                };
                tauri::async_runtime::spawn(read_host(
                    app.clone(),
                    host_id.clone(),
                    interval,
                    reader.cancel.clone(),
                    Arc::clone(&reader.retry),
                ));
                entry.reader = Some(reader);
            }
        }
        // What is neither read, connected nor held is forgotten.
        inner
            .hosts
            .retain(|_, e| e.reader.is_some() || e.own.is_some() || e.hold.is_some());
        drop(inner);
        for host_id in released {
            self.release(&host_id);
        }
        id
    }

    /// Stops the watch `id`: every reader stops, and the connections close a minute later.
    pub fn unwatch(&self, id: u64) {
        let released: Vec<String> = {
            let mut inner = lock(&self.inner);
            if inner.watch.as_ref().is_none_or(|w| w.id != id) {
                return;
            }
            inner.watch = None;
            inner
                .hosts
                .iter_mut()
                .filter_map(|(host_id, entry)| {
                    let reader = entry.reader.take()?;
                    reader.cancel.cancel();
                    Some(host_id.clone())
                })
                .collect()
        };
        for host_id in released {
            self.release(&host_id);
        }
    }

    /// **Retry**: signs in to a held host again, and looks at an offline or failed one now.
    pub fn retry(&self, host_id: &str) {
        let mut inner = lock(&self.inner);
        if let Some(entry) = inner.hosts.get_mut(host_id) {
            entry.hold = None;
            if let Some(reader) = &entry.reader {
                reader.retry.notify_one();
            }
        }
    }

    /// The lock (SEC-03): every reader stops and every connection of the dashboard closes now.
    /// Held hosts stay held.
    pub fn stop_all(&self) {
        let sessions: Vec<SshSession> = {
            let mut inner = lock(&self.inner);
            inner.watch = None;
            inner.probes.clear();
            inner
                .hosts
                .values_mut()
                .filter_map(|entry| {
                    if let Some(reader) = entry.reader.take() {
                        reader.cancel.cancel();
                    }
                    entry.unsupported = None;
                    entry.own.take().map(|own| own.session)
                })
                .collect()
        };
        sessions.into_iter().for_each(close);
    }

    /// A terminal tab on the host takes over the dashboard's connection to it, while it is
    /// open and was made for the host as it is now (`config` is [`Versions::config`]). The
    /// dashboard then reads over the tab's connection like any other.
    pub fn take_connection(&self, host_id: &str, config: &str) -> Option<SshSession> {
        let mut inner = lock(&self.inner);
        let entry = inner.hosts.get_mut(host_id)?;
        let own = entry.own.take()?;
        if own.config == config && !own.session.is_closed() {
            Some(own.session)
        } else {
            close(own.session);
            None
        }
    }

    fn emit(&self, event: DashboardEvent) {
        let Some((id, sink)) = lock(&self.inner)
            .watch
            .as_ref()
            .map(|w| (w.id, Arc::clone(&w.sink)))
        else {
            return;
        };
        if !sink(event) {
            // The page is gone (a reload, say) without unwatching.
            self.unwatch(id);
        }
    }

    /// The dashboard's open connection to the host, made for the host as it is now.
    fn own(&self, host_id: &str, config: &str) -> Option<SshSession> {
        let mut inner = lock(&self.inner);
        let entry = inner.hosts.get_mut(host_id)?;
        let fresh = entry
            .own
            .as_ref()
            .is_some_and(|o| o.config == config && !o.session.is_closed());
        if fresh {
            return entry.own.as_ref().map(|o| o.session.clone());
        }
        if let Some(own) = entry.own.take() {
            close(own.session);
        }
        None
    }

    fn keep(&self, host_id: &str, session: SshSession, config: String) {
        let mut inner = lock(&self.inner);
        let entry = inner.hosts.entry(host_id.to_owned()).or_default();
        if let Some(old) = entry.own.replace(Own { session, config }) {
            close(old.session);
        }
    }

    /// Closes the dashboard's connection to the host a minute from now, unless the host is read
    /// again before then or a tab took the connection over.
    fn release(&self, host_id: &str) {
        let generation = {
            let mut inner = lock(&self.inner);
            let Some(entry) = inner.hosts.get_mut(host_id) else {
                return;
            };
            entry.released += 1;
            if entry.own.is_none() {
                return;
            }
            entry.released
        };
        let inner = Arc::clone(&self.inner);
        let host_id = host_id.to_owned();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(LINGER).await;
            let own = {
                let mut inner = lock(&inner);
                inner.hosts.get_mut(&host_id).and_then(|entry| {
                    (entry.reader.is_none() && entry.released == generation)
                        .then(|| entry.own.take())
                        .flatten()
                })
            };
            if let Some(own) = own {
                own.session.disconnect().await;
            }
        });
    }

    fn held(&self, host_id: &str, version: &str) -> Option<(DashboardHold, Option<AppError>)> {
        let mut inner = lock(&self.inner);
        let entry = inner.hosts.get_mut(host_id)?;
        if entry.hold.as_ref().is_some_and(|h| h.version != version) {
            entry.hold = None;
        }
        entry.hold.as_ref().map(|h| (h.hold, h.error.clone()))
    }

    fn hold(&self, host_id: &str, hold: DashboardHold, error: Option<AppError>, version: String) {
        lock(&self.inner)
            .hosts
            .entry(host_id.to_owned())
            .or_default()
            .hold = Some(Held {
            hold,
            error,
            version,
        });
    }

    fn unsupported(&self, host_id: &str, config: &str) -> Option<String> {
        let inner = lock(&self.inner);
        let (version, system) = inner.hosts.get(host_id)?.unsupported.as_ref()?;
        (version == config).then(|| system.clone())
    }

    fn set_unsupported(&self, host_id: &str, config: String, system: String) {
        let own = {
            let mut inner = lock(&self.inner);
            let entry = inner.hosts.entry(host_id.to_owned()).or_default();
            entry.unsupported = Some((config, system));
            entry.own.take()
        };
        // Nothing to read over it.
        if let Some(own) = own {
            close(own.session);
        }
    }

    /// Whether `target` answers the HOST-10 probe, asked once per [`PROBE_FRESH`] for every
    /// host it is the first hop of.
    async fn probe(&self, target: &ProbeTarget) -> bool {
        let probe = {
            let mut inner = lock(&self.inner);
            let key = target.key();
            match inner.probes.get(&key) {
                Some(p) if p.at.elapsed() < PROBE_FRESH => Arc::clone(p),
                _ => {
                    let p = Arc::new(Probe {
                        at: Instant::now(),
                        online: OnceCell::new(),
                    });
                    inner.probes.insert(key, Arc::clone(&p));
                    p
                }
            }
        };
        *probe
            .online
            .get_or_init(|| async {
                hatoba_ssh::tcp_probe_via(
                    target.proxy.as_ref(),
                    &target.host,
                    target.port,
                    PROBE_TIMEOUT,
                )
                .await
                .is_some()
            })
            .await
    }
}

/// What a host's connection depends on. A change to `config` (the host or a jump host was
/// edited) closes the dashboard's connection made before it; a change to either ends a hold.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Versions {
    config: String,
    /// The trusted host keys of every hop: a key accepted in a tab ends a host key hold.
    trust: String,
}

impl Versions {
    fn of(vault: &Vault, host: &Host) -> Self {
        let mut config = Vec::new();
        let mut trust = Vec::new();
        for hop in hops(vault, host) {
            config.push(format!(
                "{}@{}:{}#{}",
                hop.username, hop.address, hop.port, hop.updated_at
            ));
            let mut keys: Vec<String> = known_host_keys(vault, &hop.address, hop.port)
                .into_iter()
                .map(|(_, k)| k.fingerprint)
                .collect();
            keys.sort();
            trust.push(keys.join(","));
        }
        Self {
            config: config.join(";"),
            trust: trust.join(";"),
        }
    }

    fn all(&self) -> String {
        format!("{}|{}", self.config, self.trust)
    }
}

/// The host, then its jump hosts outwards, as far as the chain goes without looping.
fn hops<'a>(vault: &'a Vault, host: &'a Host) -> Vec<&'a Host> {
    let mut hops = vec![host];
    let mut seen = HashSet::new();
    let mut next = host.jump_host_id.as_deref();
    while let Some(id) = next.filter(|id| seen.insert(*id) && seen.len() <= 8) {
        let Some(jump) = vault.get(id).and_then(Item::as_host) else {
            break;
        };
        hops.push(jump);
        next = jump.jump_host_id.as_deref();
    }
    hops
}

/// Whether a hop asks for its password on every connection (SSH-03).
fn asks_password(host: &Host) -> bool {
    match &host.auth {
        HostAuth::Ask => true,
        HostAuth::Password { password } => password.is_empty(),
        HostAuth::Key { .. } | HostAuth::Agent => false,
    }
}

/// The address the probe tries: where the connection's TCP connection goes.
#[derive(Debug, Clone)]
struct ProbeTarget {
    host: String,
    port: u16,
    proxy: Option<ProxyConfig>,
}

impl ProbeTarget {
    fn key(&self) -> String {
        let via = self.proxy.as_ref().map_or_else(String::new, |p| {
            format!(" via {:?} {}:{}", p.kind, p.host, p.port)
        });
        format!("{}:{}{via}", self.host, self.port)
    }
}

/// Why a host is not connected to, found before trying.
#[derive(Debug)]
enum Stop {
    Hold(DashboardHold),
    Failed(AppError),
}

/// What the reader needs to know about the host as it is now.
struct Plan {
    versions: Versions,
    connect: Result<ConnectConfig, Stop>,
    probe: Result<ProbeTarget, AppError>,
    /// The system of a server the host was last seen on, when it has no `/proc`.
    no_proc: Option<&'static str>,
}

/// `None` once the host is gone.
fn plan(vault: &Vault, host_id: &str) -> Option<Plan> {
    let host = vault.get(host_id).and_then(Item::as_host)?;
    let versions = Versions::of(vault, host);
    let connect = if hops(vault, host).into_iter().any(asks_password) {
        Err(Stop::Hold(DashboardHold::Password))
    } else {
        build_config(vault, host, Some(host_id), None).map_err(Stop::Failed)
    };
    let first = first_hop(vault, host);
    let probe = crate::ssh::resolve_proxy(vault, &first.proxy).map(|proxy| ProbeTarget {
        host: first.address.clone(),
        port: first.port,
        proxy,
    });
    let os = vault.host_os(host_id);
    let no_proc = NO_PROC
        .iter()
        .find(|(id, _)| os.as_deref() == Some(*id))
        .map(|(_, name)| *name);
    Some(Plan {
        versions,
        connect,
        probe,
        no_proc,
    })
}

/// The hold a failed unattended connection puts the host in, if it needs the user.
fn hold_for(err: &AppError, outcome: &Unattended) -> Option<DashboardHold> {
    if let Some(kind) = outcome.host_key() {
        return Some(match kind {
            HostKeyPromptKind::New => DashboardHold::HostKeyNew,
            HostKeyPromptKind::Changed => DashboardHold::HostKeyChanged,
        });
    }
    if outcome.prompted() {
        return Some(DashboardHold::Prompt);
    }
    if matches!(
        err.key_kind,
        Some(KeyParseErrorKind::PassphraseRequired | KeyParseErrorKind::WrongPassphrase)
    ) {
        return Some(DashboardHold::Passphrase);
    }
    (err.ssh_kind == Some(SshErrorKind::AuthFailed)).then_some(DashboardHold::SignInFailed)
}

/// What the reader does after a step: read over a connection, or wait (`None`: not at all).
enum Next {
    Read { session: SshSession, tab: bool },
    Wait(Option<Duration>),
}

/// Reads one host until it is cancelled, the vault locks or the host is deleted, then lets its
/// connection linger: one kept just as the page went away would otherwise stay open.
async fn read_host(
    app: AppHandle,
    host_id: String,
    interval: Duration,
    cancel: CancellationToken,
    retry: Arc<Notify>,
) {
    let state = state(&app);
    loop {
        let Ok(Some(plan)) = state.with_unlocked(|v| Ok(plan(v, &host_id))) else {
            break;
        };
        let config = plan.versions.config.clone();
        let next = tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            next = connect_step(&app, &host_id, plan) => next,
        };
        let wait = match next {
            Next::Wait(wait) => wait,
            Next::Read { session, tab } => tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                wait = read_step(&app, &host_id, &session, tab, interval, config) => wait,
            },
        };
        if let Some(wait) = wait.filter(|w| !w.is_zero()) {
            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(wait) => {}
                () = retry.notified() => {}
            }
        }
    }
    if cancel.is_cancelled() {
        state.dashboard.release(&host_id);
    }
}

/// Finds or makes a connection to read the host over.
async fn connect_step(app: &AppHandle, host_id: &str, plan: Plan) -> Next {
    let state = state(app);
    let dash = &state.dashboard;
    let host = host_id.to_owned();
    let tab = state.ssh.connection_for_host(host_id);
    let unsupported = plan
        .no_proc
        .map(str::to_owned)
        .or_else(|| dash.unsupported(host_id, &plan.versions.config));
    if let Some(system) = unsupported {
        // Online status only, as in the host list; nothing to read, even over a tab.
        let online = match (&tab, &plan.probe) {
            (Some(_), _) => true,
            (None, Ok(target)) => dash.probe(target).await,
            (None, Err(_)) => false,
        };
        dash.emit(DashboardEvent::Probe {
            host_id: host.clone(),
            online,
        });
        dash.emit(DashboardEvent::Unsupported {
            host_id: host,
            system,
        });
        return Next::Wait(Some(RECHECK));
    }
    if let Some(session) = tab {
        return Next::Read { session, tab: true };
    }
    if let Some(session) = dash.own(host_id, &plan.versions.config) {
        return Next::Read {
            session,
            tab: false,
        };
    }
    let version = plan.versions.all();
    if let Some((hold, error)) = dash.held(host_id, &version) {
        dash.emit(DashboardEvent::Held {
            host_id: host,
            hold,
            error,
        });
        // Never signs in on its own, but finds a tab opened on the host.
        return Next::Wait(Some(RECHECK));
    }
    let probe = match plan.probe {
        Ok(target) => target,
        Err(error) => {
            dash.emit(DashboardEvent::Failed {
                host_id: host,
                error,
            });
            return Next::Wait(Some(RECHECK));
        }
    };
    let cfg = match plan.connect {
        Ok(cfg) => cfg,
        Err(Stop::Hold(hold)) => {
            let online = dash.probe(&probe).await;
            dash.emit(DashboardEvent::Probe {
                host_id: host.clone(),
                online,
            });
            dash.emit(DashboardEvent::Held {
                host_id: host,
                hold,
                error: None,
            });
            return Next::Wait(Some(RECHECK));
        }
        Err(Stop::Failed(error)) => {
            dash.emit(DashboardEvent::Failed {
                host_id: host,
                error,
            });
            return Next::Wait(Some(RECHECK));
        }
    };
    // A dead host would hold a slot until its connection timed out.
    let online = dash.probe(&probe).await;
    dash.emit(DashboardEvent::Probe {
        host_id: host.clone(),
        online,
    });
    if !online {
        return Next::Wait(Some(RECHECK));
    }
    dash.emit(DashboardEvent::Connecting {
        host_id: host.clone(),
    });
    let Ok(permit) = Arc::clone(&dash.setups).acquire_owned().await else {
        return Next::Wait(None);
    };
    let (result, outcome) = connect_unattended(app, cfg).await;
    drop(permit);
    let session = match result {
        Ok(session) => session,
        Err(error) => {
            match hold_for(&error, &outcome) {
                Some(hold) => {
                    dash.hold(host_id, hold, Some(error.clone()), version);
                    dash.emit(DashboardEvent::Held {
                        host_id: host,
                        hold,
                        error: Some(error),
                    });
                }
                None => dash.emit(DashboardEvent::Failed {
                    host_id: host,
                    error,
                }),
            }
            return Next::Wait(Some(RECHECK));
        }
    };
    // A tab may have connected while this one did.
    if let Some(tab) = state.ssh.connection_for_host(host_id) {
        close(session);
        return Next::Read {
            session: tab,
            tab: true,
        };
    }
    let os = session.server_os().map(hatoba_ssh::ServerOs::as_str);
    if let Err(e) = state.with_unlocked(|v| Ok(v.set_host_os(host_id, os)?)) {
        tracing::debug!("server OS not recorded: {}", e.detail);
    }
    dash.keep(host_id, session.clone(), plan.versions.config);
    Next::Read {
        session,
        tab: false,
    }
}

/// Reads the host over `session` until the readings stop. Returns how long to wait before the
/// next step.
async fn read_step(
    app: &AppHandle,
    host_id: &str,
    session: &SshSession,
    tab: bool,
    interval: Duration,
    config: String,
) -> Option<Duration> {
    let dash = &state(app).dashboard;
    let host = host_id.to_owned();
    let (handle, mut events) = match session.open_stats(interval).await {
        Ok(started) => started,
        Err(error) => {
            if session.is_closed() {
                return Some(Duration::ZERO);
            }
            dash.emit(DashboardEvent::Failed {
                host_id: host,
                error: error.into(),
            });
            return Some(RECHECK);
        }
    };
    dash.emit(DashboardEvent::Reading {
        host_id: host.clone(),
        os: session
            .server_os()
            .map(|os| hatoba_ssh::ServerOs::as_str(os).to_owned()),
        tab,
    });
    let wait = loop {
        match events.recv().await {
            Some(hatoba_ssh::StatsEvent::Stats(stats)) => dash.emit(DashboardEvent::Stats {
                host_id: host.clone(),
                stats: Box::new(stats_view(*stats)),
            }),
            Some(hatoba_ssh::StatsEvent::Unsupported(system)) => {
                dash.set_unsupported(host_id, config, system);
                break Some(Duration::ZERO);
            }
            Some(hatoba_ssh::StatsEvent::Ended(error)) => {
                // The connection ended (a tab on it closed, say): connect again at once.
                if session.is_closed() {
                    break Some(Duration::ZERO);
                }
                dash.emit(DashboardEvent::Failed {
                    host_id: host,
                    error: error.into(),
                });
                break Some(RECHECK);
            }
            None => break Some(RECHECK),
        }
    };
    drop(handle);
    wait
}

/// The version `take_connection` compares, for `ssh_connect`.
pub fn config_version(vault: &Vault, host: &Host) -> String {
    Versions::of(vault, host).config
}

#[cfg(test)]
mod tests {
    use hatoba_core::KdfParams;
    use hatoba_core::model::{HostProxy, KnownHost};

    use super::*;

    fn vault() -> Vault {
        let mut vault = Vault::open_in_memory().unwrap();
        vault
            .create_with_params(&hatoba_core::new_id(), KdfParams::for_tests())
            .unwrap();
        vault
    }

    fn host(address: &str, auth: HostAuth, jump: Option<&str>) -> Item {
        Item::Host(Host {
            name: address.to_owned(),
            address: address.to_owned(),
            port: 22,
            username: "deploy".to_owned(),
            auth,
            jump_host_id: jump.map(str::to_owned),
            proxy: HostProxy::Direct,
            ..Host::default()
        })
    }

    fn password(p: &str) -> HostAuth {
        HostAuth::Password {
            password: zeroize::Zeroizing::new(p.to_owned()),
        }
    }

    fn get(v: &Vault, id: &str) -> Host {
        v.get(id).and_then(Item::as_host).cloned().unwrap()
    }

    #[test]
    fn a_password_asked_on_any_hop_holds_the_host_before_connecting() {
        let mut v = vault();
        let saved = v.put(None, host("db", password("pw"), None)).unwrap();
        let plan = plan(&v, &saved).unwrap();
        assert!(plan.connect.is_ok());
        assert_eq!(plan.probe.unwrap().host, "db");

        let ask = v.put(None, host("ask", HostAuth::Ask, None)).unwrap();
        assert!(matches!(
            plan_of(&v, &ask).connect,
            Err(Stop::Hold(DashboardHold::Password))
        ));
        let empty = v.put(None, host("empty", password(""), None)).unwrap();
        assert!(matches!(
            plan_of(&v, &empty).connect,
            Err(Stop::Hold(DashboardHold::Password))
        ));

        let bastion = v.put(None, host("bastion", HostAuth::Ask, None)).unwrap();
        let behind = v
            .put(None, host("app", password("pw"), Some(&bastion)))
            .unwrap();
        let plan = plan_of(&v, &behind);
        assert!(matches!(
            plan.connect,
            Err(Stop::Hold(DashboardHold::Password))
        ));
        // The probe goes to the first hop, where the TCP connection goes.
        assert_eq!(plan.probe.unwrap().host, "bastion");
        assert!(super::plan(&v, "missing").is_none());
    }

    fn plan_of(v: &Vault, id: &str) -> Plan {
        plan(v, id).unwrap()
    }

    #[test]
    fn a_server_without_proc_is_not_signed_in_to() {
        let mut v = vault();
        let id = v.put(None, host("bsd", password("pw"), None)).unwrap();
        assert_eq!(plan_of(&v, &id).no_proc, None);
        v.set_host_os(&id, Some("freebsd")).unwrap();
        assert_eq!(plan_of(&v, &id).no_proc, Some("FreeBSD"));
        v.set_host_os(&id, Some("ubuntu")).unwrap();
        assert_eq!(plan_of(&v, &id).no_proc, None);
    }

    #[test]
    fn versions_follow_the_chain_and_its_trusted_keys() {
        let mut v = vault();
        let bastion = v.put(None, host("bastion", password("pw"), None)).unwrap();
        let id = v
            .put(None, host("app", password("pw"), Some(&bastion)))
            .unwrap();
        let before = Versions::of(&v, &get(&v, &id));
        assert_eq!(before, Versions::of(&v, &get(&v, &id)));

        // A key accepted for the jump host changes the trust, not the configuration.
        for fingerprint in ["SHA256:b", "SHA256:a"] {
            v.put(
                None,
                Item::KnownHost(KnownHost {
                    host: "bastion".to_owned(),
                    port: 22,
                    key_type: fingerprint.to_owned(),
                    public_key: fingerprint.to_owned(),
                    fingerprint: fingerprint.to_owned(),
                    first_seen_at: 1,
                    updated_at: 0,
                }),
            )
            .unwrap();
        }
        let trusted = Versions::of(&v, &get(&v, &id));
        assert_eq!(trusted.config, before.config);
        assert_ne!(trusted.trust, before.trust);
        assert!(trusted.trust.contains("SHA256:a,SHA256:b"));

        // Editing the jump host changes the configuration.
        let mut jump = get(&v, &bastion);
        jump.port = 2222;
        v.put(Some(&bastion), Item::Host(jump)).unwrap();
        let edited = Versions::of(&v, &get(&v, &id));
        assert_ne!(edited.config, trusted.config);
        assert_ne!(edited.all(), trusted.all());
    }

    #[test]
    fn a_looping_chain_ends() {
        let mut v = vault();
        let a = v.put(None, host("a", password("pw"), None)).unwrap();
        let b = v.put(None, host("b", password("pw"), Some(&a))).unwrap();
        let mut first = get(&v, &a);
        first.jump_host_id = Some(b.clone());
        v.put(Some(&a), Item::Host(first)).unwrap();
        let host = get(&v, &b);
        assert!(hops(&v, &host).len() <= 9);
        assert!(matches!(plan_of(&v, &b).connect, Err(Stop::Failed(_))));
    }

    #[test]
    fn failures_that_need_the_user_hold_the_host() {
        let quiet = Unattended::default();
        let auth = AppError::ssh(crate::error::SshErrorKind::AuthFailed, "denied");
        assert_eq!(hold_for(&auth, &quiet), Some(DashboardHold::SignInFailed));
        let timeout = AppError::ssh(crate::error::SshErrorKind::Timeout, "timed out");
        assert_eq!(hold_for(&timeout, &quiet), None);
        let mut passphrase = AppError::new(crate::error::ErrorCode::KeyParse, "encrypted");
        passphrase.key_kind = Some(KeyParseErrorKind::PassphraseRequired);
        assert_eq!(
            hold_for(&passphrase, &quiet),
            Some(DashboardHold::Passphrase)
        );
    }

    #[test]
    fn probes_of_one_first_hop_share_a_key() {
        let target = |host: &str, proxy: Option<ProxyConfig>| ProbeTarget {
            host: host.to_owned(),
            port: 22,
            proxy,
        };
        assert_eq!(target("bastion", None).key(), target("bastion", None).key());
        assert_ne!(target("bastion", None).key(), target("other", None).key());
        let proxy = ProxyConfig {
            kind: hatoba_ssh::ProxyKind::Socks5,
            host: "127.0.0.1".to_owned(),
            port: 1080,
            username: String::new(),
            password: zeroize::Zeroizing::new("secret".to_owned()),
        };
        let via = target("bastion", Some(proxy)).key();
        assert_ne!(via, target("bastion", None).key());
        assert!(!via.contains("secret"));
    }

    #[tokio::test]
    async fn no_more_than_eight_connections_are_set_up_at_once() {
        let dash = Dashboard::default();
        let running = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let most = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let tasks = (0..30).map(|_| {
            let setups = Arc::clone(&dash.setups);
            let (running, most) = (Arc::clone(&running), Arc::clone(&most));
            tokio::spawn(async move {
                let _permit = setups.acquire_owned().await.unwrap();
                let now = running.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                most.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(5)).await;
                running.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            })
        });
        futures::future::join_all(tasks).await;
        assert_eq!(most.load(std::sync::atomic::Ordering::SeqCst), MAX_SETUPS);
    }
}
