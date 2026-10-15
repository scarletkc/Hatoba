//! This device's ChatGPT sessions: access tokens on demand, with one refresh at a time per
//! session.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use super::oauth::{self, RefreshError};
use super::{Credentials, Endpoints, SessionStatus, now_ms, sign_in_needed};
use crate::error::AiError;

/// An access token with less than this left is refreshed before a request.
const REFRESH_MARGIN_MS: i64 = 5 * 60 * 1000;

/// Where the sessions live: the desktop shell seals them under the vault key, on this device
/// only. `id` is the provider's item id.
pub trait CredentialStore: Send + Sync + 'static {
    /// The session of provider `id`, `None` when this device has none.
    ///
    /// # Errors
    /// The store cannot be read, for example while the vault is locked.
    fn load(&self, id: &str) -> Result<Option<Credentials>, AiError>;

    /// Replaces the session of provider `id`; `None` removes it.
    ///
    /// # Errors
    /// The store cannot be written, for example while the vault is locked.
    fn store(&self, id: &str, credentials: Option<&Credentials>) -> Result<(), AiError>;
}

/// A refresh that arrived while the store could not take it (the vault locked meanwhile). The
/// replacement refresh token is the only one that still works, so it is kept until the store
/// takes it, provided the session still holds the token it replaced.
struct Pending {
    replaced: Zeroizing<String>,
    next: Credentials,
}

struct Inner {
    http: reqwest::Client,
    endpoints: Endpoints,
    store: Arc<dyn CredentialStore>,
    /// One lock per session: refreshes, sign-ins and sign-outs of a session run one at a time.
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    pending: Mutex<HashMap<String, Pending>>,
}

/// This device's ChatGPT sessions. Cheap to clone; every clone is the same.
#[derive(Clone)]
pub struct Sessions(Arc<Inner>);

impl fmt::Debug for Sessions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sessions")
            .field("endpoints", &self.0.endpoints)
            .finish_non_exhaustive()
    }
}

impl PartialEq for Sessions {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Sessions {}

/// How a provider request reaches its ChatGPT session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatgptAuth {
    /// This device's sessions.
    pub sessions: Sessions,
    /// The provider's item id, which names its session.
    pub id: String,
}

impl ChatgptAuth {
    /// An access token for a request (see [`Sessions::access_token`]).
    ///
    /// # Errors
    /// As [`Sessions::access_token`].
    pub async fn access_token(
        &self,
        failed: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<Zeroizing<String>, AiError> {
        self.sessions.access_token(&self.id, failed, cancel).await
    }
}

fn guard<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The access token of `credentials` when a request can use it now: signed in with plan use,
/// not the token that just failed, and not due for a refresh (unless the token response said not
/// to refresh yet and it has not expired).
fn usable(credentials: &Credentials, failed: Option<&str>, now: i64) -> Option<Zeroizing<String>> {
    if credentials.ended()
        || !credentials.plan_enabled()
        || credentials.access_token.is_empty()
        || failed == Some(credentials.access_token.as_str())
    {
        return None;
    }
    let fresh = credentials.expires_at - now > REFRESH_MARGIN_MS;
    let too_early = credentials
        .refresh_after
        .is_some_and(|after| now < after && credentials.expires_at > now);
    (fresh || too_early).then(|| credentials.access_token.clone())
}

impl Sessions {
    /// Sessions read and written through `store`, signing in at `endpoints` with `http`.
    #[must_use]
    pub fn new(
        http: reqwest::Client,
        endpoints: Endpoints,
        store: Arc<dyn CredentialStore>,
    ) -> Self {
        Self(Arc::new(Inner {
            http,
            endpoints,
            store,
            locks: Mutex::default(),
            pending: Mutex::default(),
        }))
    }

    /// The endpoints sign-ins go to.
    #[must_use]
    pub fn endpoints(&self) -> &Endpoints {
        &self.0.endpoints
    }

    /// The session of provider `id` as this device holds it.
    ///
    /// # Errors
    /// The store cannot be read.
    pub fn credentials(&self, id: &str) -> Result<Option<Credentials>, AiError> {
        self.0.load(id)
    }

    /// Where provider `id`'s session stands on this device.
    ///
    /// # Errors
    /// The store cannot be read.
    pub fn status(&self, id: &str) -> Result<SessionStatus, AiError> {
        Ok(self
            .0
            .load(id)?
            .map_or(SessionStatus::SignedOut, |c| c.status()))
    }

    fn lock_for(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(guard(&self.0.locks).entry(id.to_owned()).or_default())
    }

    /// An access token for a request to provider `id`, refreshed first when it is due. With
    /// `failed`, the token a request was just refused with (401), a token other than that one:
    /// the session is refreshed unless another request did so meanwhile.
    ///
    /// Refreshes of a session run one at a time, and a request that needs a token meanwhile waits
    /// for the refresh in flight and uses its token. A refresh runs to its end once sent, also
    /// when `cancel` fires (the caller just stops waiting), so the replacement refresh token is
    /// never lost.
    ///
    /// # Errors
    /// [`AiError::SignIn`] when the provider is not signed in on this device, its session ended
    /// (it is marked so) or plan use is not allowed; network and HTTP errors of a refresh that
    /// failed while the current token can no longer be used; [`AiError::Cancelled`].
    pub async fn access_token(
        &self,
        id: &str,
        failed: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<Zeroizing<String>, AiError> {
        if !guard(&self.0.pending).contains_key(id)
            && let Some(credentials) = self.0.load(id)?
            && let Some(token) = usable(&credentials, failed, now_ms())
        {
            return Ok(token);
        }
        let lock = self.lock_for(id);
        let inner = Arc::clone(&self.0);
        let id = id.to_owned();
        let failed = failed.map(|f| Zeroizing::new(f.to_owned()));
        let task = tokio::spawn(async move {
            let _held = lock.lock_owned().await;
            inner.flush_pending(&id);
            let Some(credentials) = inner.load(&id)? else {
                return Err(AiError::SignIn(sign_in_needed(SessionStatus::SignedOut)));
            };
            let status = credentials.status();
            if status != SessionStatus::SignedIn {
                return Err(AiError::SignIn(sign_in_needed(status)));
            }
            let now = now_ms();
            if let Some(token) = usable(&credentials, failed.as_deref().map(String::as_str), now) {
                return Ok(token);
            }
            // Not cancellable: a refresh that was sent must be stored.
            let refreshed = oauth::refresh(
                &inner.http,
                &inner.endpoints,
                &credentials,
                &CancellationToken::new(),
            )
            .await;
            match refreshed {
                Ok(next) => {
                    let token = next.access_token.clone();
                    inner.save_refreshed(&id, &credentials, next);
                    Ok(token)
                }
                Err(RefreshError::Ended(code)) => {
                    tracing::info!(code = %code, "ChatGPT session ended on refresh");
                    let _ = inner.store.store(&id, Some(&credentials.without_tokens()));
                    Err(AiError::SignIn(sign_in_needed(SessionStatus::Ended)))
                }
                Err(RefreshError::Failed(err)) => {
                    // A refresh ahead of expiry may fail; the current token still serves.
                    if failed.is_none() && credentials.expires_at > now {
                        Ok(credentials.access_token.clone())
                    } else {
                        Err(err)
                    }
                }
            }
        });
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(AiError::Cancelled),
            joined = task => joined.unwrap_or_else(|_| {
                Err(AiError::SignIn("Renewing the ChatGPT sign-in failed.".into()))
            }),
        }
    }

    /// Stores the session of a finished sign-in, after any refresh of the provider's previous
    /// session in flight. The session it replaces is revoked at OpenAI in the background.
    ///
    /// # Errors
    /// The store cannot be written.
    pub async fn signed_in(&self, id: &str, credentials: &Credentials) -> Result<(), AiError> {
        let lock = self.lock_for(id);
        let _held = lock.lock().await;
        self.0.flush_pending(id);
        let replaced = self.0.load(id).ok().flatten();
        self.0.store.store(id, Some(credentials))?;
        guard(&self.0.pending).remove(id);
        if let Some(replaced) = replaced
            .filter(|r| !r.refresh_token.is_empty() && r.refresh_token != credentials.refresh_token)
        {
            self.0.revoke_later(replaced);
        }
        Ok(())
    }

    /// Revokes the refresh token of a session this device no longer keeps, at OpenAI in the
    /// background (retrying failures with backoff).
    pub fn revoke_later(&self, credentials: Credentials) {
        self.0.revoke_later(credentials);
    }

    /// Signs provider `id` out on this device: once any refresh in flight has ended, removes the
    /// session from the store, so no request uses it any more, then revokes its refresh token
    /// at OpenAI (retrying failures with backoff). Returns whether OpenAI confirmed the
    /// revocation; the session is gone from this device either way. `true` when there was none.
    ///
    /// # Errors
    /// The store cannot be written; nothing was removed or revoked then.
    pub async fn sign_out(&self, id: &str, cancel: &CancellationToken) -> Result<bool, AiError> {
        let lock = self.lock_for(id);
        let credentials = {
            let _held = lock.lock().await;
            self.0.flush_pending(id);
            // A kept refresh comes first: its refresh token is the one that works.
            let credentials = self.0.load(id)?;
            self.0.store.store(id, None)?;
            guard(&self.0.pending).remove(id);
            credentials
        };
        Ok(match credentials {
            Some(credentials) => {
                oauth::revoke(&self.0.http, &self.0.endpoints, &credentials, cancel).await
            }
            None => true,
        })
    }
}

impl Inner {
    fn revoke_later(&self, credentials: Credentials) {
        let (http, endpoints) = (self.http.clone(), self.endpoints.clone());
        tokio::spawn(async move {
            oauth::revoke(&http, &endpoints, &credentials, &CancellationToken::new()).await;
        });
    }

    fn load(&self, id: &str) -> Result<Option<Credentials>, AiError> {
        if let Some(pending) = guard(&self.pending).get(id) {
            return Ok(Some(pending.next.clone()));
        }
        self.store.load(id)
    }

    /// Stores a refresh's credentials, or keeps them until the store takes them.
    fn save_refreshed(&self, id: &str, previous: &Credentials, next: Credentials) {
        if self.store.store(id, Some(&next)).is_err() {
            guard(&self.pending).insert(
                id.to_owned(),
                Pending {
                    replaced: previous.refresh_token.clone(),
                    next,
                },
            );
        }
    }

    /// Writes a kept refresh once the store takes it, unless the session changed meanwhile.
    /// Called with the session's lock held.
    fn flush_pending(&self, id: &str) {
        let Some(pending) = guard(&self.pending).remove(id) else {
            return;
        };
        match self.store.load(id) {
            Ok(Some(current)) if current.refresh_token == pending.replaced => {
                if self.store.store(id, Some(&pending.next)).is_err() {
                    guard(&self.pending).insert(id.to_owned(), pending);
                }
            }
            // Signed out or signed in again meanwhile: the kept tokens are of no use.
            Ok(_) => self.revoke_later(pending.next),
            Err(_) => {
                guard(&self.pending).insert(id.to_owned(), pending);
            }
        }
    }
}
