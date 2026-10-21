//! The dashboard (HOST-14): the resource usage of every host the page shows, read in the
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
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use hatoba_core::model::{Host, HostAuth, Item};
use hatoba_core::vault::Vault;
use hatoba_ssh::{ConnectConfig, ProxyConfig, SshSession};
use tauri::AppHandle;
use tokio::sync::{Notify, OnceCell, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::convert::stats_view;
use crate::dto::{DashboardEvent, DashboardHold, HostKeyPromptKind};
use crate::error::{AppError, KeyParseErrorKind, SshErrorKind};
use crate::ssh::{Unattended, build_config, connect_unattended, known_host_keys};
use crate::state::state;

/// Connections being set up at the same time, from the TCP connection to the end of sign-in.
/// OpenSSH's default `MaxStartups` starts dropping unauthenticated connections past 10, which
/// matters when many hosts sit behind one jump host.
pub const MAX_SETUPS: usize = 8;
/// How long a connection stays open after the page stops reading its host.
const LINGER: Duration = Duration::from_secs(60);
/// How long a host that is offline, failed or held waits before it is looked at again.
const RECHECK: Duration = Duration::from_secs(60);
/// How often a reader checks that the host its connection was made for has not changed (a
/// sync pull, say), while it reads.
const CONFIG_CHECK: Duration = Duration::from_secs(10);
/// A probe answer is shared by every host with the same first hop for this long, so 30 hosts
/// behind one jump host probe it once.
const PROBE_FRESH: Duration = Duration::from_secs(30);
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

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
    next_reader: u64,
    /// Numbers every release, so a linger that a later one replaced leaves the connection.
    next_release: u64,
    /// The page's watch, while it shows.
    watch: Option<Watch>,
    hosts: HashMap<String, HostEntry>,
    probes: HashMap<String, Arc<Probe>>,
    /// Jump hosts that failed to sign in (or need the user) for one host, by item id: every
    /// host behind one waits with it, so its rejected credentials are not tried again for each.
    hop_holds: HashMap<String, Held>,
    /// Jump hosts by item id: until one has let a connection through, the hosts behind it try
    /// it one at a time.
    hop_gates: HashMap<String, HopGate>,
}

struct HopGate {
    /// [`JumpVersion::version`] when it last let a connection through.
    passed: Option<String>,
    gate: Arc<Semaphore>,
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
    /// The last release's number.
    released: u64,
    /// **Retry** asks the probe again rather than taking a recent answer.
    reprobe: bool,
    /// The jump host whose hold the host waits on, which **Retry** also ends.
    blocked_by: Option<String>,
    /// [`Versions::config`] of the connection the page's readings of the host came from.
    read_with: Option<String>,
}

struct Reader {
    id: u64,
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
    /// [`Versions::all`] when it was held, or [`JumpVersion::version`] for a jump host's hold.
    version: String,
    /// The jump host it failed at, as it connected then, whose hold **Retry** ends too.
    hop: Option<JumpVersion>,
    /// For a jump host's hold: the host whose **Retry** tries the jump host again. The other
    /// hosts behind it keep waiting until that one gets past it.
    retry_by: Option<String>,
}

impl HopGate {
    fn new() -> Self {
        Self {
            passed: None,
            gate: Arc::new(Semaphore::new(1)),
        }
    }
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
        let wanted: HashSet<&str> = host_ids.iter().map(String::as_str).collect();
        let mut inner = lock(&self.inner);
        inner.next_watch += 1;
        let id = inner.next_watch;
        let restart = inner.watch.as_ref().is_some_and(|w| w.interval != interval);
        inner.watch = Some(Watch { id, interval, sink });
        // A stopped reader lets its connection linger as it ends.
        for (host_id, entry) in &mut inner.hosts {
            if (restart || !wanted.contains(host_id.as_str()))
                && let Some(reader) = entry.reader.take()
            {
                reader.cancel.cancel();
            }
        }
        for host_id in wanted {
            inner.next_reader += 1;
            let reader_id = inner.next_reader;
            let entry = inner.hosts.entry(host_id.to_owned()).or_default();
            if entry.reader.is_none() {
                let reader = Reader {
                    id: reader_id,
                    cancel: CancellationToken::new(),
                    retry: Arc::new(Notify::new()),
                };
                tauri::async_runtime::spawn(read_host(
                    app.clone(),
                    host_id.to_owned(),
                    reader_id,
                    interval,
                    reader.cancel.clone(),
                    Arc::clone(&reader.retry),
                ));
                entry.reader = Some(reader);
            }
        }
        // What is neither read, connected nor held is forgotten. The settings the page's readings
        // came from, and a server found not to run Linux, stay until the lock: a host read again
        // later drops readings made with other settings, and is not signed in to again to learn
        // that it has no `/proc`.
        inner.hosts.retain(|_, e| {
            e.reader.is_some()
                || e.own.is_some()
                || e.hold.is_some()
                || e.read_with.is_some()
                || e.unsupported.is_some()
        });
        id
    }

    /// Stops the watch `id`: every reader stops, and the connections close a minute later.
    pub fn unwatch(&self, id: u64) {
        let mut inner = lock(&self.inner);
        if inner.watch.as_ref().is_none_or(|w| w.id != id) {
            return;
        }
        inner.watch = None;
        for entry in inner.hosts.values_mut() {
            if let Some(reader) = entry.reader.take() {
                reader.cancel.cancel();
            }
        }
    }

    /// A terminal tab on the host connected: its reader looks at once, so a held host is read
    /// over the tab without waiting for the next minute.
    pub fn wake(&self, host_id: &str) {
        if let Some(reader) = lock(&self.inner)
            .hosts
            .get(host_id)
            .and_then(|e| e.reader.as_ref())
        {
            reader.retry.notify_one();
        }
    }

    /// **Retry**: signs in to a held host again, and looks at an offline or failed one now.
    pub fn retry(&self, host_id: &str) {
        let mut inner = lock(&self.inner);
        let Some(entry) = inner.hosts.get_mut(host_id) else {
            return;
        };
        let hops: Vec<String> = entry
            .hold
            .take()
            .and_then(|h| h.hop)
            .map(|jump| jump.id)
            .into_iter()
            .chain(entry.blocked_by.take())
            .collect();
        entry.reprobe = true;
        if let Some(reader) = &entry.reader {
            reader.retry.notify_one();
        }
        // Only this host tries the jump host again; the others follow once it got past it.
        for hop in hops {
            if let Some(held) = inner.hop_holds.get_mut(&hop) {
                held.retry_by = Some(host_id.to_owned());
            }
        }
    }

    /// The lock (SEC-03): every reader stops and every connection of the dashboard closes now.
    /// The holds go too: they were made with hashes of the credentials, which start over at the
    /// lock ([`forget_secret_hashes`]), so a held host is tried once more after the unlock.
    pub fn stop_all(&self) {
        let sessions: Vec<SshSession> = {
            let mut inner = lock(&self.inner);
            inner.watch = None;
            inner.probes.clear();
            inner.hop_gates.clear();
            inner.hop_holds.clear();
            inner
                .hosts
                .values_mut()
                .filter_map(|entry| {
                    if let Some(reader) = entry.reader.take() {
                        reader.cancel.cancel();
                    }
                    entry.unsupported = None;
                    entry.read_with = None;
                    entry.hold = None;
                    entry.blocked_by = None;
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

    /// Keeps the connection a reader made, unless the reader was stopped meanwhile (the page
    /// went away, or the lock), when it closes at once. Returns whether it was kept.
    fn keep(
        &self,
        host_id: &str,
        session: SshSession,
        config: String,
        cancel: &CancellationToken,
    ) -> bool {
        let mut inner = lock(&self.inner);
        // Checked under the lock that `unwatch` and `stop_all` cancel under.
        if cancel.is_cancelled() {
            close(session);
            return false;
        }
        let entry = inner.hosts.entry(host_id.to_owned()).or_default();
        if let Some(old) = entry.own.replace(Own { session, config }) {
            close(old.session);
        }
        true
    }

    /// Whether the dashboard still holds its own connection to the host.
    fn owns(&self, host_id: &str) -> bool {
        lock(&self.inner)
            .hosts
            .get(host_id)
            .is_some_and(|e| e.own.is_some())
    }

    /// Whether the page's readings of the host came from a connection made for other settings
    /// (another server, perhaps), and are to be dropped. Once.
    fn settings_changed(&self, host_id: &str, config: &str) -> bool {
        let mut inner = lock(&self.inner);
        let Some(entry) = inner.hosts.get_mut(host_id) else {
            return false;
        };
        let changed = entry.read_with.as_deref().is_some_and(|c| c != config);
        if changed {
            entry.read_with = None;
        }
        changed
    }

    fn reading_with(&self, host_id: &str, config: &str) {
        let mut inner = lock(&self.inner);
        let entry = inner.hosts.entry(host_id.to_owned()).or_default();
        entry.read_with = Some(config.to_owned());
    }

    /// Closes the dashboard's own connection to the host, now that a tab's serves it.
    fn drop_own(&self, host_id: &str) {
        let own = lock(&self.inner)
            .hosts
            .get_mut(host_id)
            .and_then(|e| e.own.take());
        if let Some(own) = own {
            close(own.session);
        }
    }

    /// The reader `reader_id` of the host ended: it is forgotten, so a later watch starts
    /// another, and the connection lingers.
    fn reader_ended(&self, host_id: &str, reader_id: u64) {
        if let Some(entry) = lock(&self.inner).hosts.get_mut(host_id)
            && entry.reader.as_ref().is_some_and(|r| r.id == reader_id)
        {
            entry.reader = None;
        }
        self.release(host_id);
    }

    /// Closes the dashboard's connection to the host a minute from now, unless the host is read
    /// again before then or a tab took the connection over.
    fn release(&self, host_id: &str) {
        let generation = {
            let mut inner = lock(&self.inner);
            inner.next_release += 1;
            let generation = inner.next_release;
            let Some(entry) = inner.hosts.get_mut(host_id) else {
                return;
            };
            entry.released = generation;
            if entry.own.is_none() {
                return;
            }
            generation
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

    fn hold(
        &self,
        host_id: &str,
        hold: DashboardHold,
        error: Option<AppError>,
        version: String,
        hop: Option<&JumpVersion>,
    ) {
        let mut inner = lock(&self.inner);
        if let Some(jump) = hop {
            if let Some(gate) = inner.hop_gates.get_mut(&jump.id) {
                gate.passed = None;
            }
            inner.hop_holds.insert(
                jump.id.clone(),
                Held {
                    hold,
                    error: error.clone(),
                    version: jump.version.clone(),
                    hop: None,
                    retry_by: None,
                },
            );
        }
        inner.hosts.entry(host_id.to_owned()).or_default().hold = Some(Held {
            hold,
            error,
            version,
            hop: hop.cloned(),
            retry_by: None,
        });
    }

    /// A connection got past these jump hosts, so their holds end for every host behind them,
    /// the one whose connection failed at them included, and those hosts no longer try them
    /// one at a time.
    fn passed(&self, jumps: &[JumpVersion]) {
        let mut inner = lock(&self.inner);
        for jump in jumps {
            inner
                .hop_gates
                .entry(jump.id.clone())
                .or_insert_with(HopGate::new)
                .passed = Some(jump.version.clone());
            if inner
                .hop_holds
                .get(&jump.id)
                .is_some_and(|h| h.version == jump.version)
            {
                inner.hop_holds.remove(&jump.id);
            }
            // A host held at the jump host as it connects now follows as well; one turned away
            // after the jump host changed stays held.
            for entry in inner.hosts.values_mut() {
                if entry
                    .hold
                    .as_ref()
                    .is_some_and(|h| h.hop.as_ref() == Some(jump))
                {
                    entry.hold = None;
                }
            }
        }
    }

    /// Waits until the host may try its jump hosts: through one that has not let a connection
    /// through as it connects now, one connection at a time goes, so its rejected password is
    /// not sent once per host behind it before the first rejection is known. The permits are
    /// held until the attempt's outcome is recorded.
    async fn through(&self, jumps: &[JumpVersion]) -> Option<Vec<OwnedSemaphorePermit>> {
        let mut permits = Vec::new();
        // In one order for every host, so two hosts whose chains were edited in between never
        // wait on each other's gates.
        let mut order: Vec<&JumpVersion> = jumps.iter().collect();
        order.sort_by(|a, b| a.id.cmp(&b.id));
        order.dedup_by(|a, b| a.id == b.id);
        for jump in order {
            let gate = {
                let mut inner = lock(&self.inner);
                let entry = inner
                    .hop_gates
                    .entry(jump.id.clone())
                    .or_insert_with(HopGate::new);
                if entry.passed.as_deref() == Some(jump.version.as_str()) {
                    continue;
                }
                Arc::clone(&entry.gate)
            };
            let permit = gate.acquire_owned().await.ok()?;
            // The connection before this one may have got through.
            let passed = lock(&self.inner)
                .hop_gates
                .get(&jump.id)
                .and_then(|g| g.passed.as_deref())
                == Some(jump.version.as_str());
            if !passed {
                permits.push(permit);
            }
        }
        Some(permits)
    }

    /// The hold of a jump host the host goes through, while the jump host connects as it did
    /// when it was held, unless the host is the one whose **Retry** tries it again.
    fn jump_held(
        &self,
        host_id: &str,
        jumps: &[JumpVersion],
    ) -> Option<(DashboardHold, Option<AppError>)> {
        let mut inner = lock(&self.inner);
        let mut found = None;
        for jump in jumps {
            match inner.hop_holds.get(&jump.id) {
                Some(h) if h.version == jump.version => {
                    if h.retry_by.as_deref() == Some(host_id) {
                        continue;
                    }
                    found = Some((jump.id.clone(), h.hold, h.error.clone()));
                    break;
                }
                Some(_) => {
                    inner.hop_holds.remove(&jump.id);
                }
                None => {}
            }
        }
        let entry = inner.hosts.entry(host_id.to_owned()).or_default();
        entry.blocked_by = found.as_ref().map(|(id, ..)| id.clone());
        found.map(|(_, hold, error)| (hold, error))
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

    /// Whether `target`, the first hop of `host_id`, answers the HOST-10 probe, asked once per
    /// [`PROBE_FRESH`] for every host it is the first hop of, and again after **Retry**.
    async fn probe(&self, host_id: &str, target: &ProbeTarget) -> bool {
        let probe = {
            let mut inner = lock(&self.inner);
            let reprobe = inner
                .hosts
                .get_mut(host_id)
                .is_some_and(|e| std::mem::take(&mut e.reprobe));
            let key = target.key();
            match inner.probes.get(&key) {
                Some(p) if !reprobe && p.at.elapsed() < PROBE_FRESH => Arc::clone(p),
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
    /// Each jump host, outwards, with its own part of the two.
    jumps: Vec<JumpVersion>,
}

/// A jump host of a chain, and how it connects: what its shared hold is valid for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct JumpVersion {
    id: String,
    version: String,
}

impl Versions {
    /// Only what the connection is made from counts, so starring a host or a sync pull that
    /// changes its notes neither ends a hold nor closes a connection.
    fn of(vault: &Vault, host: &Host) -> Self {
        let hops = chain(vault, host);
        let mut config = Vec::new();
        let mut trust = Vec::new();
        let mut jumps: Vec<(usize, JumpVersion)> = Vec::new();
        for (i, (id, hop)) in hops.iter().enumerate() {
            let hop_config = hop_config(vault, hop, i + 1 == hops.len());
            let mut keys: Vec<String> = known_host_keys(vault, &hop.address, hop.port)
                .into_iter()
                .map(|(_, k)| k.fingerprint)
                .collect();
            keys.sort();
            let hop_trust = keys.join(",");
            if let Some(id) = id {
                jumps.push((
                    i,
                    JumpVersion {
                        id: (*id).to_owned(),
                        version: String::new(),
                    },
                ));
            }
            config.push(hop_config);
            trust.push(hop_trust);
        }
        // A jump host connects through the hops outwards of it as well.
        let jumps = jumps
            .into_iter()
            .map(|(i, jump)| JumpVersion {
                version: format!("{}|{}", config[i..].join(";"), trust[i..].join(";")),
                ..jump
            })
            .collect();
        Self {
            config: config.join(";"),
            trust: trust.join(";"),
            jumps,
        }
    }

    fn all(&self) -> String {
        format!("{}|{}", self.config, self.trust)
    }

    /// [`Versions::config`] alone, without reading the trusted keys of every hop.
    fn config_of(vault: &Vault, host: &Host) -> String {
        let hops = chain(vault, host);
        hops.iter()
            .enumerate()
            .map(|(i, (_, hop))| hop_config(vault, hop, i + 1 == hops.len()))
            .collect::<Vec<_>>()
            .join(";")
    }
}

/// How one hop of a chain connects: its target, its credentials (hashed) and, on the first
/// hop (`outermost`), the proxy.
fn hop_config(vault: &Vault, hop: &Host, outermost: bool) -> String {
    let auth = match &hop.auth {
        HostAuth::Password { password } => {
            format!("password#{:x}", secret_hash(&[password]))
        }
        HostAuth::Key { key_id } => {
            let key = vault.get(key_id).and_then(Item::as_key).map(|k| {
                let passphrase = k.passphrase.as_deref().map_or("", |p| p.as_str());
                secret_hash(&[&k.private_key, passphrase])
            });
            format!("key:{key_id}#{:x}", key.unwrap_or(0))
        }
        HostAuth::Agent => "agent".to_owned(),
        HostAuth::Ask => "ask".to_owned(),
    };
    // The proxy only matters on the first hop, the outermost one.
    let proxy = if outermost {
        match crate::ssh::resolve_proxy(vault, &hop.proxy) {
            Ok(Some(p)) => format!(
                "{:?} {}@{}:{}#{:x}",
                p.kind,
                p.username,
                p.host,
                p.port,
                secret_hash(&[&p.password])
            ),
            Ok(None) => "direct".to_owned(),
            Err(_) => "missing".to_owned(),
        }
    } else {
        String::new()
    };
    format!(
        "{}@{}:{} {auth} {proxy}",
        hop.username, hop.address, hop.port
    )
}

/// Mixed into every hash of secrets, and replaced at the lock: a hash made before it, which
/// may linger in memory, can then no longer be checked against guesses. 0 until first used.
static SALT: AtomicU64 = AtomicU64::new(0);

/// A salt from the system's random source, never 0.
fn new_salt() -> u64 {
    let random = uuid::Uuid::new_v4().as_u128();
    ((random as u64) ^ ((random >> 64) as u64)).max(1)
}

/// The lock (SEC-03): hashes of secrets start over, so none made while the vault was unlocked
/// stays useful after it locks. Versions made before it no longer match any made after it.
pub fn forget_secret_hashes() {
    SALT.store(new_salt(), Ordering::Relaxed);
}

/// A hash of secrets, so a version can tell a changed password from the old one without
/// holding either, or copying it.
fn secret_hash(parts: &[&str]) -> u64 {
    let salt = match SALT.load(Ordering::Relaxed) {
        0 => {
            let fresh = new_salt();
            SALT.compare_exchange(0, fresh, Ordering::Relaxed, Ordering::Relaxed)
                .map_or_else(|set| set, |_| fresh)
        }
        salt => salt,
    };
    salted_hash(salt, parts)
}

fn salted_hash(salt: u64, parts: &[&str]) -> u64 {
    static KEY: std::sync::OnceLock<std::collections::hash_map::RandomState> =
        std::sync::OnceLock::new();
    let mut hasher = KEY.get_or_init(Default::default).build_hasher();
    salt.hash(&mut hasher);
    for part in parts {
        part.hash(&mut hasher);
    }
    hasher.finish()
}

/// The host, then its jump hosts outwards with their ids, as far as the chain goes without
/// looping.
fn chain<'a>(vault: &'a Vault, host: &'a Host) -> Vec<(Option<&'a str>, &'a Host)> {
    let mut hops = vec![(None, host)];
    let mut seen = HashSet::new();
    let mut next = host.jump_host_id.as_deref();
    while let Some(id) = next.filter(|id| seen.insert(*id) && seen.len() <= 8) {
        let Some(jump) = vault.get(id).and_then(Item::as_host) else {
            break;
        };
        hops.push((Some(id), jump));
        next = jump.jump_host_id.as_deref();
    }
    hops
}

/// The host, then its jump hosts outwards.
fn hops<'a>(vault: &'a Vault, host: &'a Host) -> Vec<&'a Host> {
    chain(vault, host).into_iter().map(|(_, h)| h).collect()
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
    /// The proxy's endpoint counts, not its credentials: two saved proxies on one endpoint with
    /// different credentials practically never happen.
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
}

/// `None` once the host is gone.
fn plan(vault: &Vault, host_id: &str) -> Option<Plan> {
    let host = vault.get(host_id).and_then(Item::as_host)?;
    let versions = Versions::of(vault, host);
    let hops = hops(vault, host);
    let connect = if hops.iter().any(|h| asks_password(h)) {
        Err(Stop::Hold(DashboardHold::Password))
    } else {
        build_config(vault, host, Some(host_id), None).map_err(Stop::Failed)
    };
    // The first hop, the outermost one: where the TCP connection goes.
    let first = hops.last().copied().unwrap_or(host);
    let probe = crate::ssh::resolve_proxy(vault, &first.proxy).map(|proxy| ProbeTarget {
        host: first.address.clone(),
        port: first.port,
        proxy,
    });
    Some(Plan {
        versions,
        connect,
        probe,
    })
}

/// The hops (`hops` is the host, then its jump hosts outwards) a connection got past before it
/// failed at hop `hop`, counted from the outermost one.
fn passed(hops: &[JumpVersion], hop: Option<usize>) -> &[JumpVersion] {
    match hop {
        Some(at) => &hops[hops.len() - at.min(hops.len())..],
        None => &[],
    }
}

/// The hop at place `hop`, counted from the outermost one.
fn jump_at(hops: &[JumpVersion], hop: Option<usize>) -> Option<&JumpVersion> {
    let at = hop?;
    (at < hops.len()).then(|| &hops[hops.len() - 1 - at])
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
        // The saved password answers the first lone password prompt itself, so another one
        // means it was rejected.
        return Some(if outcome.password_prompted() {
            DashboardHold::SignInFailed
        } else {
            DashboardHold::Prompt
        });
    }
    if err.ssh_kind == Some(SshErrorKind::KeyParse)
        || matches!(
            err.key_kind,
            Some(KeyParseErrorKind::PassphraseRequired | KeyParseErrorKind::WrongPassphrase)
        )
    {
        return Some(DashboardHold::Passphrase);
    }
    match err.ssh_kind {
        Some(SshErrorKind::AuthFailed) => Some(DashboardHold::SignInFailed),
        // A server that hangs up once sign-in began (too many keys from the agent, or one
        // rejected password on a device that allows one) failed the sign-in as much as one
        // that says no, and tools like fail2ban count it the same.
        Some(SshErrorKind::Disconnected | SshErrorKind::Protocol | SshErrorKind::Other)
            if outcome.signing_in() =>
        {
            Some(DashboardHold::SignInFailed)
        }
        _ => None,
    }
}

/// What the reader does after a step: read over a connection, or wait (`None`: not at all).
enum Next {
    Read { session: SshSession, tab: bool },
    Wait(Option<Duration>),
}

/// Reads one host until it is cancelled, the vault locks or the host is deleted, then lets its
/// connection linger.
async fn read_host(
    app: AppHandle,
    host_id: String,
    reader_id: u64,
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
            next = connect_step(&app, &host_id, plan, &cancel) => next,
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
    state.dashboard.reader_ended(&host_id, reader_id);
}

/// Finds or makes a connection to read the host over.
async fn connect_step(
    app: &AppHandle,
    host_id: &str,
    plan: Plan,
    cancel: &CancellationToken,
) -> Next {
    let state = state(app);
    let dash = &state.dashboard;
    let host = host_id.to_owned();
    if dash.settings_changed(host_id, &plan.versions.config) {
        dash.emit(DashboardEvent::Reset {
            host_id: host.clone(),
        });
    }
    let tab = state
        .connections
        .session_for(host_id, &plan.versions.config);
    // Known only from a connection made for the host as it is now: the system HOST-11 recorded
    // may be that of a server the host no longer names, or one since replaced.
    if let Some(system) = dash.unsupported(host_id, &plan.versions.config) {
        // Online status only, as in the host list; nothing to read, even over a tab.
        let online = match (&tab, &plan.probe) {
            (Some(_), _) => true,
            (None, Ok(target)) => dash.probe(host_id, target).await,
            (None, Err(_)) => false,
        };
        dash.emit(DashboardEvent::Probe {
            host_id: host.clone(),
            online,
        });
        // An offline host shows as offline.
        if online {
            dash.emit(DashboardEvent::Unsupported {
                host_id: host,
                system,
            });
        }
        return Next::Wait(Some(RECHECK));
    }
    if let Some(session) = tab {
        dash.drop_own(host_id);
        return Next::Read { session, tab: true };
    }
    if let Some(session) = dash.own(host_id, &plan.versions.config) {
        return Next::Read {
            session,
            tab: false,
        };
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
    // An offline host shows as offline, whatever else would keep it from signing in. A dead
    // host would also hold a slot until its connection timed out.
    let online = dash.probe(host_id, &probe).await;
    dash.emit(DashboardEvent::Probe {
        host_id: host.clone(),
        online,
    });
    if !online {
        return Next::Wait(Some(RECHECK));
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
    // The host's hops: the host itself, which other hosts may be reached through, then its jump
    // hosts outwards. Its version as a jump host is that of its own chain.
    let hops: Vec<JumpVersion> = std::iter::once(JumpVersion {
        id: host_id.to_owned(),
        version: version.clone(),
    })
    .chain(plan.versions.jumps.iter().cloned())
    .collect();
    // It, or a jump host it goes through, failed for another host: not tried again from here.
    if let Some((hold, error)) = dash.jump_held(host_id, &hops) {
        dash.emit(DashboardEvent::Held {
            host_id: host,
            hold,
            error,
        });
        return Next::Wait(Some(RECHECK));
    }
    let cfg = match plan.connect {
        Ok(cfg) => cfg,
        Err(Stop::Hold(hold)) => {
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
    dash.emit(DashboardEvent::Connecting {
        host_id: host.clone(),
    });
    let Some(gates) = dash.through(&hops).await else {
        return Next::Wait(None);
    };
    let Ok(permit) = Arc::clone(&dash.setups).acquire_owned().await else {
        return Next::Wait(None);
    };
    // Another host behind the same jump host may have been turned away while this one waited.
    if let Some((hold, error)) = dash.jump_held(host_id, &hops) {
        drop(permit);
        dash.emit(DashboardEvent::Held {
            host_id: host,
            hold,
            error,
        });
        return Next::Wait(Some(RECHECK));
    }
    // The connection's hops are those of the plan's chain, unless the vault changed in between.
    let same_chain = cfg.jump.len() + 1 == hops.len();
    let (result, outcome) = connect_unattended(app, cfg).await;
    let hop = outcome.hop().filter(|_| same_chain);
    drop(permit);
    let session = match result {
        Ok(session) => {
            dash.passed(&hops);
            session
        }
        Err(error) => {
            dash.passed(passed(&hops, hop));
            match hold_for(&error, &outcome) {
                Some(hold) => {
                    // At a jump host, every host behind it waits with this one.
                    let jump = jump_at(&hops, hop);
                    dash.hold(host_id, hold, Some(error.clone()), version, jump);
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
    // The outcome is recorded: the next host behind the jump hosts may try them.
    drop(gates);
    // A tab may have connected while this one did.
    if let Some(tab) = state
        .connections
        .session_for(host_id, &plan.versions.config)
    {
        close(session);
        return Next::Read {
            session: tab,
            tab: true,
        };
    }
    if !dash.keep(host_id, session.clone(), plan.versions.config, cancel) {
        return Next::Wait(None);
    }
    let os = session.server_os().map(hatoba_ssh::ServerOs::as_str);
    if let Err(e) = state.with_unlocked(|v| Ok(v.set_host_os(host_id, os)?)) {
        tracing::debug!("server OS not recorded: {}", e.detail);
    }
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
    mut tab: bool,
    interval: Duration,
    config: String,
) -> Option<Duration> {
    let state = state(app);
    let dash = &state.dashboard;
    let host = host_id.to_owned();
    let (handle, mut events) = match session.open_dashboard_stats(interval).await {
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
    dash.reading_with(host_id, &config);
    dash.emit(DashboardEvent::Reading {
        host_id: host.clone(),
        os: session
            .server_os()
            .map(|os| hatoba_ssh::ServerOs::as_str(os).to_owned()),
        tab,
    });
    let mut checked = tokio::time::Instant::now();
    let wait = loop {
        match events.recv().await {
            Some(hatoba_ssh::StatsEvent::Stats(stats)) => {
                dash.emit(DashboardEvent::Stats {
                    host_id: host.clone(),
                    stats: Box::new(stats_view(*stats)),
                });
                // A tab took the connection over: it is the tab's from now on.
                if !tab && !dash.owns(host_id) {
                    tab = true;
                    dash.emit(DashboardEvent::Reading {
                        host_id: host.clone(),
                        os: session
                            .server_os()
                            .map(|os| hatoba_ssh::ServerOs::as_str(os).to_owned()),
                        tab,
                    });
                }
                // A connection serves the host as it was when it connected: after an edit or a
                // sync pull, the reader connects again (a tab keeps its connection).
                if checked.elapsed() >= CONFIG_CHECK {
                    checked = tokio::time::Instant::now();
                    let current = state.with_unlocked(|v| {
                        Ok(v.get(host_id)
                            .and_then(Item::as_host)
                            .map(|h| Versions::config_of(v, h)))
                    });
                    if !matches!(current, Ok(Some(c)) if c == config) {
                        break Some(Duration::ZERO);
                    }
                }
            }
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
    Versions::config_of(vault, host)
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
    fn a_server_without_proc_is_known_for_the_settings_it_was_seen_with() {
        let mut v = vault();
        let id = v.put(None, host("bsd", password("pw"), None)).unwrap();
        let dash = Dashboard::default();
        let before = plan_of(&v, &id).versions.config;
        dash.set_unsupported(&id, before.clone(), "FreeBSD".into());
        assert_eq!(dash.unsupported(&id, &before).as_deref(), Some("FreeBSD"));
        // The system HOST-11 recorded does not stop a sign-in.
        v.set_host_os(&id, Some("freebsd")).unwrap();
        assert_eq!(plan_of(&v, &id).versions.config, before);
        // Pointed at another server, the host is signed in to again.
        let mut edited = get(&v, &id);
        edited.address = "linux.example".to_owned();
        v.put(Some(&id), Item::Host(edited)).unwrap();
        let after = plan_of(&v, &id).versions.config;
        assert_eq!(dash.unsupported(&id, &after), None);
        // The lock forgets it.
        dash.set_unsupported(&id, after.clone(), "FreeBSD".into());
        dash.stop_all();
        assert_eq!(dash.unsupported(&id, &after), None);
    }

    #[test]
    fn the_lock_starts_hashes_of_secrets_over_and_ends_the_holds() {
        // Another salt, another hash: one made before the lock matches none made after it.
        assert_eq!(salted_hash(7, &["pw"]), salted_hash(7, &["pw"]));
        assert_ne!(salted_hash(7, &["pw"]), salted_hash(8, &["pw"]));
        let (a, b) = (new_salt(), new_salt());
        assert!(a != 0 && b != 0 && a != b);
        // The holds made with them go at the lock.
        let dash = Dashboard::default();
        let bastion = JumpVersion {
            id: "bastion".to_owned(),
            version: "v".to_owned(),
        };
        dash.hold(
            "app",
            DashboardHold::SignInFailed,
            None,
            "app v".to_owned(),
            Some(&bastion),
        );
        dash.stop_all();
        assert!(dash.held("app", "app v").is_none());
        assert!(
            dash.jump_held("other", std::slice::from_ref(&bastion))
                .is_none()
        );
    }

    #[test]
    fn readings_made_with_other_settings_are_dropped_once() {
        let dash = Dashboard::default();
        assert!(!dash.settings_changed("h", "a"));
        dash.reading_with("h", "a");
        assert!(!dash.settings_changed("h", "a"));
        assert!(dash.settings_changed("h", "b"));
        // Dropped: nothing more to drop until the host is read again.
        assert!(!dash.settings_changed("h", "b"));
        assert!(!dash.settings_changed("h", "c"));
        dash.reading_with("h", "b");
        dash.stop_all();
        assert!(!dash.settings_changed("h", "c"));
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
        assert_eq!(config_version(&v, &get(&v, &id)), trusted.config);
        assert_ne!(trusted.trust, before.trust);
        assert!(trusted.trust.contains("SHA256:a,SHA256:b"));

        // Starring or renaming a host changes nothing a connection is made from.
        let mut starred = get(&v, &id);
        starred.favorite = true;
        starred.name = "renamed".to_owned();
        v.put(Some(&id), Item::Host(starred)).unwrap();
        assert_eq!(Versions::of(&v, &get(&v, &id)), trusted);

        // A new password does, as does editing the jump host.
        let mut repassed = get(&v, &id);
        repassed.auth = password("new");
        v.put(Some(&id), Item::Host(repassed)).unwrap();
        let repassed = Versions::of(&v, &get(&v, &id));
        assert_ne!(repassed.config, trusted.config);
        assert!(!repassed.config.contains("new"));
        let mut jump = get(&v, &bastion);
        jump.port = 2222;
        v.put(Some(&bastion), Item::Host(jump)).unwrap();
        let edited = Versions::of(&v, &get(&v, &id));
        assert_ne!(edited.config, repassed.config);
        assert_eq!(config_version(&v, &get(&v, &id)), edited.config);
        assert_ne!(edited.all(), repassed.all());
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
        let ssh = |kind| AppError::ssh(kind, "detail");
        assert_eq!(
            hold_for(&ssh(SshErrorKind::AuthFailed), &quiet),
            Some(DashboardHold::SignInFailed)
        );
        assert_eq!(hold_for(&ssh(SshErrorKind::Timeout), &quiet), None);
        // An encrypted key without its passphrase fails as the connection reports it.
        assert_eq!(
            hold_for(&ssh(SshErrorKind::KeyParse), &quiet),
            Some(DashboardHold::Passphrase)
        );
        let mut passphrase = AppError::new(crate::error::ErrorCode::KeyParse, "encrypted");
        passphrase.key_kind = Some(KeyParseErrorKind::PassphraseRequired);
        assert_eq!(
            hold_for(&passphrase, &quiet),
            Some(DashboardHold::Passphrase)
        );
    }

    #[test]
    fn a_server_that_hangs_up_during_sign_in_failed_it() {
        let lost = AppError::ssh(SshErrorKind::Disconnected, "connection lost");
        // Before any host key checked out, a lost connection is the network's.
        assert_eq!(hold_for(&lost, &Unattended::default()), None);
        let signing_in = Unattended::for_tests(true, false, false);
        assert_eq!(
            hold_for(&lost, &signing_in),
            Some(DashboardHold::SignInFailed)
        );
        let timeout = AppError::ssh(SshErrorKind::Timeout, "timed out");
        assert_eq!(hold_for(&timeout, &signing_in), None);
        let cancelled = AppError::ssh(SshErrorKind::Cancelled, "cancelled");
        // A verification code, and a password asked again because the saved one was wrong.
        assert_eq!(
            hold_for(&cancelled, &Unattended::for_tests(true, true, false)),
            Some(DashboardHold::Prompt)
        );
        assert_eq!(
            hold_for(&cancelled, &Unattended::for_tests(true, true, true)),
            Some(DashboardHold::SignInFailed)
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
        let via = target("bastion", Some(proxy.clone())).key();
        assert_ne!(via, target("bastion", None).key());
        assert!(!via.contains("secret"));
    }

    #[test]
    fn a_jump_host_that_turned_one_host_away_holds_every_host_behind_it() {
        let dash = Dashboard::default();
        let bastion = JumpVersion {
            id: "bastion".to_owned(),
            version: "v1".to_owned(),
        };
        let rejected = AppError::ssh(SshErrorKind::AuthFailed, "denied");
        dash.hold(
            "app-1",
            DashboardHold::SignInFailed,
            Some(rejected),
            "app-1 v".to_owned(),
            Some(&bastion),
        );
        // Another host behind it waits without trying the rejected credentials.
        let held = dash.jump_held("app-2", std::slice::from_ref(&bastion));
        assert_eq!(held.map(|(h, _)| h), Some(DashboardHold::SignInFailed));
        // A host that goes through other jump hosts does not.
        let other = JumpVersion {
            id: "other".to_owned(),
            ..bastion.clone()
        };
        assert!(
            dash.jump_held("app-3", std::slice::from_ref(&other))
                .is_none()
        );

        // Retry on any of them lets that one try the jump host again, and only that one.
        dash.jump_held("app-2", std::slice::from_ref(&bastion));
        dash.retry("app-2");
        assert!(
            dash.jump_held("app-2", std::slice::from_ref(&bastion))
                .is_none()
        );
        assert!(
            dash.jump_held("app-4", std::slice::from_ref(&bastion))
                .is_some()
        );
        // Once it got past the jump host, the others follow, the one turned away first included.
        assert!(dash.held("app-1", "app-1 v").is_some());
        dash.passed(std::slice::from_ref(&bastion));
        assert!(
            dash.jump_held("app-4", std::slice::from_ref(&bastion))
                .is_none()
        );
        assert!(dash.held("app-1", "app-1 v").is_none());
        // One turned away after the jump host changed stays held when a connection made with
        // its old settings gets past it.
        let changed = JumpVersion {
            version: "v-new".to_owned(),
            ..bastion.clone()
        };
        dash.hold(
            "app-5",
            DashboardHold::SignInFailed,
            None,
            "app-5 v".to_owned(),
            Some(&changed),
        );
        dash.passed(std::slice::from_ref(&bastion));
        assert!(dash.held("app-5", "app-5 v").is_some());

        // So does a change to how the jump host connects.
        dash.hold(
            "app-1",
            DashboardHold::SignInFailed,
            None,
            "app-1 v".to_owned(),
            Some(&bastion),
        );
        let edited = JumpVersion {
            version: "v2".to_owned(),
            ..bastion.clone()
        };
        assert!(dash.jump_held("app-2", &[edited]).is_none());
        assert!(dash.jump_held("app-2", &[bastion]).is_none());
    }

    #[test]
    fn each_jump_host_has_a_version_of_its_own() {
        let mut v = vault();
        let bastion = v.put(None, host("bastion", password("pw"), None)).unwrap();
        let id = v
            .put(None, host("app", password("pw"), Some(&bastion)))
            .unwrap();
        let before = Versions::of(&v, &get(&v, &id));
        assert_eq!(before.jumps.len(), 1);
        assert_eq!(before.jumps[0].id, bastion);
        // The jump host's own connection shares its gate and holds with the hosts behind it.
        assert_eq!(
            Versions::of(&v, &get(&v, &bastion)).all(),
            before.jumps[0].version
        );
        // The destination's own password does not change the jump host's version.
        let mut app = get(&v, &id);
        app.auth = password("new");
        v.put(Some(&id), Item::Host(app)).unwrap();
        let after = Versions::of(&v, &get(&v, &id));
        assert_eq!(after.jumps, before.jumps);
        let mut jump = get(&v, &bastion);
        jump.auth = password("new");
        v.put(Some(&bastion), Item::Host(jump)).unwrap();
        let repassed = Versions::of(&v, &get(&v, &id)).jumps;
        assert_ne!(repassed, before.jumps);
        // So does a change to the hops outwards of the jump host.
        let outer = v.put(None, host("outer", password("pw"), None)).unwrap();
        let mut jump = get(&v, &bastion);
        jump.jump_host_id = Some(outer);
        v.put(Some(&bastion), Item::Host(jump)).unwrap();
        assert_ne!(Versions::of(&v, &get(&v, &id)).jumps[0], repassed[0]);
    }

    #[tokio::test]
    async fn hosts_behind_a_jump_host_try_it_one_at_a_time_until_one_got_through() {
        let dash = Dashboard::default();
        let bastion = JumpVersion {
            id: "bastion".to_owned(),
            version: "v1".to_owned(),
        };
        let jumps = std::slice::from_ref(&bastion);
        let first = dash.through(jumps).await.unwrap();
        assert_eq!(first.len(), 1);
        // The next host waits for the first one's outcome.
        let second = dash.through(jumps);
        tokio::pin!(second);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut second)
                .await
                .is_err()
        );
        // It got through, so the others no longer wait for each other.
        dash.passed(jumps);
        drop(first);
        assert!(second.await.unwrap().is_empty());
        assert!(dash.through(jumps).await.unwrap().is_empty());
        // Turned away later, or connecting another way, the jump host is tried by one again.
        dash.hold(
            "app",
            DashboardHold::SignInFailed,
            None,
            "app v".to_owned(),
            Some(&bastion),
        );
        assert_eq!(dash.through(jumps).await.unwrap().len(), 1);
        dash.passed(jumps);
        let edited = JumpVersion {
            version: "v2".to_owned(),
            ..bastion.clone()
        };
        assert_eq!(dash.through(&[edited]).await.unwrap().len(), 1);
    }

    #[test]
    fn a_connection_that_failed_further_in_got_past_the_jump_hosts_outwards() {
        let jump = |id: &str| JumpVersion {
            id: id.to_owned(),
            version: "v".to_owned(),
        };
        // Outwards: the host, the inner jump host, then the outer one, which is connected to
        // first.
        let hops = [jump("app"), jump("inner"), jump("outer")];
        let ids = |hop: Option<usize>| -> Vec<String> {
            passed(&hops, hop).iter().map(|j| j.id.clone()).collect()
        };
        assert!(ids(None).is_empty());
        assert!(ids(Some(0)).is_empty());
        assert_eq!(ids(Some(1)), ["outer"]);
        assert_eq!(ids(Some(2)), ["inner", "outer"]);
        // Hops are told apart by place, so two on one address and port are not confused.
        let at = |hop: Option<usize>| jump_at(&hops, hop).map(|j| j.id.as_str());
        assert_eq!(at(Some(0)), Some("outer"));
        assert_eq!(at(Some(1)), Some("inner"));
        assert_eq!(at(Some(2)), Some("app"));
        assert_eq!(at(None), None);
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
