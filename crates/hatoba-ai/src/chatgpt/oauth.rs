//! The OAuth side of Sign in with ChatGPT: authorization with PKCE through the system browser and
//! a loopback callback, the code exchange, ID token validation, refresh and revocation.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::header::{CONTENT_TYPE, HeaderValue};
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use url::Url;
use zeroize::Zeroizing;

use super::{Credentials, Endpoints, now_ms};
use crate::error::AiError;
use crate::net;

/// The client id of a first sign-in, which registers Hatoba for the account. Never saved.
const DYNAMIC_CLIENT: &str = "dynamic_agent_client";
/// The name the first sign-in suggests for the registration (the user can change it).
const AGENT_NAME: &str = "Hatoba";
/// Identity scopes, then the scopes that let the tokens use the plan.
const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
/// The API the tokens are for.
pub(crate) const RESOURCE: &str = "https://api.openai.com/v1";
/// The callback's path; only the port may change between sign-ins.
const CALLBACK_PATH: &str = "/auth/callback";
/// The port a sign-in listens on when it is free, as in OpenAI's examples.
const PREFERRED_PORT: u16 = 1455;
/// How long a sign-in waits for the browser to come back.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// Longest a callback request may take to arrive whole.
const CALLBACK_READ_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest callback request read.
const MAX_CALLBACK_BYTES: usize = 16 * 1024;
/// Overall limit of a token, JWKS or revocation request.
const TOKEN_TIMEOUT: Duration = Duration::from_secs(30);
/// How far the clock may be off when the ID token's expiry is checked.
const CLOCK_SKEW_SECS: i64 = 60;
/// Attempts of a revocation that fails with a network error or a 5xx.
const REVOKE_ATTEMPTS: u32 = 3;

/// The refresh errors that mean the refresh token cannot be used again: sign in again.
const ENDED_CODES: &[&str] = &[
    "invalid_grant",
    "invalid_refresh_token",
    "token_expired",
    "refresh_token_expired",
    "refresh_token_invalidated",
    "refresh_token_reused",
];

/// `count` random bytes, base64url without padding.
fn random_b64(count: usize) -> Result<String, AiError> {
    let mut bytes = Zeroizing::new(vec![0u8; count]);
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| AiError::Config("the system has no random number generator".into()))?;
    Ok(URL_SAFE_NO_PAD.encode(&*bytes))
}

/// PKCE S256: base64url of the verifier's SHA-256, without padding.
pub(crate) fn code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(ring::digest::digest(
        &ring::digest::SHA256,
        verifier.as_bytes(),
    ))
}

/// A new `ext_agent_host_id`: a UUID v4 URN, generated once for a device and kept.
#[must_use]
pub fn new_host_id() -> String {
    let mut bytes = [0u8; 16];
    // Without a random generator the id is still unique enough to tell hosts apart.
    if SystemRandom::new().fill(&mut bytes).is_err() {
        bytes = now_ms()
            .to_le_bytes()
            .repeat(2)
            .try_into()
            .unwrap_or([7; 16]);
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "urn:uuid:{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// The account a sign-in is for, when it was signed in before (with this device or another).
#[derive(Clone, Debug, Default)]
pub struct Account {
    /// Its issued client id.
    pub client_id: String,
    /// Its verified `sub`; the new ID token must name the same account.
    pub subject: String,
    /// Its email, sent as `login_hint`.
    pub email: String,
    /// The last ID token this device has of it, sent as `id_token_hint`.
    pub id_token_hint: Option<Zeroizing<String>>,
}

/// What a sign-in asks for.
#[derive(Clone, Debug, Default)]
pub struct SignInRequest {
    /// This device's `ext_agent_host_id`.
    pub host_id: String,
    /// The account to sign in again; `None` registers Hatoba for a new account.
    pub account: Option<Account>,
    /// Ask for consent again (`prompt=consent`): the last sign-in did not allow the plan's use.
    pub consent: bool,
}

/// A sign-in waiting for the browser: the callback listens from [`SignIn::start`] on, before the
/// browser opens [`SignIn::url`].
pub struct SignIn {
    listener: TcpListener,
    url: Url,
    redirect_uri: String,
    state: String,
    nonce: String,
    verifier: Zeroizing<String>,
    account: Option<Account>,
    endpoints: Endpoints,
}

impl std::fmt::Debug for SignIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignIn")
            .field("redirect_uri", &self.redirect_uri)
            .finish_non_exhaustive()
    }
}

/// The parameters of the callback that answered the sign-in.
#[derive(Debug, Default)]
struct Callback {
    code: Zeroizing<String>,
    client_id: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

impl SignIn {
    /// Starts listening on `127.0.0.1` (port 1455 when free, else any) and builds the
    /// authorization URL with fresh `state`, `nonce` and PKCE values.
    ///
    /// # Errors
    /// No loopback port could be opened, or no random numbers.
    pub async fn start(endpoints: &Endpoints, request: &SignInRequest) -> Result<Self, AiError> {
        let listener = match TcpListener::bind(("127.0.0.1", PREFERRED_PORT)).await {
            Ok(listener) => listener,
            Err(_) => TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| {
                AiError::Network(format!("could not listen for the sign-in callback: {e}"))
            })?,
        };
        let port = listener
            .local_addr()
            .map_err(|e| AiError::Network(format!("could not listen for the sign-in: {e}")))?
            .port();
        let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
        let state = random_b64(32)?;
        let nonce = random_b64(32)?;
        let verifier = Zeroizing::new(random_b64(32)?);
        let account = request.account.clone();

        let mut url = endpoints.authorize.clone();
        {
            let mut query = url.query_pairs_mut();
            match &account {
                Some(account) => {
                    query.append_pair("client_id", &account.client_id);
                    if let Some(hint) = &account.id_token_hint {
                        query.append_pair("id_token_hint", hint);
                    }
                    if !account.email.is_empty() {
                        query.append_pair("login_hint", &account.email);
                    }
                }
                None => {
                    query.append_pair("client_id", DYNAMIC_CLIENT);
                    query.append_pair("agent_name_hint", AGENT_NAME);
                }
            }
            query
                .append_pair("ext_agent_host_id", &request.host_id)
                .append_pair("response_type", "code")
                .append_pair("redirect_uri", &redirect_uri)
                .append_pair("scope", SCOPES)
                .append_pair("resource", RESOURCE)
                .append_pair("state", &state)
                .append_pair("nonce", &nonce)
                .append_pair("code_challenge_method", "S256")
                .append_pair("code_challenge", &code_challenge(&verifier));
            if request.consent {
                query.append_pair("prompt", "consent");
            }
        }
        Ok(Self {
            listener,
            url,
            redirect_uri,
            state,
            nonce,
            verifier,
            account,
            endpoints: endpoints.clone(),
        })
    }

    /// The page to open in the system browser. It may carry the account's ID token as a hint:
    /// never log it.
    #[must_use]
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// The callback URI this sign-in listens on.
    #[must_use]
    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    /// Waits for the browser to come back (at most 15 minutes), then exchanges the code and
    /// checks the ID token. A callback with another `state` (a page left over from an earlier
    /// attempt) is answered and ignored.
    ///
    /// # Errors
    /// [`AiError::SignIn`] for a refused or failed sign-in, [`AiError::Cancelled`] when `cancel`
    /// fires, network and HTTP errors of the exchange.
    pub async fn finish(
        self,
        http: &reqwest::Client,
        cancel: &CancellationToken,
    ) -> Result<Credentials, AiError> {
        let callback = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(AiError::Cancelled),
            result = tokio::time::timeout(SIGN_IN_TIMEOUT, self.callback()) => result
                .unwrap_or_else(|_| Err(AiError::SignIn(
                    "The sign-in timed out. Choose Continue with ChatGPT to try again.".into(),
                )))?,
        };
        if let Some(error) = &callback.error {
            return Err(AiError::SignIn(match error.as_str() {
                "access_denied" => "The sign-in was cancelled in the browser.".to_owned(),
                _ => format!(
                    "ChatGPT refused the sign-in: {}",
                    net::clean_message(callback.error_description.as_deref().unwrap_or(error))
                ),
            }));
        }
        let client_id = match (&self.account, &callback.client_id) {
            (Some(account), Some(returned)) if *returned != account.client_id => {
                return Err(AiError::SignIn(
                    "ChatGPT answered for another registration of Hatoba than this provider's. \
                     Sign in again."
                        .into(),
                ));
            }
            (Some(account), _) => account.client_id.clone(),
            (None, Some(issued)) if !issued.is_empty() && issued != DYNAMIC_CLIENT => {
                issued.clone()
            }
            (None, _) => {
                return Err(AiError::SignIn(
                    "ChatGPT did not finish registering Hatoba. Choose Continue with ChatGPT to \
                     try again."
                        .into(),
                ));
            }
        };
        let form = [
            ("grant_type", "authorization_code"),
            ("client_id", &client_id),
            ("code", &callback.code),
            ("code_verifier", &self.verifier),
            ("redirect_uri", &self.redirect_uri),
            ("resource", RESOURCE),
        ];
        let body = match post_form(http, &self.endpoints.token, &form, cancel).await? {
            Ok(body) => body,
            Err(failure) if failure.code.as_deref() == Some("invalid_grant") => {
                return Err(AiError::SignIn(
                    "The sign-in code was refused or has expired. Choose Continue with ChatGPT to \
                     try again."
                        .into(),
                ));
            }
            Err(failure) => return Err(failure.into_error("the sign-in")),
        };
        let tokens = TokenResponse::parse(&body)?;
        let claims = match self.verify(http, &tokens, &client_id, cancel).await {
            Ok(claims) => claims,
            Err(err) => {
                // The tokens were issued but are not kept: end them at OpenAI.
                if let Some(refresh_token) = tokens.refresh_token {
                    let issued = Credentials {
                        client_id,
                        refresh_token,
                        ..Credentials::default()
                    };
                    let (http, endpoints) = (http.clone(), self.endpoints.clone());
                    tokio::spawn(async move {
                        revoke(&http, &endpoints, &issued, &CancellationToken::new()).await;
                    });
                }
                return Err(err);
            }
        };
        if tokens.refresh_token.is_none() {
            return Err(AiError::SignIn(
                "ChatGPT's answer to the sign-in has no refresh token.".into(),
            ));
        }
        let now = now_ms();
        Ok(tokens.into_credentials(
            Credentials {
                client_id,
                subject: claims.subject,
                email: claims.email,
                ..Credentials::default()
            },
            now,
        ))
    }

    /// Checks the ID token of the code exchange's answer, and that it names the provider's
    /// account on a sign-in again.
    async fn verify(
        &self,
        http: &reqwest::Client,
        tokens: &TokenResponse,
        client_id: &str,
        cancel: &CancellationToken,
    ) -> Result<IdClaims, AiError> {
        let id_token = tokens.id_token.as_ref().ok_or_else(|| {
            AiError::SignIn("ChatGPT's answer to the sign-in has no ID token.".into())
        })?;
        let jwks = get_json(http, &self.endpoints.jwks, cancel).await?;
        let claims = verify_id_token(
            id_token,
            &jwks,
            &self.endpoints.issuer,
            client_id,
            &self.nonce,
            now_ms() / 1000,
        )?;
        if let Some(account) = &self.account
            && !account.subject.is_empty()
            && account.subject != claims.subject
        {
            return Err(AiError::SignIn(
                "You signed in with another ChatGPT account than this provider's. Sign in with \
                 the same account, or add a provider for the other one."
                    .into(),
            ));
        }
        Ok(claims)
    }

    /// Accepts connections until one carries this sign-in's callback.
    async fn callback(&self) -> Result<Callback, AiError> {
        let (sender, mut receiver) = mpsc::channel(1);
        let state: Arc<str> = Arc::from(self.state.as_str());
        let mut handlers = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                biased;
                Some(callback) = receiver.recv() => return Ok(callback),
                accepted = self.listener.accept() => {
                    let (stream, _) = accepted.map_err(|e| {
                        AiError::Network(format!("the sign-in callback failed: {e}"))
                    })?;
                    let (sender, state) = (sender.clone(), Arc::clone(&state));
                    // Browsers open spare connections that send nothing; each is read on its own.
                    handlers.spawn(async move {
                        if let Ok(Some(callback)) = tokio::time::timeout(
                            CALLBACK_READ_TIMEOUT,
                            serve_callback(stream, &state),
                        )
                        .await
                        {
                            let _ = sender.send(callback).await;
                        }
                    });
                }
            }
        }
    }
}

/// The page the browser shows once it has come back.
fn page(status: &str, title: &str, text: &str) -> String {
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title>\
         <style>body{{font-family:system-ui,sans-serif;max-width:32rem;margin:15vh auto;\
         padding:0 1rem;line-height:1.5}}</style></head>\
         <body><h1>{title}</h1><p>{text}</p></body></html>"
    );
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Reads one request: the callback when it carries `state`, answered with a page either way.
async fn serve_callback(mut stream: TcpStream, state: &str) -> Option<Callback> {
    let mut request = Zeroizing::new(Vec::new());
    let mut chunk = [0u8; 2048];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 || request.len() + read > MAX_CALLBACK_BYTES {
            return None;
        }
        request.extend_from_slice(&chunk[..read]);
    }
    let line = request.split(|b| *b == b'\n').next()?;
    let line = std::str::from_utf8(line).ok()?.trim_end();
    let mut parts = line.split(' ');
    let (method, target) = (parts.next()?, parts.next()?);
    let url = Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
    let (answer, callback) = if method != "GET" || url.path() != CALLBACK_PATH {
        (page("404 Not Found", "Not found", "Nothing is here."), None)
    } else {
        let params: HashMap<String, Zeroizing<String>> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), Zeroizing::new(v.into_owned())))
            .collect();
        let value = |key: &str| params.get(key).map(|v| v.as_str().to_owned());
        if value("state").as_deref() != Some(state) {
            (
                page(
                    "400 Bad Request",
                    "This sign-in page is out of date",
                    "Go back to Hatoba and choose Continue with ChatGPT again.",
                ),
                None,
            )
        } else if let Some(error) = value("error") {
            (
                page(
                    "200 OK",
                    "Sign-in cancelled",
                    "You can close this tab and go back to Hatoba.",
                ),
                Some(Callback {
                    error: Some(error),
                    error_description: value("error_description"),
                    ..Callback::default()
                }),
            )
        } else if let Some(code) = params.get("code").filter(|c| !c.is_empty()) {
            (
                page(
                    "200 OK",
                    "Signed in",
                    "You can close this tab and go back to Hatoba.",
                ),
                Some(Callback {
                    code: code.clone(),
                    client_id: value("client_id").filter(|c| !c.is_empty()),
                    ..Callback::default()
                }),
            )
        } else {
            (
                page(
                    "400 Bad Request",
                    "Sign-in failed",
                    "ChatGPT sent no sign-in code. Go back to Hatoba and try again.",
                ),
                Some(Callback {
                    error: Some("no_code".into()),
                    error_description: Some("no authorization code came back".into()),
                    ..Callback::default()
                }),
            )
        }
    };
    let _ = stream.write_all(answer.as_bytes()).await;
    let _ = stream.shutdown().await;
    callback
}

/// A token endpoint response.
struct TokenResponse {
    access_token: Zeroizing<String>,
    refresh_token: Option<Zeroizing<String>>,
    id_token: Option<Zeroizing<String>>,
    expires_in: Option<i64>,
    scopes: Option<Vec<String>>,
    refresh_after: Option<i64>,
}

impl TokenResponse {
    fn parse(body: &Value) -> Result<Self, AiError> {
        let text = |key: &str| {
            body.get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(|s| Zeroizing::new(s.to_owned()))
        };
        let access_token = text("access_token")
            .ok_or_else(|| AiError::Protocol("the token response has no access token".into()))?;
        Ok(Self {
            access_token,
            refresh_token: text("refresh_token"),
            id_token: text("id_token"),
            expires_in: body.get("expires_in").and_then(Value::as_i64),
            scopes: body
                .get("scope")
                .and_then(Value::as_str)
                .map(|s| s.split_whitespace().map(str::to_owned).collect()),
            refresh_after: body.get("earliest_refresh_at").and_then(unix_ms),
        })
    }

    /// `base` with these tokens: a refresh keeps what the response leaves out.
    fn into_credentials(self, base: Credentials, now: i64) -> Credentials {
        let expires_in = self.expires_in.filter(|s| *s > 0).unwrap_or(3600);
        Credentials {
            access_token: self.access_token,
            refresh_token: self.refresh_token.unwrap_or(base.refresh_token),
            id_token: self.id_token.unwrap_or(base.id_token),
            expires_at: now.saturating_add(expires_in.saturating_mul(1000)),
            refresh_after: self.refresh_after,
            scopes: self.scopes.unwrap_or(base.scopes),
            saved_at: now,
            client_id: base.client_id,
            subject: base.subject,
            email: base.email,
        }
    }
}

/// A time as Unix ms: seconds or milliseconds as a number, or an RFC 3339 UTC time.
fn unix_ms(value: &Value) -> Option<i64> {
    if let Some(n) = value.as_i64() {
        return Some(if n < 100_000_000_000 { n * 1000 } else { n });
    }
    if let Some(f) = value.as_f64() {
        #[allow(clippy::cast_possible_truncation)]
        return Some((f * 1000.0) as i64);
    }
    parse_rfc3339_utc(value.as_str()?)
}

/// `YYYY-MM-DDTHH:MM:SS[.fff]Z` as Unix ms. Other forms are left out.
fn parse_rfc3339_utc(text: &str) -> Option<i64> {
    let text = text.strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut date = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (date.next()??, date.next()??, date.next()??);
    let time = time.split('.').next()?;
    let mut time = time.split(':').map(|p| p.parse::<i64>().ok());
    let (h, min, s) = (time.next()??, time.next()??, time.next()??);
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400) + h * 3600 + min * 60 + s) * 1000)
}

/// A refused token or revocation request.
#[derive(Debug)]
pub(crate) struct OAuthFailure {
    status: u16,
    pub(crate) code: Option<String>,
    message: String,
}

impl OAuthFailure {
    fn into_error(self, what: &str) -> AiError {
        AiError::SignIn(format!(
            "ChatGPT refused {what} (HTTP {}{}): {}",
            self.status,
            self.code.map(|c| format!(", {c}")).unwrap_or_default(),
            self.message
        ))
    }
}

/// Posts a form; a 2xx is its JSON body (`Null` when empty), anything else an [`OAuthFailure`].
async fn post_form(
    http: &reqwest::Client,
    url: &Url,
    form: &[(&str, &str)],
    cancel: &CancellationToken,
) -> Result<Result<Value, OAuthFailure>, AiError> {
    let body = Zeroizing::new(
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish(),
    );
    let request = http
        .post(url.clone())
        .header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        )
        .timeout(TOKEN_TIMEOUT)
        .body(body.as_bytes().to_vec());
    let response = net::send(request, cancel).await?;
    let status = response.status();
    let (bytes, _) = net::read_capped(response, 1024 * 1024, cancel).await?;
    let bytes = Zeroizing::new(bytes);
    let value: Value = if bytes.iter().all(u8::is_ascii_whitespace) {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    if status.is_success() {
        return Ok(Ok(value));
    }
    let code = value
        .get("error")
        .and_then(|e| e.as_str().or_else(|| e.get("code").and_then(Value::as_str)))
        .or_else(|| value.get("code").and_then(Value::as_str))
        .map(str::to_owned);
    let message = value
        .get("error_description")
        .and_then(Value::as_str)
        .map(net::clean_message)
        .or_else(|| net::message_of(&value))
        .unwrap_or_else(|| status.canonical_reason().unwrap_or("refused").to_owned());
    Ok(Err(OAuthFailure {
        status: status.as_u16(),
        code,
        message,
    }))
}

async fn get_json(
    http: &reqwest::Client,
    url: &Url,
    cancel: &CancellationToken,
) -> Result<Value, AiError> {
    let response = net::send(http.get(url.clone()).timeout(TOKEN_TIMEOUT), cancel).await?;
    if !response.status().is_success() {
        return Err(net::http_error(response, cancel).await);
    }
    net::read_json(response, cancel).await
}

/// What a valid ID token says about the account.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct IdClaims {
    pub(crate) subject: String,
    pub(crate) email: String,
}

fn b64_json(part: &str) -> Option<Value> {
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part.trim_end_matches('=')).ok()?).ok()
}

/// Checks an ID token: an RS256 signature by a key of `jwks`, the issuer, the audience (the
/// issued client id), the expiry (with a minute of clock skew), the nonce and a subject.
pub(crate) fn verify_id_token(
    token: &str,
    jwks: &Value,
    issuer: &str,
    client_id: &str,
    nonce: &str,
    now_secs: i64,
) -> Result<IdClaims, AiError> {
    let invalid = |why: &str| AiError::SignIn(format!("ChatGPT's ID token is not valid: {why}."));
    let mut parts = token.split('.');
    let (Some(header), Some(payload), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(invalid("it is not a JWT"));
    };
    let header = b64_json(header).ok_or_else(|| invalid("its header cannot be read"))?;
    if header.get("alg").and_then(Value::as_str) != Some("RS256") {
        return Err(invalid("it is not signed with RS256"));
    }
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| invalid("its signature cannot be read"))?;
    let signed = &token[..header_and_payload_len(token)];
    let kid = header.get("kid").and_then(Value::as_str);
    let keys = jwks
        .get("keys")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let verified = keys
        .iter()
        .filter(|key| key.get("kty").and_then(Value::as_str) == Some("RSA"))
        .filter(|key| kid.is_none() || key.get("kid").and_then(Value::as_str) == kid)
        .any(|key| {
            let component = |name: &str| {
                key.get(name)
                    .and_then(Value::as_str)
                    .and_then(|v| URL_SAFE_NO_PAD.decode(v).ok())
            };
            let (Some(n), Some(e)) = (component("n"), component("e")) else {
                return false;
            };
            RsaPublicKeyComponents { n, e }
                .verify(&RSA_PKCS1_2048_8192_SHA256, signed.as_bytes(), &signature)
                .is_ok()
        });
    if !verified {
        return Err(invalid("its signature does not match OpenAI's keys"));
    }
    let claims = b64_json(payload).ok_or_else(|| invalid("its claims cannot be read"))?;
    if claims.get("iss").and_then(Value::as_str) != Some(issuer) {
        return Err(invalid("it is from another issuer"));
    }
    let audience_ok = match claims.get("aud") {
        Some(Value::String(aud)) => aud == client_id,
        Some(Value::Array(auds)) => auds.iter().any(|a| a.as_str() == Some(client_id)),
        _ => false,
    };
    if !audience_ok {
        return Err(invalid("it is for another client"));
    }
    match claims.get("exp").and_then(Value::as_i64) {
        Some(exp) if exp + CLOCK_SKEW_SECS >= now_secs => {}
        _ => return Err(invalid("it has expired; check this device's clock")),
    }
    if claims.get("nonce").and_then(Value::as_str) != Some(nonce) {
        return Err(invalid("its nonce does not match this sign-in"));
    }
    let subject = claims
        .get("sub")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("it names no account"))?
        .to_owned();
    let email = claims
        .get("email")
        .or_else(|| claims.pointer("/https:~1~1api.openai.com~1profile/email"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Ok(IdClaims { subject, email })
}

/// The length of `header.payload`, the part of a JWT its signature covers.
fn header_and_payload_len(token: &str) -> usize {
    token.rfind('.').unwrap_or(token.len())
}

/// Why a refresh failed.
#[derive(Debug)]
pub(crate) enum RefreshError {
    /// The refresh token cannot be used again (expired, revoked, reused): sign in again.
    Ended(String),
    /// Anything else, such as a network failure or a 5xx: the session may still work later.
    Failed(AiError),
}

/// Refreshes a session: a new access token and a replacement refresh token. What the response
/// leaves out (the ID token, the scopes) stays as it was.
pub(crate) async fn refresh(
    http: &reqwest::Client,
    endpoints: &Endpoints,
    credentials: &Credentials,
    cancel: &CancellationToken,
) -> Result<Credentials, RefreshError> {
    let form = [
        ("grant_type", "refresh_token"),
        ("client_id", credentials.client_id.as_str()),
        ("refresh_token", credentials.refresh_token.as_str()),
        ("resource", RESOURCE),
    ];
    let body = match post_form(http, &endpoints.token, &form, cancel).await {
        Ok(Ok(body)) => body,
        Ok(Err(failure)) => {
            let code = failure.code.clone().unwrap_or_default();
            if ENDED_CODES.contains(&code.as_str()) {
                return Err(RefreshError::Ended(code));
            }
            if code == "invalid_client" {
                return Err(RefreshError::Ended(code));
            }
            return Err(RefreshError::Failed(AiError::Http {
                status: failure.status,
                message: format!(
                    "ChatGPT could not renew the sign-in ({}): {}",
                    if code.is_empty() { "no code" } else { &code },
                    failure.message
                ),
            }));
        }
        Err(err) => return Err(RefreshError::Failed(err)),
    };
    let tokens = TokenResponse::parse(&body).map_err(RefreshError::Failed)?;
    Ok(tokens.into_credentials(credentials.clone(), now_ms()))
}

/// Ends a session at OpenAI: revokes its refresh token, retrying a network failure or a 5xx with
/// backoff. Returns whether OpenAI confirmed it (an empty 200, also for a token already invalid).
pub async fn revoke(
    http: &reqwest::Client,
    endpoints: &Endpoints,
    credentials: &Credentials,
    cancel: &CancellationToken,
) -> bool {
    if credentials.refresh_token.is_empty() {
        return true;
    }
    let form = [
        ("token", credentials.refresh_token.as_str()),
        ("token_type_hint", "refresh_token"),
        ("client_id", credentials.client_id.as_str()),
    ];
    for attempt in 1..=REVOKE_ATTEMPTS {
        let retry = match post_form(http, &endpoints.revoke, &form, cancel).await {
            Ok(Ok(_)) => return true,
            Ok(Err(failure)) => failure.status >= 500,
            Err(AiError::Cancelled) => return false,
            Err(_) => true,
        };
        if !retry || attempt == REVOKE_ATTEMPTS {
            break;
        }
        let wait = Duration::from_millis(500 * 2u64.pow(attempt - 1));
        tokio::select! {
            biased;
            () = cancel.cancelled() => return false,
            () = tokio::time::sleep(wait) => {}
        }
    }
    false
}
