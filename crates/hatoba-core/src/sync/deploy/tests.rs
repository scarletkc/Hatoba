//! The deployment against a local stand-in for the Cloudflare API.
//!
//! `FakeCloudflare` keeps Workers, secrets, routes, and D1 databases in memory, and runs each D1
//! query against a real SQLite database, so the migrations and checks the deployment sends are
//! actually executed. The same server answers the deployed Worker's `/v1/health`, for upgrades
//! too.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::types::ValueRef;
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use super::*;

const ACCOUNT: &str = "0123456789abcdef0123456789abcdef";
const TOKEN: &str = "cf-deploy-token-for-tests";
const API: &str = "/client/v4";
const VERSION: &str = "0.3.0";
const MIGRATION: &str = include_str!("../../../../../workers/sync/migrations/0001_init.sql");
/// The bundle of the upgrade tests: the next Worker version, with one more migration.
const NEXT_VERSION: &str = "0.4.0";
const NEXT_MIGRATION: (&str, &str) = (
    "0002_item_hints.sql",
    "ALTER TABLE items ADD COLUMN hint TEXT;",
);

fn bundle_of(version: &str, migrations: &[(&str, &str)]) -> Arc<WorkerBundle> {
    let migrations: Vec<Value> = migrations
        .iter()
        .map(|(name, sql)| json!({ "name": name, "sql": sql }))
        .collect();
    Arc::new(
        WorkerBundle::parse(
            &json!({
                "format": 1,
                "version": version,
                "name": "hatoba-sync",
                "compatibility_date": "2026-09-01",
                "compatibility_flags": [],
                "module": { "name": "index.js", "content": "export default { fetch() {} };" },
                "migrations": migrations,
                "d1": { "binding": "DB", "database_name": "hatoba" },
                "ratelimit": { "name": "AUTH_LIMITER", "namespace_id": "1001", "limit": 10, "period": 60 },
            })
            .to_string(),
        )
        .unwrap(),
    )
}

fn bundle() -> Arc<WorkerBundle> {
    bundle_of(VERSION, &[("0001_init.sql", MIGRATION)])
}

fn next_bundle() -> Arc<WorkerBundle> {
    bundle_of(
        NEXT_VERSION,
        &[("0001_init.sql", MIGRATION), NEXT_MIGRATION],
    )
}

fn target() -> Target {
    Target {
        account_id: ACCOUNT.into(),
        worker_name: "hatoba-sync".into(),
        database_name: "hatoba".into(),
        subdomain: None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TokenMode {
    User,
    AccountOwned,
    Disabled,
}

#[derive(Default)]
struct Script {
    bindings: Vec<Value>,
    secrets: BTreeSet<String>,
    route: bool,
    /// Reachable on a custom domain, without its workers.dev route.
    custom_domain: bool,
    /// What `/v1/health` reports: set by an upload, or by a test for an existing Worker.
    version: Option<String>,
}

struct Database {
    uuid: String,
    name: String,
    conn: rusqlite::Connection,
}

struct State {
    token: TokenMode,
    subdomain: Option<String>,
    scripts: BTreeMap<String, Script>,
    databases: Vec<Database>,
    /// Calls that need this permission are refused.
    denied: Option<Permission>,
    /// The next request whose `METHOD path` contains this fails with HTTP 500.
    fail_once: Option<String>,
    /// `/v1/health` answers only while this is set, as after DNS propagation.
    health: bool,
    next_id: u32,
    /// `METHOD path` of every API call.
    log: Vec<String>,
    /// The metadata part of the last upload.
    upload: Option<Value>,
    /// The version an upload installs.
    upload_version: String,
    /// The migrations the uploaded Worker's database had applied when the upload arrived.
    migrated_at_upload: Vec<String>,
}

#[derive(Clone)]
struct FakeCloudflare(Arc<Mutex<State>>);

fn ok(result: Value) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_json(json!({ "success": true, "errors": [], "messages": [], "result": result }))
}

fn cf_error(status: u16, code: u32) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({
        "success": false, "errors": [{ "code": code, "message": "error" }], "messages": [], "result": null
    }))
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

/// Runs every statement in `sql` as one transaction, as D1's query endpoint does.
fn run_batch(conn: &rusqlite::Connection, sql: &str) -> rusqlite::Result<Vec<Value>> {
    conn.execute_batch("SAVEPOINT batch")?;
    let run = || -> rusqlite::Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut batch = rusqlite::Batch::new(conn, sql);
        while let Some(mut stmt) = batch.next()? {
            let names: Vec<String> = stmt
                .column_names()
                .iter()
                .map(|s| (*s).to_owned())
                .collect();
            let mut results = Vec::new();
            if names.is_empty() {
                stmt.raw_execute()?;
            } else {
                let mut rows = stmt.raw_query();
                while let Some(row) = rows.next()? {
                    let mut obj = serde_json::Map::new();
                    for (i, name) in names.iter().enumerate() {
                        obj.insert(name.clone(), from_sql(row.get_ref(i)?));
                    }
                    results.push(Value::Object(obj));
                }
            }
            out.push(json!({ "results": results, "success": true, "meta": {} }));
        }
        Ok(out)
    };
    match run() {
        Ok(out) => {
            conn.execute_batch("RELEASE batch")?;
            Ok(out)
        }
        Err(e) => {
            conn.execute_batch("ROLLBACK TO batch; RELEASE batch")?;
            Err(e)
        }
    }
}

/// The JSON metadata part of a Workers upload, after checking the module part.
fn upload_metadata(request: &Request) -> Value {
    let content_type = request
        .headers
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    let boundary = content_type
        .strip_prefix("multipart/form-data; boundary=")
        .unwrap();
    let body = String::from_utf8(request.body.clone()).unwrap();
    let parts: Vec<&str> = body.split(&format!("--{boundary}")).collect();
    assert_eq!(
        parts.last().map(|p| p.trim()),
        Some("--"),
        "closing boundary"
    );
    let module = parts
        .iter()
        .find(|p| p.contains("name=\"index.js\""))
        .expect("module part");
    assert!(module.contains("filename=\"index.js\""));
    assert!(module.contains("Content-Type: application/javascript+module"));
    assert!(module.contains("export default { fetch() {} };"));
    let metadata = parts
        .iter()
        .find(|p| p.contains("name=\"metadata\""))
        .expect("metadata part");
    let json = metadata.split("\r\n\r\n").nth(1).unwrap().trim_end();
    serde_json::from_str(json).unwrap()
}

impl State {
    fn database(&self, id: &str) -> Option<&Database> {
        self.databases.iter().find(|d| d.uuid == id)
    }

    fn health(&self) -> ResponseTemplate {
        let Some(script) = self
            .scripts
            .values()
            .find(|s| (s.route || s.custom_domain) && s.version.is_some())
        else {
            return ResponseTemplate::new(404);
        };
        if !self.health {
            return ResponseTemplate::new(522);
        }
        let db = script
            .bindings
            .iter()
            .find(|b| b["type"] == "d1")
            .and_then(|b| self.database(b["database_id"].as_str().unwrap()));
        let initialized = db.is_some_and(|d| {
            d.conn
                .query_row("SELECT COUNT(*) FROM meta WHERE id = 1", [], |r| {
                    r.get::<_, i64>(0)
                })
                .is_ok_and(|n| n > 0)
        });
        ResponseTemplate::new(200).set_body_json(json!({
            "service": "hatoba-sync", "version": script.version, "api": 1, "initialized": initialized
        }))
    }

    fn handle(&mut self, request: &Request) -> ResponseTemplate {
        let full = request.url.path().to_owned();
        if full == "/v1/health" {
            return self.health();
        }
        let path = full.strip_prefix(API).expect("API path").to_owned();
        let method = request.method.to_string();
        let line = format!("{method} {path}");
        self.log.push(line.clone());
        let auth = request
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok());
        if auth != Some(&format!("Bearer {TOKEN}")) {
            return cf_error(401, 1000);
        }
        if self
            .fail_once
            .as_ref()
            .is_some_and(|f| line.contains(f.as_str()))
        {
            self.fail_once = None;
            return cf_error(500, 10013);
        }
        let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let workers = self.denied != Some(Permission::WorkersScripts);
        let d1 = self.denied != Some(Permission::D1);

        match (method.as_str(), segments.as_slice()) {
            ("GET", ["user", "tokens", "verify"]) => match self.token {
                TokenMode::User => ok(json!({ "id": "t1", "status": "active" })),
                TokenMode::Disabled => ok(json!({ "id": "t1", "status": "disabled" })),
                TokenMode::AccountOwned => cf_error(401, 1000),
            },
            ("GET", ["accounts", ACCOUNT, "tokens", "verify"]) => match self.token {
                TokenMode::AccountOwned => ok(json!({ "id": "t2", "status": "active" })),
                _ => cf_error(401, 1000),
            },
            ("GET", ["accounts"]) => {
                let page = request
                    .url
                    .query_pairs()
                    .find(|(k, _)| k == "page")
                    .map(|(_, v)| v.into_owned());
                if page.as_deref() == Some("1") {
                    ok(json!([{ "id": ACCOUNT, "name": "Kc's Account" }]))
                } else {
                    ok(json!([]))
                }
            }
            (_, ["accounts", ACCOUNT, "workers", ..]) if !workers => cf_error(403, 10000),
            (_, ["accounts", ACCOUNT, "d1", ..]) if !d1 => cf_error(403, 10000),
            ("GET", ["accounts", ACCOUNT, "workers", "subdomain"]) => match &self.subdomain {
                Some(s) => ok(json!({ "subdomain": s })),
                None => cf_error(404, 10007),
            },
            ("PUT", ["accounts", ACCOUNT, "workers", "subdomain"]) => {
                let wanted = body["subdomain"].as_str().unwrap().to_owned();
                if wanted == "taken" {
                    return cf_error(409, 10031);
                }
                self.subdomain = Some(wanted.clone());
                ok(json!({ "subdomain": wanted }))
            }
            ("GET", ["accounts", ACCOUNT, "workers", "scripts", name, "settings"]) => {
                match self.scripts.get(*name) {
                    Some(s) => {
                        ok(json!({ "bindings": s.bindings, "compatibility_date": "2026-09-01" }))
                    }
                    None => cf_error(404, 10007),
                }
            }
            ("PUT", ["accounts", ACCOUNT, "workers", "scripts", name]) => {
                let metadata = upload_metadata(request);
                let bound = metadata["bindings"][0]["database_id"].as_str().unwrap();
                self.migrated_at_upload = self
                    .database(bound)
                    .and_then(|d| {
                        let mut stmt = d.conn.prepare("SELECT name FROM d1_migrations").ok()?;
                        let rows = stmt.query_map([], |r| r.get(0)).ok()?;
                        Some(rows.map(Result::unwrap).collect())
                    })
                    .unwrap_or_default();
                let version = self.upload_version.clone();
                let script = self.scripts.entry((*name).to_owned()).or_default();
                script.bindings = metadata["bindings"].as_array().unwrap().clone();
                script.version = Some(version);
                if !metadata["keep_bindings"]
                    .as_array()
                    .is_some_and(|k| k.contains(&json!("secret_text")))
                {
                    script.secrets.clear();
                }
                self.upload = Some(metadata);
                ok(json!({ "id": name }))
            }
            ("DELETE", ["accounts", ACCOUNT, "workers", "scripts", name]) => {
                match self.scripts.remove(*name) {
                    Some(_) => ok(Value::Null),
                    None => cf_error(404, 10007),
                }
            }
            ("PUT", ["accounts", ACCOUNT, "workers", "scripts", name, "secrets"]) => {
                assert_eq!(body["type"], "secret_text");
                match self.scripts.get_mut(*name) {
                    Some(s) => {
                        s.secrets.insert(body["name"].as_str().unwrap().to_owned());
                        ok(json!({ "name": body["name"], "type": "secret_text" }))
                    }
                    None => cf_error(404, 10007),
                }
            }
            (
                "DELETE",
                [
                    "accounts",
                    ACCOUNT,
                    "workers",
                    "scripts",
                    name,
                    "secrets",
                    secret,
                ],
            ) => {
                let removed = self
                    .scripts
                    .get_mut(*name)
                    .is_some_and(|s| s.secrets.remove(*secret));
                if removed {
                    ok(Value::Null)
                } else {
                    cf_error(404, 10056)
                }
            }
            ("POST", ["accounts", ACCOUNT, "workers", "scripts", name, "subdomain"]) => {
                assert_eq!(body, json!({ "enabled": true, "previews_enabled": false }));
                match self.scripts.get_mut(*name) {
                    Some(s) => {
                        s.route = true;
                        ok(body)
                    }
                    None => cf_error(404, 10007),
                }
            }
            ("GET", ["accounts", ACCOUNT, "d1", "database"]) => {
                let filter = request
                    .url
                    .query_pairs()
                    .find(|(k, _)| k == "name")
                    .map(|(_, v)| v.into_owned())
                    .unwrap_or_default();
                let page = request
                    .url
                    .query_pairs()
                    .find(|(k, _)| k == "page")
                    .map(|(_, v)| v.into_owned());
                let list: Vec<Value> = if page.as_deref() == Some("1") {
                    self.databases
                        .iter()
                        .filter(|d| d.name.contains(&filter))
                        .map(|d| json!({ "uuid": d.uuid, "name": d.name, "version": "production" }))
                        .collect()
                } else {
                    Vec::new()
                };
                ok(json!(list))
            }
            ("POST", ["accounts", ACCOUNT, "d1", "database"]) => {
                let name = body["name"].as_str().unwrap().to_owned();
                if self.databases.iter().any(|d| d.name == name) {
                    return cf_error(400, 7502);
                }
                self.next_id += 1;
                let uuid = format!("db-{}", self.next_id);
                self.databases.push(Database {
                    uuid: uuid.clone(),
                    name: name.clone(),
                    conn: rusqlite::Connection::open_in_memory().unwrap(),
                });
                ok(json!({ "uuid": uuid, "name": name }))
            }
            ("GET", ["accounts", ACCOUNT, "d1", "database", id]) => match self.database(id) {
                Some(d) => ok(json!({ "uuid": d.uuid, "name": d.name })),
                None => cf_error(404, 7404),
            },
            ("DELETE", ["accounts", ACCOUNT, "d1", "database", id]) => {
                let before = self.databases.len();
                self.databases.retain(|d| d.uuid != *id);
                if self.databases.len() < before {
                    ok(Value::Null)
                } else {
                    cf_error(404, 7404)
                }
            }
            ("POST", ["accounts", ACCOUNT, "d1", "database", id, "query"]) => {
                let Some(db) = self.database(id) else {
                    return cf_error(404, 7404);
                };
                match run_batch(&db.conn, body["sql"].as_str().unwrap()) {
                    Ok(results) => ok(json!(results)),
                    Err(_) => cf_error(400, 7500),
                }
            }
            _ => panic!("unexpected request: {line}"),
        }
    }
}

impl Respond for FakeCloudflare {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        self.0.lock().unwrap().handle(request)
    }
}

struct Harness {
    server: MockServer,
    cf: FakeCloudflare,
}

impl Harness {
    async fn new() -> Self {
        let cf = FakeCloudflare(Arc::new(Mutex::new(State {
            token: TokenMode::User,
            subdomain: Some("kc".into()),
            scripts: BTreeMap::new(),
            databases: Vec::new(),
            denied: None,
            fail_once: None,
            health: true,
            next_id: 0,
            log: Vec::new(),
            upload: None,
            upload_version: VERSION.into(),
            migrated_at_upload: Vec::new(),
        })));
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::any())
            .respond_with(cf.clone())
            .mount(&server)
            .await;
        Self { server, cf }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.cf.0.lock().unwrap()
    }

    fn deployment(&self) -> Deployment {
        self.deployment_of(bundle())
    }

    fn deployment_of(&self, bundle: Arc<WorkerBundle>) -> Deployment {
        let mut d = Deployment::with_endpoints(
            &format!("{}{API}", self.server.uri()),
            Some(&self.server.uri()),
            TOKEN,
            bundle,
        )
        .unwrap();
        d.set_polling(Polling {
            interval: Duration::from_millis(10),
            timeout: Duration::from_millis(60),
        });
        d
    }

    fn add_database(&self, name: &str, sql: &str) -> String {
        let mut st = self.state();
        st.next_id += 1;
        let uuid = format!("db-{}", st.next_id);
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(sql).unwrap();
        st.databases.push(Database {
            uuid: uuid.clone(),
            name: name.into(),
            conn,
        });
        uuid
    }

    fn add_hatoba_worker(&self, name: &str, database_id: &str) {
        self.state().scripts.insert(
            name.into(),
            Script {
                bindings: vec![
                    json!({ "type": "d1", "name": "DB", "id": database_id }),
                    json!({ "type": "ratelimit", "name": "AUTH_LIMITER", "namespace_id": "1001", "simple": { "limit": 10, "period": 60 } }),
                ],
                ..Script::default()
            },
        );
    }

    /// A Worker that holds a vault and answers `/v1/health` with `version`, as a deployment
    /// from an earlier app version leaves it, with a secret of the user's own.
    fn add_synced_worker(&self, name: &str, version: &str) -> String {
        let db = self.add_database("hatoba", &with_vault());
        self.add_hatoba_worker(name, &db);
        let mut st = self.state();
        let script = st.scripts.get_mut(name).unwrap();
        script.bindings[0] = json!({ "type": "d1", "name": "DB", "database_id": db });
        script.route = true;
        script.version = Some(version.into());
        script.secrets.insert("OWN_SECRET".into());
        st.upload_version = NEXT_VERSION.into();
        db
    }

    fn upgrader(&self) -> Deployment {
        self.deployment_of(next_bundle())
    }

    fn log(&self) -> Vec<String> {
        self.state().log.clone()
    }

    /// `/v1/health` as the configured URL answers it.
    async fn health(&self) -> ServerInfo {
        WorkerBackend::new(&self.server.uri())
            .unwrap()
            .health()
            .await
            .unwrap()
    }
}

const WORKERS_DEV: &str = "https://hatoba-sync.kc.workers.dev";

fn upgrade_target(url: &str) -> UpgradeTarget {
    UpgradeTarget {
        account_id: ACCOUNT.into(),
        worker_name: "hatoba-sync".into(),
        url: url.into(),
    }
}

/// What wrangler's migration leaves behind.
fn migrated() -> String {
    format!(
        "CREATE TABLE d1_migrations(id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT UNIQUE, applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP NOT NULL);\n{MIGRATION}\nINSERT INTO d1_migrations (name) VALUES ('0001_init.sql');"
    )
}

fn with_vault() -> String {
    format!(
        "{}\nINSERT INTO meta VALUES (1, 1, 's', '{{}}', 'h', 'p', 'r', 'rh', 0, 0);",
        migrated()
    )
}

type Events = Arc<Mutex<Vec<(Step, StepStatus)>>>;

fn recorder() -> (Events, impl Fn(Step, StepStatus) + Send + Sync) {
    let events: Events = Arc::default();
    let sink = Arc::clone(&events);
    (events, move |step, status| {
        sink.lock().unwrap().push((step, status))
    })
}

// ---- step 1 ----------------------------------------------------------------------------------

#[tokio::test]
async fn verify_lists_the_accounts_of_a_user_token() {
    let h = Harness::new().await;
    let info = h.deployment().verify(None).await.unwrap();
    assert_eq!(
        info,
        TokenInfo {
            account_owned: false,
            accounts: vec![Account {
                id: ACCOUNT.into(),
                name: "Kc's Account".into()
            }],
        }
    );
}

#[tokio::test]
async fn verify_accepts_an_account_owned_token_only_with_its_account() {
    let h = Harness::new().await;
    h.state().token = TokenMode::AccountOwned;
    let d = h.deployment();
    assert!(matches!(d.verify(None).await, Err(Error::CloudflareToken)));
    let info = d.verify(Some(ACCOUNT)).await.unwrap();
    assert!(info.account_owned);
    assert!(info.accounts.is_empty());
}

#[tokio::test]
async fn verify_rejects_disabled_and_unknown_tokens() {
    let h = Harness::new().await;
    h.state().token = TokenMode::Disabled;
    assert!(matches!(
        h.deployment().verify(None).await,
        Err(Error::CloudflareToken)
    ));
    let wrong = Deployment::with_endpoints(
        &format!("{}{API}", h.server.uri()),
        None,
        "some-other-token",
        bundle(),
    )
    .unwrap();
    assert!(matches!(
        wrong.verify(Some(ACCOUNT)).await,
        Err(Error::CloudflareToken)
    ));
    assert!(matches!(
        Deployment::new("  ", bundle()),
        Err(Error::CloudflareToken)
    ));
}

// ---- step 2 ----------------------------------------------------------------------------------

#[tokio::test]
async fn inspect_plans_a_fresh_account_and_writes_nothing() {
    let h = Harness::new().await;
    let plan = h.deployment().inspect(&target()).await.unwrap();
    assert_eq!(
        plan,
        Inspection {
            subdomain: Some("kc".into()),
            worker: WorkerPlan::Create,
            database: Some(DatabasePlan::Create {
                name: "hatoba".into()
            }),
        }
    );
    assert!(
        h.log().iter().all(|l| l.starts_with("GET ")),
        "{:?}",
        h.log()
    );
}

#[tokio::test]
async fn inspect_names_the_missing_permission() {
    let h = Harness::new().await;
    h.state().denied = Some(Permission::WorkersScripts);
    assert!(matches!(
        h.deployment().inspect(&target()).await,
        Err(Error::CloudflarePermission(Permission::WorkersScripts))
    ));
    h.state().denied = Some(Permission::D1);
    assert!(matches!(
        h.deployment().inspect(&target()).await,
        Err(Error::CloudflarePermission(Permission::D1))
    ));
}

#[tokio::test]
async fn inspect_never_plans_over_a_foreign_worker_or_a_vault() {
    let h = Harness::new().await;
    h.state().scripts.insert(
        "hatoba-sync".into(),
        Script {
            bindings: vec![json!({ "type": "kv_namespace", "name": "DB", "namespace_id": "x" })],
            ..Script::default()
        },
    );
    let plan = h.deployment().inspect(&target()).await.unwrap();
    assert_eq!((plan.worker, plan.database), (WorkerPlan::Foreign, None));

    let db = h.add_database("synced", &with_vault());
    h.add_hatoba_worker("hatoba-sync", &db);
    let plan = h.deployment().inspect(&target()).await.unwrap();
    assert_eq!((plan.worker, plan.database), (WorkerPlan::HasVault, None));
}

#[tokio::test]
async fn inspect_reuses_a_hatoba_worker_without_a_vault_and_its_database() {
    let h = Harness::new().await;
    let db = h.add_database("from-wrangler", &migrated());
    h.add_hatoba_worker("hatoba-sync", &db);
    let plan = h.deployment().inspect(&target()).await.unwrap();
    assert_eq!(plan.worker, WorkerPlan::Update);
    assert_eq!(
        plan.database,
        Some(DatabasePlan::Bound {
            id: db,
            name: "from-wrangler".into()
        })
    );
}

#[tokio::test]
async fn inspect_skips_databases_that_hold_anything_else() {
    let h = Harness::new().await;
    h.add_database("hatoba", &with_vault());
    h.add_database("hatoba-2", "CREATE TABLE notes (body TEXT);");
    // A D1 direct mode database without a vault has Hatoba's tables but no migration records.
    h.add_database(
        "hatoba-3",
        "CREATE TABLE meta (id INTEGER PRIMARY KEY, x TEXT); CREATE TABLE items (id TEXT);",
    );
    h.add_database("hatoba-30", "");
    let plan = h.deployment().inspect(&target()).await.unwrap();
    assert_eq!(
        plan.database,
        Some(DatabasePlan::Create {
            name: "hatoba-4".into()
        })
    );

    let empty = h.add_database("hatoba-4", "");
    let plan = h.deployment().inspect(&target()).await.unwrap();
    assert_eq!(
        plan.database,
        Some(DatabasePlan::Use {
            id: empty,
            name: "hatoba-4".into()
        })
    );
}

#[tokio::test]
async fn inspect_checks_the_names() {
    let h = Harness::new().await;
    let d = h.deployment();
    for (field, t) in [
        (
            "worker_name",
            Target {
                worker_name: "Hatoba".into(),
                ..target()
            },
        ),
        (
            "worker_name",
            Target {
                worker_name: "-sync".into(),
                ..target()
            },
        ),
        (
            "database_name",
            Target {
                database_name: "hat oba".into(),
                ..target()
            },
        ),
        (
            "subdomain",
            Target {
                subdomain: Some("a.b".into()),
                ..target()
            },
        ),
        (
            "account_id",
            Target {
                account_id: "../x".into(),
                ..target()
            },
        ),
    ] {
        match d.inspect(&t).await {
            Err(Error::InvalidItem(f)) => assert_eq!(f, field),
            other => panic!("{field}: {other:?}"),
        }
    }
    assert!(h.log().is_empty());
}

// ---- steps 3 to 8 ----------------------------------------------------------------------------

#[tokio::test]
async fn deploys_to_a_fresh_account() {
    let h = Harness::new().await;
    let mut d = h.deployment();
    let (events, progress) = recorder();
    assert_eq!(
        d.deploy(&target(), &progress).await.unwrap(),
        Outcome::Ready
    );

    use StepStatus::*;
    assert_eq!(
        *events.lock().unwrap(),
        [
            (Step::Inspect, Running),
            (Step::Inspect, Done),
            (Step::CreateDatabase, Running),
            (Step::CreateDatabase, Done),
            (Step::Migrate, Running),
            (Step::Migrate, Done),
            (Step::Upload, Running),
            (Step::Upload, Done),
            (Step::SetupToken, Running),
            (Step::SetupToken, Done),
            (Step::Route, Running),
            (Step::Route, Done),
            (Step::Wait, Running),
            (Step::Wait, Done),
        ]
    );

    let st = h.state();
    let db = &st.databases[0];
    assert_eq!(db.name, "hatoba");
    let recorded: String = db
        .conn
        .query_row("SELECT name FROM d1_migrations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(recorded, "0001_init.sql");
    let script = &st.scripts["hatoba-sync"];
    assert!(script.route);
    assert_eq!(script.secrets, BTreeSet::from(["SETUP_TOKEN".to_owned()]));
    let metadata = st.upload.clone().unwrap();
    assert_eq!(metadata["main_module"], "index.js");
    assert_eq!(metadata["compatibility_date"], "2026-09-01");
    assert_eq!(metadata["keep_bindings"], json!(["secret_text"]));
    assert_eq!(
        metadata["bindings"],
        json!([
            { "type": "d1", "name": "DB", "database_id": db.uuid },
            { "type": "ratelimit", "name": "AUTH_LIMITER", "namespace_id": "1001", "simple": { "limit": 10, "period": 60 } },
        ])
    );
    drop(st);

    let deployed = d.deployed().unwrap();
    assert_eq!(deployed.url, "https://hatoba-sync.kc.workers.dev");
    assert_eq!(deployed.worker_name, "hatoba-sync");
    let token = d.setup_token().unwrap();
    assert_eq!(token.len(), 43, "32 bytes as unpadded base64url");
    assert!(
        token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    );
    assert_eq!(
        *d.created(),
        Created {
            worker: Some((ACCOUNT.into(), "hatoba-sync".into())),
            database: Some((ACCOUNT.into(), "db-1".into())),
        }
    );
    assert!(!format!("{d:?}").contains(token));
}

#[tokio::test]
async fn creates_the_workers_dev_subdomain_when_the_account_has_none() {
    let h = Harness::new().await;
    h.state().subdomain = None;
    let mut d = h.deployment();
    let (_, progress) = recorder();

    let failure = d.deploy(&target(), &progress).await.unwrap_err();
    assert_eq!(failure.step, Step::Inspect);
    assert!(matches!(failure.error, Error::SubdomainRequired));

    let taken = Target {
        subdomain: Some("taken".into()),
        ..target()
    };
    let failure = d.deploy(&taken, &progress).await.unwrap_err();
    assert!(matches!(failure.error, Error::SubdomainUnavailable));
    assert!(
        d.created().is_empty(),
        "nothing is written before the subdomain"
    );

    let mine = Target {
        subdomain: Some("kc-2".into()),
        ..target()
    };
    assert_eq!(d.deploy(&mine, &progress).await.unwrap(), Outcome::Ready);
    assert_eq!(h.state().subdomain.as_deref(), Some("kc-2"));
    assert_eq!(
        d.deployed().unwrap().url,
        "https://hatoba-sync.kc-2.workers.dev"
    );
}

#[tokio::test]
async fn stops_before_writing_over_a_vault_or_another_worker() {
    let h = Harness::new().await;
    let db = h.add_database("hatoba", &with_vault());
    h.add_hatoba_worker("hatoba-sync", &db);
    let (_, progress) = recorder();
    let failure = h
        .deployment()
        .deploy(&target(), &progress)
        .await
        .unwrap_err();
    assert_eq!(failure.step, Step::Inspect);
    assert!(matches!(failure.error, Error::RemoteInitialized));

    h.state()
        .scripts
        .get_mut("hatoba-sync")
        .unwrap()
        .bindings
        .clear();
    let failure = h
        .deployment()
        .deploy(&target(), &progress)
        .await
        .unwrap_err();
    assert!(matches!(failure.error, Error::WorkerNameTaken));
    assert!(
        h.log()
            .iter()
            .all(|l| l.starts_with("GET ") || l.ends_with("/query"))
    );
}

#[tokio::test]
async fn retry_continues_where_the_last_attempt_stopped() {
    let h = Harness::new().await;
    let mut d = h.deployment();
    let (_, progress) = recorder();

    h.state().fail_once =
        Some("PUT /accounts/".to_owned() + ACCOUNT + "/workers/scripts/hatoba-sync");
    let failure = d.deploy(&target(), &progress).await.unwrap_err();
    assert_eq!(failure.step, Step::Upload);
    assert!(matches!(
        failure.error,
        Error::Cloudflare {
            status: 500,
            code: Some(10013)
        }
    ));

    let (events, progress) = recorder();
    assert_eq!(
        d.deploy(&target(), &progress).await.unwrap(),
        Outcome::Ready
    );
    let skipped = |step| {
        events
            .lock()
            .unwrap()
            .contains(&(step, StepStatus::Skipped))
    };
    assert!(skipped(Step::CreateDatabase));
    assert!(skipped(Step::Migrate));
    assert_eq!(
        h.state().databases.len(),
        1,
        "the first attempt's database is reused"
    );
    assert_eq!(
        d.created().database,
        Some((ACCOUNT.into(), "db-1".into())),
        "and still counts as created by this deployment"
    );

    // A third run, as after leaving the wizard before setup, deploys over its own Worker.
    let mut again = h.deployment();
    assert_eq!(
        again.deploy(&target(), &progress).await.unwrap(),
        Outcome::Ready
    );
    assert!(again.created().is_empty());
}

#[tokio::test]
async fn applies_only_missing_migrations_to_a_reused_database() {
    let h = Harness::new().await;
    let db = h.add_database("hatoba", &migrated());
    let mut d = h.deployment();
    let (events, progress) = recorder();
    assert_eq!(
        d.deploy(&target(), &progress).await.unwrap(),
        Outcome::Ready
    );
    assert!(
        events
            .lock()
            .unwrap()
            .contains(&(Step::Migrate, StepStatus::Skipped))
    );
    assert_eq!(
        d.created().database,
        None,
        "a database that existed before is never cleanup's to delete"
    );
    assert_eq!(
        h.state().upload.clone().unwrap()["bindings"][0]["database_id"],
        db
    );
}

#[tokio::test]
async fn waits_for_the_worker_and_checks_again() {
    let h = Harness::new().await;
    h.state().health = false;
    let mut d = h.deployment();
    let (events, progress) = recorder();
    assert_eq!(
        d.deploy(&target(), &progress).await.unwrap(),
        Outcome::Waiting
    );
    assert_eq!(
        events.lock().unwrap().last(),
        Some(&(Step::Wait, StepStatus::Running))
    );
    assert!(!d.check_ready().await.unwrap());
    h.state().health = true;
    assert!(d.check_ready().await.unwrap());
}

#[tokio::test]
async fn finish_deletes_the_setup_token() {
    let h = Harness::new().await;
    let mut d = h.deployment();
    let (_, progress) = recorder();
    d.deploy(&target(), &progress).await.unwrap();
    d.finish().await.unwrap();
    assert!(d.setup_token().is_none());
    assert!(h.state().scripts["hatoba-sync"].secrets.is_empty());
    // A second call finds nothing to delete.
    d.finish().await.unwrap();
}

#[tokio::test]
async fn cleanup_removes_only_what_this_deployment_created() {
    let h = Harness::new().await;
    let older = h.add_database("unrelated", "CREATE TABLE notes (body TEXT);");
    let mut d = h.deployment();
    let (_, progress) = recorder();
    h.state().fail_once = Some("scripts/hatoba-sync/subdomain".into());
    let failure = d.deploy(&target(), &progress).await.unwrap_err();
    assert_eq!(failure.step, Step::Route);

    d.remove_created().await.unwrap();
    {
        let st = h.state();
        assert!(st.scripts.is_empty());
        assert_eq!(
            st.databases
                .iter()
                .map(|d| d.uuid.clone())
                .collect::<Vec<_>>(),
            [older]
        );
        let deletes: Vec<&String> = st.log.iter().filter(|l| l.starts_with("DELETE")).collect();
        assert_eq!(
            deletes,
            [
                &format!("DELETE /accounts/{ACCOUNT}/workers/scripts/hatoba-sync"),
                &format!("DELETE /accounts/{ACCOUNT}/d1/database/db-2"),
            ],
            "the Worker before its database"
        );
    }
    assert!(d.created().is_empty());
    assert!(d.setup_token().is_none());
    d.remove_created().await.unwrap();
}

#[tokio::test]
async fn reused_resources_are_never_removed() {
    let h = Harness::new().await;
    let db = h.add_database("from-wrangler", &migrated());
    h.add_hatoba_worker("hatoba-sync", &db);
    let mut d = h.deployment();
    let (_, progress) = recorder();
    assert_eq!(
        d.deploy(&target(), &progress).await.unwrap(),
        Outcome::Ready
    );
    assert!(d.created().is_empty());
    d.remove_created().await.unwrap();
    assert!(h.state().scripts.contains_key("hatoba-sync"));
    assert_eq!(h.state().databases.len(), 1);
}

// ---- upgrades --------------------------------------------------------------------------------

#[tokio::test]
async fn upgrade_applies_the_new_migration_before_the_new_code() {
    let h = Harness::new().await;
    let db = h.add_synced_worker("hatoba-sync", VERSION);
    let mut d = h.upgrader();

    let plan = d
        .inspect_upgrade(&upgrade_target(WORKERS_DEV))
        .await
        .unwrap();
    assert_eq!(
        plan,
        UpgradeInspection {
            worker: UpgradeWorker::Ready {
                database_id: db.clone(),
                database_name: "hatoba".into()
            },
            migrations: 1,
            route: true,
            version: Some(VERSION.into()),
        }
    );
    assert!(
        h.log()
            .iter()
            .all(|l| l.starts_with("GET ") || l.ends_with("/query"))
    );

    let (events, progress) = recorder();
    assert_eq!(
        d.upgrade(&upgrade_target(WORKERS_DEV), &progress)
            .await
            .unwrap(),
        Outcome::Ready
    );
    use StepStatus::*;
    assert_eq!(
        *events.lock().unwrap(),
        [
            (Step::Inspect, Running),
            (Step::Inspect, Done),
            (Step::Migrate, Running),
            (Step::Migrate, Done),
            (Step::Upload, Running),
            (Step::Upload, Done),
            (Step::Route, Running),
            (Step::Route, Done),
            (Step::Wait, Running),
            (Step::Wait, Done),
        ],
        "steps 3 and 6 never run"
    );

    let st = h.state();
    assert_eq!(
        st.migrated_at_upload,
        ["0001_init.sql", NEXT_MIGRATION.0],
        "the new code arrived after its migration"
    );
    assert_eq!(st.databases.len(), 1);
    let conn = &st.databases[0].conn;
    let vaults: i64 = conn
        .query_row("SELECT COUNT(*) FROM meta WHERE id = 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(vaults, 1, "the vault is untouched");
    conn.execute("UPDATE items SET hint = 'x'", []).unwrap();
    let script = &st.scripts["hatoba-sync"];
    assert_eq!(script.version.as_deref(), Some(NEXT_VERSION));
    assert_eq!(
        script.secrets,
        BTreeSet::from(["OWN_SECRET".to_owned()]),
        "the Worker keeps its secrets and gets no setup token"
    );
    assert_eq!(st.upload.clone().unwrap()["bindings"][0]["database_id"], db);
    assert!(
        !st.log
            .iter()
            .any(|l| l.ends_with("/d1/database") || l.contains("/secrets")),
        "{:?}",
        st.log
    );
    drop(st);

    assert!(d.is_upgrade());
    assert!(
        d.created().is_empty(),
        "an upgrade creates nothing to clean up"
    );
    assert!(d.setup_token().is_none());
    assert_eq!(d.deployed().unwrap().url, WORKERS_DEV);
}

#[tokio::test]
async fn retry_continues_a_failed_upgrade() {
    let h = Harness::new().await;
    h.add_synced_worker("hatoba-sync", VERSION);
    let mut d = h.upgrader();
    let (_, progress) = recorder();

    h.state().fail_once = Some(format!(
        "PUT /accounts/{ACCOUNT}/workers/scripts/hatoba-sync"
    ));
    let failure = d
        .upgrade(&upgrade_target(WORKERS_DEV), &progress)
        .await
        .unwrap_err();
    assert_eq!(failure.step, Step::Upload);
    // The old code runs on the new migration in between, and still holds the vault.
    let info = h.health().await;
    assert_eq!((info.version.as_str(), info.initialized), (VERSION, true));

    let (events, progress) = recorder();
    assert_eq!(
        d.upgrade(&upgrade_target(WORKERS_DEV), &progress)
            .await
            .unwrap(),
        Outcome::Ready
    );
    let events = events.lock().unwrap();
    assert!(events.contains(&(Step::Migrate, StepStatus::Skipped)));
    assert!(events.contains(&(Step::Upload, StepStatus::Done)));
    assert_eq!(h.health().await.version, NEXT_VERSION);
}

#[tokio::test]
async fn upgrade_waits_for_the_new_version_and_checks_again() {
    let h = Harness::new().await;
    h.add_synced_worker("hatoba-sync", VERSION);
    h.state().health = false;
    let mut d = h.upgrader();
    let (_, progress) = recorder();
    assert_eq!(
        d.upgrade(&upgrade_target(WORKERS_DEV), &progress)
            .await
            .unwrap(),
        Outcome::Waiting
    );
    assert!(!d.check_ready().await.unwrap());
    h.state().health = true;
    assert!(d.check_ready().await.unwrap());
}

#[tokio::test]
async fn upgrade_on_a_custom_domain_leaves_workers_dev_alone() {
    let h = Harness::new().await;
    h.add_synced_worker("hatoba-sync", VERSION);
    {
        let mut st = h.state();
        let script = st.scripts.get_mut("hatoba-sync").unwrap();
        script.route = false;
        script.custom_domain = true;
    }
    let mut d = h.upgrader();
    let target = upgrade_target("https://sync.example.com/");
    assert!(!d.inspect_upgrade(&target).await.unwrap().route);

    let (events, progress) = recorder();
    assert_eq!(d.upgrade(&target, &progress).await.unwrap(), Outcome::Ready);
    assert!(
        events
            .lock()
            .unwrap()
            .contains(&(Step::Route, StepStatus::Skipped))
    );
    assert!(!h.state().scripts["hatoba-sync"].route);
    assert!(
        !h.log()
            .iter()
            .any(|l| l.ends_with("/subdomain") && l.starts_with("POST"))
    );
    assert_eq!(d.deployed().unwrap().url, "https://sync.example.com");
}

#[tokio::test]
async fn upgrade_requires_a_hatoba_worker_with_a_vault() {
    let h = Harness::new().await;
    let d = h.upgrader();
    let target = upgrade_target(WORKERS_DEV);
    let plan = |worker| UpgradeInspection {
        worker,
        migrations: 0,
        route: true,
        version: None,
    };
    assert_eq!(
        d.inspect_upgrade(&target).await.unwrap(),
        plan(UpgradeWorker::Missing)
    );

    // A Worker deployed but never set up holds no vault.
    let empty = h.add_database("hatoba", &migrated());
    h.add_hatoba_worker("hatoba-sync", &empty);
    assert_eq!(
        d.inspect_upgrade(&target).await.unwrap(),
        plan(UpgradeWorker::NoVault)
    );
    h.state().databases.clear();
    assert_eq!(
        d.inspect_upgrade(&target).await.unwrap(),
        plan(UpgradeWorker::NoVault)
    );
    h.state()
        .scripts
        .get_mut("hatoba-sync")
        .unwrap()
        .bindings
        .clear();
    assert_eq!(
        d.inspect_upgrade(&target).await.unwrap(),
        plan(UpgradeWorker::Foreign)
    );

    let mut d = h.upgrader();
    let (_, progress) = recorder();
    let failure = d.upgrade(&target, &progress).await.unwrap_err();
    assert_eq!(failure.step, Step::Inspect);
    assert!(matches!(failure.error, Error::WorkerNotFound));
    assert!(
        h.log()
            .iter()
            .all(|l| l.starts_with("GET ") || l.ends_with("/query"))
    );
}

#[tokio::test]
async fn upgrade_never_replaces_a_newer_worker() {
    let h = Harness::new().await;
    h.add_synced_worker("hatoba-sync", "0.5.0");
    let mut d = h.upgrader();
    let plan = d
        .inspect_upgrade(&upgrade_target(WORKERS_DEV))
        .await
        .unwrap();
    assert_eq!(
        (plan.worker, plan.version.as_deref()),
        (UpgradeWorker::Newer, Some("0.5.0"))
    );
    let (_, progress) = recorder();
    let failure = d
        .upgrade(&upgrade_target(WORKERS_DEV), &progress)
        .await
        .unwrap_err();
    assert!(matches!(failure.error, Error::WorkerNewer));
    assert!(!h.log().iter().any(|l| l.starts_with("PUT ")));
}

#[tokio::test]
async fn upgrade_checks_its_target() {
    let h = Harness::new().await;
    let d = h.upgrader();
    for (field, target) in [
        (
            "account_id",
            UpgradeTarget {
                account_id: "../x".into(),
                ..upgrade_target(WORKERS_DEV)
            },
        ),
        (
            "worker_name",
            UpgradeTarget {
                worker_name: "Sync".into(),
                ..upgrade_target(WORKERS_DEV)
            },
        ),
        ("url", upgrade_target("http://sync.example.com")),
    ] {
        match d.inspect_upgrade(&target).await {
            Err(Error::InvalidItem(f)) => assert_eq!(f, field),
            other => panic!("{field}: {other:?}"),
        }
    }
    assert!(h.log().is_empty());
}
