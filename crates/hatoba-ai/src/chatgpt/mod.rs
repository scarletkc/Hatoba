//! Sign in with ChatGPT (AI-52): a provider that uses the user's ChatGPT plan through OpenAI's
//! Responses API instead of an API key, following OpenAI's contract for open-source apps
//! (developers.openai.com/siwc/token-sharing-open-source).
//!
//! - [`oauth`]: the authorization code flow with PKCE (S256) in the system browser, with a
//!   loopback callback on `127.0.0.1` that listens before the browser opens. The first sign-in
//!   registers Hatoba with `client_id=dynamic_agent_client` and `agent_name_hint=Hatoba`; the
//!   issued client id is kept and reused, with this device's `ext_agent_host_id`. The ID token is
//!   checked against OpenAI's JWKS, and plan use needs the `chatgpt.tokens.use.direct` scope.
//!   Also the refresh and the revocation of a session.
//! - [`Sessions`]: the signed-in sessions of this device, read and written through a
//!   [`CredentialStore`] (the desktop shell seals them under the vault key and never syncs them).
//!   It hands out access tokens, refreshing one that is due. A refresh token is replaced by every
//!   refresh and reusing an old one revokes the session, so the refreshes of one session run one
//!   at a time: requests that need a token meanwhile (a turn, the judge of Auto mode, Compact)
//!   wait for the refresh in flight and use its token.
//! - Requests on this route (see [`ProviderConfig::chatgpt`](crate::provider::ProviderConfig)):
//!   always `stream: true` and `store: false`, without the fields the route refuses (Hatoba sends
//!   none of them anyway), with the function tools grouped in the [`NAMESPACE`] namespace. The
//!   route's error codes become the messages of [`explain`].
//!
//! Nothing here logs a token, an authorization URL (it may carry an ID token as a hint), a code or
//! a verifier (SEC-04).

pub mod oauth;
mod sessions;

#[cfg(test)]
mod tests;

use std::fmt;

use serde::{Deserialize, Serialize};
use url::Url;
use zeroize::Zeroizing;

pub use self::sessions::{ChatgptAuth, CredentialStore, Sessions};

/// The base URL of every ChatGPT provider: OpenAI's public API, which takes the plan's tokens on
/// `POST /responses` and `GET /models`.
pub const BASE_URL: &str = "https://api.openai.com/v1";

/// ChatGPT's settings page for the usage of the user's plan by apps.
pub const USAGE_URL: &str = "https://chatgpt.com/settings/usage";

/// The namespace that holds Hatoba's function tools on this route, which takes function tools
/// only in a namespace (or in `additional_tools` input items). Calls come back as
/// `function_call` items with the tool's own `name` and this `namespace`.
pub const NAMESPACE: &str = "hatoba";

/// What the model reads about [`NAMESPACE`].
pub const NAMESPACE_DESCRIPTION: &str =
    "Hatoba's tools: the user's terminal and SSH hosts, files, the web, skills and MCP servers.";

/// The scope that lets an access token use the ChatGPT plan.
pub const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";

/// OpenAI's sign-in endpoints. [`Endpoints::default`] is production; tests point them at a mock
/// server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    /// The ID token's required `iss`.
    pub issuer: String,
    /// Where the browser goes to sign in.
    pub authorize: Url,
    /// The token endpoint: code exchange and refresh.
    pub token: Url,
    /// The revocation endpoint (sign-out).
    pub revoke: Url,
    /// The keys that sign ID tokens.
    pub jwks: Url,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self::at("https://auth.openai.com").expect("the production issuer is a valid URL")
    }
}

impl Endpoints {
    /// The endpoints of an issuer at `issuer`, with OpenAI's paths.
    ///
    /// # Errors
    /// `issuer` is not a URL.
    pub fn at(issuer: &str) -> Result<Self, url::ParseError> {
        let issuer = issuer.trim_end_matches('/');
        let join = |path: &str| Url::parse(&format!("{issuer}{path}"));
        Ok(Self {
            issuer: issuer.to_owned(),
            authorize: join("/api/accounts/authorize")?,
            token: join("/api/accounts/oauth/token")?,
            revoke: join("/api/accounts/oauth/revoke")?,
            jwks: join("/.well-known/jwks.json")?,
        })
    }
}

/// A signed-in session of one ChatGPT account on this device. Device-local: it never leaves the
/// device and never syncs. Its tokens are wiped when dropped and never printed.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Credentials {
    /// The issued client id (`oaiapp_…`) these tokens belong to, which refreshes use.
    pub client_id: String,
    /// The account's verified `sub`.
    pub subject: String,
    /// The account's email, from the ID token; may be empty.
    pub email: String,
    /// The latest ID token, kept as `id_token_hint` for the next sign-in.
    pub id_token: Zeroizing<String>,
    /// Empty once the session has ended (a refresh said the refresh token is no longer usable).
    pub access_token: Zeroizing<String>,
    /// Empty once the session has ended.
    pub refresh_token: Zeroizing<String>,
    /// When the access token expires, Unix ms.
    pub expires_at: i64,
    /// The token response's `earliest_refresh_at`, Unix ms, when it gave one.
    pub refresh_after: Option<i64>,
    /// The granted scopes.
    pub scopes: Vec<String>,
    /// When these tokens arrived, Unix ms.
    pub saved_at: i64,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted = |s: &str| {
            if s.is_empty() {
                "<empty>"
            } else {
                "<redacted>"
            }
        };
        f.debug_struct("Credentials")
            .field("client_id", &self.client_id)
            .field("subject", &"<redacted>")
            .field("email", &"<redacted>")
            .field("id_token", &redacted(&self.id_token))
            .field("access_token", &redacted(&self.access_token))
            .field("refresh_token", &redacted(&self.refresh_token))
            .field("expires_at", &self.expires_at)
            .field("refresh_after", &self.refresh_after)
            .field("scopes", &self.scopes)
            .field("saved_at", &self.saved_at)
            .finish()
    }
}

impl Credentials {
    /// The granted scopes include [`PLAN_SCOPE`], so requests may use the plan.
    #[must_use]
    pub fn plan_enabled(&self) -> bool {
        self.scopes.iter().any(|s| s == PLAN_SCOPE)
    }

    /// The session ended: a refresh found its refresh token unusable. The account and the ID
    /// token stay for the next sign-in.
    #[must_use]
    pub fn ended(&self) -> bool {
        self.refresh_token.is_empty()
    }

    /// The same account without its tokens, except the ID token (a hint for the next sign-in).
    #[must_use]
    pub fn without_tokens(&self) -> Self {
        Self {
            access_token: Zeroizing::default(),
            refresh_token: Zeroizing::default(),
            expires_at: 0,
            refresh_after: None,
            ..self.clone()
        }
    }

    /// The state the provider settings show.
    #[must_use]
    pub fn status(&self) -> SessionStatus {
        if self.ended() {
            SessionStatus::Ended
        } else if self.plan_enabled() {
            SessionStatus::SignedIn
        } else {
            SessionStatus::PlanDisabled
        }
    }
}

/// Where a ChatGPT provider's session stands on this device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// Signed in, with plan use allowed.
    SignedIn,
    /// Signed in, but the user did not allow the plan's use (no [`PLAN_SCOPE`]).
    PlanDisabled,
    /// The session ended (signed out from ChatGPT, or the refresh token expired or was revoked).
    Ended,
    /// Never signed in on this device, or signed out here.
    SignedOut,
}

/// Unix ms now.
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Says what to do about a request the ChatGPT route refused with error `code` (the route's
/// `subscription_sharing_*` and `chatpass_v2_*` codes), or `None` for another code. `param` is
/// the error's `param`, which names what an unsupported capability is.
#[must_use]
pub fn explain(code: &str, param: Option<&str>) -> Option<String> {
    let param = param.map(str::trim).filter(|p| !p.is_empty());
    let text = match code {
        "subscription_sharing_usage_limit_exceeded" => format!(
            "You have reached a usage limit of your ChatGPT plan, for the plan or for Hatoba. \
             Check the limits under Usage in ChatGPT's settings: {USAGE_URL}"
        ),
        "subscription_sharing_usage_unavailable" => {
            "ChatGPT could not check your plan's usage right now. Try again in a few minutes."
                .to_owned()
        }
        "subscription_sharing_user_not_eligible" => {
            "Your ChatGPT account or workspace cannot use its plan in Hatoba. Use an API key \
             provider instead, or ask your workspace admin."
                .to_owned()
        }
        "subscription_sharing_unsupported_capability" => match param {
            Some(param) => format!("Requests with a ChatGPT plan do not support `{param}`."),
            None => "Requests with a ChatGPT plan do not support part of this request.".to_owned(),
        },
        "subscription_sharing_route_not_supported" => {
            "Requests with a ChatGPT plan do not support this endpoint.".to_owned()
        }
        "subscription_sharing_invalid_user" => {
            "ChatGPT could not verify your account. If it keeps happening, sign in again in \
             Settings → AI."
                .to_owned()
        }
        "chatpass_v2_scope_not_authorized" | "chatpass_v2_invalid_authorization_context" => {
            "Your ChatGPT sign-in does not allow this request. Sign in again in Settings → AI and \
             allow Hatoba to use your plan."
                .to_owned()
        }
        "subscription_sharing_user_unavailable" => {
            "Your ChatGPT account information is unavailable right now. Try again in a few minutes."
                .to_owned()
        }
        _ => return None,
    };
    Some(text)
}

/// The HTTP status a route error `code` stands for, for one that arrived mid-stream.
#[must_use]
pub(crate) fn status_of(code: &str) -> Option<u16> {
    Some(match code {
        "subscription_sharing_usage_limit_exceeded" => 429,
        "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable" => 503,
        "subscription_sharing_user_not_eligible"
        | "subscription_sharing_route_not_supported"
        | "chatpass_v2_scope_not_authorized"
        | "chatpass_v2_invalid_authorization_context" => 403,
        "subscription_sharing_unsupported_capability" => 400,
        "subscription_sharing_invalid_user" => 401,
        _ => return None,
    })
}

/// The message of a request this route refused before streaming, without a code it knows: an
/// admission refusal such as `{"detail": "…"}`.
pub(crate) fn admission_message(status: u16, message: &str) -> Option<String> {
    let text = match status {
        401 => format!(
            "ChatGPT did not accept the sign-in for this request ({message}). If it keeps \
             happening, sign in again in Settings → AI."
        ),
        403 => format!("ChatGPT refused to use your plan for this request ({message})."),
        503 => format!(
            "Using your ChatGPT plan is unavailable right now ({message}). Try again later."
        ),
        _ => return None,
    };
    Some(text)
}

/// Why a provider of this route sends no request: not signed in on this device, the session
/// ended, or plan use was not allowed.
pub(crate) fn sign_in_needed(status: SessionStatus) -> String {
    match status {
        SessionStatus::SignedIn => String::new(),
        SessionStatus::SignedOut => {
            "This ChatGPT provider is not signed in on this device. Choose Continue with ChatGPT \
             in Settings → AI."
                .to_owned()
        }
        SessionStatus::Ended => "Your ChatGPT sign-in has ended. Choose Continue with ChatGPT in \
             Settings → AI to sign in again."
            .to_owned(),
        SessionStatus::PlanDisabled => "This ChatGPT sign-in does not allow Hatoba to use your \
             plan. Choose Continue with ChatGPT in Settings → AI and allow it, or use an API key \
             provider."
            .to_owned(),
    }
}
