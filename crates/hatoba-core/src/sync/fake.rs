//! An in-memory sync server with the Worker's semantics, for tests.
//!
//! One [`FakeServer`] plays the Worker + D1; each simulated device talks to it through its own
//! [`FakeBackend`] (its own session). It reproduces what the sync engine depends on:
//! per-item revisions with compare-and-swap pushes, a global monotonically increasing `seq`
//! (with holes after lost races), paginated pulls, hashed credentials, per-device sessions that
//! a password change revokes, and restricted recovery sessions.
//!
//! Test hooks: take the server offline, run code in the middle of a push (to edit locally while
//! a push is in flight), inject raw rows, lose a push response after the server applied it, and
//! hold a pull that never answers.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use async_trait::async_trait;

use crate::crypto::{auth_hash, b64_encode, random_bytes, sha256_hex};
use crate::error::{Error, Result};
use crate::sync::backend::{
    Change, DeviceLogin, KdfInfo, PullPage, PushResult, Recovered, RemoteDevice, RemoteItem,
    ServerInfo, Session, SyncBackend, VaultInit, VaultMeta, VaultMetaUpdate,
};

pub(crate) const SETUP_TOKEN: &str = "setup-token-for-tests";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ServerItem {
    pub envelope: Option<String>,
    pub revision: u64,
    pub seq: u64,
    pub deleted: bool,
    pub updated_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    Full,
    Recovery,
}

struct SessionRec {
    token_hash: String,
    device_id: String,
    device_name: String,
    scope: Scope,
    created_at: i64,
    expires_at: i64,
}

#[derive(Default)]
struct ServerMeta {
    schema_version: u32,
    kdf_salt: String,
    kdf_params: String,
    auth_hash: String,
    protected_vault_key: String,
    recovery_vault_key: String,
    recovery_auth_hash: String,
}

#[derive(Default)]
struct State {
    meta: Option<ServerMeta>,
    seq: u64,
    items: BTreeMap<String, ServerItem>,
    sessions: Vec<SessionRec>,
    offline: bool,
    lose_next_push_response: bool,
    fail_next: Vec<&'static str>,
    pull_calls: u32,
    push_calls: u32,
    pushed_change_count: u32,
    pull_page_cap: Option<usize>,
}

/// The shared "cloud".
#[derive(Default)]
pub(crate) struct FakeServer {
    state: Mutex<State>,
}

impl FakeServer {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn st(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    pub(crate) fn set_offline(&self, offline: bool) {
        self.st().offline = offline;
    }

    pub(crate) fn lose_next_push_response(&self) {
        self.st().lose_next_push_response = true;
    }

    /// Makes the next call to `op` (`"setup"`, `"login"` or `"pull"`) fail with
    /// [`Error::Offline`]. A failed `setup` has still been applied: its response is lost.
    pub(crate) fn fail_next(&self, op: &'static str) {
        self.st().fail_next.push(op);
    }

    fn take_failure(&self, op: &str) -> Result<()> {
        let mut s = self.st();
        match s.fail_next.iter().position(|o| *o == op) {
            Some(i) => {
                s.fail_next.remove(i);
                Err(Error::Offline)
            }
            None => Ok(()),
        }
    }

    /// Limits how many items one pull page returns (to exercise pagination).
    pub(crate) fn set_pull_page_cap(&self, cap: usize) {
        self.st().pull_page_cap = Some(cap);
    }

    pub(crate) fn item(&self, id: &str) -> Option<ServerItem> {
        self.st().items.get(id).cloned()
    }

    pub(crate) fn item_count(&self) -> usize {
        self.st().items.len()
    }

    pub(crate) fn current_seq(&self) -> u64 {
        self.st().seq
    }

    pub(crate) fn push_calls(&self) -> u32 {
        self.st().push_calls
    }

    pub(crate) fn pull_calls(&self) -> u32 {
        self.st().pull_calls
    }

    pub(crate) fn pushed_change_count(&self) -> u32 {
        self.st().pushed_change_count
    }

    /// Writes a row directly, as if another client had pushed it.
    pub(crate) fn inject_item(
        &self,
        id: &str,
        envelope: Option<&str>,
        deleted: bool,
        updated_at: i64,
    ) {
        let mut s = self.st();
        s.seq += 1;
        let seq = s.seq;
        let revision = s.items.get(id).map_or(1, |i| i.revision + 1);
        s.items.insert(
            id.to_owned(),
            ServerItem {
                envelope: envelope.map(str::to_owned),
                revision,
                seq,
                deleted,
                updated_at,
            },
        );
    }

    /// Forgets every item row but keeps the vault metadata (simulates a reset database).
    pub(crate) fn reset_items_for_test(&self) {
        self.st().items.clear();
    }

    /// Deletes every session (simulates expiry / revocation).
    pub(crate) fn revoke_all_sessions(&self) {
        self.st().sessions.clear();
    }

    pub(crate) fn session_count(&self) -> usize {
        self.st().sessions.len()
    }

    /// Everything the server stores, as one string (for "only ciphertext" assertions).
    pub(crate) fn dump(&self) -> String {
        let s = self.st();
        let mut out = String::new();
        if let Some(m) = &s.meta {
            out.push_str(&format!(
                "{} {} {} {} {} {} {}\n",
                m.kdf_salt,
                m.kdf_params,
                m.auth_hash,
                m.protected_vault_key,
                m.recovery_vault_key,
                m.recovery_auth_hash,
                m.schema_version
            ));
        }
        for (id, i) in &s.items {
            out.push_str(&format!(
                "{id} {:?} {} {} {} {}\n",
                i.envelope, i.revision, i.seq, i.deleted, i.updated_at
            ));
        }
        for sess in &s.sessions {
            out.push_str(&format!(
                "{} {} {}\n",
                sess.token_hash, sess.device_id, sess.device_name
            ));
        }
        out
    }
}

type PushHook = Box<dyn Fn() + Send + Sync>;

/// One device's connection to the [`FakeServer`].
pub(crate) struct FakeBackend {
    server: Arc<FakeServer>,
    session: RwLock<Option<Session>>,
    during_push: Mutex<Option<PushHook>>,
    after_pull: Mutex<Option<PushHook>>,
    stall_pull: Mutex<Option<Arc<tokio::sync::Notify>>>,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl FakeBackend {
    pub(crate) fn new(server: &Arc<FakeServer>) -> Self {
        Self {
            server: Arc::clone(server),
            session: RwLock::new(None),
            during_push: Mutex::new(None),
            after_pull: Mutex::new(None),
            stall_pull: Mutex::new(None),
            clock: Arc::new(|| 1_700_000_000_000),
        }
    }

    /// Runs `hook` inside the next pushes, after the server applied them but before the client
    /// sees the response.
    pub(crate) fn on_push(&self, hook: impl Fn() + Send + Sync + 'static) {
        *self.during_push.lock().unwrap() = Some(Box::new(hook));
    }

    /// Runs `hook` once, right after the next pull has been answered (so the server can change
    /// between the engine's pull and its push).
    pub(crate) fn on_next_pull(&self, hook: impl Fn() + Send + Sync + 'static) {
        *self.after_pull.lock().unwrap() = Some(Box::new(hook));
    }

    /// Makes the next pull hang until it is dropped, like a server that never answers. The
    /// returned notifier fires once the pull is waiting.
    pub(crate) fn stall_next_pull(&self) -> Arc<tokio::sync::Notify> {
        let waiting = Arc::new(tokio::sync::Notify::new());
        *self.stall_pull.lock().unwrap() = Some(Arc::clone(&waiting));
        waiting
    }

    pub(crate) fn clear_push_hook(&self) {
        *self.during_push.lock().unwrap() = None;
    }

    fn check_online(&self) -> Result<()> {
        if self.server.st().offline {
            Err(Error::Offline)
        } else {
            Ok(())
        }
    }

    /// Resolves the current session to its scope, like the Worker's `requireSession`.
    fn authenticate(&self, allow_recovery: bool) -> Result<(String, Scope)> {
        self.check_online()?;
        let token = self
            .session
            .read()
            .unwrap()
            .as_ref()
            .map(|s| s.token.to_string())
            .ok_or(Error::Unauthorized)?;
        let hash = sha256_hex(token.as_bytes());
        let s = self.server.st();
        let rec = s
            .sessions
            .iter()
            .find(|r| r.token_hash == hash)
            .ok_or(Error::Unauthorized)?;
        if rec.expires_at <= (self.clock)() {
            return Err(Error::Unauthorized);
        }
        if rec.scope == Scope::Recovery && !allow_recovery {
            return Err(Error::Unauthorized); // 403 insufficient_scope
        }
        Ok((rec.device_id.clone(), rec.scope))
    }

    fn new_session(&self, s: &mut State, device: &DeviceLogin, scope: Scope) -> Result<Session> {
        let token = b64_encode(&random_bytes::<32>()?).replace(['+', '/', '='], "x");
        let expires_at = (self.clock)()
            + if scope == Scope::Full {
                30 * 86_400_000
            } else {
                15 * 60_000
            };
        s.sessions
            .retain(|r| !(r.device_id == device.device_id && r.scope == scope));
        s.sessions.push(SessionRec {
            token_hash: sha256_hex(token.as_bytes()),
            device_id: device.device_id.clone(),
            device_name: device.sealed_name.clone(),
            scope,
            created_at: (self.clock)(),
            expires_at,
        });
        let session = Session {
            token: zeroize::Zeroizing::new(token),
            expires_at,
        };
        *self.session.write().unwrap() = Some(session.clone());
        Ok(session)
    }
}

fn to_remote(id: &str, i: &ServerItem) -> RemoteItem {
    RemoteItem {
        id: id.to_owned(),
        envelope: i.envelope.clone(),
        revision: i.revision,
        seq: i.seq,
        deleted: i.deleted,
        updated_at: i.updated_at,
    }
}

#[async_trait]
impl SyncBackend for FakeBackend {
    async fn health(&self) -> Result<ServerInfo> {
        self.check_online()?;
        let initialized = self.server.st().meta.is_some();
        Ok(ServerInfo {
            service: "hatoba-sync".into(),
            version: "test".into(),
            api: 1,
            initialized,
        })
    }

    async fn prelogin(&self) -> Result<KdfInfo> {
        self.check_online()?;
        let s = self.server.st();
        let m = s.meta.as_ref().ok_or(Error::RemoteNotInitialized)?;
        Ok(KdfInfo {
            kdf_salt: m.kdf_salt.clone(),
            kdf_params: m.kdf_params.clone(),
        })
    }

    async fn setup(&self, init: VaultInit) -> Result<()> {
        self.check_online()?;
        if init.setup_token.as_ref().map(|t| t.as_str()) != Some(SETUP_TOKEN) {
            return Err(Error::InvalidSetupToken);
        }
        let mut s = self.server.st();
        if s.meta.is_some() {
            return Err(Error::RemoteInitialized);
        }
        s.meta = Some(ServerMeta {
            schema_version: init.schema_version,
            kdf_salt: init.kdf_salt,
            kdf_params: init.kdf_params,
            auth_hash: auth_hash(&init.auth_key),
            protected_vault_key: init.protected_vault_key,
            recovery_vault_key: init.recovery_vault_key,
            recovery_auth_hash: auth_hash(&init.recovery_auth),
        });
        drop(s);
        self.server.take_failure("setup")
    }

    async fn login(&self, auth_key: &[u8; 32], device: &DeviceLogin) -> Result<Session> {
        self.check_online()?;
        self.server.take_failure("login")?;
        let mut s = self.server.st();
        let m = s.meta.as_ref().ok_or(Error::RemoteNotInitialized)?;
        if m.auth_hash != auth_hash(auth_key) {
            return Err(Error::WrongPassword);
        }
        self.new_session(&mut s, device, Scope::Full)
    }

    async fn recover(&self, recovery_auth: &[u8; 32], device: &DeviceLogin) -> Result<Recovered> {
        self.check_online()?;
        let mut s = self.server.st();
        let m = s.meta.as_ref().ok_or(Error::RemoteNotInitialized)?;
        if m.recovery_auth_hash != auth_hash(recovery_auth) {
            return Err(Error::WrongRecoveryCode);
        }
        let (rvk, salt, params) = (
            m.recovery_vault_key.clone(),
            m.kdf_salt.clone(),
            m.kdf_params.clone(),
        );
        let session = self.new_session(&mut s, device, Scope::Recovery)?;
        Ok(Recovered {
            recovery_vault_key: rvk,
            kdf: KdfInfo {
                kdf_salt: salt,
                kdf_params: params,
            },
            session,
        })
    }

    async fn fetch_vault(&self) -> Result<VaultMeta> {
        self.authenticate(false)?;
        let s = self.server.st();
        let m = s.meta.as_ref().ok_or(Error::RemoteNotInitialized)?;
        Ok(VaultMeta {
            schema_version: m.schema_version,
            kdf_salt: m.kdf_salt.clone(),
            kdf_params: m.kdf_params.clone(),
            protected_vault_key: m.protected_vault_key.clone(),
            recovery_vault_key: m.recovery_vault_key.clone(),
            seq: s.seq,
        })
    }

    async fn pull(&self, since_seq: u64, limit: u32) -> Result<PullPage> {
        let stall = self.stall_pull.lock().unwrap().take();
        if let Some(waiting) = stall {
            waiting.notify_one();
            std::future::pending::<()>().await;
        }
        self.authenticate(false)?;
        self.server.take_failure("pull")?;
        let page = {
            let mut s = self.server.st();
            s.pull_calls += 1;
            let limit = usize::try_from(limit)
                .unwrap_or(usize::MAX)
                .min(s.pull_page_cap.unwrap_or(usize::MAX));
            let mut rows: Vec<(&String, &ServerItem)> =
                s.items.iter().filter(|(_, i)| i.seq > since_seq).collect();
            rows.sort_by_key(|(_, i)| i.seq);
            let has_more = rows.len() > limit;
            rows.truncate(limit);
            let next_since = rows.last().map_or(since_seq, |(_, i)| i.seq);
            PullPage {
                items: rows.iter().map(|(id, i)| to_remote(id, i)).collect(),
                next_since,
                has_more,
            }
        };
        let hook = self.after_pull.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
        Ok(page)
    }

    async fn push(&self, changes: Vec<Change>) -> Result<Vec<PushResult>> {
        self.authenticate(false)?;
        if changes.len() > 100 {
            return Err(Error::Server("http 413 too_many_changes".into()));
        }
        let (results, lose) = {
            let mut s = self.server.st();
            s.push_calls += 1;
            s.pushed_change_count += u32::try_from(changes.len()).unwrap();
            let mut seen = std::collections::HashSet::new();
            let mut results = Vec::new();
            for c in changes {
                assert!(seen.insert(c.id.clone()), "duplicate id in one push");
                assert_eq!(
                    c.deleted,
                    c.envelope.is_none(),
                    "deleted <=> envelope is null"
                );
                if c.envelope.as_ref().is_some_and(|e| e.len() > 64 * 1024) {
                    results.push(PushResult::Error {
                        id: c.id,
                        error: "too_large".into(),
                    });
                    continue;
                }
                s.seq += 1; // consumed even if the write loses the race: holes are allowed
                let seq = s.seq;
                let existing = s.items.get(&c.id).cloned();
                let applied = match (&existing, c.base_revision) {
                    (None, 0) => Some(1),
                    (Some(cur), base) if base > 0 && cur.revision == base => Some(base + 1),
                    _ => None,
                };
                if let Some(revision) = applied {
                    s.items.insert(
                        c.id.clone(),
                        ServerItem {
                            envelope: c.envelope,
                            revision,
                            seq,
                            deleted: c.deleted,
                            updated_at: c.updated_at,
                        },
                    );
                    results.push(PushResult::Ok {
                        id: c.id,
                        revision,
                        seq,
                    });
                } else if let Some(cur) = existing {
                    results.push(PushResult::Conflict {
                        server: to_remote(&c.id, &cur),
                        id: c.id,
                    });
                } else {
                    results.push(PushResult::Error {
                        id: c.id,
                        error: "not_found".into(),
                    });
                }
            }
            (results, std::mem::take(&mut s.lose_next_push_response))
        };
        if let Some(hook) = self.during_push.lock().unwrap().as_ref() {
            hook();
        }
        if lose {
            return Err(Error::Offline);
        }
        Ok(results)
    }

    async fn update_vault_meta(&self, update: VaultMetaUpdate) -> Result<()> {
        let (device_id, scope) = self.authenticate(true)?;
        let mut s = self.server.st();
        let m = s.meta.as_mut().ok_or(Error::RemoteNotInitialized)?;
        m.kdf_salt = update.kdf_salt;
        m.kdf_params = update.kdf_params;
        m.auth_hash = auth_hash(&update.auth_key);
        m.protected_vault_key = update.protected_vault_key;
        if let Some(r) = update.recovery {
            m.recovery_vault_key = r.recovery_vault_key;
            m.recovery_auth_hash = auth_hash(&r.recovery_auth);
        }
        match scope {
            Scope::Full => {
                let token_hash = self
                    .session
                    .read()
                    .unwrap()
                    .as_ref()
                    .map(|x| sha256_hex(x.token.as_bytes()));
                s.sessions
                    .retain(|r| Some(&r.token_hash) == token_hash.as_ref());
                let _ = device_id;
            }
            Scope::Recovery => s.sessions.clear(),
        }
        Ok(())
    }

    async fn devices(&self) -> Result<Vec<RemoteDevice>> {
        let (me, _) = self.authenticate(false)?;
        let s = self.server.st();
        Ok(s.sessions
            .iter()
            .filter(|r| r.scope == Scope::Full)
            .map(|r| RemoteDevice {
                device_id: r.device_id.clone(),
                device_name: r.device_name.clone(),
                created_at: r.created_at,
                last_seen: r.created_at,
                expires_at: r.expires_at,
                current: r.device_id == me,
            })
            .collect())
    }

    async fn revoke_device(&self, device_id: &str) -> Result<()> {
        self.authenticate(false)?;
        self.server
            .st()
            .sessions
            .retain(|r| r.device_id != device_id);
        Ok(())
    }

    fn set_session(&self, session: Option<Session>) {
        *self.session.write().unwrap() = session;
    }
}
