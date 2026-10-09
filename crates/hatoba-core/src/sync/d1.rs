//! [`D1Backend`]: direct mode (P1), talking to Cloudflare D1 through its REST API with a
//! Cloudflare API token instead of a Worker.
//!
//! The tables are the Worker's (spec §5.3) minus `sessions`; there is no server logic, so the
//! guarantees the Worker provides are re-created on the client with plain SQL:
//!
//! * **Login** fetches `auth_hash` and compares it with `SHA-256(auth_key)` in constant time.
//!   The "session" is a placeholder: the API token is the real credential.
//! * **Push** is optimistic concurrency per item, one atomic statement each:
//!   `INSERT … ON CONFLICT(id) DO NOTHING` for new items, `UPDATE … WHERE id = ? AND revision = ?`
//!   otherwise; zero `meta.changes` means somebody else got there first, and the current row is
//!   read back as a conflict.
//! * **`seq`** is assigned *inside* the write statement as one more than the larger of
//!   `meta.seq` and `MAX(items.seq)`. The Worker bumps `meta.seq` to the same value and writes in
//!   one D1 batch; over the REST API two separate calls would not be atomic, and a writer that
//!   allocated a lower number but committed later could slip behind another client's pull cursor
//!   and be missed forever. A single statement cannot. Both backends take the larger of the two
//!   counters because each one may lag: `meta.seq` behind a D1-direct write that has not caught it
//!   up yet, `MAX(items.seq)` behind a seq the Worker used up on a lost race. Using only one of
//!   them could hand out a seq that is already taken. `meta.seq` is still brought up to date at
//!   the end of each push.
//!
//! The Cloudflare API token usually grants edit access to *every* D1 database on the account;
//! the UI must say so. The token is held only in memory here and stored by the shell in the OS
//! credential store.

use std::sync::RwLock;

use async_trait::async_trait;
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::crypto::auth_hash;
use crate::error::{Error, Result};
use crate::sync::backend::{
    Change, DeviceLogin, KdfInfo, PullPage, PushResult, Recovered, RemoteDevice, RemoteItem,
    ServerInfo, Session, SyncBackend, VaultInit, VaultMeta, VaultMetaUpdate,
};
use crate::sync::http::{
    build_client, error_code, is_loopback_host, map_transport, read_body, retry_after,
};

/// Production API base.
pub const CLOUDFLARE_API_BASE: &str = "https://api.cloudflare.com/client/v4";
/// Largest accepted envelope, as on the Worker.
const MAX_ENVELOPE_BYTES: usize = 64 * 1024;
/// Placeholder expiry for the dummy session (year 9999).
const FAR_FUTURE_MS: i64 = 253_402_300_799_000;
/// Marker for "the table does not exist yet".
const NO_SUCH_TABLE: &str = "d1 no such table";

const SCHEMA: [&str; 3] = [
    "CREATE TABLE IF NOT EXISTS meta (
        id                  INTEGER PRIMARY KEY CHECK (id = 1),
        schema_version      INTEGER NOT NULL,
        kdf_salt            TEXT NOT NULL,
        kdf_params          TEXT NOT NULL,
        auth_hash           TEXT NOT NULL,
        protected_vault_key TEXT NOT NULL,
        recovery_vault_key  TEXT NOT NULL,
        recovery_auth_hash  TEXT NOT NULL,
        seq                 INTEGER NOT NULL DEFAULT 0,
        created_at          INTEGER NOT NULL
     )",
    "CREATE TABLE IF NOT EXISTS items (
        id         TEXT PRIMARY KEY,
        envelope   TEXT,
        revision   INTEGER NOT NULL,
        seq        INTEGER NOT NULL,
        deleted    INTEGER NOT NULL DEFAULT 0,
        updated_at INTEGER NOT NULL
     )",
    "CREATE INDEX IF NOT EXISTS idx_items_seq ON items(seq)",
];

/// The next `seq`, shared with the Worker (`workers/sync/src/routes/items.ts`): one more than
/// both `meta.seq` and every item's `seq`.
const NEXT_SEQ: &str = "(SELECT MAX(COALESCE((SELECT seq FROM meta WHERE id = 1), 0), \
                        COALESCE((SELECT MAX(seq) FROM items), 0)) + 1)";

fn valid_identifier(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

// ---- wire types -----------------------------------------------------------------------------

#[derive(Deserialize)]
struct CfError {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct CfEnvelope<T> {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    errors: Vec<CfError>,
    result: Option<T>,
}

#[derive(Deserialize)]
struct StatementWire {
    #[serde(default)]
    results: Vec<serde_json::Map<String, Value>>,
    #[serde(default = "default_true")]
    success: bool,
    #[serde(default)]
    meta: MetaWire,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize, Default)]
struct MetaWire {
    #[serde(default)]
    changes: u64,
}

/// Rows and change count of one statement.
struct Rows {
    rows: Vec<serde_json::Map<String, Value>>,
    changes: u64,
}

impl Rows {
    fn first(&self) -> Option<&serde_json::Map<String, Value>> {
        self.rows.first()
    }
}

fn str_of(row: &serde_json::Map<String, Value>, key: &str) -> Result<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Error::Protocol(format!("missing column {key}")))
}

fn opt_str_of(row: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    row.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn u64_of(row: &serde_json::Map<String, Value>, key: &str) -> Result<u64> {
    row.get(key)
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
        })
        .ok_or_else(|| Error::Protocol(format!("missing column {key}")))
}

fn i64_of(row: &serde_json::Map<String, Value>, key: &str) -> i64 {
    row.get(key)
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
        .unwrap_or(0)
}

fn item_from_row(row: &serde_json::Map<String, Value>) -> Result<RemoteItem> {
    let deleted = i64_of(row, "deleted") != 0;
    Ok(RemoteItem {
        id: str_of(row, "id")?,
        envelope: opt_str_of(row, "envelope").filter(|_| !deleted),
        revision: u64_of(row, "revision")?,
        seq: u64_of(row, "seq")?,
        deleted,
        updated_at: i64_of(row, "updated_at"),
    })
}

/// Maps the Cloudflare error list of a failed call.
fn map_cf_errors(status: StatusCode, errors: &[CfError], body: &[u8], retry: Option<u64>) -> Error {
    if errors.iter().any(|e| e.message.contains("no such table")) {
        return Error::Server(NO_SUCH_TABLE.to_owned());
    }
    // 10000 = "Authentication error" (token lacks the permission or is for another account).
    if status == StatusCode::FORBIDDEN
        || errors.iter().any(|e| matches!(e.code, 10000 | 9109 | 9106))
    {
        return Error::D1Permission;
    }
    match status {
        StatusCode::UNAUTHORIZED => Error::Unauthorized,
        StatusCode::TOO_MANY_REQUESTS => Error::RateLimited {
            retry_after_secs: retry,
        },
        StatusCode::NOT_FOUND => Error::Server("d1 database not found".into()),
        other => {
            let code = errors
                .first()
                .map(|e| e.code.to_string())
                .or_else(|| error_code(body));
            Error::Server(format!(
                "http {}{}",
                other.as_u16(),
                code.map(|c| format!(" code {c}")).unwrap_or_default()
            ))
        }
    }
}

/// Where the REST API lives, with the credentials for it.
struct Api {
    client: reqwest::Client,
    base: String,
    token: Zeroizing<String>,
}

impl Api {
    fn new(base: &str, token: &str) -> Result<Self> {
        let base = base.trim().trim_end_matches('/').to_owned();
        let parsed = reqwest::Url::parse(&base)
            .map_err(|_| Error::InvalidUrl("invalid API base URL".into()))?;
        let host = parsed.host_str().unwrap_or_default();
        let ok =
            parsed.scheme() == "https" || (parsed.scheme() == "http" && is_loopback_host(host));
        if !ok {
            return Err(Error::InvalidUrl("API base must be https".into()));
        }
        if token.trim().is_empty() {
            return Err(Error::Unauthorized);
        }
        Ok(Self {
            client: build_client(is_loopback_host(host))?,
            base,
            token: Zeroizing::new(token.trim().to_owned()),
        })
    }

    /// Sends a request and decodes Cloudflare's `{success, errors, result}` envelope.
    async fn call<T: for<'de> Deserialize<'de>>(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<T> {
        let mut auth =
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", self.token.as_str()))
                .map_err(|_| Error::Unauthorized)?;
        auth.set_sensitive(true);
        let mut req = self
            .client
            .request(method, format!("{}{path}", self.base))
            .header(reqwest::header::AUTHORIZATION, auth);
        if let Some(body) = body {
            req = req
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(serde_json::to_vec(&body)?);
        }
        let resp = req.send().await.map_err(|e| map_transport(&e))?;
        let (status, headers, bytes) = read_body(resp).await?;
        let envelope: Option<CfEnvelope<T>> = serde_json::from_slice(&bytes).ok();
        match envelope {
            Some(CfEnvelope {
                success: true,
                result: Some(result),
                ..
            }) if status.is_success() => Ok(result),
            Some(env) => Err(map_cf_errors(
                status,
                &env.errors,
                &bytes,
                retry_after(&headers),
            )),
            None if status.is_success() => Err(Error::Protocol("malformed response".into())),
            None => Err(map_cf_errors(status, &[], &bytes, retry_after(&headers))),
        }
    }
}

/// Talks to a D1 database directly over Cloudflare's REST API.
pub struct D1Backend {
    api: Api,
    account_id: String,
    database_id: String,
    session: RwLock<Option<Session>>,
}

impl D1Backend {
    /// A backend for `database_id` on `account_id`, authenticated by `api_token`.
    ///
    /// # Errors
    /// [`Error::InvalidUrl`] for malformed identifiers; [`Error::Unauthorized`] for an empty token.
    pub fn new(account_id: &str, database_id: &str, api_token: &str) -> Result<Self> {
        Self::with_base_url(CLOUDFLARE_API_BASE, account_id, database_id, api_token)
    }

    /// As [`new`](Self::new) against another API base (tests; `http` allowed for loopback).
    ///
    /// # Errors
    /// As [`new`](Self::new).
    pub fn with_base_url(
        base: &str,
        account_id: &str,
        database_id: &str,
        api_token: &str,
    ) -> Result<Self> {
        if !valid_identifier(account_id) || !valid_identifier(database_id) {
            return Err(Error::InvalidUrl("invalid account or database id".into()));
        }
        Ok(Self {
            api: Api::new(base, api_token)?,
            account_id: account_id.to_owned(),
            database_id: database_id.to_owned(),
            session: RwLock::new(None),
        })
    }

    async fn query(&self, sql: &str, params: Vec<Value>) -> Result<Rows> {
        let path = format!(
            "/accounts/{}/d1/database/{}/query",
            self.account_id, self.database_id
        );
        let statements: Vec<StatementWire> = self
            .api
            .call(
                Method::POST,
                &path,
                Some(json!({ "sql": sql, "params": params })),
            )
            .await?;
        let stmt = statements
            .into_iter()
            .next()
            .ok_or_else(|| Error::Protocol("empty query result".into()))?;
        if !stmt.success {
            return Err(Error::Server("d1 statement failed".into()));
        }
        Ok(Rows {
            rows: stmt.results,
            changes: stmt.meta.changes,
        })
    }

    /// Runs a query, treating a missing `meta` table as "no rows".
    async fn query_or_empty(&self, sql: &str, params: Vec<Value>) -> Result<Rows> {
        match self.query(sql, params).await {
            Err(Error::Server(m)) if m == NO_SUCH_TABLE => Ok(Rows {
                rows: Vec::new(),
                changes: 0,
            }),
            other => other,
        }
    }

    fn dummy_session(&self) -> Session {
        let session = Session {
            token: Zeroizing::new("d1-direct".to_owned()),
            expires_at: FAR_FUTURE_MS,
        };
        if let Ok(mut guard) = self.session.write() {
            *guard = Some(session.clone());
        }
        session
    }

    /// Fetches the row and checks `presented` against the hash in `column`
    /// (`auth_hash` or `recovery_auth_hash`).
    async fn stored_hash(
        &self,
        column: &str,
        presented: &[u8; 32],
    ) -> Result<serde_json::Map<String, Value>> {
        debug_assert!(matches!(column, "auth_hash" | "recovery_auth_hash"));
        let rows = self
            .query_or_empty(
                &format!("SELECT {column}, recovery_vault_key, kdf_salt, kdf_params FROM meta WHERE id = 1"),
                vec![],
            )
            .await?;
        let row = rows.first().ok_or(Error::RemoteNotInitialized)?.clone();
        let stored = str_of(&row, column)?;
        let presented = auth_hash(presented);
        // Constant-time comparison of the hex digests.
        if !bool::from(stored.as_bytes().ct_eq(presented.as_bytes())) {
            return Err(if column == "auth_hash" {
                Error::WrongPassword
            } else {
                Error::WrongRecoveryCode
            });
        }
        Ok(row)
    }

    async fn push_one(&self, c: &Change) -> Result<PushResult> {
        if c.envelope
            .as_ref()
            .is_some_and(|e| e.len() > MAX_ENVELOPE_BYTES)
        {
            return Ok(PushResult::Error {
                id: c.id.clone(),
                error: "too_large".into(),
            });
        }
        let deleted = i64::from(c.deleted);
        let applied = if c.base_revision == 0 {
            self.query(
                &format!(
                    "INSERT INTO items (id, envelope, revision, seq, deleted, updated_at) \
                     VALUES (?, ?, 1, {NEXT_SEQ}, ?, ?) ON CONFLICT(id) DO NOTHING RETURNING revision, seq"
                ),
                vec![json!(c.id), json!(c.envelope), json!(deleted), json!(c.updated_at)],
            )
            .await?
        } else {
            self.query(
                &format!(
                    "UPDATE items SET envelope = ?, deleted = ?, revision = revision + 1, seq = {NEXT_SEQ}, \
                     updated_at = ? WHERE id = ? AND revision = ? RETURNING revision, seq"
                ),
                vec![json!(c.envelope), json!(deleted), json!(c.updated_at), json!(c.id), json!(c.base_revision)],
            )
            .await?
        };
        if applied.changes > 0 {
            let row = applied
                .first()
                .ok_or_else(|| Error::Protocol("write returned no row".into()))?;
            return Ok(PushResult::Ok {
                id: c.id.clone(),
                revision: u64_of(row, "revision")?,
                seq: u64_of(row, "seq")?,
            });
        }
        // Zero rows changed: somebody else changed the item (or it does not exist).
        let current = self
            .query(
                "SELECT id, envelope, revision, seq, deleted, updated_at FROM items WHERE id = ?",
                vec![json!(c.id)],
            )
            .await?;
        match current.first() {
            Some(row) => Ok(PushResult::Conflict {
                id: c.id.clone(),
                server: item_from_row(row)?,
            }),
            None => Ok(PushResult::Error {
                id: c.id.clone(),
                error: "not_found".into(),
            }),
        }
    }
}

// ---- wizard helpers -------------------------------------------------------------------------

/// Result of checking an API token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenStatus {
    /// Token id.
    pub id: String,
    /// `active`, `disabled` or `expired`.
    pub status: String,
    /// Expiry timestamp as Cloudflare reports it, if the token expires.
    pub expires_on: Option<String>,
}

impl TokenStatus {
    /// Whether the token can be used.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.status == "active"
    }
}

/// A D1 database on the account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct D1Database {
    /// Database UUID (what `D1Backend::new` takes as `database_id`).
    pub id: String,
    /// Database name.
    pub name: String,
    /// Location hint, if the database has one.
    pub region: Option<String>,
}

#[derive(Deserialize)]
struct TokenWire {
    id: String,
    status: String,
    expires_on: Option<String>,
}

#[derive(Deserialize)]
struct DatabaseWire {
    uuid: String,
    name: String,
    primary_location_hint: Option<String>,
}

/// Checks an API token (`GET /user/tokens/verify`, falling back to the account-scoped endpoint
/// for account-owned tokens).
///
/// # Errors
/// [`Error::Unauthorized`] if Cloudflare does not recognise the token; transport errors.
pub async fn verify_token(account_id: &str, token: &str) -> Result<TokenStatus> {
    verify_token_at(CLOUDFLARE_API_BASE, account_id, token).await
}

/// As [`verify_token`] against another API base.
///
/// # Errors
/// As [`verify_token`].
pub async fn verify_token_at(base: &str, account_id: &str, token: &str) -> Result<TokenStatus> {
    if !valid_identifier(account_id) {
        return Err(Error::InvalidUrl("invalid account id".into()));
    }
    let api = Api::new(base, token)?;
    let wire: TokenWire = match api.call(Method::GET, "/user/tokens/verify", None).await {
        Ok(w) => w,
        // Account-owned tokens are not valid on the user endpoint.
        Err(Error::Unauthorized | Error::D1Permission) => {
            api.call(
                Method::GET,
                &format!("/accounts/{account_id}/tokens/verify"),
                None,
            )
            .await?
        }
        Err(e) => return Err(e),
    };
    Ok(TokenStatus {
        id: wire.id,
        status: wire.status,
        expires_on: wire.expires_on,
    })
}

/// Lists the account's D1 databases.
///
/// # Errors
/// [`Error::D1Permission`] if the token cannot read D1; [`Error::Unauthorized`] for a bad token.
pub async fn list_databases(account_id: &str, token: &str) -> Result<Vec<D1Database>> {
    list_databases_at(CLOUDFLARE_API_BASE, account_id, token).await
}

/// As [`list_databases`] against another API base.
///
/// # Errors
/// As [`list_databases`].
pub async fn list_databases_at(
    base: &str,
    account_id: &str,
    token: &str,
) -> Result<Vec<D1Database>> {
    if !valid_identifier(account_id) {
        return Err(Error::InvalidUrl("invalid account id".into()));
    }
    let api = Api::new(base, token)?;
    let mut out = Vec::new();
    for page in 1..=20 {
        let wires: Vec<DatabaseWire> = api
            .call(
                Method::GET,
                &format!("/accounts/{account_id}/d1/database?per_page=100&page={page}"),
                None,
            )
            .await?;
        let n = wires.len();
        out.extend(wires.into_iter().map(|w| D1Database {
            id: w.uuid,
            name: w.name,
            region: w.primary_location_hint,
        }));
        if n < 100 {
            break;
        }
    }
    Ok(out)
}

// ---- the backend ----------------------------------------------------------------------------

#[async_trait]
impl SyncBackend for D1Backend {
    async fn health(&self) -> Result<ServerInfo> {
        let rows = self
            .query_or_empty("SELECT 1 AS present FROM meta WHERE id = 1", vec![])
            .await?;
        Ok(ServerInfo {
            service: "hatoba-d1-direct".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            api: 1,
            initialized: !rows.rows.is_empty(),
        })
    }

    async fn prelogin(&self) -> Result<KdfInfo> {
        let rows = self
            .query_or_empty("SELECT kdf_salt, kdf_params FROM meta WHERE id = 1", vec![])
            .await?;
        let row = rows.first().ok_or(Error::RemoteNotInitialized)?;
        Ok(KdfInfo {
            kdf_salt: str_of(row, "kdf_salt")?,
            kdf_params: str_of(row, "kdf_params")?,
        })
    }

    async fn setup(&self, init: VaultInit) -> Result<()> {
        for statement in SCHEMA {
            self.query(statement, vec![]).await?;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
        let inserted = self
            .query(
                "INSERT INTO meta (id, schema_version, kdf_salt, kdf_params, auth_hash, protected_vault_key, \
                 recovery_vault_key, recovery_auth_hash, seq, created_at) VALUES (1, ?, ?, ?, ?, ?, ?, ?, 0, ?) \
                 ON CONFLICT(id) DO NOTHING",
                vec![
                    json!(init.schema_version),
                    json!(init.kdf_salt),
                    json!(init.kdf_params),
                    json!(auth_hash(&init.auth_key)),
                    json!(init.protected_vault_key),
                    json!(init.recovery_vault_key),
                    json!(auth_hash(&init.recovery_auth)),
                    json!(now),
                ],
            )
            .await?;
        if inserted.changes == 0 {
            return Err(Error::RemoteInitialized);
        }
        Ok(())
    }

    async fn login(&self, auth_key: &[u8; 32], _device: &DeviceLogin) -> Result<Session> {
        self.stored_hash("auth_hash", auth_key).await?;
        Ok(self.dummy_session())
    }

    async fn recover(&self, recovery_auth: &[u8; 32], _device: &DeviceLogin) -> Result<Recovered> {
        let row = self
            .stored_hash("recovery_auth_hash", recovery_auth)
            .await?;
        Ok(Recovered {
            recovery_vault_key: str_of(&row, "recovery_vault_key")?,
            kdf: KdfInfo {
                kdf_salt: str_of(&row, "kdf_salt")?,
                kdf_params: str_of(&row, "kdf_params")?,
            },
            session: self.dummy_session(),
        })
    }

    async fn fetch_vault(&self) -> Result<VaultMeta> {
        let rows = self
            .query_or_empty(
                "SELECT schema_version, kdf_salt, kdf_params, protected_vault_key, recovery_vault_key, seq FROM meta WHERE id = 1",
                vec![],
            )
            .await?;
        let row = rows.first().ok_or(Error::RemoteNotInitialized)?;
        Ok(VaultMeta {
            schema_version: u32::try_from(u64_of(row, "schema_version")?).unwrap_or(1),
            kdf_salt: str_of(row, "kdf_salt")?,
            kdf_params: str_of(row, "kdf_params")?,
            protected_vault_key: str_of(row, "protected_vault_key")?,
            recovery_vault_key: str_of(row, "recovery_vault_key")?,
            seq: u64_of(row, "seq")?,
        })
    }

    async fn pull(&self, since_seq: u64, limit: u32) -> Result<PullPage> {
        let limit = limit.clamp(1, 1000);
        let rows = self
            .query(
                "SELECT id, envelope, revision, seq, deleted, updated_at FROM items WHERE seq > ? ORDER BY seq LIMIT ?",
                vec![json!(since_seq), json!(limit + 1)],
            )
            .await?;
        let has_more = rows.rows.len() > limit as usize;
        let items = rows
            .rows
            .iter()
            .take(limit as usize)
            .map(item_from_row)
            .collect::<Result<Vec<_>>>()?;
        let next_since = items.last().map_or(since_seq, |i| i.seq);
        Ok(PullPage {
            items,
            next_since,
            has_more,
        })
    }

    async fn push(&self, changes: Vec<Change>) -> Result<Vec<PushResult>> {
        let mut results = Vec::with_capacity(changes.len());
        for change in &changes {
            results.push(self.push_one(change).await?);
        }
        if results.iter().any(|r| matches!(r, PushResult::Ok { .. })) {
            // Keep `meta.seq` (what a Worker would use) at least as high as any item.
            self.query("UPDATE meta SET seq = (SELECT COALESCE(MAX(seq), 0) FROM items) WHERE id = 1 AND seq < (SELECT COALESCE(MAX(seq), 0) FROM items)", vec![])
                .await?;
        }
        Ok(results)
    }

    async fn update_vault_meta(&self, update: VaultMetaUpdate) -> Result<()> {
        let mut sql = String::from(
            "UPDATE meta SET kdf_salt = ?, kdf_params = ?, auth_hash = ?, protected_vault_key = ?",
        );
        let mut params = vec![
            json!(update.kdf_salt),
            json!(update.kdf_params),
            json!(auth_hash(&update.auth_key)),
            json!(update.protected_vault_key),
        ];
        if let Some(recovery) = &update.recovery {
            sql.push_str(", recovery_vault_key = ?, recovery_auth_hash = ?");
            params.push(json!(recovery.recovery_vault_key));
            params.push(json!(auth_hash(&recovery.recovery_auth)));
        }
        sql.push_str(" WHERE id = 1");
        let result = self.query_or_empty(&sql, params).await?;
        if result.changes == 0 {
            return Err(Error::RemoteNotInitialized);
        }
        Ok(())
    }

    async fn devices(&self) -> Result<Vec<RemoteDevice>> {
        // No sessions table in direct mode, hence no device list.
        Ok(Vec::new())
    }

    async fn revoke_device(&self, _device_id: &str) -> Result<()> {
        Err(Error::Unsupported)
    }

    fn set_session(&self, session: Option<Session>) {
        if let Ok(mut guard) = self.session.write() {
            *guard = session;
        }
    }
}
