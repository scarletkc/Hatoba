//! D1 direct mode against a local stand-in for Cloudflare's REST API.
//!
//! The stand-in (`FakeD1`) answers `POST …/d1/database/{id}/query` by executing the SQL against a
//! real SQLite database, so the statements `D1Backend` sends are actually run, not just matched.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rusqlite::types::{Value as SqlValue, ValueRef};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};
use zeroize::Zeroizing;

use super::d1::{D1Backend, list_databases_at, verify_token_at};
use super::*;
use crate::crypto::{KdfParams, auth_hash, random_key};
use crate::error::Error;
use crate::model::{Host, Item};
use crate::platform::DeviceInfo;
use crate::vault::{ManualClock, Vault};

const ACCOUNT: &str = "0123456789abcdef0123456789abcdef";
const DATABASE: &str = "11111111-2222-3333-4444-555555555555";
const TOKEN: &str = "cf-api-token-for-tests";
const PW: &str = "d1-test-password";

/// A SQLite-backed imitation of D1's query endpoint.
struct FakeD1 {
    db: Mutex<rusqlite::Connection>,
    statements: Mutex<Vec<String>>,
    /// Holds back the `meta.seq` catch-up at the end of a push, as if it had not run yet.
    hold_meta_catch_up: AtomicBool,
}

impl FakeD1 {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            db: Mutex::new(rusqlite::Connection::open_in_memory().unwrap()),
            statements: Mutex::new(Vec::new()),
            hold_meta_catch_up: AtomicBool::new(false),
        })
    }

    fn scalar(&self, sql: &str) -> Option<String> {
        let db = self.db.lock().unwrap();
        db.query_row(sql, [], |r| r.get::<_, rusqlite::types::Value>(0))
            .ok()
            .map(|v| match v {
                SqlValue::Text(t) => t,
                SqlValue::Integer(i) => i.to_string(),
                other => format!("{other:?}"),
            })
    }

    fn all_text(&self) -> String {
        let db = self.db.lock().unwrap();
        let mut out = String::new();
        for table in ["meta", "items"] {
            let Ok(mut stmt) = db.prepare(&format!("SELECT * FROM {table}")) else {
                continue;
            };
            let cols = stmt.column_count();
            let mut rows = stmt.query([]).unwrap();
            while let Some(row) = rows.next().unwrap() {
                for i in 0..cols {
                    out.push_str(&format!("{:?} ", row.get_ref(i).unwrap()));
                }
                out.push('\n');
            }
        }
        out
    }
}

fn to_sql(v: &Value) -> SqlValue {
    match v {
        Value::Null => SqlValue::Null,
        Value::Bool(b) => SqlValue::Integer(i64::from(*b)),
        Value::Number(n) => n
            .as_i64()
            .map_or_else(|| SqlValue::Real(n.as_f64().unwrap()), SqlValue::Integer),
        Value::String(s) => SqlValue::Text(s.clone()),
        other => SqlValue::Text(other.to_string()),
    }
}

fn from_sql(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(f) => json!(f),
        ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
        ValueRef::Blob(_) => Value::Null,
    }
}

fn cf_error(status: u16, code: i64, message: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({
        "success": false, "errors": [{"code": code, "message": message}], "messages": [], "result": null
    }))
}

struct Responder(Arc<FakeD1>);

impl Respond for Responder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let auth = request
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        if auth != format!("Bearer {TOKEN}") {
            return cf_error(401, 10001, "Unable to authenticate request");
        }
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let sql = body["sql"].as_str().unwrap().to_owned();
        let params: Vec<SqlValue> = body["params"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(to_sql)
            .collect();
        self.0.statements.lock().unwrap().push(sql.clone());
        if self.0.hold_meta_catch_up.load(Ordering::SeqCst)
            && sql.starts_with("UPDATE meta SET seq")
        {
            return ResponseTemplate::new(200).set_body_json(json!({
                "success": true, "errors": [], "messages": [],
                "result": [{ "results": [], "success": true, "meta": { "changes": 0, "duration": 0.1 } }]
            }));
        }

        let db = self.0.db.lock().unwrap();
        let mut stmt = match db.prepare(&sql) {
            Ok(s) => s,
            Err(e) => return cf_error(400, 7500, &format!("{e}: SQLITE_ERROR")),
        };
        let names: Vec<String> = stmt
            .column_names()
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let mut results = Vec::new();
        let outcome = if names.is_empty() {
            stmt.execute(rusqlite::params_from_iter(params)).map(|_| ())
        } else {
            (|| {
                let mut rows = stmt.query(rusqlite::params_from_iter(params))?;
                while let Some(row) = rows.next()? {
                    let mut obj = serde_json::Map::new();
                    for (i, name) in names.iter().enumerate() {
                        obj.insert(name.clone(), from_sql(row.get_ref(i)?));
                    }
                    results.push(Value::Object(obj));
                }
                Ok(())
            })()
        };
        drop(stmt);
        if let Err(e) = outcome {
            return cf_error(400, 7500, &format!("{e}: SQLITE_ERROR"));
        }
        let changes = db.changes();
        ResponseTemplate::new(200).set_body_json(json!({
            "success": true, "errors": [], "messages": [],
            "result": [{ "results": results, "success": true, "meta": { "changes": changes, "duration": 0.1 } }]
        }))
    }
}

async fn start() -> (MockServer, Arc<FakeD1>) {
    let server = MockServer::start().await;
    let fake = FakeD1::new();
    Mock::given(method("POST"))
        .and(path_regex(
            r"^/client/v4/accounts/[^/]+/d1/database/[^/]+/query$",
        ))
        .respond_with(Responder(fake.clone()))
        .mount(&server)
        .await;
    (server, fake)
}

fn backend(server: &MockServer) -> D1Backend {
    D1Backend::with_base_url(
        &format!("{}/client/v4", server.uri()),
        ACCOUNT,
        DATABASE,
        TOKEN,
    )
    .unwrap()
}

fn init(auth: &[u8; 32], recovery: &[u8; 32]) -> VaultInit {
    VaultInit {
        schema_version: 1,
        kdf_salt: "c2FsdHNhbHRzYWx0c2FsdA==".into(),
        kdf_params: KdfParams::for_tests().to_json(),
        auth_key: Zeroizing::new(*auth),
        protected_vault_key: "{\"v\":1,\"n\":\"p\",\"c\":\"p\"}".into(),
        recovery_vault_key: "{\"v\":1,\"n\":\"r\",\"c\":\"r\"}".into(),
        recovery_auth: Zeroizing::new(*recovery),
        setup_token: None,
    }
}

fn login_info() -> DeviceLogin {
    DeviceLogin {
        device_id: "dev".into(),
        sealed_name: "sealed".into(),
    }
}

fn change(id: &str, base: u64, env: Option<&str>, at: i64) -> Change {
    Change {
        id: id.into(),
        base_revision: base,
        deleted: env.is_none(),
        envelope: env.map(str::to_owned),
        updated_at: at,
    }
}

// ---- backend semantics ----------------------------------------------------------------------

#[tokio::test]
async fn empty_database_is_reported_as_not_initialised() {
    let (server, _fake) = start().await;
    let b = backend(&server);
    let info = b.health().await.unwrap();
    assert!(
        !info.initialized,
        "no tables yet counts as uninitialised, not as an error"
    );
    assert!(matches!(
        b.prelogin().await,
        Err(Error::RemoteNotInitialized)
    ));
    assert!(matches!(
        b.login(&[1; 32], &login_info()).await,
        Err(Error::RemoteNotInitialized)
    ));
    assert!(matches!(
        b.fetch_vault().await,
        Err(Error::RemoteNotInitialized)
    ));
}

#[tokio::test]
async fn setup_creates_the_schema_and_stores_only_hashes() {
    let (server, fake) = start().await;
    let b = backend(&server);
    let (auth, recovery) = (random_key().unwrap(), random_key().unwrap());
    b.setup(init(&auth, &recovery)).await.unwrap();

    assert!(b.health().await.unwrap().initialized);
    assert_eq!(
        fake.scalar("SELECT auth_hash FROM meta").unwrap(),
        auth_hash(&auth)
    );
    assert_eq!(
        fake.scalar("SELECT recovery_auth_hash FROM meta").unwrap(),
        auth_hash(&recovery)
    );
    assert_eq!(fake.scalar("SELECT seq FROM meta").unwrap(), "0");
    // Only the Worker's two data tables (no `sessions`) and the seq index exist.
    assert_eq!(fake.scalar("SELECT group_concat(name) FROM (SELECT name FROM sqlite_master WHERE type='table' ORDER BY name)").unwrap(), "items,meta");
    assert!(
        fake.scalar("SELECT name FROM sqlite_master WHERE name = 'idx_items_seq'")
            .is_some()
    );
    let dump = fake.all_text();
    assert!(!dump.contains(&crate::crypto::b64_encode(&*auth)));

    // A second setup must not overwrite the vault.
    assert!(matches!(
        b.setup(init(&random_key().unwrap(), &random_key().unwrap()))
            .await,
        Err(Error::RemoteInitialized)
    ));
    assert_eq!(
        fake.scalar("SELECT auth_hash FROM meta").unwrap(),
        auth_hash(&auth)
    );

    let kdf = b.prelogin().await.unwrap();
    assert_eq!(kdf.kdf_salt, "c2FsdHNhbHRzYWx0c2FsdA==");
    let meta = b.fetch_vault().await.unwrap();
    assert_eq!((meta.schema_version, meta.seq), (1, 0));
}

#[tokio::test]
async fn login_and_recover_compare_hashes() {
    let (server, _fake) = start().await;
    let b = backend(&server);
    let (auth, recovery) = (random_key().unwrap(), random_key().unwrap());
    b.setup(init(&auth, &recovery)).await.unwrap();

    assert!(matches!(
        b.login(&[9; 32], &login_info()).await,
        Err(Error::WrongPassword)
    ));
    let session = b.login(&auth, &login_info()).await.unwrap();
    assert_eq!(session.token.as_str(), "d1-direct");
    assert!(session.expires_at > 4_000_000_000_000);

    assert!(matches!(
        b.recover(&[9; 32], &login_info()).await,
        Err(Error::WrongRecoveryCode)
    ));
    // The password is not a recovery code and vice versa.
    assert!(matches!(
        b.recover(&auth, &login_info()).await,
        Err(Error::WrongRecoveryCode)
    ));
    let rec = b.recover(&recovery, &login_info()).await.unwrap();
    assert_eq!(rec.recovery_vault_key, "{\"v\":1,\"n\":\"r\",\"c\":\"r\"}");
    assert_eq!(rec.kdf.kdf_params, KdfParams::for_tests().to_json());
}

#[tokio::test]
async fn push_uses_optimistic_concurrency_and_monotonic_seq() {
    let (server, fake) = start().await;
    let b = backend(&server);
    b.setup(init(&random_key().unwrap(), &random_key().unwrap()))
        .await
        .unwrap();

    let results = b
        .push(vec![
            change("a", 0, Some("env-a1"), 10),
            change("b", 0, Some("env-b1"), 11),
        ])
        .await
        .unwrap();
    assert_eq!(
        results[0],
        PushResult::Ok {
            id: "a".into(),
            revision: 1,
            seq: 1
        }
    );
    assert_eq!(
        results[1],
        PushResult::Ok {
            id: "b".into(),
            revision: 1,
            seq: 2
        }
    );
    assert_eq!(
        fake.scalar("SELECT seq FROM meta").unwrap(),
        "2",
        "meta.seq follows the items"
    );

    // Correct base revision: accepted, gets a new, higher seq.
    let r = b
        .push(vec![change("a", 1, Some("env-a2"), 12)])
        .await
        .unwrap();
    assert_eq!(
        r[0],
        PushResult::Ok {
            id: "a".into(),
            revision: 2,
            seq: 3
        }
    );

    // Stale base revision: conflict, carrying the server's current row.
    let r = b
        .push(vec![change("a", 1, Some("env-a-stale"), 13)])
        .await
        .unwrap();
    let PushResult::Conflict { id, server } = &r[0] else {
        panic!("expected conflict, got {r:?}")
    };
    assert_eq!(id, "a");
    assert_eq!(
        (
            server.revision,
            server.seq,
            server.deleted,
            server.updated_at
        ),
        (2, 3, false, 12)
    );
    assert_eq!(server.envelope.as_deref(), Some("env-a2"));
    assert_eq!(
        fake.scalar("SELECT envelope FROM items WHERE id='a'")
            .unwrap(),
        "env-a2",
        "a lost race changes nothing"
    );

    // Insert of an id that already exists is a conflict too, not an overwrite.
    let r = b
        .push(vec![change("b", 0, Some("env-b-dup"), 14)])
        .await
        .unwrap();
    assert!(matches!(&r[0], PushResult::Conflict { server, .. } if server.revision == 1));

    // Update of an id the server has never seen.
    let r = b
        .push(vec![change("ghost", 3, Some("x"), 15)])
        .await
        .unwrap();
    assert_eq!(
        r[0],
        PushResult::Error {
            id: "ghost".into(),
            error: "not_found".into()
        }
    );

    // Oversized envelope is rejected per item without touching the others.
    let big = "x".repeat(70 * 1024);
    let r = b
        .push(vec![
            change("big", 0, Some(&big), 16),
            change("c", 0, Some("env-c"), 17),
        ])
        .await
        .unwrap();
    assert_eq!(
        r[0],
        PushResult::Error {
            id: "big".into(),
            error: "too_large".into()
        }
    );
    assert!(matches!(r[1], PushResult::Ok { .. }));

    // Tombstone.
    let r = b.push(vec![change("a", 2, None, 18)]).await.unwrap();
    assert!(matches!(r[0], PushResult::Ok { revision: 3, .. }));
    assert_eq!(
        fake.scalar("SELECT deleted FROM items WHERE id='a'")
            .unwrap(),
        "1"
    );
    assert_eq!(
        fake.scalar("SELECT envelope IS NULL FROM items WHERE id='a'")
            .unwrap(),
        "1"
    );

    // seq values are unique and strictly increasing in commit order.
    assert_eq!(
        fake.scalar("SELECT COUNT(*) = COUNT(DISTINCT seq) FROM items")
            .unwrap(),
        "1"
    );
}

/// One change written the way the sync Worker writes it (`applyChange` in
/// `workers/sync/src/routes/items.ts`): bump `meta.seq`, then insert at it, in one transaction.
fn worker_insert(fake: &FakeD1, id: &str) -> u64 {
    let mut db = fake.db.lock().unwrap();
    let tx = db.transaction().unwrap();
    tx.execute(
        "UPDATE meta SET seq = MAX(seq, (SELECT COALESCE(MAX(seq), 0) FROM items)) + 1 WHERE id = 1",
        [],
    )
    .unwrap();
    let seq: i64 = tx
        .query_row(
            "INSERT INTO items (id, envelope, revision, seq, deleted, updated_at) \
             VALUES (?1, 'env', 1, (SELECT seq FROM meta WHERE id = 1), 0, 0) \
             ON CONFLICT(id) DO NOTHING RETURNING seq",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    tx.commit().unwrap();
    u64::try_from(seq).unwrap()
}

#[tokio::test]
async fn direct_mode_and_worker_writes_never_share_a_seq() {
    let (server, fake) = start().await;
    let b = backend(&server);
    b.setup(init(&random_key().unwrap(), &random_key().unwrap()))
        .await
        .unwrap();

    // A direct-mode write lands, but its `meta.seq` catch-up has not run yet.
    fake.hold_meta_catch_up.store(true, Ordering::SeqCst);
    b.push(vec![change("a", 0, Some("env-a"), 1)])
        .await
        .unwrap();
    fake.hold_meta_catch_up.store(false, Ordering::SeqCst);
    assert_eq!(fake.scalar("SELECT seq FROM meta").unwrap(), "0");

    // Another device pulls it and moves its cursor past it.
    let page = b.pull(0, 100).await.unwrap();
    assert_eq!(page.items.len(), 1);
    let cursor = page.next_since;

    // A device on the Worker writes next: its change must sort after the cursor.
    let seq = worker_insert(&fake, "b");
    assert!(seq > cursor, "worker seq {seq} reused cursor {cursor}");
    let page = b.pull(cursor, 100).await.unwrap();
    assert_eq!(
        page.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        ["b"]
    );
    let cursor = page.next_since;

    // The Worker used up a seq on a lost race, so `meta.seq` is ahead of every item.
    fake.db
        .lock()
        .unwrap()
        .execute("UPDATE meta SET seq = seq + 1 WHERE id = 1", [])
        .unwrap();
    let r = b
        .push(vec![change("c", 0, Some("env-c"), 2)])
        .await
        .unwrap();
    let PushResult::Ok { seq, .. } = r[0] else {
        panic!("expected ok, got {r:?}")
    };
    assert_eq!(
        seq,
        cursor + 2,
        "a direct-mode write skips the seq the Worker used up"
    );
    assert_eq!(
        fake.scalar("SELECT COUNT(*) = COUNT(DISTINCT seq) FROM items")
            .unwrap(),
        "1"
    );
}

#[tokio::test]
async fn pull_orders_by_seq_and_paginates() {
    let (server, _fake) = start().await;
    let b = backend(&server);
    b.setup(init(&random_key().unwrap(), &random_key().unwrap()))
        .await
        .unwrap();
    for i in 0..5 {
        b.push(vec![change(&format!("i{i}"), 0, Some(&format!("e{i}")), i)])
            .await
            .unwrap();
    }
    // Re-touch i1 so it moves to the end of the sequence.
    b.push(vec![change("i1", 1, Some("e1b"), 99)])
        .await
        .unwrap();

    let page = b.pull(0, 4).await.unwrap();
    assert_eq!(
        page.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        ["i0", "i2", "i3", "i4"]
    );
    assert!(page.has_more);
    assert_eq!(page.next_since, page.items.last().unwrap().seq);
    let page2 = b.pull(page.next_since, 4).await.unwrap();
    assert_eq!(
        page2
            .items
            .iter()
            .map(|i| (i.id.as_str(), i.revision))
            .collect::<Vec<_>>(),
        [("i1", 2)]
    );
    assert!(!page2.has_more);
    let empty = b.pull(page2.next_since, 4).await.unwrap();
    assert!(empty.items.is_empty() && !empty.has_more);
    assert_eq!(empty.next_since, page2.next_since);

    b.push(vec![change("i0", 1, None, 100)]).await.unwrap();
    let page = b.pull(page2.next_since, 10).await.unwrap();
    assert!(page.items[0].deleted && page.items[0].envelope.is_none());
}

#[tokio::test]
async fn password_change_replaces_hashes_and_optionally_recovery() {
    let (server, fake) = start().await;
    let b = backend(&server);
    let (auth, recovery) = (random_key().unwrap(), random_key().unwrap());
    b.setup(init(&auth, &recovery)).await.unwrap();

    let new_auth = random_key().unwrap();
    b.update_vault_meta(VaultMetaUpdate {
        kdf_salt: "bmV3c2FsdG5ld3NhbHQ=".into(),
        kdf_params: KdfParams::for_tests().to_json(),
        auth_key: new_auth.clone(),
        protected_vault_key: "new-protected".into(),
        recovery: None,
    })
    .await
    .unwrap();
    assert!(matches!(
        b.login(&auth, &login_info()).await,
        Err(Error::WrongPassword)
    ));
    b.login(&new_auth, &login_info()).await.unwrap();
    assert_eq!(
        fake.scalar("SELECT protected_vault_key FROM meta").unwrap(),
        "new-protected"
    );
    assert_eq!(
        fake.scalar("SELECT recovery_auth_hash FROM meta").unwrap(),
        auth_hash(&recovery),
        "recovery untouched"
    );

    let new_recovery = random_key().unwrap();
    b.update_vault_meta(VaultMetaUpdate {
        kdf_salt: "bmV3c2FsdG5ld3NhbHQ=".into(),
        kdf_params: KdfParams::for_tests().to_json(),
        auth_key: new_auth.clone(),
        protected_vault_key: "new-protected".into(),
        recovery: Some(RecoveryUpdate {
            recovery_vault_key: "new-recovery".into(),
            recovery_auth: new_recovery.clone(),
        }),
    })
    .await
    .unwrap();
    assert!(matches!(
        b.recover(&recovery, &login_info()).await,
        Err(Error::WrongRecoveryCode)
    ));
    assert_eq!(
        b.recover(&new_recovery, &login_info())
            .await
            .unwrap()
            .recovery_vault_key,
        "new-recovery"
    );
}

#[tokio::test]
async fn device_management_is_unsupported_in_direct_mode() {
    let (server, _fake) = start().await;
    let b = backend(&server);
    assert!(b.devices().await.unwrap().is_empty());
    assert!(matches!(
        b.revoke_device("x").await,
        Err(Error::Unsupported)
    ));
}

#[tokio::test]
async fn forgotten_credentials_stop_every_query_before_it_is_sent() {
    let (server, fake) = start().await;
    let b = backend(&server);
    b.setup(init(&[1; 32], &[2; 32])).await.unwrap();
    b.login(&[1; 32], &login_info()).await.unwrap();
    let sent = server.received_requests().await.unwrap().len();

    b.forget_credentials();
    assert!(matches!(b.pull(0, 10).await, Err(Error::Unauthorized)));
    assert!(matches!(
        b.push(vec![change("h1", 0, Some("{}"), 1)]).await,
        Err(Error::Unauthorized)
    ));
    assert!(matches!(
        b.login(&[1; 32], &login_info()).await,
        Err(Error::Unauthorized)
    ));
    assert_eq!(server.received_requests().await.unwrap().len(), sent);
    assert_eq!(
        fake.scalar("SELECT COUNT(*) FROM items").as_deref(),
        Some("0")
    );
}

// ---- error mapping and wizard helpers -------------------------------------------------------

#[tokio::test]
async fn token_and_permission_errors_are_mapped() {
    let (server, _fake) = start().await;
    // Wrong token → Cloudflare answers 401.
    let b = D1Backend::with_base_url(
        &format!("{}/client/v4", server.uri()),
        ACCOUNT,
        DATABASE,
        "wrong-token",
    )
    .unwrap();
    assert!(matches!(b.health().await, Err(Error::Unauthorized)));

    // A valid token without D1 permission → 403 / code 10000.
    let denied = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(cf_error(403, 10000, "Authentication error"))
        .mount(&denied)
        .await;
    let b = D1Backend::with_base_url(
        &format!("{}/client/v4", denied.uri()),
        ACCOUNT,
        DATABASE,
        TOKEN,
    )
    .unwrap();
    assert!(matches!(b.health().await, Err(Error::D1Permission)));
    assert!(matches!(
        b.push(vec![change("a", 0, Some("e"), 1)]).await,
        Err(Error::D1Permission)
    ));

    // 10000 with a 400 status is still a permission problem.
    let odd = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(cf_error(400, 10000, "Authentication error"))
        .mount(&odd)
        .await;
    let b = D1Backend::with_base_url(
        &format!("{}/client/v4", odd.uri()),
        ACCOUNT,
        DATABASE,
        TOKEN,
    )
    .unwrap();
    assert!(matches!(b.health().await, Err(Error::D1Permission)));

    // Rate limiting and server failures.
    let limited = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(cf_error(429, 971, "rate limited").insert_header("retry-after", "30"))
        .mount(&limited)
        .await;
    let b = D1Backend::with_base_url(
        &format!("{}/client/v4", limited.uri()),
        ACCOUNT,
        DATABASE,
        TOKEN,
    )
    .unwrap();
    assert!(matches!(
        b.health().await,
        Err(Error::RateLimited {
            retry_after_secs: Some(30)
        })
    ));
    let broken = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(502).set_body_string("<html>bad gateway</html>"))
        .mount(&broken)
        .await;
    let b = D1Backend::with_base_url(
        &format!("{}/client/v4", broken.uri()),
        ACCOUNT,
        DATABASE,
        TOKEN,
    )
    .unwrap();
    assert!(matches!(b.health().await, Err(Error::Server(m)) if m == "http 502"));
}

#[tokio::test]
async fn offline_is_reported() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let b = D1Backend::with_base_url(
        &format!("http://127.0.0.1:{port}/client/v4"),
        ACCOUNT,
        DATABASE,
        TOKEN,
    )
    .unwrap();
    assert!(matches!(b.health().await, Err(Error::Offline)));
}

#[test]
fn constructor_validation() {
    assert!(D1Backend::new("../x", DATABASE, TOKEN).is_err());
    assert!(D1Backend::new(ACCOUNT, "a/b", TOKEN).is_err());
    assert!(D1Backend::new(ACCOUNT, DATABASE, "  ").is_err());
    assert!(D1Backend::with_base_url("http://example.com", ACCOUNT, DATABASE, TOKEN).is_err());
    assert!(D1Backend::new(ACCOUNT, DATABASE, TOKEN).is_ok());
}

#[tokio::test]
async fn verify_token_tries_user_then_account_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/client/v4/user/tokens/verify"))
        .and(header("authorization", "Bearer user-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true, "errors": [], "messages": [],
            "result": { "id": "tok1", "status": "active", "expires_on": "2030-01-01T00:00:00Z" }
        })))
        .mount(&server)
        .await;
    // Account-owned token: the user endpoint rejects it, the account endpoint accepts it.
    Mock::given(method("GET"))
        .and(path("/client/v4/user/tokens/verify"))
        .and(header("authorization", "Bearer account-token"))
        .respond_with(cf_error(401, 1000, "Invalid API Token"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/client/v4/accounts/{ACCOUNT}/tokens/verify")))
        .and(header("authorization", "Bearer account-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true, "errors": [], "messages": [],
            "result": { "id": "tok2", "status": "disabled" }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/client/v4/user/tokens/verify"))
        .respond_with(cf_error(401, 1000, "Invalid API Token"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/client/v4/accounts/{ACCOUNT}/tokens/verify")))
        .respond_with(cf_error(401, 1000, "Invalid API Token"))
        .mount(&server)
        .await;

    let base = format!("{}/client/v4", server.uri());
    let ok = verify_token_at(&base, ACCOUNT, "user-token").await.unwrap();
    assert_eq!((ok.id.as_str(), ok.is_active()), ("tok1", true));
    assert_eq!(ok.expires_on.as_deref(), Some("2030-01-01T00:00:00Z"));
    let acct = verify_token_at(&base, ACCOUNT, "account-token")
        .await
        .unwrap();
    assert_eq!((acct.id.as_str(), acct.is_active()), ("tok2", false));
    assert!(matches!(
        verify_token_at(&base, ACCOUNT, "garbage").await,
        Err(Error::Unauthorized)
    ));
}

#[tokio::test]
async fn list_databases_parses_pages_and_reports_missing_permission() {
    let server = MockServer::start().await;
    let page = |range: std::ops::Range<usize>| {
        let dbs: Vec<Value> = range
            .map(|i| json!({"uuid": format!("uuid-{i}"), "name": format!("db-{i}"), "version": "production", "primary_location_hint": if i == 0 { json!("weur") } else { Value::Null }}))
            .collect();
        ResponseTemplate::new(200)
            .set_body_json(json!({"success": true, "errors": [], "messages": [], "result": dbs}))
    };
    let p = format!("/client/v4/accounts/{ACCOUNT}/d1/database");
    Mock::given(method("GET"))
        .and(path(p.clone()))
        .and(query_param("page", "1"))
        .respond_with(page(0..100))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(p.clone()))
        .and(query_param("page", "2"))
        .respond_with(page(100..103))
        .mount(&server)
        .await;
    let base = format!("{}/client/v4", server.uri());
    let dbs = list_databases_at(&base, ACCOUNT, TOKEN).await.unwrap();
    assert_eq!(dbs.len(), 103);
    assert_eq!(
        (
            dbs[0].id.as_str(),
            dbs[0].name.as_str(),
            dbs[0].region.as_deref()
        ),
        ("uuid-0", "db-0", Some("weur"))
    );
    assert_eq!(dbs[102].region, None);

    let denied = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(cf_error(403, 10000, "Authentication error"))
        .mount(&denied)
        .await;
    let err = list_databases_at(&format!("{}/client/v4", denied.uri()), ACCOUNT, TOKEN)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::D1Permission), "{err:?}");
}

// ---- the engine over D1 ---------------------------------------------------------------------

fn device(
    server: &MockServer,
    clock: &Arc<ManualClock>,
    created: bool,
) -> (SharedVault, D1Backend) {
    let mut vault = Vault::open_in_memory().unwrap().with_clock(clock.clone());
    if created {
        vault
            .create_with_params(PW, KdfParams::for_tests())
            .unwrap();
    }
    (share(vault), backend(server))
}

fn config() -> SyncConfig {
    SyncConfig::D1 {
        account_id: "acc".into(),
        database_id: "db".into(),
    }
}

fn info(name: &str) -> DeviceInfo {
    DeviceInfo {
        name: name.into(),
        platform: "test".into(),
    }
}

fn names(v: &SharedVault) -> Vec<String> {
    let g = v.lock().unwrap();
    let mut n: Vec<String> = g.hosts().into_iter().map(|(_, h)| h.name).collect();
    n.sort();
    n
}

#[tokio::test]
async fn two_devices_sync_through_d1_direct_mode() {
    let (server, fake) = start().await;
    let clock = Arc::new(ManualClock::new(1_700_000_000_000));
    let (a, a_backend) = device(&server, &clock, true);
    let (b, b_backend) = device(&server, &clock, false);

    let host_id = a
        .lock()
        .unwrap()
        .put(
            None,
            Item::Host(Host {
                name: "d1-host".into(),
                ..Host::default()
            }),
        )
        .unwrap();
    // No setup token in direct mode.
    enable_sync(&a, &a_backend, PW, None, info("a"))
        .await
        .unwrap();
    restore_from_cloud(&b, &b_backend, PW, info("b"), &config())
        .await
        .unwrap();
    engine::sync_round(&b, &b_backend, &SyncOptions::default())
        .await
        .unwrap();
    assert_eq!(names(&b), ["d1-host"]);

    // D1 only ever sees ciphertext.
    let dump = fake.all_text();
    assert!(!dump.contains("d1-host") && !dump.contains(PW));

    // Concurrent edits to the same host: the newer one wins on both devices.
    clock.advance(1000);
    {
        let mut g = a.lock().unwrap();
        let mut item = g.get(&host_id).unwrap().clone();
        if let Item::Host(h) = &mut item {
            h.name = "edited-on-a".into();
        }
        g.put(Some(&host_id), item).unwrap();
    }
    clock.advance(1000);
    {
        let mut g = b.lock().unwrap();
        let mut item = g.get(&host_id).unwrap().clone();
        if let Item::Host(h) = &mut item {
            h.name = "edited-on-b".into();
        }
        g.put(Some(&host_id), item).unwrap();
    }
    for (vault, backend) in [(&a, &a_backend), (&b, &b_backend), (&a, &a_backend)] {
        engine::sync_round(vault, backend, &SyncOptions::default())
            .await
            .unwrap();
    }
    assert_eq!(names(&a), ["edited-on-b"]);
    assert_eq!(names(&b), ["edited-on-b"]);
    assert_eq!(b.lock().unwrap().unreviewed_conflict_count(), 1);

    // Password change through D1: the old password stops working for new devices.
    change_password_remote(&a, &a_backend, PW, "new-d1-password")
        .await
        .unwrap();
    let (c, c_backend) = device(&server, &clock, false);
    assert!(matches!(
        restore_from_cloud(&c, &c_backend, PW, info("c"), &config()).await,
        Err(Error::WrongPassword)
    ));
    restore_from_cloud(&c, &c_backend, "new-d1-password", info("c"), &config())
        .await
        .unwrap();
    engine::sync_round(&c, &c_backend, &SyncOptions::default())
        .await
        .unwrap();
    assert_eq!(names(&c), ["edited-on-b"]);

    // meta.seq tracks the highest item seq.
    assert_eq!(
        fake.scalar("SELECT seq FROM meta"),
        fake.scalar("SELECT MAX(seq) FROM items")
    );
}

#[tokio::test]
async fn direct_mode_push_conflict_triggers_the_engine_retry_path() {
    let (server, _fake) = start().await;
    let clock = Arc::new(ManualClock::new(1_700_000_000_000));
    let (a, a_backend) = device(&server, &clock, true);
    let (b, b_backend) = device(&server, &clock, false);
    let id = a
        .lock()
        .unwrap()
        .put(
            None,
            Item::Host(Host {
                name: "shared".into(),
                ..Host::default()
            }),
        )
        .unwrap();
    enable_sync(&a, &a_backend, PW, None, info("a"))
        .await
        .unwrap();
    restore_from_cloud(&b, &b_backend, PW, info("b"), &config())
        .await
        .unwrap();
    engine::sync_round(&b, &b_backend, &SyncOptions::default())
        .await
        .unwrap();

    // B edits and pushes; A, still on the old revision, edits later and syncs (pull first, so
    // the conflict is resolved at pull time and A's newer edit is pushed on top).
    clock.advance(1000);
    let edit = |vault: &SharedVault, name: &str| {
        let mut g = vault.lock().unwrap();
        let mut item = g.get(&id).unwrap().clone();
        if let Item::Host(h) = &mut item {
            h.name = name.into();
        }
        g.put(Some(&id), item).unwrap();
    };
    edit(&b, "b-first");
    engine::sync_round(&b, &b_backend, &SyncOptions::default())
        .await
        .unwrap();
    clock.advance(1000);
    edit(&a, "a-later");
    let report = engine::sync_round(&a, &a_backend, &SyncOptions::default())
        .await
        .unwrap();
    assert_eq!(
        (
            report.conflicts_resolved,
            report.pushed,
            report.pending_after
        ),
        (1, 1, 0)
    );
    engine::sync_round(&b, &b_backend, &SyncOptions::default())
        .await
        .unwrap();
    assert_eq!(names(&a), ["a-later"]);
    assert_eq!(names(&b), ["a-later"]);
}
