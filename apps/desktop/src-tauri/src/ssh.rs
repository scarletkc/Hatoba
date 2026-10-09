//! Live SSH sessions, host-key verification (SSH-04) and interactive prompts (SSH-08).

use std::collections::{HashMap, HashSet};
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
    pub sftp: OnceCell<SftpClient>,
    /// Running local port forwards by forward item id (FWD-01).
    pub forwards: Mutex<HashMap<String, ForwardHandle>>,
    /// Resource usage sampling while the terminal shows it (TERM-12), with the id of its start.
    stats: Mutex<Option<(u64, StatsHandle)>>,
}

impl LiveSession {
    pub fn new(session: SshSession, shell: ShellHandle) -> Self {
        Self {
            session,
            shell,
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

    /// Removes the session only if it is still the given instance (a reconnect may have replaced it).
    pub fn remove_if_same(&self, id: &str, live: &Arc<LiveSession>) {
        let mut sessions = lock(&self.sessions);
        if sessions.get(id).is_some_and(|s| Arc::ptr_eq(s, live)) {
            sessions.remove(id);
        }
    }

    pub fn take(&self, id: &str) -> Option<Arc<LiveSession>> {
        lock(&self.sessions).remove(id)
    }

    /// SEC-03 "lock disconnects sessions".
    pub async fn disconnect_all(&self) {
        let all: Vec<Arc<LiveSession>> = lock(&self.sessions).drain().map(|(_, s)| s).collect();
        for live in all {
            live.close().await;
        }
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

/// The proxy of a connection to `host`: the choice of its first hop, which is its outermost
/// jump host, or the host itself without one. A broken jump chain ends where it breaks.
pub fn connection_proxy(vault: &Vault, host: &Host) -> AppResult<Option<ProxyConfig>> {
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
    resolve_proxy(vault, &first.proxy)
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

#[async_trait]
impl HostKeyVerifier for Verifier {
    async fn verify(&self, host: &str, port: u16, key: &HostKeyInfo) -> bool {
        let state = state(&self.app);
        let known: Vec<(String, KnownHost)> = {
            let vault = state.vault();
            if !vault.is_unlocked() {
                return false;
            }
            vault
                .known_hosts()
                .into_iter()
                .filter(|(_, k)| k.host.eq_ignore_ascii_case(host) && k.port == port)
                .collect()
        };
        if known
            .iter()
            .any(|(_, k)| k.key_type == key.key_type && k.public_key == key.public_key)
        {
            return true;
        }
        // A known host presenting a different key — of any type — is blocked until the user
        // explicitly chooses "update fingerprint" (downgrade to another key type included).
        let previous = known.first().map(|(_, k)| k.clone());
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
            kind: if previous.is_some() {
                HostKeyPromptKind::Changed
            } else {
                HostKeyPromptKind::New
            },
            known_fingerprint: previous.as_ref().map(|k| k.fingerprint.clone()),
            known_key_type: previous.as_ref().map(|k| k.key_type.clone()),
        };
        if prompt.emit(&self.app).is_err() {
            lock(&state.ssh.hostkey_waiters).remove(&request_id);
            return false;
        }
        let accepted = matches!(tokio::time::timeout(PROMPT_TIMEOUT, rx).await, Ok(Ok(true)));
        lock(&state.ssh.hostkey_waiters).remove(&request_id);
        if accepted {
            let saved = state.with_unlocked(|v| {
                for (id, _) in &known {
                    v.delete(id)?;
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
                v.put(None, Item::KnownHost(record))?;
                Ok(())
            });
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

pub async fn connect_with(
    app: &AppHandle,
    mut cfg: ConnectConfig,
    session_id: Option<String>,
) -> AppResult<SshSession> {
    cfg.keyboard_interactive = Some(Arc::new(Interactive {
        app: app.clone(),
        session_id: session_id.clone(),
    }));
    let verifier = Arc::new(Verifier {
        app: app.clone(),
        session_id,
    });
    Ok(hatoba_ssh::connect(cfg, verifier).await?)
}
