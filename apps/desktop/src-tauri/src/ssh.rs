//! Live SSH sessions, host-key verification (SSH-04) and interactive prompts (SSH-08).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use hatoba_core::model::{Host, HostAuth, HostProxy, Item, KnownHost, ProxyKind as CoreProxyKind};
use hatoba_core::vault::Vault;
use hatoba_ssh::{
    AuthMethod, ConnectConfig, ForwardHandle, HostKeyInfo, HostKeyVerifier, JumpHop,
    KeyboardInteractive, PromptRequest, ProxyConfig, ProxyKind, SftpClient, ShellHandle,
    SshSession, StatsHandle,
};
use tauri::AppHandle;
use tauri_specta::Event;
use tokio::sync::{OnceCell, oneshot};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::dto::{AuthPrompt, AuthPromptField, HostKeyPrompt, HostKeyPromptKind};
use crate::error::{AppError, AppResult, SshErrorKind};
use crate::state::{AppState, now_ms, state};
use crate::sync;

/// How long a host-key or 2FA prompt may stay unanswered before the connection gives up.
const PROMPT_TIMEOUT: Duration = Duration::from_secs(600);

pub struct LiveSession {
    pub session: SshSession,
    pub shell: ShellHandle,
    /// The saved host the terminal connected to; `None` for a quick connection (HOST-12).
    pub host_id: Option<String>,
    /// What the connection was made from (`dashboard::config_version`), so that the dashboard
    /// reads a host over it only while the host still connects the same way (HOST-14).
    pub config: Option<String>,
    pub sftp: OnceCell<SftpClient>,
    /// Running local port forwards by forward item id (FWD-01).
    pub forwards: Mutex<HashMap<String, ForwardHandle>>,
    /// Resource usage sampling while the terminal shows it (TERM-12), with the id of its start.
    stats: Mutex<Option<(u64, StatsHandle)>>,
}

impl LiveSession {
    pub fn new(
        session: SshSession,
        shell: ShellHandle,
        host_id: Option<String>,
        config: Option<String>,
    ) -> Self {
        Self {
            session,
            shell,
            host_id,
            config,
            sftp: OnceCell::new(),
            forwards: Mutex::new(HashMap::new()),
            stats: Mutex::new(None),
        }
    }

    /// Keeps the sampling started as `id` in place of an earlier one, which stops. Starts can
    /// finish out of order: one that a later start already replaced is dropped, and so stops.
    pub fn set_stats(&self, id: u64, handle: StatsHandle) {
        let mut stats = lock(&self.stats);
        if stats.as_ref().is_none_or(|(current, _)| *current < id) {
            *stats = Some((id, handle));
        }
    }

    /// Stops the sampling started as `id`, unless a later one replaced it.
    pub fn stop_stats(&self, id: u64) {
        let mut stats = lock(&self.stats);
        if stats.as_ref().is_some_and(|(current, _)| *current == id) {
            *stats = None;
        }
    }

    pub fn add_forward(&self, id: &str, handle: ForwardHandle) {
        if let Some(old) = lock(&self.forwards).insert(id.to_owned(), handle) {
            old.stop();
        }
    }

    pub fn stop_forward(&self, id: &str) {
        if let Some(handle) = lock(&self.forwards).remove(id) {
            handle.stop();
        }
    }

    pub fn forward_port(&self, id: &str) -> Option<u16> {
        lock(&self.forwards)
            .get(id)
            .filter(|h| h.is_running())
            .map(ForwardHandle::local_port)
    }

    pub fn active_forwards(&self) -> Vec<(String, u16)> {
        lock(&self.forwards)
            .iter()
            .filter(|(_, h)| h.is_running())
            .map(|(id, h)| (id.clone(), h.local_port()))
            .collect()
    }

    /// Stops forwards and sampling, and closes the shell and connection.
    pub async fn close(&self) {
        for (_, handle) in lock(&self.forwards).drain() {
            handle.stop();
        }
        lock(&self.stats).take();
        self.shell.close().await;
        self.session.disconnect().await;
    }
}

#[derive(Default)]
pub struct SshManager {
    sessions: Mutex<HashMap<String, Arc<LiveSession>>>,
    hostkey_waiters: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    auth_waiters: Mutex<HashMap<String, oneshot::Sender<Option<Vec<String>>>>>,
    transfers: Mutex<HashMap<String, CancellationToken>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl SshManager {
    pub fn insert(&self, id: String, session: Arc<LiveSession>) {
        lock(&self.sessions).insert(id, session);
    }

    pub fn get(&self, id: &str) -> AppResult<Arc<LiveSession>> {
        lock(&self.sessions)
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::not_found("session"))
    }

    /// Removes the session only if it is still the given instance (a reconnect may have replaced
    /// it).
    pub fn remove_if_same(&self, id: &str, live: &Arc<LiveSession>) {
        let mut sessions = lock(&self.sessions);
        if sessions.get(id).is_some_and(|s| Arc::ptr_eq(s, live)) {
            sessions.remove(id);
        }
    }

    /// The connection of a terminal open on the saved host and made the way the host connects
    /// now (`config`), which the dashboard reads over (HOST-14). The newest tab's, when several
    /// are open; a tab still connected to the address the host had before an edit is not one.
    pub fn connection_for_host(&self, host_id: &str, config: &str) -> Option<SshSession> {
        lock(&self.sessions)
            .iter()
            .filter(|(_, live)| {
                live.host_id.as_deref() == Some(host_id)
                    && live.config.as_deref() == Some(config)
                    && !live.session.is_closed()
            })
            // Session ids are UUIDv7, so the largest is the newest.
            .max_by(|(a, _), (b, _)| a.cmp(b))
            .map(|(_, live)| live.session.clone())
    }

    pub fn take(&self, id: &str) -> Option<Arc<LiveSession>> {
        lock(&self.sessions).remove(id)
    }

    /// SEC-03 "lock disconnects sessions": takes every session out, by id, for the lock to close.
    pub fn take_all(&self) -> Vec<(String, Arc<LiveSession>)> {
        lock(&self.sessions).drain().collect()
    }

    pub fn answer_hostkey(&self, request_id: &str, accept: bool) {
        if let Some(tx) = lock(&self.hostkey_waiters).remove(request_id) {
            let _ = tx.send(accept);
        }
    }

    pub fn answer_auth(&self, request_id: &str, answers: Option<Vec<String>>) {
        if let Some(tx) = lock(&self.auth_waiters).remove(request_id) {
            let _ = tx.send(answers);
        }
    }

    pub fn add_transfer(&self, id: String, token: CancellationToken) {
        lock(&self.transfers).insert(id, token);
    }

    pub fn finish_transfer(&self, id: &str) {
        lock(&self.transfers).remove(id);
    }

    pub fn cancel_transfer(&self, id: &str) {
        if let Some(token) = lock(&self.transfers).get(id) {
            token.cancel();
        }
    }
}

/// One-shot credentials for this connection only (SSH-03); never stored.
#[derive(Default)]
pub struct Overrides {
    pub password: Option<Zeroizing<String>>,
    pub passphrase: Option<Zeroizing<String>>,
}

fn auth_method(
    vault: &Vault,
    host: &Host,
    overrides: Option<&Overrides>,
    hop: bool,
) -> AppResult<AuthMethod> {
    Ok(match &host.auth {
        HostAuth::Password { password } if !password.is_empty() => {
            AuthMethod::Password(password.clone())
        }
        HostAuth::Password { .. } | HostAuth::Ask => {
            match overrides.and_then(|o| o.password.clone()) {
                Some(pw) if !hop => AuthMethod::Password(pw),
                _ if hop => {
                    return Err(AppError::invalid(
                        "jump_host_id",
                        format!("jump host {} needs a saved password or key", host.name),
                    ));
                }
                _ => {
                    return Err(AppError::invalid(
                        "password",
                        "a password is required for this host",
                    ));
                }
            }
        }
        HostAuth::Key { key_id } => {
            let key = vault.get(key_id).and_then(Item::as_key).ok_or_else(|| {
                AppError::invalid(
                    "key_id",
                    format!("the key used by {} no longer exists", host.name),
                )
            })?;
            let passphrase = overrides
                .and_then(|o| o.passphrase.clone())
                .filter(|_| !hop)
                .or_else(|| key.passphrase.clone());
            AuthMethod::PrivateKey {
                openssh: key.private_key.clone(),
                passphrase,
            }
        }
        HostAuth::Agent => AuthMethod::Agent,
    })
}

fn hop_target(host: &Host) -> (String, u16, String) {
    (host.address.clone(), host.port, host.username.clone())
}

/// The proxy a connection whose first hop chose `choice` goes through (SSH-13). A proxy that was
/// deleted fails the connection instead of connecting around it.
pub fn resolve_proxy(vault: &Vault, choice: &HostProxy) -> AppResult<Option<ProxyConfig>> {
    let (id, missing) = match choice {
        HostProxy::Direct => return Ok(None),
        HostProxy::Proxy { proxy_id } => (proxy_id.clone(), "the host's proxy was deleted"),
        HostProxy::DeviceDefault => match crate::commands::proxies::device_default(vault) {
            Some(id) => (id, "this device's default proxy was deleted"),
            None => return Ok(None),
        },
    };
    let proxy = vault
        .get(&id)
        .and_then(Item::as_proxy)
        .ok_or_else(|| AppError::ssh(SshErrorKind::ProxyMissing, missing))?;
    Ok(Some(ProxyConfig {
        kind: match proxy.kind {
            CoreProxyKind::Socks5 => ProxyKind::Socks5,
            CoreProxyKind::Http => ProxyKind::Http,
        },
        host: proxy.address.clone(),
        port: proxy.port,
        username: proxy.username.clone(),
        password: proxy.password.clone(),
    }))
}

/// The first hop of a connection to `host`: its outermost jump host, or the host itself without
/// one. A broken jump chain ends where it breaks.
pub fn first_hop<'a>(vault: &'a Vault, host: &'a Host) -> &'a Host {
    let mut first = host;
    let mut seen = HashSet::new();
    while let Some(jump) = first
        .jump_host_id
        .as_deref()
        .filter(|id| seen.insert(id.to_owned()) && seen.len() <= 8)
        .and_then(|id| vault.get(id).and_then(Item::as_host))
    {
        first = jump;
    }
    first
}

/// The proxy of a connection to `host`: the choice of its first hop.
pub fn connection_proxy(vault: &Vault, host: &Host) -> AppResult<Option<ProxyConfig>> {
    resolve_proxy(vault, &first_hop(vault, host).proxy)
}

/// Builds the connection config for `host`, resolving its ProxyJump chain (SSH-10) and the
/// proxy of the first hop (SSH-13).
pub fn build_config(
    vault: &Vault,
    host: &Host,
    self_id: Option<&str>,
    overrides: Option<&Overrides>,
) -> AppResult<ConnectConfig> {
    let mut chain: Vec<JumpHop> = Vec::new();
    let mut seen: HashSet<String> = self_id.into_iter().map(str::to_owned).collect();
    let mut next = host.jump_host_id.clone();
    // The first hop is the last jump host reached, or the host itself.
    let mut first_proxy = &host.proxy;
    while let Some(jump_id) = next {
        if !seen.insert(jump_id.clone()) || chain.len() >= 8 {
            return Err(AppError::invalid(
                "jump_host_id",
                "the jump host chain loops",
            ));
        }
        let jump = vault
            .get(&jump_id)
            .and_then(Item::as_host)
            .ok_or_else(|| AppError::invalid("jump_host_id", "jump host not found"))?;
        first_proxy = &jump.proxy;
        let (h, p, u) = hop_target(jump);
        chain.push(JumpHop {
            host: h,
            port: p,
            username: u,
            auth: auth_method(vault, jump, None, true)?,
        });
        next = jump.jump_host_id.clone();
    }
    chain.reverse(); // outermost hop first
    let (h, p, u) = hop_target(host);
    let mut cfg = ConnectConfig::new(h, p, u, auth_method(vault, host, overrides, false)?);
    cfg.jump = chain;
    cfg.proxy = resolve_proxy(vault, first_proxy)?;
    Ok(cfg)
}

/// TOFU host-key check backed by the synced `known_host` items (SSH-04).
pub struct Verifier {
    pub app: AppHandle,
    pub session_id: Option<String>,
}

pub(crate) fn known_host_keys(vault: &Vault, host: &str, port: u16) -> Vec<(String, KnownHost)> {
    vault
        .known_hosts()
        .into_iter()
        .filter(|(_, k)| k.host.eq_ignore_ascii_case(host) && k.port == port)
        .collect()
}

fn host_key_status<'a>(
    known: &'a [(String, KnownHost)],
    key: &HostKeyInfo,
) -> (HostKeyPromptKind, Option<&'a KnownHost>) {
    let same_type = known.iter().find(|(_, k)| k.key_type == key.key_type);
    let kind = if same_type.is_some() {
        HostKeyPromptKind::Changed
    } else {
        HostKeyPromptKind::New
    };
    let previous = same_type.or_else(|| known.first()).map(|(_, k)| k);
    (kind, previous)
}

fn save_host_key(vault: &mut Vault, host: &str, port: u16, key: &HostKeyInfo) -> AppResult<()> {
    for (id, _) in known_host_keys(vault, host, port)
        .into_iter()
        .filter(|(_, k)| k.key_type == key.key_type)
    {
        vault.delete(&id)?;
    }
    let now = now_ms();
    let record = KnownHost {
        host: host.to_owned(),
        port,
        key_type: key.key_type.clone(),
        public_key: key.public_key.clone(),
        fingerprint: key.fingerprint.clone(),
        first_seen_at: now,
        updated_at: 0,
    };
    vault.put(None, Item::KnownHost(record))?;
    Ok(())
}

#[async_trait]
impl HostKeyVerifier for Verifier {
    async fn known_key_types(&self, host: &str, port: u16) -> Vec<String> {
        let state = state(&self.app);
        let vault = state.vault();
        if !vault.is_unlocked() {
            return Vec::new();
        }
        known_host_keys(&vault, host, port)
            .into_iter()
            .map(|(_, k)| k.key_type)
            .collect()
    }

    async fn verify(&self, host: &str, port: u16, key: &HostKeyInfo) -> bool {
        let state = state(&self.app);
        let known: Vec<(String, KnownHost)> = {
            let vault = state.vault();
            if !vault.is_unlocked() {
                return false;
            }
            known_host_keys(&vault, host, port)
        };
        if known
            .iter()
            .any(|(_, k)| k.key_type == key.key_type && k.public_key == key.public_key)
        {
            return true;
        }
        // A known host presenting a different key of a saved type is blocked until the user
        // explicitly chooses "update fingerprint"; another type needs a new trust decision.
        let (kind, previous) = host_key_status(&known, key);
        let request_id = uuid::Uuid::now_v7().to_string();
        let (tx, rx) = oneshot::channel();
        lock(&state.ssh.hostkey_waiters).insert(request_id.clone(), tx);
        let prompt = HostKeyPrompt {
            request_id: request_id.clone(),
            session_id: self.session_id.clone(),
            host: host.to_owned(),
            port,
            key_type: key.key_type.clone(),
            fingerprint: key.fingerprint.clone(),
            kind,
            known_fingerprint: previous.map(|k| k.fingerprint.clone()),
            known_key_type: previous.map(|k| k.key_type.clone()),
        };
        if prompt.emit(&self.app).is_err() {
            lock(&state.ssh.hostkey_waiters).remove(&request_id);
            return false;
        }
        let accepted = matches!(tokio::time::timeout(PROMPT_TIMEOUT, rx).await, Ok(Ok(true)));
        lock(&state.ssh.hostkey_waiters).remove(&request_id);
        if accepted {
            let saved = state.with_unlocked(|v| save_host_key(v, host, port, key));
            match saved {
                Ok(()) => sync::local_change(&self.app),
                Err(e) => tracing::warn!("could not save the accepted host key: {}", e.detail),
            }
        }
        accepted
    }
}

/// keyboard-interactive / 2FA prompts relayed to the WebView (SSH-08).
pub struct Interactive {
    pub app: AppHandle,
    pub session_id: Option<String>,
}

#[async_trait]
impl KeyboardInteractive for Interactive {
    async fn respond(&self, request: PromptRequest) -> Option<Vec<Zeroizing<String>>> {
        let state = state(&self.app);
        let request_id = uuid::Uuid::now_v7().to_string();
        let (tx, rx) = oneshot::channel();
        lock(&state.ssh.auth_waiters).insert(request_id.clone(), tx);
        let event = AuthPrompt {
            request_id: request_id.clone(),
            session_id: self.session_id.clone(),
            name: if request.name.is_empty() {
                request.host.clone()
            } else {
                request.name.clone()
            },
            instructions: request.instructions.clone(),
            prompts: request
                .prompts
                .iter()
                .map(|p| AuthPromptField {
                    prompt: p.text.clone(),
                    echo: p.echo,
                })
                .collect(),
            password: request.password,
            target: target_label(&request.username, &request.host, request.port),
        };
        if event.emit(&self.app).is_err() {
            lock(&state.ssh.auth_waiters).remove(&request_id);
            return None;
        }
        let answers = tokio::time::timeout(PROMPT_TIMEOUT, rx)
            .await
            .ok()
            .and_then(Result::ok)
            .flatten();
        lock(&state.ssh.auth_waiters).remove(&request_id);
        let answers = answers?;
        (answers.len() == request.prompts.len())
            .then(|| answers.into_iter().map(Zeroizing::new).collect())
    }
}

/// What stopped a connection made without the user (HOST-14): the host key it could not check,
/// or a prompt the server wanted answered. Nothing is ever asked.
#[derive(Default)]
pub struct Unattended {
    host_key: Mutex<Option<HostKeyPromptKind>>,
    /// The hops connected to so far, the first one outermost.
    hops: AtomicUsize,
    /// That hop's host key was trusted, so signing in to it began.
    signing_in: AtomicBool,
    prompted: AtomicBool,
    /// One of the prompts was a lone password prompt at a hop that signs in with a saved
    /// password.
    password_prompted: AtomicBool,
    /// Whether each hop, the first one outermost, signs in with a saved password, which answers
    /// the first lone password prompt itself.
    saved_password: Vec<bool>,
}

impl Unattended {
    /// The host key was not trusted yet (`New`) or differs from the trusted one (`Changed`).
    pub fn host_key(&self) -> Option<HostKeyPromptKind> {
        *lock(&self.host_key)
    }

    /// The server asked for keyboard-interactive answers beyond the saved password (2FA).
    pub fn prompted(&self) -> bool {
        self.prompted.load(Ordering::Relaxed)
    }

    /// One of those was a lone password prompt: the saved password, which answers the first
    /// one itself, was rejected (a PAM server asks again).
    pub fn password_prompted(&self) -> bool {
        self.password_prompted.load(Ordering::Relaxed)
    }

    /// The last hop's host key checked out, so a failure after it may come from signing in to
    /// that hop. An earlier hop that signed in does not count.
    pub fn signing_in(&self) -> bool {
        self.signing_in.load(Ordering::Relaxed)
    }

    /// The hop a failure happened at, the last one the connection reached, counted from the
    /// first (outermost) one: past the jump hosts, the destination. `None` before any.
    pub fn hop(&self) -> Option<usize> {
        self.hops.load(Ordering::Relaxed).checked_sub(1)
    }

    #[cfg(test)]
    pub fn for_tests(signing_in: bool, prompted: bool, password_prompted: bool) -> Self {
        Self {
            host_key: Mutex::new(None),
            hops: AtomicUsize::new(0),
            signing_in: AtomicBool::new(signing_in),
            prompted: AtomicBool::new(prompted),
            password_prompted: AtomicBool::new(password_prompted),
            saved_password: Vec::new(),
        }
    }
}

/// Accepts only host keys already trusted (SSH-04) and records any other, without asking.
struct UnattendedVerifier {
    app: AppHandle,
    outcome: Arc<Unattended>,
}

#[async_trait]
impl HostKeyVerifier for UnattendedVerifier {
    /// Asked as each hop's connection begins, so it also marks the hop being connected to.
    async fn known_key_types(&self, host: &str, port: u16) -> Vec<String> {
        self.outcome.hops.fetch_add(1, Ordering::Relaxed);
        self.outcome.signing_in.store(false, Ordering::Relaxed);
        let state = state(&self.app);
        let vault = state.vault();
        if !vault.is_unlocked() {
            return Vec::new();
        }
        known_host_keys(&vault, host, port)
            .into_iter()
            .map(|(_, k)| k.key_type)
            .collect()
    }

    async fn verify(&self, host: &str, port: u16, key: &HostKeyInfo) -> bool {
        let known = {
            let state = state(&self.app);
            let vault = state.vault();
            if !vault.is_unlocked() {
                return false;
            }
            known_host_keys(&vault, host, port)
        };
        if known
            .iter()
            .any(|(_, k)| k.key_type == key.key_type && k.public_key == key.public_key)
        {
            self.outcome.signing_in.store(true, Ordering::Relaxed);
            return true;
        }
        let (kind, _) = host_key_status(&known, key);
        *lock(&self.outcome.host_key) = Some(kind);
        false
    }
}

/// Answers no keyboard-interactive prompt, so a login that needs one fails at once.
struct UnattendedPrompts(Arc<Unattended>);

#[async_trait]
impl KeyboardInteractive for UnattendedPrompts {
    async fn respond(&self, request: PromptRequest) -> Option<Vec<Zeroizing<String>>> {
        self.0.prompted.store(true, Ordering::Relaxed);
        let lone_password = matches!(request.prompts.as_slice(), [p]
            if !p.echo && p.text.to_ascii_lowercase().contains("password"));
        // Only a hop with a saved password had it rejected; at a hop signing in with a key or
        // the agent, a password asked after it is a second factor the user has to give.
        let hop = self.0.hops.load(Ordering::Relaxed).saturating_sub(1);
        let saved = self.0.saved_password.get(hop).copied().unwrap_or(false);
        if saved && (request.password || lone_password) {
            self.0.password_prompted.store(true, Ordering::Relaxed);
        }
        None
    }
}

/// Connects without the user (HOST-14): a host key that is not trusted yet and a prompt from
/// the server both fail the connection, and the returned [`Unattended`] says which did.
pub async fn connect_unattended(
    app: &AppHandle,
    mut cfg: ConnectConfig,
) -> (AppResult<SshSession>, Arc<Unattended>) {
    let saved_password = cfg
        .jump
        .iter()
        .map(|j| &j.auth)
        .chain(std::iter::once(&cfg.auth))
        .map(|auth| matches!(auth, AuthMethod::Password(_)))
        .collect();
    let outcome = Arc::new(Unattended {
        saved_password,
        ..Unattended::default()
    });
    let attempt = Attempt::of(&cfg);
    attempt.log_start();
    cfg.keyboard_interactive = Some(Arc::new(UnattendedPrompts(Arc::clone(&outcome))));
    let verifier = Arc::new(UnattendedVerifier {
        app: app.clone(),
        outcome: Arc::clone(&outcome),
    });
    let result = hatoba_ssh::connect(cfg, verifier)
        .await
        .map_err(AppError::from);
    attempt.log_end(&result);
    (result, outcome)
}

/// `user@host:port`, with an IPv6 address in brackets.
pub fn target_label(username: &str, host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("{username}@[{host}]:{port}")
    } else {
        format!("{username}@{host}:{port}")
    }
}

/// Connects to a saved host for a short task (key deploy, connection test).
pub async fn connect_for_task(
    app: &AppHandle,
    state: &AppState,
    host_id: &str,
    overrides: Option<&Overrides>,
) -> AppResult<SshSession> {
    let cfg = state.with_unlocked(|v| {
        let host = v
            .get(host_id)
            .and_then(Item::as_host)
            .cloned()
            .ok_or_else(|| AppError::not_found("host"))?;
        build_config(v, &host, Some(host_id), overrides)
    })?;
    connect_with(app, cfg, None).await
}

/// Every connection goes through here: terminals, quick connect, connection tests and tasks.
pub async fn connect_with(
    app: &AppHandle,
    mut cfg: ConnectConfig,
    session_id: Option<String>,
) -> AppResult<SshSession> {
    let attempt = Attempt::of(&cfg);
    attempt.log_start();
    cfg.keyboard_interactive = Some(Arc::new(Interactive {
        app: app.clone(),
        session_id: session_id.clone(),
    }));
    let verifier = Arc::new(Verifier {
        app: app.clone(),
        session_id,
    });
    let result = hatoba_ssh::connect(cfg, verifier)
        .await
        .map_err(AppError::from);
    attempt.log_end(&result);
    result
}

/// What the log says about a connection (SEC-04): the target, the user name and the kind of
/// authentication, and on failure the kind of error. Never a password, key or prompt answer,
/// and not the error's text, which can quote the server.
struct Attempt {
    host: String,
    port: u16,
    user: String,
    auth: &'static str,
    jump_hosts: usize,
    proxy: bool,
}

impl Attempt {
    fn of(cfg: &ConnectConfig) -> Self {
        Self {
            host: cfg.host.clone(),
            port: cfg.port,
            user: cfg.username.clone(),
            auth: auth_kind(&cfg.auth),
            jump_hosts: cfg.jump.len(),
            proxy: cfg.proxy.is_some(),
        }
    }

    fn log_start(&self) {
        tracing::info!(
            host = %self.host,
            port = self.port,
            user = %self.user,
            auth = self.auth,
            jump_hosts = self.jump_hosts,
            proxy = self.proxy,
            "ssh connecting"
        );
    }

    fn log_end<T>(&self, result: &AppResult<T>) {
        match result {
            Ok(_) => tracing::info!(host = %self.host, port = self.port, "ssh connected"),
            Err(e) => tracing::info!(
                host = %self.host,
                port = self.port,
                user = %self.user,
                auth = self.auth,
                code = ?e.code,
                kind = ?e.ssh_kind,
                "ssh connection failed"
            ),
        }
    }
}

fn auth_kind(auth: &AuthMethod) -> &'static str {
    match auth {
        AuthMethod::Password(_) => "password",
        AuthMethod::PrivateKey { .. } => "key",
        AuthMethod::Agent => "agent",
        AuthMethod::AgentThenAsk => "agent_then_ask",
        AuthMethod::Ask => "ask",
        AuthMethod::None => "none",
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use hatoba_core::KdfParams;
    use tracing_subscriber::{fmt, prelude::*};

    use super::*;

    /// A writer the test reads back.
    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn attempts_are_logged_without_credentials_or_error_text() {
        let buffer = Buffer::default();
        let writer = buffer.clone();
        let subscriber = tracing_subscriber::registry().with(
            fmt::layer()
                .with_ansi(false)
                .with_writer(move || writer.clone()),
        );
        let password = AuthMethod::Password(Zeroizing::new("PASSWORD-SECRET".into()));
        let cfg = ConnectConfig::new("db.internal", 2222, "deploy", password);
        tracing::subscriber::with_default(subscriber, || {
            let attempt = Attempt::of(&cfg);
            attempt.log_start();
            let failed: Result<(), _> = Err(AppError::ssh(
                SshErrorKind::AuthFailed,
                "server said SERVER-TEXT",
            ));
            attempt.log_end(&failed);
        });
        let text = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        for expected in [
            "ssh connecting",
            "host=db.internal",
            "port=2222",
            "user=deploy",
            "auth=\"password\"",
            "ssh connection failed",
            "code=Ssh",
            "kind=Some(AuthFailed)",
        ] {
            assert!(text.contains(expected), "{expected} missing from {text}");
        }
        assert!(!text.contains("SECRET"), "{text}");
        assert!(!text.contains("SERVER-TEXT"), "{text}");
    }

    fn key(key_type: &str, public_key: &str) -> HostKeyInfo {
        HostKeyInfo {
            key_type: key_type.to_owned(),
            public_key: public_key.to_owned(),
            fingerprint: format!("SHA256:{public_key}"),
        }
    }

    fn record(host: &str, port: u16, key: &HostKeyInfo) -> KnownHost {
        KnownHost {
            host: host.to_owned(),
            port,
            key_type: key.key_type.clone(),
            public_key: key.public_key.clone(),
            fingerprint: key.fingerprint.clone(),
            first_seen_at: 1,
            updated_at: 0,
        }
    }

    fn known(key: &HostKeyInfo) -> (String, KnownHost) {
        (hatoba_core::new_id(), record("demo.example", 22, key))
    }

    #[test]
    fn host_key_of_another_type_is_new_with_the_previous_key_as_context() {
        let rsa = key("ssh-rsa", "trusted-rsa");
        let ed25519 = key("ssh-ed25519", "received-ed25519");
        let known = vec![known(&rsa)];
        let (kind, previous) = host_key_status(&known, &ed25519);
        assert_eq!(kind, HostKeyPromptKind::New);
        assert_eq!(previous.unwrap().fingerprint, rsa.fingerprint);
    }

    #[test]
    fn changed_key_uses_the_previous_key_of_the_same_type() {
        let rsa = key("ssh-rsa", "trusted-rsa");
        let ed25519 = key("ssh-ed25519", "trusted-ed25519");
        let changed = key("ssh-ed25519", "changed-ed25519");
        let known = vec![known(&rsa), known(&ed25519)];
        let (kind, previous) = host_key_status(&known, &changed);
        assert_eq!(kind, HostKeyPromptKind::Changed);
        assert_eq!(previous.unwrap().fingerprint, ed25519.fingerprint);
    }

    #[test]
    fn unknown_host_key_is_new_without_previous_context() {
        let (kind, previous) = host_key_status(&[], &key("ssh-ed25519", "received"));
        assert_eq!(kind, HostKeyPromptKind::New);
        assert!(previous.is_none());
    }

    #[test]
    fn saving_a_key_preserves_other_types_hosts_and_ports() {
        let mut vault = Vault::open_in_memory().unwrap();
        vault
            .create_with_params(&hatoba_core::new_id(), KdfParams::for_tests())
            .unwrap();
        let rsa = key("ssh-rsa", "trusted-rsa");
        let ed25519 = key("ssh-ed25519", "trusted-ed25519");
        let changed = key("ssh-ed25519", "changed-ed25519");
        for (host, port, key) in [
            ("demo.example", 22, &rsa),
            ("DEMO.EXAMPLE", 22, &ed25519),
            ("demo.example", 2222, &ed25519),
            ("other.example", 22, &ed25519),
        ] {
            vault
                .put(None, Item::KnownHost(record(host, port, key)))
                .unwrap();
        }
        save_host_key(&mut vault, "demo.example", 22, &changed).unwrap();
        let records = vault.known_hosts();
        assert_eq!(records.len(), 4);
        assert!(
            records
                .iter()
                .any(|(_, k)| k.port == 22 && k.public_key == rsa.public_key)
        );
        assert!(
            !records
                .iter()
                .any(|(_, k)| k.host.eq_ignore_ascii_case("demo.example")
                    && k.port == 22
                    && k.public_key == ed25519.public_key)
        );
        assert!(
            records
                .iter()
                .any(|(_, k)| k.port == 22 && k.public_key == changed.public_key)
        );
    }

    #[test]
    fn trusting_another_type_keeps_the_original_key() {
        let mut vault = Vault::open_in_memory().unwrap();
        vault
            .create_with_params(&hatoba_core::new_id(), KdfParams::for_tests())
            .unwrap();
        let rsa = key("ssh-rsa", "trusted-rsa");
        let ed25519 = key("ssh-ed25519", "received-ed25519");
        vault
            .put(None, Item::KnownHost(record("demo.example", 22, &rsa)))
            .unwrap();
        save_host_key(&mut vault, "demo.example", 22, &ed25519).unwrap();
        assert_eq!(vault.known_hosts().len(), 2);
    }

    #[tokio::test]
    async fn a_password_asked_after_a_key_is_a_prompt_not_a_rejection() {
        let request = || PromptRequest {
            host: "h".to_owned(),
            port: 22,
            username: "u".to_owned(),
            password: false,
            name: String::new(),
            instructions: String::new(),
            prompts: vec![hatoba_ssh::Prompt {
                text: "Password: ".to_owned(),
                echo: false,
            }],
        };
        // A jump host signing in with a saved password, then the destination with a key.
        let outcome = |at: usize| {
            let outcome = Arc::new(Unattended {
                saved_password: vec![true, false],
                ..Unattended::default()
            });
            outcome.hops.store(at, Ordering::Relaxed);
            outcome
        };
        let at_jump = outcome(1);
        assert!(
            UnattendedPrompts(Arc::clone(&at_jump))
                .respond(request())
                .await
                .is_none()
        );
        assert!(at_jump.prompted() && at_jump.password_prompted());
        let at_key = outcome(2);
        UnattendedPrompts(Arc::clone(&at_key))
            .respond(request())
            .await;
        assert!(at_key.prompted() && !at_key.password_prompted());
    }
}
