//! `WorkerBackend` request/response mapping against a local HTTP server (wiremock).
//!
//! These tests pin the wire contract of the Worker API (spec §6.2 and the teammate's Worker):
//! paths, methods, auth headers, JSON bodies (standard padded base64 for raw keys) and how each
//! status code maps to an [`Error`].

use serde_json::{Value, json};
use wiremock::matchers::{body_json, header, header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};
use zeroize::Zeroizing;

use super::*;
use crate::crypto::{b64_decode, b64_encode};
use crate::error::Error;

const SALT: &str = "AAECAwQFBgcICQoLDA0ODw==";
const PARAMS: &str = r#"{"alg":"argon2id","version":19,"m_kib":65536,"t":3,"p":4}"#;
const TOKEN: &str = "session-token-0123456789abcdefghijklmnopqrstuvw";

fn backend(server: &MockServer) -> WorkerBackend {
    WorkerBackend::new(&server.uri()).unwrap()
}

fn signed_in(server: &MockServer) -> WorkerBackend {
    let b = backend(server);
    b.set_session(Some(Session {
        token: Zeroizing::new(TOKEN.into()),
        expires_at: 1,
    }));
    b
}

fn device() -> DeviceLogin {
    DeviceLogin {
        device_id: "0192aaaa-bbbb-7ccc-8ddd-eeeeffff0001".into(),
        sealed_name: "{\"v\":1,\"n\":\"x\",\"c\":\"y\"}".into(),
    }
}

fn err(status: u16, code: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({ "error": code }))
}

async fn body_of(server: &MockServer, index: usize) -> Value {
    let requests = server.received_requests().await.unwrap();
    serde_json::from_slice(&requests[index].body).unwrap()
}

#[tokio::test]
async fn health_is_unauthenticated() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"service": "hatoba-sync", "version": "0.1.0", "api": 1, "initialized": true}),
        ))
        .mount(&server)
        .await;
    let info = backend(&server).health().await.unwrap();
    assert_eq!(
        info,
        ServerInfo {
            service: "hatoba-sync".into(),
            version: "0.1.0".into(),
            api: 1,
            initialized: true
        }
    );
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests[0].headers.get("authorization").is_none(),
        "health must not send credentials"
    );
    assert!(
        requests[0]
            .headers
            .get("user-agent")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("hatoba-core/")
    );
}

#[tokio::test]
async fn prelogin_returns_the_raw_params_string_and_maps_404() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/prelogin"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"kdf_salt": SALT, "kdf_params": PARAMS})),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/prelogin"))
        .respond_with(err(404, "not_initialized"))
        .mount(&server)
        .await;
    let b = backend(&server);
    let kdf = b.prelogin().await.unwrap();
    assert_eq!(
        (kdf.kdf_salt.as_str(), kdf.kdf_params.as_str()),
        (SALT, PARAMS)
    );
    assert!(matches!(
        b.prelogin().await,
        Err(Error::RemoteNotInitialized)
    ));
}

#[tokio::test]
async fn setup_sends_the_setup_token_and_base64_keys() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/setup"))
        .and(header("authorization", "Bearer the-setup-token"))
        .and(header("content-type", "application/json"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"initialized": true})))
        .mount(&server)
        .await;
    let (auth, recovery) = (Zeroizing::new([0xAB_u8; 32]), Zeroizing::new([0xCD_u8; 32]));
    backend(&server)
        .setup(VaultInit {
            schema_version: 1,
            kdf_salt: SALT.into(),
            kdf_params: PARAMS.into(),
            auth_key: auth.clone(),
            protected_vault_key: "PVK".into(),
            recovery_vault_key: "RVK".into(),
            recovery_auth: recovery.clone(),
            setup_token: Some(Zeroizing::new("the-setup-token".into())),
        })
        .await
        .unwrap();
    let body = body_of(&server, 0).await;
    assert_eq!(
        body,
        json!({
            "schema_version": 1,
            "kdf_salt": SALT,
            "kdf_params": PARAMS,
            "auth_key": b64_encode(&*auth),
            "protected_vault_key": "PVK",
            "recovery_vault_key": "RVK",
            "recovery_auth": b64_encode(&*recovery),
        })
    );
    // Strict standard base64 *with* padding: 32 bytes → 44 characters ending in '='.
    let encoded = body["auth_key"].as_str().unwrap();
    assert_eq!(encoded.len(), 44);
    assert!(encoded.ends_with('='));
    assert_eq!(b64_decode(encoded).unwrap(), vec![0xAB; 32]);
    assert!(
        !body.to_string().contains("the-setup-token"),
        "the setup token goes in the header only"
    );
}

#[tokio::test]
async fn setup_error_mapping() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/setup"))
        .and(header("authorization", "Bearer good"))
        .respond_with(err(409, "already_initialized"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/setup"))
        .respond_with(err(401, "invalid_setup_token"))
        .mount(&server)
        .await;
    let init = |token: Option<&str>| VaultInit {
        schema_version: 1,
        kdf_salt: SALT.into(),
        kdf_params: PARAMS.into(),
        auth_key: Zeroizing::new([1; 32]),
        protected_vault_key: String::new(),
        recovery_vault_key: String::new(),
        recovery_auth: Zeroizing::new([2; 32]),
        setup_token: token.map(|t| Zeroizing::new(t.to_owned())),
    };
    let b = backend(&server);
    assert!(matches!(
        b.setup(init(Some("good"))).await,
        Err(Error::RemoteInitialized)
    ));
    assert!(matches!(
        b.setup(init(Some("bad"))).await,
        Err(Error::InvalidSetupToken)
    ));
    let before = server.received_requests().await.unwrap().len();
    assert!(matches!(
        b.setup(init(None)).await,
        Err(Error::InvalidSetupToken)
    ));
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        before,
        "no token → nothing is sent"
    );
}

#[tokio::test]
async fn login_installs_the_session_and_later_calls_use_it() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"session_token": TOKEN, "expires_at": 1_800_000_000_000_i64}),
            ),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/vault"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "schema_version": 1, "kdf_salt": SALT, "kdf_params": PARAMS,
            "protected_vault_key": "PVK", "recovery_vault_key": "RVK", "seq": 7
        })))
        .mount(&server)
        .await;
    let b = backend(&server);
    let key = [0x42u8; 32];
    let session = b.login(&key, &device()).await.unwrap();
    assert_eq!(
        (session.token.as_str(), session.expires_at),
        (TOKEN, 1_800_000_000_000)
    );

    let body = body_of(&server, 0).await;
    assert_eq!(body["auth_key"], b64_encode(&key));
    assert_eq!(body["device_id"], "0192aaaa-bbbb-7ccc-8ddd-eeeeffff0001");
    assert_eq!(body["device_name"], device().sealed_name);

    let meta = b.fetch_vault().await.unwrap();
    assert_eq!(
        meta,
        VaultMeta {
            schema_version: 1,
            kdf_salt: SALT.into(),
            kdf_params: PARAMS.into(),
            protected_vault_key: "PVK".into(),
            recovery_vault_key: "RVK".into(),
            seq: 7
        }
    );
}

#[tokio::test]
async fn login_error_mapping() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/login")).and(body_json(json!({
        "auth_key": b64_encode(&[1u8; 32]), "device_id": device().device_id, "device_name": device().sealed_name
    }))).respond_with(err(401, "invalid_credentials")).mount(&server).await;
    Mock::given(method("POST")).and(path("/v1/login")).and(body_json(json!({
        "auth_key": b64_encode(&[2u8; 32]), "device_id": device().device_id, "device_name": device().sealed_name
    }))).respond_with(err(404, "not_initialized")).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/v1/login"))
        .respond_with(err(429, "rate_limited").insert_header("Retry-After", "60"))
        .mount(&server)
        .await;
    let b = backend(&server);
    assert!(matches!(
        b.login(&[1; 32], &device()).await,
        Err(Error::WrongPassword)
    ));
    assert!(matches!(
        b.login(&[2; 32], &device()).await,
        Err(Error::RemoteNotInitialized)
    ));
    assert!(matches!(
        b.login(&[3; 32], &device()).await,
        Err(Error::RateLimited {
            retry_after_secs: Some(60)
        })
    ));
}

#[tokio::test]
async fn recover_returns_the_restricted_session() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/recover"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "recovery_vault_key": "RVK", "kdf_salt": SALT, "kdf_params": PARAMS,
            "session_token": TOKEN, "expires_at": 1_800_000_000_000_i64
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/recover"))
        .respond_with(err(401, "invalid_credentials"))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/v1/vault/password"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok": true, "relogin_required": true})),
        )
        .mount(&server)
        .await;
    let b = backend(&server);
    let rec = b.recover(&[5; 32], &device()).await.unwrap();
    assert_eq!(rec.recovery_vault_key, "RVK");
    assert_eq!(
        (rec.kdf.kdf_salt.as_str(), rec.kdf.kdf_params.as_str()),
        (SALT, PARAMS)
    );
    assert_eq!(rec.session.token.as_str(), TOKEN);
    assert_eq!(
        body_of(&server, 0).await["recovery_auth"],
        b64_encode(&[5u8; 32])
    );
    // The restricted session is installed, so the password change can use it.
    b.update_vault_meta(VaultMetaUpdate {
        kdf_salt: SALT.into(),
        kdf_params: PARAMS.into(),
        auth_key: Zeroizing::new([7; 32]),
        protected_vault_key: "NEW-PVK".into(),
        recovery: None,
    })
    .await
    .unwrap();
    assert!(matches!(
        b.recover(&[6; 32], &device()).await,
        Err(Error::WrongRecoveryCode)
    ));
}

#[tokio::test]
async fn pull_sends_since_and_limit_and_parses_rows() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/items"))
        .and(query_param("since", "41"))
        .and(query_param("limit", "500"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [
                {"id": "a", "envelope": "{\"v\":1}", "revision": 3, "seq": 42, "deleted": false, "updated_at": 1700000000001_i64},
                {"id": "b", "envelope": null, "revision": 2, "seq": 45, "deleted": true, "updated_at": 1700000000002_i64}
            ],
            "next_since": 45,
            "has_more": true
        })))
        .mount(&server)
        .await;
    let page = signed_in(&server).pull(41, 500).await.unwrap();
    assert_eq!(page.next_since, 45);
    assert!(page.has_more);
    assert_eq!(
        page.items,
        vec![
            RemoteItem {
                id: "a".into(),
                envelope: Some("{\"v\":1}".into()),
                revision: 3,
                seq: 42,
                deleted: false,
                updated_at: 1_700_000_000_001
            },
            RemoteItem {
                id: "b".into(),
                envelope: None,
                revision: 2,
                seq: 45,
                deleted: true,
                updated_at: 1_700_000_000_002
            },
        ]
    );
}

#[tokio::test]
async fn push_body_and_result_mapping() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/items"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results": [
            {"id": "a", "status": "ok", "revision": 4, "seq": 1207},
            {"id": "b", "status": "conflict", "server": {"revision": 6, "seq": 1190, "deleted": false, "envelope": "SRV", "updated_at": 55}},
            {"id": "c", "status": "conflict", "server": {"revision": 2, "seq": 1191, "deleted": true, "envelope": null, "updated_at": 56}},
            {"id": "d", "status": "error", "error": "too_large"},
            {"id": "e", "status": "error", "error": "not_found"}
        ]})))
        .mount(&server)
        .await;
    let results = signed_in(&server)
        .push(vec![
            Change {
                id: "a".into(),
                base_revision: 3,
                deleted: false,
                envelope: Some("ENV-A".into()),
                updated_at: 10,
            },
            Change {
                id: "b".into(),
                base_revision: 0,
                deleted: false,
                envelope: Some("ENV-B".into()),
                updated_at: 11,
            },
            Change {
                id: "c".into(),
                base_revision: 1,
                deleted: true,
                envelope: None,
                updated_at: 12,
            },
            Change {
                id: "d".into(),
                base_revision: 0,
                deleted: false,
                envelope: Some("ENV-D".into()),
                updated_at: 13,
            },
            Change {
                id: "e".into(),
                base_revision: 9,
                deleted: false,
                envelope: Some("ENV-E".into()),
                updated_at: 14,
            },
        ])
        .await
        .unwrap();

    assert_eq!(
        body_of(&server, 0).await,
        json!({"changes": [
            {"id": "a", "base_revision": 3, "deleted": false, "envelope": "ENV-A", "updated_at": 10},
            {"id": "b", "base_revision": 0, "deleted": false, "envelope": "ENV-B", "updated_at": 11},
            {"id": "c", "base_revision": 1, "deleted": true, "envelope": null, "updated_at": 12},
            {"id": "d", "base_revision": 0, "deleted": false, "envelope": "ENV-D", "updated_at": 13},
            {"id": "e", "base_revision": 9, "deleted": false, "envelope": "ENV-E", "updated_at": 14}
        ]})
    );
    assert_eq!(results.len(), 5);
    assert_eq!(
        results[0],
        PushResult::Ok {
            id: "a".into(),
            revision: 4,
            seq: 1207
        }
    );
    assert_eq!(
        results[1],
        PushResult::Conflict {
            id: "b".into(),
            server: RemoteItem {
                id: "b".into(),
                envelope: Some("SRV".into()),
                revision: 6,
                seq: 1190,
                deleted: false,
                updated_at: 55
            }
        }
    );
    assert!(
        matches!(&results[2], PushResult::Conflict { server, .. } if server.deleted && server.envelope.is_none())
    );
    assert_eq!(
        results[3],
        PushResult::Error {
            id: "d".into(),
            error: "too_large".into()
        }
    );
    assert_eq!(
        results[4],
        PushResult::Error {
            id: "e".into(),
            error: "not_found".into()
        }
    );
}

#[tokio::test]
async fn push_is_split_into_requests_of_at_most_100_changes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/items"))
        .respond_with(|req: &Request| {
            let body: Value = serde_json::from_slice(&req.body).unwrap();
            let results: Vec<Value> = body["changes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| json!({"id": c["id"], "status": "ok", "revision": 1, "seq": 1}))
                .collect();
            ResponseTemplate::new(200).set_body_json(json!({ "results": results }))
        })
        .mount(&server)
        .await;
    let changes: Vec<Change> = (0..250)
        .map(|i| Change {
            id: format!("id-{i}"),
            base_revision: 0,
            deleted: false,
            envelope: Some("e".into()),
            updated_at: i,
        })
        .collect();
    let results = signed_in(&server).push(changes).await.unwrap();
    assert_eq!(results.len(), 250);
    assert_eq!(
        results[249].id(),
        "id-249",
        "order is preserved across requests"
    );
    let sizes: Vec<usize> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| {
            serde_json::from_slice::<Value>(&r.body).unwrap()["changes"]
                .as_array()
                .unwrap()
                .len()
        })
        .collect();
    assert_eq!(sizes, [100, 100, 50]);
}

#[tokio::test]
async fn password_update_sends_the_recovery_pair_only_when_rotating() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/v1/vault/password"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": true, "relogin_required": false})),
        )
        .mount(&server)
        .await;
    let b = signed_in(&server);
    let update = |recovery: Option<RecoveryUpdate>| VaultMetaUpdate {
        kdf_salt: SALT.into(),
        kdf_params: PARAMS.into(),
        auth_key: Zeroizing::new([9; 32]),
        protected_vault_key: "PVK2".into(),
        recovery,
    };
    b.update_vault_meta(update(None)).await.unwrap();
    b.update_vault_meta(update(Some(RecoveryUpdate {
        recovery_vault_key: "RVK2".into(),
        recovery_auth: Zeroizing::new([8; 32]),
    })))
    .await
    .unwrap();
    let plain = body_of(&server, 0).await;
    assert_eq!(
        plain,
        json!({"kdf_salt": SALT, "kdf_params": PARAMS, "auth_key": b64_encode(&[9u8; 32]), "protected_vault_key": "PVK2"})
    );
    let rotating = body_of(&server, 1).await;
    assert_eq!(rotating["recovery_vault_key"], "RVK2");
    assert_eq!(rotating["recovery_auth"], b64_encode(&[8u8; 32]));
}

#[tokio::test]
async fn devices_and_revocation() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/devices"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"devices": [
            {"device_id": "d1", "device_name": "SEALED1", "created_at": 1, "last_seen": 2, "expires_at": 3, "current": true},
            {"device_id": "d2", "device_name": "SEALED2", "created_at": 4, "last_seen": 5, "expires_at": 6, "current": false}
        ]})))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/devices/d2"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let b = signed_in(&server);
    let devices = b.devices().await.unwrap();
    assert_eq!(devices.len(), 2);
    assert_eq!(
        devices[0],
        RemoteDevice {
            device_id: "d1".into(),
            device_name: "SEALED1".into(),
            created_at: 1,
            last_seen: 2,
            expires_at: 3,
            current: true
        }
    );
    assert!(!devices[1].current);
    b.revoke_device("d2").await.unwrap();
    // A device id cannot smuggle path segments.
    let before = server.received_requests().await.unwrap().len();
    assert!(b.revoke_device("../vault").await.is_err());
    assert!(b.revoke_device("").await.is_err());
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}

#[tokio::test]
async fn session_errors_map_to_unauthorized() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/vault"))
        .respond_with(err(401, "invalid_session"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/devices"))
        .respond_with(err(403, "insufficient_scope"))
        .mount(&server)
        .await;
    let b = signed_in(&server);
    assert!(matches!(b.fetch_vault().await, Err(Error::Unauthorized)));
    assert!(matches!(b.devices().await, Err(Error::Unauthorized)));

    // Without any session nothing is even sent.
    let bare = backend(&server);
    let before = server.received_requests().await.unwrap().len();
    assert!(matches!(bare.pull(0, 10).await, Err(Error::Unauthorized)));
    assert_eq!(server.received_requests().await.unwrap().len(), before);

    // Clearing the session logs the backend out.
    b.set_session(None);
    assert!(matches!(b.fetch_vault().await, Err(Error::Unauthorized)));
}

#[tokio::test]
async fn server_errors_and_garbage_are_mapped() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/health"))
        .respond_with(err(503, "database_unavailable"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/health"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>captive portal</html>"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/prelogin"))
        .respond_with(ResponseTemplate::new(404).set_body_string("<html>nope</html>"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/vault"))
        .respond_with(
            ResponseTemplate::new(307)
                .insert_header("location", "https://elsewhere.example/v1/vault"),
        )
        .mount(&server)
        .await;
    let b = signed_in(&server);
    assert!(
        matches!(b.health().await, Err(Error::Server(m)) if m == "http 503 database_unavailable")
    );
    assert!(
        matches!(b.health().await, Err(Error::Protocol(_))),
        "a captive portal's HTML is not a health answer"
    );
    // A bare 404 on prelogin is "not initialised" (spec); redirects are never followed.
    assert!(matches!(
        b.prelogin().await,
        Err(Error::RemoteNotInitialized)
    ));
    assert!(matches!(b.fetch_vault().await, Err(Error::Server(m)) if m == "http 307"));
}

#[tokio::test]
async fn unreachable_server_is_offline() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener); // nothing listens here any more
    let b = WorkerBackend::new(&format!("http://127.0.0.1:{port}")).unwrap();
    assert!(matches!(b.health().await, Err(Error::Offline)));
    assert!(matches!(b.prelogin().await, Err(Error::Offline)));
    assert!(matches!(
        b.login(&[1; 32], &device()).await,
        Err(Error::Offline)
    ));
    assert!(Error::Offline.is_transient());
}

#[tokio::test]
async fn bearer_header_is_marked_sensitive_and_base_path_is_kept() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/hatoba/v1/vault")).and(header_exists("authorization")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "schema_version": 1, "kdf_salt": SALT, "kdf_params": PARAMS, "protected_vault_key": "P", "recovery_vault_key": "R", "seq": 0
    }))).mount(&server).await;
    let b = WorkerBackend::new(&format!("{}/hatoba/", server.uri())).unwrap();
    assert!(b.base_url().ends_with("/hatoba"));
    b.set_session(Some(Session {
        token: Zeroizing::new(TOKEN.into()),
        expires_at: 1,
    }));
    b.fetch_vault().await.unwrap();
}

#[tokio::test]
async fn forgotten_credentials_stop_authenticated_calls_before_any_request() {
    let server = MockServer::start().await;
    let b = signed_in(&server);
    b.forget_credentials();
    assert!(matches!(b.pull(0, 10).await, Err(Error::Unauthorized)));
    assert!(matches!(b.fetch_vault().await, Err(Error::Unauthorized)));
    assert!(matches!(b.devices().await, Err(Error::Unauthorized)));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[test]
fn rejects_unsafe_urls_at_construction() {
    assert!(matches!(
        WorkerBackend::new("http://example.com"),
        Err(Error::InvalidUrl(_))
    ));
    assert!(WorkerBackend::new("sync.example.workers.dev").is_ok());
}
