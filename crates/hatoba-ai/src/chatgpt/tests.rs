//! Sign in with ChatGPT against a mock of OpenAI's sign-in and API endpoints: the sign-in and
//! code exchange, the ID token checks, refresh rotation with one refresh at a time, sessions that
//! expired or were revoked, the route's errors, namespaced tools, and the requests that stream on
//! this route (the judge, Test Connection, the model list).

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::rand::SystemRandom;
use ring::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};
use zeroize::Zeroizing;

use super::oauth::{self, Account, SignIn, SignInRequest, code_challenge};
use super::*;
use crate::chat::{ChatRequest, StreamEvent, ToolDef, stream_chat};
use crate::decision::{self, Answer, Question};
use crate::entry::{AiEntry, AssistantEntry, Finish, ToolCall};
use crate::error::AiError;
use crate::models::{self, TestFailure};
use crate::provider::{AuthHeader, ModelSpec, Protocol, ProviderConfig, http_client};

const KEY_PK8: &[u8] = include_bytes!("../../tests/fixtures/chatgpt_test_rsa.pk8");
const HOST_ID: &str = "urn:uuid:7d444840-9dc0-41d1-b245-6e5a0e5f4b7e";
const CLIENT: &str = "oaiapp_test123";
const SUBJECT: &str = "user-abc";
const PROVIDER: &str = "0192aaaa-bbbb-7ccc-8ddd-0000000000c1";

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

fn b64(value: &Value) -> String {
    URL_SAFE_NO_PAD.encode(value.to_string())
}

/// The test key's JWKS.
fn jwks() -> Value {
    let key = RsaKeyPair::from_pkcs8(KEY_PK8).unwrap();
    let public = ring::rsa::PublicKeyComponents::<Vec<u8>>::from(key.public());
    json!({"keys": [{
        "kty": "RSA", "kid": "test-key", "use": "sig", "alg": "RS256",
        "n": URL_SAFE_NO_PAD.encode(&public.n), "e": URL_SAFE_NO_PAD.encode(&public.e)
    }]})
}

/// A JWT signed with the test key.
fn signed(claims: &Value) -> String {
    let key = RsaKeyPair::from_pkcs8(KEY_PK8).unwrap();
    let input = format!(
        "{}.{}",
        b64(&json!({"alg": "RS256", "kid": "test-key", "typ": "JWT"})),
        b64(claims)
    );
    let mut signature = vec![0u8; key.public().modulus_len()];
    key.sign(
        &RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        input.as_bytes(),
        &mut signature,
    )
    .unwrap();
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature))
}

fn now_secs() -> i64 {
    now_ms() / 1000
}

fn id_claims(issuer: &str, nonce: &str, subject: &str) -> Value {
    json!({
        "iss": issuer, "aud": CLIENT, "sub": subject, "email": "user@example.com",
        "nonce": nonce, "iat": now_secs(), "exp": now_secs() + 3600
    })
}

/// Sessions in memory; `locked` makes every read and write fail, as a locked vault does.
#[derive(Default)]
struct MemoryStore {
    sessions: Mutex<HashMap<String, Credentials>>,
    locked: AtomicBool,
}

impl MemoryStore {
    fn get(&self, id: &str) -> Option<Credentials> {
        self.sessions.lock().unwrap().get(id).cloned()
    }
}

impl CredentialStore for MemoryStore {
    fn load(&self, id: &str) -> Result<Option<Credentials>, AiError> {
        if self.locked.load(Ordering::SeqCst) {
            return Err(AiError::Config("locked".into()));
        }
        Ok(self.get(id))
    }

    fn store(&self, id: &str, credentials: Option<&Credentials>) -> Result<(), AiError> {
        if self.locked.load(Ordering::SeqCst) {
            return Err(AiError::Config("locked".into()));
        }
        let mut sessions = self.sessions.lock().unwrap();
        match credentials {
            Some(c) => sessions.insert(id.to_owned(), c.clone()),
            None => sessions.remove(id),
        };
        Ok(())
    }
}

fn credentials(access: &str, refresh: &str, expires_in_ms: i64) -> Credentials {
    Credentials {
        client_id: CLIENT.into(),
        subject: SUBJECT.into(),
        email: "user@example.com".into(),
        id_token: Zeroizing::new("old-id-token".into()),
        access_token: Zeroizing::new(access.into()),
        refresh_token: Zeroizing::new(refresh.into()),
        expires_at: now_ms() + expires_in_ms,
        refresh_after: None,
        scopes: vec!["openid".into(), PLAN_SCOPE.into()],
        saved_at: now_ms(),
    }
}

struct Setup {
    server: MockServer,
    store: Arc<MemoryStore>,
    sessions: Sessions,
}

async fn setup(initial: Option<Credentials>) -> Setup {
    let server = MockServer::start().await;
    let store = Arc::new(MemoryStore::default());
    if let Some(c) = initial {
        store.store(PROVIDER, Some(&c)).unwrap();
    }
    let endpoints = Endpoints::at(&server.uri()).unwrap();
    let sessions = Sessions::new(http_client(), endpoints, Arc::clone(&store) as _);
    Setup {
        server,
        store,
        sessions,
    }
}

impl Setup {
    fn provider(&self) -> ProviderConfig {
        ProviderConfig {
            protocol: Protocol::Responses,
            base_url: format!("{}/v1", self.server.uri()),
            api_key: Zeroizing::default(),
            auth_header: AuthHeader::XApiKey,
            chatgpt: Some(ChatgptAuth {
                sessions: self.sessions.clone(),
                id: PROVIDER.into(),
            }),
        }
    }

    /// The token endpoint swaps refresh token `from` for `access` and `to`, once.
    async fn refresh_once(&self, from: &str, access: &str, to: &str, delay: Duration) {
        Mock::given(method("POST"))
            .and(path("/api/accounts/oauth/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains(format!("refresh_token={from}")))
            .and(body_string_contains(format!("client_id={CLIENT}")))
            .and(body_string_contains("resource=https%3A%2F%2Fapi.openai.com%2Fv1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(delay)
                    .set_body_json(json!({
                        "access_token": access, "refresh_token": to, "token_type": "Bearer",
                        "expires_in": 3600,
                        "scope": "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct"
                    })),
            )
            .expect(1)
            .mount(&self.server)
            .await;
    }
}

fn sse(events: &[(&str, Value)]) -> ResponseTemplate {
    let body: String = events
        .iter()
        .map(|(name, data)| format!("event: {name}\ndata: {data}\n\n"))
        .collect();
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body)
}

fn completed(output: &Value) -> (&'static str, Value) {
    (
        "response.completed",
        json!({"type": "response.completed", "response": {
            "status": "completed", "output": output,
            "usage": {"input_tokens": 10, "output_tokens": 2}
        }}),
    )
}

fn hello_stream() -> ResponseTemplate {
    let message = json!({"type": "message", "id": "msg_1", "role": "assistant",
        "content": [{"type": "output_text", "text": "Hello", "annotations": []}]});
    sse(&[
        (
            "response.output_item.done",
            json!({"type": "response.output_item.done", "output_index": 0, "item": message}),
        ),
        completed(&json!([message])),
    ])
}

fn tool(name: &str) -> ToolDef {
    ToolDef {
        name: name.into(),
        description: format!("The {name} tool."),
        input_schema: json!({"type": "object", "properties": {}}),
    }
}

async fn chat(
    provider: &ProviderConfig,
    model: &ModelSpec,
    entries: &[AiEntry],
    tools: &[ToolDef],
) -> (Result<AssistantEntry, AiError>, Vec<StreamEvent>) {
    let req = ChatRequest {
        provider_id: PROVIDER,
        model,
        system: "SYS",
        tools,
        entries,
        effort: None,
    };
    let mut events = Vec::new();
    let result = stream_chat(
        &http_client(),
        provider,
        &req,
        &CancellationToken::new(),
        &mut |e| events.push(e),
    )
    .await;
    (result, events)
}

async fn bodies(server: &MockServer, at: &str) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == at)
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Sign-in
// ---------------------------------------------------------------------------------------------

fn params(url: &url::Url) -> HashMap<String, String> {
    url.query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

/// The browser's visit to the callback; returns the page's status line.
async fn visit(redirect_uri: &str, query: &[(&str, &str)]) -> String {
    let mut url = url::Url::parse(redirect_uri).unwrap();
    url.query_pairs_mut().extend_pairs(query);
    let response = reqwest::Client::new().get(url).send().await.unwrap();
    response.status().as_u16().to_string()
}

/// Mounts the token endpoint's answer to the code exchange, with an ID token for `nonce`.
async fn mount_exchange(server: &MockServer, verifier_check: String, claims: Value) {
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jwks()))
        .mount(server)
        .await;
    let id_token = signed(&claims);
    Mock::given(method("POST"))
        .and(path("/api/accounts/oauth/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains(format!("client_id={CLIENT}")))
        .and(body_string_contains("code=the-code"))
        .and(body_string_contains(
            "resource=https%3A%2F%2Fapi.openai.com%2Fv1",
        ))
        .and(body_string_contains(
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A",
        ))
        .and(move |request: &Request| {
            let body = String::from_utf8_lossy(&request.body);
            let verifier = url::form_urlencoded::parse(body.as_bytes())
                .find(|(k, _)| k == "code_verifier")
                .map(|(_, v)| v.into_owned())
                .unwrap_or_default();
            code_challenge(&verifier) == verifier_check
        })
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "access-1", "refresh_token": "refresh-1", "id_token": id_token,
            "token_type": "Bearer", "expires_in": 3600, "earliest_refresh_at": now_secs() + 60,
            "scope": "chatgpt.tokens.use.direct email offline_access openid profile resource.invoke"
        })))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn first_sign_in_registers_hatoba_and_exchanges_the_code() {
    let server = MockServer::start().await;
    let endpoints = Endpoints::at(&server.uri()).unwrap();
    let sign_in = SignIn::start(
        &endpoints,
        &SignInRequest {
            host_id: HOST_ID.into(),
            account: None,
            consent: false,
        },
    )
    .await
    .unwrap();
    let url = sign_in.url().clone();
    let query = params(&url);
    assert_eq!(url.path(), "/api/accounts/authorize");
    assert_eq!(query["client_id"], "dynamic_agent_client");
    assert_eq!(query["agent_name_hint"], "Hatoba");
    assert_eq!(query["ext_agent_host_id"], HOST_ID);
    assert_eq!(query["response_type"], "code");
    assert_eq!(
        query["scope"],
        "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct"
    );
    assert_eq!(query["resource"], "https://api.openai.com/v1");
    assert_eq!(query["code_challenge_method"], "S256");
    assert!(
        query["redirect_uri"].starts_with("http://127.0.0.1:")
            && query["redirect_uri"].ends_with("/auth/callback"),
        "{}",
        query["redirect_uri"]
    );
    assert_eq!(query["redirect_uri"], sign_in.redirect_uri());
    for absent in ["id_token_hint", "login_hint", "prompt"] {
        assert!(!query.contains_key(absent), "{absent}");
    }
    assert!(query["state"].len() >= 43 && query["nonce"].len() >= 43);
    assert_ne!(query["state"], query["nonce"]);

    mount_exchange(
        &server,
        query["code_challenge"].clone(),
        id_claims(&server.uri(), &query["nonce"], SUBJECT),
    )
    .await;
    let redirect = sign_in.redirect_uri().to_owned();
    let finish = tokio::spawn(async move {
        sign_in
            .finish(&http_client(), &CancellationToken::new())
            .await
    });
    // A page left from an earlier attempt and a stray request are answered and ignored.
    assert_eq!(
        visit(&redirect, &[("code", "old"), ("state", "stale")]).await,
        "400"
    );
    assert_eq!(
        visit(&redirect.replace("/auth/callback", "/favicon.ico"), &[]).await,
        "404"
    );
    let state = query["state"].as_str();
    assert_eq!(
        visit(
            &redirect,
            &[
                ("code", "the-code"),
                ("state", state),
                ("client_id", CLIENT),
                ("scope", "openid chatgpt.tokens.use.direct"),
            ]
        )
        .await,
        "200"
    );
    let signed_in = finish.await.unwrap().unwrap();
    assert_eq!(signed_in.client_id, CLIENT);
    assert_eq!(signed_in.subject, SUBJECT);
    assert_eq!(signed_in.email, "user@example.com");
    assert_eq!(&*signed_in.access_token, "access-1");
    assert_eq!(&*signed_in.refresh_token, "refresh-1");
    assert!(signed_in.plan_enabled());
    assert_eq!(signed_in.status(), SessionStatus::SignedIn);
    assert!(signed_in.expires_at > now_ms() + 3_500_000);
    assert!(signed_in.refresh_after.unwrap() > now_ms());
    let debug = format!("{signed_in:?}");
    assert!(
        !debug.contains("access-1") && !debug.contains("refresh-1"),
        "{debug}"
    );
}

#[tokio::test]
async fn signing_in_again_reuses_the_client_and_refuses_another_account() {
    let server = MockServer::start().await;
    let endpoints = Endpoints::at(&server.uri()).unwrap();
    let request = SignInRequest {
        host_id: HOST_ID.into(),
        account: Some(Account {
            client_id: CLIENT.into(),
            subject: SUBJECT.into(),
            email: "user@example.com".into(),
            id_token_hint: Some(Zeroizing::new("old-id-token".into())),
        }),
        consent: true,
    };
    let sign_in = SignIn::start(&endpoints, &request).await.unwrap();
    let query = params(sign_in.url());
    assert_eq!(query["client_id"], CLIENT);
    assert_eq!(query["login_hint"], "user@example.com");
    assert_eq!(query["id_token_hint"], "old-id-token");
    assert_eq!(query["ext_agent_host_id"], HOST_ID);
    assert_eq!(query["prompt"], "consent");
    assert!(!query.contains_key("agent_name_hint"));

    // The browser signed in with another account.
    mount_exchange(
        &server,
        query["code_challenge"].clone(),
        id_claims(&server.uri(), &query["nonce"], "someone-else"),
    )
    .await;
    let redirect = sign_in.redirect_uri().to_owned();
    let finish = tokio::spawn(async move {
        sign_in
            .finish(&http_client(), &CancellationToken::new())
            .await
    });
    visit(
        &redirect,
        &[("code", "the-code"), ("state", query["state"].as_str())],
    )
    .await;
    let err = finish.await.unwrap().unwrap_err();
    assert!(
        matches!(&err, AiError::SignIn(m) if m.contains("another ChatGPT account")),
        "{err:?}"
    );
    // The tokens issued for that account are not kept, so they are revoked.
    let revoked = || async {
        server.received_requests().await.unwrap().iter().any(|r| {
            r.url.path() == "/api/accounts/oauth/revoke"
                && String::from_utf8_lossy(&r.body).contains("token=refresh-1")
        })
    };
    for _ in 0..100 {
        if revoked().await {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(revoked().await);

    // A callback for another registration is refused before any exchange.
    let sign_in = SignIn::start(&endpoints, &request).await.unwrap();
    let query = params(sign_in.url());
    let redirect = sign_in.redirect_uri().to_owned();
    let finish = tokio::spawn(async move {
        sign_in
            .finish(&http_client(), &CancellationToken::new())
            .await
    });
    visit(
        &redirect,
        &[
            ("code", "the-code"),
            ("state", query["state"].as_str()),
            ("client_id", "oaiapp_other"),
        ],
    )
    .await;
    let err = finish.await.unwrap().unwrap_err();
    assert!(
        matches!(&err, AiError::SignIn(m) if m.contains("another registration")),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_declined_or_incomplete_sign_in_exchanges_nothing() {
    let server = MockServer::start().await;
    let endpoints = Endpoints::at(&server.uri()).unwrap();
    let request = SignInRequest {
        host_id: HOST_ID.into(),
        ..SignInRequest::default()
    };
    for (query, expected) in [
        (vec![("error", "access_denied")], "cancelled in the browser"),
        // A new registration must come back with its issued client id.
        (vec![("code", "the-code")], "did not finish registering"),
    ] {
        let sign_in = SignIn::start(&endpoints, &request).await.unwrap();
        let state = params(sign_in.url())["state"].clone();
        let redirect = sign_in.redirect_uri().to_owned();
        let finish = tokio::spawn(async move {
            sign_in
                .finish(&http_client(), &CancellationToken::new())
                .await
        });
        let mut query = query.clone();
        query.push(("state", &state));
        visit(&redirect, &query).await;
        let err = finish.await.unwrap().unwrap_err();
        assert!(
            matches!(&err, AiError::SignIn(m) if m.contains(expected)),
            "{err:?}"
        );
    }
    assert!(server.received_requests().await.unwrap().is_empty());

    // Cancelling stops the wait.
    let sign_in = SignIn::start(&endpoints, &request).await.unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        sign_in.finish(&http_client(), &cancel).await.unwrap_err(),
        AiError::Cancelled
    );
}

#[test]
fn the_id_token_must_be_signed_by_openai_and_match_the_sign_in() {
    let issuer = "https://auth.example";
    let good = id_claims(issuer, "n1", SUBJECT);
    let check = |claims: &Value, jwks: &Value| {
        oauth::verify_id_token(&signed(claims), jwks, issuer, CLIENT, "n1", now_secs())
    };
    assert_eq!(check(&good, &jwks()).unwrap().subject, SUBJECT);
    let with = |key: &str, value: Value| {
        let mut claims = good.clone();
        claims[key] = value;
        claims
    };
    // An audience list naming the client is fine.
    assert!(check(&with("aud", json!(["x", CLIENT])), &jwks()).is_ok());
    for (claims, why) in [
        (with("iss", json!("https://evil.example")), "another issuer"),
        (with("aud", json!("oaiapp_other")), "another client"),
        (with("exp", json!(now_secs() - 3600)), "expired"),
        (with("nonce", json!("n2")), "nonce"),
        (with("sub", json!("")), "no account"),
    ] {
        let err = check(&claims, &jwks()).unwrap_err();
        assert!(
            matches!(&err, AiError::SignIn(m) if m.contains(why)),
            "{why}: {err:?}"
        );
    }
    // Another key, or a token changed after signing.
    let mut other = jwks();
    other["keys"][0]["n"] = json!(URL_SAFE_NO_PAD.encode([0xc3u8; 256]));
    assert!(check(&good, &other).is_err());
    let token = signed(&good);
    let (head, rest) = token.split_once('.').unwrap();
    let (_, signature) = rest.split_once('.').unwrap();
    let forged = format!("{head}.{}.{signature}", b64(&with("sub", json!("admin"))));
    assert!(oauth::verify_id_token(&forged, &jwks(), issuer, CLIENT, "n1", now_secs()).is_err());
    assert!(oauth::verify_id_token("not-a-jwt", &jwks(), issuer, CLIENT, "n1", 0).is_err());
}

#[test]
fn host_ids_are_uuid_urns() {
    let id = oauth::new_host_id();
    assert!(id.starts_with("urn:uuid:"), "{id}");
    let uuid = &id["urn:uuid:".len()..];
    assert_eq!(uuid.len(), 36);
    assert_eq!(&uuid[14..15], "4", "version 4: {uuid}");
    assert_ne!(id, oauth::new_host_id());
}

// ---------------------------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn concurrent_requests_share_one_refresh_and_the_replacement_token_is_kept() {
    // Expired: every request needs a refresh, and only one goes out.
    let setup = setup(Some(credentials("access-1", "refresh-1", -1000))).await;
    setup
        .refresh_once(
            "refresh-1",
            "access-2",
            "refresh-2",
            Duration::from_millis(300),
        )
        .await;
    let waits = (0..8).map(|_| {
        let sessions = setup.sessions.clone();
        tokio::spawn(async move {
            sessions
                .access_token(PROVIDER, None, &CancellationToken::new())
                .await
        })
    });
    for joined in futures::future::join_all(waits).await {
        assert_eq!(&*joined.unwrap().unwrap(), "access-2");
    }
    let stored = setup.store.get(PROVIDER).unwrap();
    assert_eq!(&*stored.refresh_token, "refresh-2");
    assert_eq!(
        &*stored.id_token, "old-id-token",
        "kept when the refresh has none"
    );
    assert!(stored.expires_at > now_ms() + 3_500_000);

    // Fresh now: no request to the token endpoint (the mock expects one in all).
    let token = setup
        .sessions
        .access_token(PROVIDER, None, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(&*token, "access-2");
}

#[tokio::test]
async fn a_cancelled_request_does_not_lose_the_refresh() {
    let setup = setup(Some(credentials("access-1", "refresh-1", -1000))).await;
    setup
        .refresh_once(
            "refresh-1",
            "access-2",
            "refresh-2",
            Duration::from_millis(300),
        )
        .await;
    let cancel = CancellationToken::new();
    let sessions = setup.sessions.clone();
    let waiting = {
        let cancel = cancel.clone();
        tokio::spawn(async move { sessions.access_token(PROVIDER, None, &cancel).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    cancel.cancel();
    assert_eq!(waiting.await.unwrap().unwrap_err(), AiError::Cancelled);
    // The refresh goes on, and the next request gets its token without another refresh.
    let token = setup
        .sessions
        .access_token(PROVIDER, None, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(&*token, "access-2");
    assert_eq!(
        &*setup.store.get(PROVIDER).unwrap().refresh_token,
        "refresh-2"
    );
}

#[tokio::test]
async fn a_refresh_that_lands_while_the_vault_is_locked_is_stored_after_it_unlocks() {
    let setup = setup(Some(credentials("access-1", "refresh-1", -1000))).await;
    setup
        .refresh_once(
            "refresh-1",
            "access-2",
            "refresh-2",
            Duration::from_millis(300),
        )
        .await;
    let sessions = setup.sessions.clone();
    let waiting = tokio::spawn(async move {
        sessions
            .access_token(PROVIDER, None, &CancellationToken::new())
            .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    setup.store.locked.store(true, Ordering::SeqCst);
    assert_eq!(&*waiting.await.unwrap().unwrap(), "access-2");
    setup.store.locked.store(false, Ordering::SeqCst);
    assert_eq!(
        &*setup.store.get(PROVIDER).unwrap().refresh_token,
        "refresh-1",
        "not written while locked"
    );
    let token = setup
        .sessions
        .access_token(PROVIDER, None, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(&*token, "access-2");
    assert_eq!(
        &*setup.store.get(PROVIDER).unwrap().refresh_token,
        "refresh-2"
    );
}

#[tokio::test]
async fn a_revoked_or_reused_refresh_token_ends_the_session() {
    for code in [
        "invalid_grant",
        "refresh_token_reused",
        "refresh_token_expired",
    ] {
        let setup = setup(Some(credentials("access-1", "refresh-1", -1000))).await;
        Mock::given(method("POST"))
            .and(path("/api/accounts/oauth/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": code, "error_description": "The refresh token is no longer valid."
            })))
            .expect(1)
            .mount(&setup.server)
            .await;
        let err = setup
            .sessions
            .access_token(PROVIDER, None, &CancellationToken::new())
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AiError::SignIn(m) if m.contains("has ended")),
            "{code}: {err:?}"
        );
        let stored = setup.store.get(PROVIDER).unwrap();
        assert!(stored.access_token.is_empty() && stored.refresh_token.is_empty());
        assert_eq!(&*stored.id_token, "old-id-token", "kept as the next hint");
        assert_eq!(stored.client_id, CLIENT);
        assert_eq!(
            setup.sessions.status(PROVIDER).unwrap(),
            SessionStatus::Ended
        );
        // No second refresh: the mock expects one.
        assert!(
            setup
                .sessions
                .access_token(PROVIDER, None, &CancellationToken::new())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn a_failed_early_refresh_keeps_the_current_token() {
    // Four minutes left: due for a refresh, still valid.
    let setup = setup(Some(credentials("access-1", "refresh-1", 4 * 60 * 1000))).await;
    Mock::given(method("POST"))
        .and(path("/api/accounts/oauth/token"))
        .respond_with(ResponseTemplate::new(502))
        .mount(&setup.server)
        .await;
    let token = setup
        .sessions
        .access_token(PROVIDER, None, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(&*token, "access-1");
    assert_eq!(
        setup.sessions.status(PROVIDER).unwrap(),
        SessionStatus::SignedIn
    );
}

#[tokio::test]
async fn no_session_or_no_plan_scope_sends_nothing() {
    let setup = setup(None).await;
    let err = setup
        .sessions
        .access_token(PROVIDER, None, &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(&err, AiError::SignIn(m) if m.contains("not signed in on this device")));
    assert_eq!(
        setup.sessions.status(PROVIDER).unwrap(),
        SessionStatus::SignedOut
    );

    let mut no_plan = credentials("access-1", "refresh-1", 3_600_000);
    no_plan.scopes = vec!["openid".into(), "email".into()];
    let setup_2 = self::setup(Some(no_plan)).await;
    let (result, _) = chat(
        &setup_2.provider(),
        &ModelSpec::new("gpt-x"),
        &[AiEntry::user(1, "hi")],
        &[],
    )
    .await;
    assert!(
        matches!(&result, Err(AiError::SignIn(m)) if m.contains("allow")),
        "{result:?}"
    );
    assert!(setup_2.server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn sign_out_removes_the_session_and_revokes_its_refresh_token() {
    let setup = setup(Some(credentials("access-1", "refresh-1", 3_600_000))).await;
    Mock::given(method("POST"))
        .and(path("/api/accounts/oauth/revoke"))
        .and(body_string_contains("token=refresh-1"))
        .and(body_string_contains("token_type_hint=refresh_token"))
        .and(body_string_contains(format!("client_id={CLIENT}")))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&setup.server)
        .await;
    assert!(
        setup
            .sessions
            .sign_out(PROVIDER, &CancellationToken::new())
            .await
            .unwrap()
    );
    assert!(setup.store.get(PROVIDER).is_none());
    assert_eq!(
        setup.sessions.status(PROVIDER).unwrap(),
        SessionStatus::SignedOut
    );

    // A revocation that keeps failing is reported; the session is gone anyway.
    let setup = self::setup(Some(credentials("access-1", "refresh-1", 3_600_000))).await;
    Mock::given(method("POST"))
        .and(path("/api/accounts/oauth/revoke"))
        .respond_with(ResponseTemplate::new(503))
        .expect(3)
        .mount(&setup.server)
        .await;
    assert!(
        !setup
            .sessions
            .sign_out(PROVIDER, &CancellationToken::new())
            .await
            .unwrap()
    );
    assert!(setup.store.get(PROVIDER).is_none());
}

// ---------------------------------------------------------------------------------------------
// Requests on the ChatGPT route
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_401_refreshes_the_session_once_and_sends_again() {
    let setup = setup(Some(credentials("access-1", "refresh-1", 3_600_000))).await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"detail": "expired"})))
        .expect(1)
        .mount(&setup.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer access-2"))
        .respond_with(hello_stream())
        .expect(1)
        .mount(&setup.server)
        .await;
    setup
        .refresh_once("refresh-1", "access-2", "refresh-2", Duration::ZERO)
        .await;
    let (result, _) = chat(
        &setup.provider(),
        &ModelSpec::new("gpt-x"),
        &[AiEntry::user(1, "hi")],
        &[],
    )
    .await;
    assert_eq!(result.unwrap().text, "Hello");
    let body = &bodies(&setup.server, "/v1/responses").await[1];
    assert_eq!(body["stream"], json!(true));
    assert_eq!(body["store"], json!(false));
    for refused in [
        "temperature",
        "max_output_tokens",
        "metadata",
        "previous_response_id",
        "top_p",
        "truncation",
        "user",
    ] {
        assert!(body.get(refused).is_none(), "{refused}");
    }
    assert!(body["input"].is_array());
}

#[tokio::test]
async fn a_401_after_a_refresh_says_to_sign_in_again() {
    let setup = setup(Some(credentials("access-1", "refresh-1", 3_600_000))).await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(401)
                .insert_header("x-request-id", "req_123")
                .set_body_json(json!({"detail": "Unauthorized"})),
        )
        .expect(2)
        .mount(&setup.server)
        .await;
    setup
        .refresh_once("refresh-1", "access-2", "refresh-2", Duration::ZERO)
        .await;
    let (result, _) = chat(
        &setup.provider(),
        &ModelSpec::new("gpt-x"),
        &[AiEntry::user(1, "hi")],
        &[],
    )
    .await;
    match result.unwrap_err() {
        AiError::Http { status, message } => {
            assert_eq!(status, 401);
            assert!(message.contains("sign in again"), "{message}");
            assert!(message.contains("Unauthorized"), "{message}");
            assert!(message.contains("req_123"), "{message}");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn usage_limits_say_where_to_manage_usage() {
    let setup = setup(Some(credentials("access-1", "refresh-1", 3_600_000))).await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("x-request-id", "req_429")
                .set_body_json(json!({"error": {
                    "code": "subscription_sharing_usage_limit_exceeded",
                    "message": "Usage limit reached", "type": "usage_limit"
                }})),
        )
        .expect(1)
        .mount(&setup.server)
        .await;
    let (result, _) = chat(
        &setup.provider(),
        &ModelSpec::new("gpt-x"),
        &[AiEntry::user(1, "hi")],
        &[],
    )
    .await;
    match result.unwrap_err() {
        AiError::Http { status, message } => {
            assert_eq!(status, 429);
            assert!(message.contains(USAGE_URL), "{message}");
            assert!(message.contains("req_429"), "{message}");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_usage_limit_after_the_stream_opened_is_an_error() {
    let setup = setup(Some(credentials("access-1", "refresh-1", 3_600_000))).await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(sse(&[
            (
                "response.output_text.delta",
                json!({"type": "response.output_text.delta", "output_index": 0, "delta": "Hel"}),
            ),
            (
                "response.failed",
                json!({"type": "response.failed", "response": {"status": "failed", "error": {
                    "code": "subscription_sharing_usage_limit_exceeded", "message": "limit"
                }}}),
            ),
        ])
        .insert_header("x-request-id", "req_stream"))
        .mount(&setup.server)
        .await;
    let (result, _) = chat(
        &setup.provider(),
        &ModelSpec::new("gpt-x"),
        &[AiEntry::user(1, "hi")],
        &[],
    )
    .await;
    match result.unwrap_err() {
        AiError::Http { status, message } => {
            assert_eq!(status, 429);
            assert!(message.contains(USAGE_URL), "{message}");
            assert!(message.contains("req_stream"), "{message}");
        }
        other => panic!("{other:?}"),
    }

    // The other codes of the route, mid-stream too.
    for (code, status, words) in [
        (
            "subscription_sharing_user_not_eligible",
            403,
            "cannot use its plan",
        ),
        ("subscription_sharing_usage_unavailable", 503, "Try again"),
        (
            "subscription_sharing_unsupported_capability",
            400,
            "`tools`",
        ),
        ("chatpass_v2_scope_not_authorized", 403, "Sign in again"),
    ] {
        let err = crate::chat::tests_support::responses_error(&json!({
            "type": "error", "error": {"code": code, "message": "x", "param": "tools"}
        }));
        match err {
            AiError::Http {
                status: got,
                message,
            } => {
                assert_eq!(got, status, "{code}");
                assert!(message.contains(words), "{code}: {message}");
            }
            other => panic!("{other:?}"),
        }
    }
}

#[tokio::test]
async fn a_503_is_sent_again_with_backoff() {
    let setup = setup(Some(credentials("access-1", "refresh-1", 3_600_000))).await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({"error": {
            "code": "subscription_sharing_user_unavailable", "message": "busy"
        }})))
        .up_to_n_times(1)
        .expect(1)
        .mount(&setup.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(hello_stream())
        .expect(1)
        .mount(&setup.server)
        .await;
    let (result, _) = chat(
        &setup.provider(),
        &ModelSpec::new("gpt-x"),
        &[AiEntry::user(1, "hi")],
        &[],
    )
    .await;
    assert_eq!(result.unwrap().text, "Hello");
}

#[tokio::test]
async fn function_tools_go_in_a_namespace_and_their_calls_come_back_by_name() {
    let setup = setup(Some(credentials("access-1", "refresh-1", 3_600_000))).await;
    let call_item = json!({"type": "function_call", "id": "fc_1", "call_id": "call_1",
        "name": "run_command", "namespace": NAMESPACE, "arguments": "{\"command\":\"df -h\"}",
        "status": "completed"});
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(sse(&[
            (
                "response.output_item.added",
                json!({"type": "response.output_item.added", "output_index": 0,
                       "item": {"type": "function_call", "id": "fc_1", "call_id": "call_1",
                                "name": "run_command", "namespace": NAMESPACE,
                                "arguments": ""}}),
            ),
            (
                "response.function_call_arguments.delta",
                json!({"type": "response.function_call_arguments.delta", "output_index": 0,
                       "delta": "{\"command\":\"df -h\"}"}),
            ),
            (
                "response.output_item.done",
                json!({"type": "response.output_item.done", "output_index": 0,
                       "item": call_item}),
            ),
            completed(&json!([call_item])),
        ]))
        .mount(&setup.server)
        .await;
    let tools = [tool("run_command"), tool("read_terminal")];
    let model = ModelSpec::new("gpt-x");
    let (result, events) = chat(
        &setup.provider(),
        &model,
        &[AiEntry::user(1, "How full is the disk?")],
        &tools,
    )
    .await;
    let entry = result.unwrap();
    let expected = ToolCall {
        id: "call_1".into(),
        name: "run_command".into(),
        arguments: "{\"command\":\"df -h\"}".into(),
    };
    assert_eq!(entry.tool_calls, vec![expected.clone()]);
    assert_eq!(entry.finish, Finish::ToolCalls);
    assert!(events.contains(&StreamEvent::ToolCall(expected.clone())));
    assert_eq!(entry.raw[0]["namespace"], json!(NAMESPACE));

    let body = &bodies(&setup.server, "/v1/responses").await[0];
    assert_eq!(
        body["tools"],
        json!([{
            "type": "namespace", "name": NAMESPACE, "description": NAMESPACE_DESCRIPTION,
            "tools": [
                {"type": "function", "name": "run_command", "description": "The run_command tool.",
                 "parameters": {"type": "object", "properties": {}}, "strict": false},
                {"type": "function", "name": "read_terminal",
                 "description": "The read_terminal tool.",
                 "parameters": {"type": "object", "properties": {}}, "strict": false}
            ]
        }])
    );

    // A call from another model is rebuilt with the namespace; the replayed one keeps it.
    let rebuilt = AiEntry::assistant(
        2,
        AssistantEntry {
            provider_id: "another".into(),
            model_id: "other".into(),
            tool_calls: vec![expected.clone()],
            finish: Finish::ToolCalls,
            ..AssistantEntry::default()
        },
    );
    let mut replayed = entry.clone();
    replayed.tool_calls[0].id = "call_2".into();
    replayed.raw[0]["call_id"] = json!("call_2");
    let history = vec![
        AiEntry::user(1, "How full is the disk?"),
        rebuilt,
        AiEntry::tool(3, "call_1", crate::entry::ToolStatus::Ok, "42%"),
        AiEntry::assistant(4, replayed),
        AiEntry::tool(5, "call_2", crate::entry::ToolStatus::Ok, "42%"),
    ];
    let (_, _) = chat(&setup.provider(), &model, &history, &tools).await;
    let body = &bodies(&setup.server, "/v1/responses").await[1];
    let calls: Vec<&Value> = body["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "function_call")
        .collect();
    assert_eq!(calls.len(), 2);
    for call in calls {
        assert_eq!(call["namespace"], json!(NAMESPACE), "{call}");
        assert_eq!(call["name"], json!("run_command"));
    }
}

#[tokio::test]
async fn a_qualified_name_loses_its_namespace() {
    let setup = setup(Some(credentials("access-1", "refresh-1", 3_600_000))).await;
    let call_item = json!({"type": "function_call", "call_id": "call_1",
        "name": "hatoba.read_terminal", "namespace": NAMESPACE, "arguments": "{}"});
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(sse(&[completed(&json!([call_item]))]))
        .mount(&setup.server)
        .await;
    let (result, _) = chat(
        &setup.provider(),
        &ModelSpec::new("gpt-x"),
        &[AiEntry::user(1, "hi")],
        &[tool("read_terminal")],
    )
    .await;
    assert_eq!(result.unwrap().tool_calls[0].name, "read_terminal");
}

#[tokio::test]
async fn the_judge_streams_and_forces_its_namespaced_tool() {
    let setup = setup(Some(credentials("access-1", "refresh-1", 3_600_000))).await;
    let call_item = json!({"type": "function_call", "call_id": "call_1", "name": "answer",
        "namespace": NAMESPACE, "arguments": "{\"notes\":\"Reads a file.\",\"safe\":0.97}"});
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(sse(&[
            (
                "response.output_item.done",
                json!({"type": "response.output_item.done", "output_index": 0,
                       "item": call_item}),
            ),
            completed(&json!([call_item])),
        ]))
        .expect(1)
        .mount(&setup.server)
        .await;
    let questions = BTreeMap::from([(
        "safe".to_owned(),
        Question::noul("Is it safe?", "It is.", "It is not."),
    )]);
    let evaluation = decision::evaluate(
        &http_client(),
        &setup.provider(),
        &ModelSpec::new("gpt-x"),
        &json!({"call": "cat notes.txt"}),
        None,
        &questions,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(evaluation.answers["safe"], Answer::Noul { noul: 0.97 });
    assert_eq!(evaluation.usage.unwrap().input_tokens, 10);
    let body = &bodies(&setup.server, "/v1/responses").await[0];
    assert_eq!(body["stream"], json!(true));
    assert_eq!(body["store"], json!(false));
    assert_eq!(body["tool_choice"], json!("required"));
    assert_eq!(body["tools"][0]["type"], json!("namespace"));
    assert_eq!(body["tools"][0]["tools"][0]["name"], json!("answer"));
}

#[tokio::test]
async fn test_connection_streams_and_the_model_list_keeps_listed_models() {
    let setup = setup(Some(credentials("access-1", "refresh-1", 3_600_000))).await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(hello_stream())
        .expect(1)
        .mount(&setup.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", "Bearer access-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
            {"slug": "gpt-6.1-sol", "display_name": "GPT-6.1 Sol", "visibility": "list",
             "context_window": 400000,
             "supported_reasoning_levels": [{"effort": "low"}, {"effort": "medium"},
                                            {"effort": "high"}, {"effort": "minimal"}]},
            {"slug": "gpt-6.1-internal", "display_name": "Internal", "visibility": "hide"},
            {"slug": "gpt-6-mini", "display_name": "GPT-6 mini", "visibility": "list"}
        ]})))
        .mount(&setup.server)
        .await;
    let provider = setup.provider();
    let cancel = CancellationToken::new();
    let outcome =
        models::test_connection(&http_client(), &provider, Some("gpt-6.1-sol"), &cancel).await;
    assert!(outcome.ok, "{outcome:?}");
    let body = &bodies(&setup.server, "/v1/responses").await[0];
    assert_eq!(body["stream"], json!(true));
    assert!(body["input"].is_array());

    let listed = models::list_models(&http_client(), &provider, &cancel)
        .await
        .unwrap();
    let ids: Vec<&str> = listed.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["gpt-6.1-sol", "gpt-6-mini"]);
    assert_eq!(listed[0].name, "GPT-6.1 Sol");
    assert_eq!(listed[0].context_window, Some(400_000));
    use crate::provider::Effort::{High, Low, Medium};
    assert_eq!(listed[0].efforts, Some(vec![Low, Medium, High]));
    assert_eq!(listed[1].efforts, None);

    // Without a session, Test Connection says to sign in.
    let signed_out = self::setup(None).await;
    let outcome = models::test_connection(
        &http_client(),
        &signed_out.provider(),
        Some("gpt-6.1-sol"),
        &cancel,
    )
    .await;
    assert_eq!(outcome.failure, Some(TestFailure::Auth));
    assert!(
        outcome
            .message
            .unwrap()
            .contains("not signed in on this device")
    );
}

#[test]
fn credentials_round_trip_without_printing_tokens() {
    let c = credentials("access-secret", "refresh-secret", 1000);
    let json = serde_json::to_string(&c).unwrap();
    let back: Credentials = serde_json::from_str(&json).unwrap();
    assert_eq!(back, c);
    let debug = format!("{c:?}");
    for secret in [
        "access-secret",
        "refresh-secret",
        "old-id-token",
        SUBJECT,
        "user@example",
    ] {
        assert!(!debug.contains(secret), "{debug}");
    }
    let ended = c.without_tokens();
    assert!(ended.ended() && ended.access_token.is_empty());
    assert_eq!(ended.status(), SessionStatus::Ended);
}
