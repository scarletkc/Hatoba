//! The Cloudflare API calls the deployment makes (spec §6.7, deployment steps). The token goes
//! only to the API base, and nothing here logs a request or response body.

use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::Permission;
use super::bundle::WorkerBundle;
use crate::error::{Error, Result};
use crate::sync::http::{build_client, is_loopback_host, map_transport, read_body, retry_after};

/// Cloudflare's code for a Worker, script setting, or workers.dev subdomain that does not exist.
const NOT_FOUND: u32 = 10007;
/// Cloudflare's code for a workers.dev subdomain that another account has.
const SUBDOMAIN_TAKEN: u32 = 10031;

/// A failed call, before the caller says which permission it needed.
#[derive(Debug)]
pub(crate) enum Failure {
    Transport(Error),
    Api {
        status: StatusCode,
        codes: Vec<u32>,
        retry_after: Option<u64>,
    },
}

impl Failure {
    pub(crate) fn is_not_found(&self) -> bool {
        matches!(self, Self::Api { status, codes, .. } if *status == StatusCode::NOT_FOUND || codes.contains(&NOT_FOUND))
    }

    fn has_code(&self, code: u32) -> bool {
        matches!(self, Self::Api { codes, .. } if codes.contains(&code))
    }

    /// Maps the failure for a call that needed `permission`.
    pub(crate) fn into_error(self, permission: Permission) -> Error {
        match self {
            Self::Transport(e) => e,
            Self::Api {
                status,
                codes,
                retry_after,
            } => match status {
                StatusCode::TOO_MANY_REQUESTS => Error::RateLimited {
                    retry_after_secs: retry_after,
                },
                StatusCode::UNAUTHORIZED => Error::CloudflareToken,
                // 10000 is "Authentication error", which Cloudflare also answers with when the
                // token is valid but lacks the permission.
                StatusCode::FORBIDDEN => Error::CloudflarePermission(permission),
                _ if codes.iter().any(|c| matches!(c, 10000 | 9109)) => {
                    Error::CloudflarePermission(permission)
                }
                _ => Error::Cloudflare {
                    status: status.as_u16(),
                    code: codes.first().copied(),
                },
            },
        }
    }
}

type Call<T> = std::result::Result<T, Failure>;

#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    errors: Vec<CfError>,
    #[serde(default)]
    result: Value,
}

#[derive(Deserialize)]
struct CfError {
    #[serde(default)]
    code: u32,
}

enum Body {
    Empty,
    Json(Value),
    Multipart { boundary: String, bytes: Vec<u8> },
}

/// An account the token reaches.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Account {
    /// Account ID.
    pub id: String,
    /// Account name.
    #[serde(default)]
    pub name: String,
}

#[derive(Deserialize)]
struct TokenStatus {
    status: String,
}

#[derive(Deserialize)]
struct Subdomain {
    subdomain: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Database {
    pub uuid: String,
    pub name: String,
}

#[derive(Deserialize)]
pub(crate) struct ScriptSettings {
    #[serde(default)]
    pub bindings: Vec<Value>,
}

/// The result of one SQL statement.
#[derive(Deserialize)]
pub(crate) struct Statement {
    #[serde(default)]
    pub results: Vec<serde_json::Map<String, Value>>,
}

/// The Cloudflare API, authenticated with one API token.
pub(crate) struct Client {
    http: reqwest::Client,
    base: String,
    token: Zeroizing<String>,
}

impl Client {
    pub(crate) fn new(base: &str, token: &str) -> Result<Self> {
        let base = base.trim_end_matches('/').to_owned();
        let url = reqwest::Url::parse(&base)
            .map_err(|_| Error::InvalidUrl("invalid API base URL".into()))?;
        let loopback = url.host_str().is_some_and(is_loopback_host);
        if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
            return Err(Error::InvalidUrl("API base must be https".into()));
        }
        let token = token.trim();
        if token.is_empty() {
            return Err(Error::CloudflareToken);
        }
        Ok(Self {
            http: build_client(loopback)?,
            base,
            token: Zeroizing::new(token.to_owned()),
        })
    }

    async fn send(&self, method: Method, path: &str, body: Body) -> Call<Value> {
        let mut auth = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", *self.token))
            .map_err(|_| Failure::Transport(Error::CloudflareToken))?;
        auth.set_sensitive(true);
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.base))
            .header(reqwest::header::AUTHORIZATION, auth);
        request = match body {
            Body::Empty => request,
            Body::Json(value) => request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(value.to_string()),
            Body::Multipart { boundary, bytes } => request
                .header(
                    reqwest::header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(bytes),
        };
        let response = request
            .send()
            .await
            .map_err(|e| Failure::Transport(map_transport(&e)))?;
        let (status, headers, bytes) = read_body(response).await.map_err(Failure::Transport)?;
        let envelope: Option<Envelope> = serde_json::from_slice(&bytes).ok();
        match envelope {
            Some(env) if env.success && status.is_success() => Ok(env.result),
            None if status.is_success() => Err(Failure::Transport(Error::Protocol(
                "malformed Cloudflare response".into(),
            ))),
            other => Err(Failure::Api {
                status,
                codes: other
                    .map(|env| env.errors.iter().map(|e| e.code).collect())
                    .unwrap_or_default(),
                retry_after: retry_after(&headers),
            }),
        }
    }

    async fn call<T: DeserializeOwned>(&self, method: Method, path: &str, body: Body) -> Call<T> {
        let value = self.send(method, path, body).await?;
        serde_json::from_value(value).map_err(|_| {
            Failure::Transport(Error::Protocol("unexpected Cloudflare response".into()))
        })
    }

    /// Fetches up to 20 pages of a list endpoint whose path already has a query.
    async fn list<T: DeserializeOwned>(&self, path: &str) -> Call<Vec<T>> {
        const PER_PAGE: usize = 50;
        let mut all = Vec::new();
        for page in 1..=20 {
            let items: Vec<T> = self
                .call(
                    Method::GET,
                    &format!("{path}&per_page={PER_PAGE}&page={page}"),
                    Body::Empty,
                )
                .await?;
            let n = items.len();
            all.extend(items);
            if n < PER_PAGE {
                break;
            }
        }
        Ok(all)
    }

    // ---- step 1 ------------------------------------------------------------------------------

    /// Whether the token is active: as a user token, or else as a token owned by `account_id`.
    pub(crate) async fn verify_token(&self, account_id: Option<&str>) -> Result<bool> {
        let user: Call<TokenStatus> = self
            .call(Method::GET, "/user/tokens/verify", Body::Empty)
            .await;
        let (status, account_owned) = match user {
            Ok(status) => (status, false),
            Err(Failure::Transport(e)) => return Err(e),
            Err(_) => {
                let Some(account_id) = account_id else {
                    return Err(Error::CloudflareToken);
                };
                let status: TokenStatus = self
                    .call(
                        Method::GET,
                        &format!("/accounts/{account_id}/tokens/verify"),
                        Body::Empty,
                    )
                    .await
                    .map_err(|f| match f {
                        Failure::Transport(e) => e,
                        Failure::Api { .. } => Error::CloudflareToken,
                    })?;
                (status, true)
            }
        };
        if status.status != "active" {
            return Err(Error::CloudflareToken);
        }
        Ok(account_owned)
    }

    /// The accounts a user token reaches. Empty when Cloudflare does not list them.
    pub(crate) async fn accounts(&self) -> Result<Vec<Account>> {
        match self.list("/accounts?direction=asc").await {
            Ok(accounts) => Ok(accounts),
            Err(Failure::Transport(e)) => Err(e),
            Err(_) => Ok(Vec::new()),
        }
    }

    // ---- workers.dev ------------------------------------------------------------------------

    pub(crate) async fn subdomain(&self, account_id: &str) -> Result<Option<String>> {
        match self
            .call::<Subdomain>(
                Method::GET,
                &format!("/accounts/{account_id}/workers/subdomain"),
                Body::Empty,
            )
            .await
        {
            Ok(s) if !s.subdomain.is_empty() => Ok(Some(s.subdomain)),
            Ok(_) => Ok(None),
            Err(f) if f.is_not_found() => Ok(None),
            Err(f) => Err(f.into_error(Permission::WorkersScripts)),
        }
    }

    pub(crate) async fn create_subdomain(&self, account_id: &str, name: &str) -> Result<String> {
        self.call::<Subdomain>(
            Method::PUT,
            &format!("/accounts/{account_id}/workers/subdomain"),
            Body::Json(json!({ "subdomain": name })),
        )
        .await
        .map(|s| s.subdomain)
        .map_err(|f| {
            if f.has_code(SUBDOMAIN_TAKEN)
                || matches!(
                    f,
                    Failure::Api {
                        status: StatusCode::CONFLICT,
                        ..
                    }
                )
            {
                Error::SubdomainUnavailable
            } else {
                f.into_error(Permission::WorkersScripts)
            }
        })
    }

    pub(crate) async fn enable_route(&self, account_id: &str, name: &str) -> Result<()> {
        self.send(
            Method::POST,
            &format!("/accounts/{account_id}/workers/scripts/{name}/subdomain"),
            Body::Json(json!({ "enabled": true, "previews_enabled": false })),
        )
        .await
        .map(drop)
        .map_err(|f| f.into_error(Permission::WorkersScripts))
    }

    // ---- Workers ----------------------------------------------------------------------------

    pub(crate) async fn script_settings(
        &self,
        account_id: &str,
        name: &str,
    ) -> Result<Option<ScriptSettings>> {
        match self
            .call(
                Method::GET,
                &format!("/accounts/{account_id}/workers/scripts/{name}/settings"),
                Body::Empty,
            )
            .await
        {
            Ok(settings) => Ok(Some(settings)),
            Err(f) if f.is_not_found() => Ok(None),
            Err(f) => Err(f.into_error(Permission::WorkersScripts)),
        }
    }

    pub(crate) async fn upload(
        &self,
        account_id: &str,
        name: &str,
        bundle: &WorkerBundle,
        database_id: &str,
    ) -> Result<()> {
        let metadata = json!({
            "main_module": bundle.module.name,
            "compatibility_date": bundle.compatibility_date,
            "compatibility_flags": bundle.compatibility_flags,
            "bindings": [
                { "type": "d1", "name": bundle.d1.binding, "database_id": database_id },
                {
                    "type": "ratelimit",
                    "name": bundle.ratelimit.name,
                    "namespace_id": bundle.ratelimit.namespace_id,
                    "simple": { "limit": bundle.ratelimit.limit, "period": bundle.ratelimit.period },
                },
            ],
            // Keeps SETUP_TOKEN, or any secret a wrangler deployment set.
            "keep_bindings": ["secret_text"],
        });
        let boundary = format!("hatoba-{}", hex(&crate::crypto::random_bytes::<12>()?));
        let mut bytes = Vec::with_capacity(bundle.module.content.len() + 1024);
        let mut part = |headers: String, content: &[u8]| {
            bytes.extend_from_slice(format!("--{boundary}\r\n{headers}\r\n\r\n").as_bytes());
            bytes.extend_from_slice(content);
            bytes.extend_from_slice(b"\r\n");
        };
        part(
            "Content-Disposition: form-data; name=\"metadata\"\r\nContent-Type: application/json"
                .into(),
            metadata.to_string().as_bytes(),
        );
        let module = &bundle.module.name;
        part(
            format!(
                "Content-Disposition: form-data; name=\"{module}\"; filename=\"{module}\"\r\nContent-Type: application/javascript+module"
            ),
            bundle.module.content.as_bytes(),
        );
        bytes.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        self.send(
            Method::PUT,
            &format!("/accounts/{account_id}/workers/scripts/{name}"),
            Body::Multipart { boundary, bytes },
        )
        .await
        .map(drop)
        .map_err(|f| f.into_error(Permission::WorkersScripts))
    }

    pub(crate) async fn put_secret(
        &self,
        account_id: &str,
        script: &str,
        name: &str,
        value: &str,
    ) -> Result<()> {
        self.send(
            Method::PUT,
            &format!("/accounts/{account_id}/workers/scripts/{script}/secrets"),
            Body::Json(json!({ "name": name, "text": value, "type": "secret_text" })),
        )
        .await
        .map(drop)
        .map_err(|f| f.into_error(Permission::WorkersScripts))
    }

    pub(crate) async fn delete_secret(
        &self,
        account_id: &str,
        script: &str,
        name: &str,
    ) -> Result<()> {
        match self
            .send(
                Method::DELETE,
                &format!("/accounts/{account_id}/workers/scripts/{script}/secrets/{name}"),
                Body::Empty,
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(f) if f.is_not_found() => Ok(()),
            Err(f) => Err(f.into_error(Permission::WorkersScripts)),
        }
    }

    pub(crate) async fn delete_script(&self, account_id: &str, name: &str) -> Result<()> {
        match self
            .send(
                Method::DELETE,
                &format!("/accounts/{account_id}/workers/scripts/{name}"),
                Body::Empty,
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(f) if f.is_not_found() => Ok(()),
            Err(f) => Err(f.into_error(Permission::WorkersScripts)),
        }
    }

    // ---- D1 ---------------------------------------------------------------------------------

    /// Databases whose names contain `name` (the filter is a search).
    pub(crate) async fn databases(&self, account_id: &str, name: &str) -> Result<Vec<Database>> {
        self.list(&format!("/accounts/{account_id}/d1/database?name={name}"))
            .await
            .map_err(|f| f.into_error(Permission::D1))
    }

    pub(crate) async fn database(&self, account_id: &str, id: &str) -> Result<Option<Database>> {
        match self
            .call(
                Method::GET,
                &format!("/accounts/{account_id}/d1/database/{id}"),
                Body::Empty,
            )
            .await
        {
            Ok(db) => Ok(Some(db)),
            Err(f) if f.is_not_found() => Ok(None),
            Err(f) => Err(f.into_error(Permission::D1)),
        }
    }

    pub(crate) async fn create_database(&self, account_id: &str, name: &str) -> Result<Database> {
        self.call(
            Method::POST,
            &format!("/accounts/{account_id}/d1/database"),
            Body::Json(json!({ "name": name })),
        )
        .await
        .map_err(|f| f.into_error(Permission::D1))
    }

    pub(crate) async fn delete_database(&self, account_id: &str, id: &str) -> Result<()> {
        match self
            .send(
                Method::DELETE,
                &format!("/accounts/{account_id}/d1/database/{id}"),
                Body::Empty,
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(f) if f.is_not_found() => Ok(()),
            Err(f) => Err(f.into_error(Permission::D1)),
        }
    }

    /// Runs `sql` (one or more statements, run as a batch) and returns each statement's result.
    pub(crate) async fn query(
        &self,
        account_id: &str,
        database_id: &str,
        sql: &str,
    ) -> Call<Vec<Statement>> {
        self.call(
            Method::POST,
            &format!("/accounts/{account_id}/d1/database/{database_id}/query"),
            Body::Json(json!({ "sql": sql })),
        )
        .await
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
