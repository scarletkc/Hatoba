//! Sign in with ChatGPT (AI-52) in the shell: this device's sessions, sealed in the vault, and
//! the sign-in and sign-out of a provider.
//!
//! A ChatGPT provider is a synced `ai_provider` item with `chatgpt` set: the account and the
//! client id OpenAI issued for Hatoba, which a sign-in on any device reuses. The tokens are this
//! device's own: [`VaultStore`] keeps them, with the device's `ext_agent_host_id`, in the vault's
//! sealed device-local meta value, so they never sync and never leave the vault unencrypted. A
//! device that has the provider from sync but no session of its own shows that it needs a
//! sign-in here.
//!
//! Requests reach the sessions through [`sessions`], one for the process, which
//! [`install`] binds to the app's vault. It reads the vault itself: never call into it while
//! holding the vault's guard.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock, Weak};

use hatoba_ai::AiError;
use hatoba_ai::chatgpt::{
    self, CredentialStore, Credentials, Endpoints, SessionStatus, Sessions,
    oauth::{self, Account, SignInRequest},
};
use hatoba_core::Vault;
use hatoba_core::model::{AiChatgptAccount, Item};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{AppError, AppResult};
use crate::state::SharedVault;

/// This device's sessions and host id, as the vault keeps them (sealed, never synced).
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct DeviceSessions {
    /// This device's `ext_agent_host_id`, made once before its first sign-in.
    host_id: String,
    /// The sessions by provider id.
    sessions: BTreeMap<String, Credentials>,
}

/// Reads this device's sessions; nothing stored reads as none.
fn read(v: &Vault) -> hatoba_core::Result<DeviceSessions> {
    Ok(v.chatgpt_sessions()?
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default())
}

fn write(v: &mut Vault, sessions: &DeviceSessions) -> hatoba_core::Result<()> {
    let json = Zeroizing::new(serde_json::to_string(sessions)?);
    v.set_chatgpt_sessions(&json)
}

/// The session of provider `id` on this device.
pub fn credentials(v: &Vault, id: &str) -> Option<Credentials> {
    read(v).ok()?.sessions.remove(id)
}

/// Where provider `id`'s session stands on this device.
pub fn status(v: &Vault, id: &str) -> SessionStatus {
    credentials(v, id).map_or(SessionStatus::SignedOut, |c| c.status())
}

/// This device's `ext_agent_host_id`, made and kept the first time it is asked for.
pub fn host_id(v: &mut Vault) -> AppResult<String> {
    let mut sessions = read(v)?;
    if sessions.host_id.is_empty() {
        sessions.host_id = oauth::new_host_id();
        write(v, &sessions)?;
    }
    Ok(sessions.host_id)
}

/// The sessions in the app's vault. A session whose provider no longer exists is dropped when
/// the sessions are next written.
struct VaultStore {
    vault: RwLock<Weak<Mutex<Vault>>>,
}

impl VaultStore {
    fn vault(&self) -> Option<SharedVault> {
        self.vault
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .upgrade()
    }
}

fn locked() -> AiError {
    AiError::SignIn("Unlock the vault to use the ChatGPT provider.".into())
}

impl CredentialStore for VaultStore {
    fn load(&self, id: &str) -> Result<Option<Credentials>, AiError> {
        let Some(vault) = self.vault() else {
            return Ok(None);
        };
        let v = vault.lock().unwrap_or_else(PoisonError::into_inner);
        if !v.is_unlocked() {
            return Err(locked());
        }
        Ok(read(&v).map_err(|_| locked())?.sessions.remove(id))
    }

    fn store(&self, id: &str, credentials: Option<&Credentials>) -> Result<(), AiError> {
        let Some(vault) = self.vault() else {
            return Err(locked());
        };
        let mut v = vault.lock().unwrap_or_else(PoisonError::into_inner);
        if !v.is_unlocked() {
            return Err(locked());
        }
        let mut sessions = read(&v).map_err(|_| locked())?;
        sessions
            .sessions
            .retain(|id, _| is_chatgpt_provider(&v, id));
        match credentials {
            // A provider deleted meanwhile gets no session.
            Some(credentials) if is_chatgpt_provider(&v, id) => {
                sessions.sessions.insert(id.to_owned(), credentials.clone());
            }
            Some(_) => {}
            None => {
                sessions.sessions.remove(id);
            }
        }
        write(&mut v, &sessions).map_err(|_| locked())
    }
}

fn is_chatgpt_provider(v: &Vault, id: &str) -> bool {
    v.get(id)
        .and_then(Item::as_ai_provider)
        .is_some_and(|p| p.chatgpt.is_some())
}

static STORE: OnceLock<Arc<VaultStore>> = OnceLock::new();
static SESSIONS: OnceLock<Sessions> = OnceLock::new();

fn store() -> &'static Arc<VaultStore> {
    STORE.get_or_init(|| {
        Arc::new(VaultStore {
            vault: RwLock::new(Weak::new()),
        })
    })
}

/// Binds the sessions to the app's vault (once, at startup; a later call rebinds them).
pub fn install(vault: &SharedVault) {
    *store()
        .vault
        .write()
        .unwrap_or_else(PoisonError::into_inner) = Arc::downgrade(vault);
}

/// This device's ChatGPT sessions.
pub fn sessions() -> &'static Sessions {
    SESSIONS.get_or_init(|| {
        Sessions::new(
            hatoba_ai::provider::http_client(),
            Endpoints::default(),
            Arc::clone(store()) as Arc<dyn CredentialStore>,
        )
    })
}

/// What a sign-in of provider `provider_id` asks for (`None`: a new account): the device's host
/// id, the account to sign in again with its hints, and whether to ask for consent again (the
/// last sign-in did not allow the plan's use).
pub fn sign_in_request(v: &mut Vault, provider_id: Option<&str>) -> AppResult<SignInRequest> {
    let host_id = host_id(v)?;
    let Some(id) = provider_id else {
        return Ok(SignInRequest {
            host_id,
            account: None,
            consent: false,
        });
    };
    let account = chatgpt_account(v, id)?;
    let local = credentials(v, id);
    let consent = local
        .as_ref()
        .is_some_and(|c| c.status() == SessionStatus::PlanDisabled);
    Ok(SignInRequest {
        host_id,
        account: Some(Account {
            client_id: account.client_id,
            subject: account.subject,
            email: account.email,
            id_token_hint: local.map(|c| c.id_token).filter(|token| !token.is_empty()),
        }),
        consent,
    })
}

/// The ChatGPT account of provider `id`.
pub fn chatgpt_account(v: &Vault, id: &str) -> AppResult<AiChatgptAccount> {
    v.get(id)
        .and_then(Item::as_ai_provider)
        .ok_or_else(|| AppError::not_found("provider"))?
        .chatgpt
        .clone()
        .ok_or_else(|| AppError::invalid("provider", "the provider does not sign in with ChatGPT"))
}

/// The account a sign-in found, as the provider item keeps it.
pub fn account_of(credentials: &Credentials) -> AiChatgptAccount {
    AiChatgptAccount {
        client_id: credentials.client_id.clone(),
        subject: credentials.subject.clone(),
        email: credentials.email.clone(),
    }
}

/// The display name of a new ChatGPT provider.
pub const PROVIDER_NAME: &str = "ChatGPT";

/// The base URL every ChatGPT provider has.
pub const BASE_URL: &str = chatgpt::BASE_URL;

#[cfg(test)]
mod tests {
    use hatoba_core::KdfParams;
    use hatoba_core::model::{AiProtocol, AiProvider};

    use super::*;

    fn vault() -> Vault {
        let mut vault = Vault::open_in_memory().unwrap();
        vault
            .create_with_params("correct horse battery staple", KdfParams::for_tests())
            .unwrap();
        vault
    }

    fn chatgpt_provider() -> Item {
        Item::AiProvider(AiProvider {
            name: PROVIDER_NAME.into(),
            protocol: AiProtocol::Responses,
            base_url: BASE_URL.into(),
            chatgpt: Some(AiChatgptAccount {
                client_id: "oaiapp_1".into(),
                subject: "user-1".into(),
                email: "user@example.com".into(),
            }),
            ..AiProvider::default()
        })
    }

    fn signed_in() -> Credentials {
        Credentials {
            client_id: "oaiapp_1".into(),
            subject: "user-1".into(),
            id_token: Zeroizing::new("id-token".into()),
            access_token: Zeroizing::new("access".into()),
            refresh_token: Zeroizing::new("refresh".into()),
            scopes: vec![chatgpt::PLAN_SCOPE.into()],
            ..Credentials::default()
        }
    }

    #[test]
    fn sessions_live_in_the_vault_for_providers_that_exist() {
        let shared: SharedVault = Arc::new(Mutex::new(vault()));
        let store = VaultStore {
            vault: RwLock::new(Arc::downgrade(&shared)),
        };
        let id = shared
            .lock()
            .unwrap()
            .put(None, chatgpt_provider())
            .unwrap();
        assert_eq!(store.load(&id).unwrap(), None);
        store.store(&id, Some(&signed_in())).unwrap();
        assert_eq!(store.load(&id).unwrap(), Some(signed_in()));
        assert_eq!(
            status(&shared.lock().unwrap(), &id),
            SessionStatus::SignedIn
        );
        // Nothing for a provider that does not exist.
        store.store("gone", Some(&signed_in())).unwrap();
        assert_eq!(store.load("gone").unwrap(), None);

        // The host id is made once and kept, also across a sign-out.
        let host = host_id(&mut shared.lock().unwrap()).unwrap();
        assert!(host.starts_with("urn:uuid:"));
        store.store(&id, None).unwrap();
        assert_eq!(host_id(&mut shared.lock().unwrap()).unwrap(), host);
        assert_eq!(
            status(&shared.lock().unwrap(), &id),
            SessionStatus::SignedOut
        );

        // A deleted provider's session goes with the next write.
        store.store(&id, Some(&signed_in())).unwrap();
        shared.lock().unwrap().delete(&id).unwrap();
        let other = shared
            .lock()
            .unwrap()
            .put(None, chatgpt_provider())
            .unwrap();
        store.store(&other, Some(&signed_in())).unwrap();
        assert_eq!(read(&shared.lock().unwrap()).unwrap().sessions.len(), 1);

        // Locked: nothing is read or written.
        shared.lock().unwrap().lock();
        assert!(matches!(store.load(&other), Err(AiError::SignIn(_))));
        assert!(store.store(&other, None).is_err());
    }

    #[test]
    fn a_sign_in_again_names_the_account_and_its_hints() {
        let mut v = vault();
        let id = v.put(None, chatgpt_provider()).unwrap();
        let fresh = sign_in_request(&mut v, None).unwrap();
        assert!(fresh.account.is_none() && !fresh.consent);

        let again = sign_in_request(&mut v, Some(&id)).unwrap();
        assert_eq!(again.host_id, fresh.host_id);
        let account = again.account.unwrap();
        assert_eq!(account.client_id, "oaiapp_1");
        assert_eq!(account.email, "user@example.com");
        assert!(account.id_token_hint.is_none(), "signed out here: no hint");

        // Signed in without plan use: the next sign-in asks for consent again.
        let mut no_plan = signed_in();
        no_plan.scopes = vec!["openid".into()];
        let mut sessions = read(&v).unwrap();
        sessions.sessions.insert(id.clone(), no_plan);
        write(&mut v, &sessions).unwrap();
        let again = sign_in_request(&mut v, Some(&id)).unwrap();
        assert!(again.consent);
        assert_eq!(
            again
                .account
                .unwrap()
                .id_token_hint
                .as_deref()
                .map(String::as_str),
            Some("id-token")
        );

        let key = v
            .put(
                None,
                Item::AiProvider(AiProvider {
                    name: "Key".into(),
                    ..AiProvider::default()
                }),
            )
            .unwrap();
        assert!(sign_in_request(&mut v, Some(&key)).is_err());
    }
}
