//! Shared Cloudflare `/client/v4` HTTP client for [`super::d1`] and [`super::deploy`].
//!
//! Sends requests only to the configured API base with a sensitive `Bearer` header, enforces the
//! [`super::http`] response size cap, and decodes Cloudflare's `{ success, errors, result }`
//! envelope without logging URLs, headers, tokens or bodies.

use std::fmt;
use std::sync::{PoisonError, RwLock};

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use zeroize::Zeroizing;

use crate::error::Error;
use crate::sync::http::{build_client, is_loopback_host, map_transport, read_body, retry_after};

/// Production Cloudflare v4 API base.
pub const CLOUDFLARE_API_BASE: &str = "https://api.cloudflare.com/client/v4";

/// Maximum pages [`CfClient::list`] fetches before stopping.
const MAX_LIST_PAGES: usize = 20;

/// Whether `s` is a safe Cloudflare path identifier (`[A-Za-z0-9_-]{1,64}`).
pub(crate) fn is_identifier(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Request payload for a Cloudflare API call.
pub(crate) enum CfBody {
    Empty,
    Json(Value),
    Multipart { boundary: String, bytes: Vec<u8> },
}

/// One entry in Cloudflare's `errors` array.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub(crate) struct CfError {
    #[serde(default)]
    pub code: i64,
    #[serde(default)]
    pub message: String,
}

/// A failed Cloudflare API call before domain-specific error mapping.
#[derive(Debug)]
pub(crate) enum CfFailure {
    /// The API token was empty, forgotten, or not a valid HTTP header value.
    NoToken,
    /// Invalid base URL, transport/network failure, oversized body, or malformed 2xx payload.
    Transport(Error),
    /// Non-2xx HTTP response, or a 2xx envelope with `success: false`.
    Api {
        status: StatusCode,
        errors: Vec<CfError>,
        body: Vec<u8>,
        retry_after: Option<u64>,
    },
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    errors: Vec<CfError>,
    #[serde(default)]
    result: Value,
}

/// Authenticated client for Cloudflare's `/client/v4` REST API.
pub(crate) struct CfClient {
    http: reqwest::Client,
    base: String,
    /// `None` once [`Self::forget`] wipes the token.
    token: RwLock<Option<Zeroizing<String>>>,
}

impl fmt::Debug for CfClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CfClient")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl CfClient {
    /// Builds a client for `base` (`https`, or `http` on loopback) authenticated by `token`.
    pub(crate) fn new(base: &str, token: &str) -> Result<Self, CfFailure> {
        let base = base.trim().trim_end_matches('/').to_owned();
        let url = reqwest::Url::parse(&base)
            .map_err(|_| CfFailure::Transport(Error::InvalidUrl("invalid API base URL".into())))?;
        let loopback = url.host_str().is_some_and(is_loopback_host);
        if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
            return Err(CfFailure::Transport(Error::InvalidUrl(
                "API base must be https".into(),
            )));
        }
        let token = token.trim();
        if token.is_empty() {
            return Err(CfFailure::NoToken);
        }
        Ok(Self {
            http: build_client(loopback).map_err(CfFailure::Transport)?,
            base,
            token: RwLock::new(Some(Zeroizing::new(token.to_owned()))),
        })
    }

    /// Wipes the API token in memory; every later request fails with [`CfFailure::NoToken`].
    pub(crate) fn forget(&self) {
        *self.token.write().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// Sends a request and returns `result` from a successful `{ success, errors, result }`
    /// envelope, including `Value::Null` when an endpoint returns no result object.
    pub(crate) async fn send(
        &self,
        method: Method,
        path: &str,
        body: CfBody,
    ) -> Result<Value, CfFailure> {
        let mut auth = {
            let guard = self.token.read().unwrap_or_else(PoisonError::into_inner);
            let token = guard.as_ref().ok_or(CfFailure::NoToken)?;
            let bearer = Zeroizing::new(format!("Bearer {}", token.as_str()));
            HeaderValue::from_str(&bearer).map_err(|_| CfFailure::NoToken)?
        };
        auth.set_sensitive(true);
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.base))
            .header(AUTHORIZATION, auth);
        request = match body {
            CfBody::Empty => request,
            CfBody::Json(value) => request
                .header(CONTENT_TYPE, "application/json")
                .body(value.to_string()),
            CfBody::Multipart { boundary, bytes } => request
                .header(
                    CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(bytes),
        };
        let response = request
            .send()
            .await
            .map_err(|e| CfFailure::Transport(map_transport(&e)))?;
        let (status, headers, bytes) = read_body(response).await.map_err(CfFailure::Transport)?;
        let envelope: Option<Envelope> = serde_json::from_slice(&bytes).ok();
        match envelope {
            Some(env) if env.success && status.is_success() => Ok(env.result),
            None if status.is_success() => Err(CfFailure::Transport(Error::Protocol(
                "malformed Cloudflare response".into(),
            ))),
            other => Err(CfFailure::Api {
                status,
                errors: other.map(|env| env.errors).unwrap_or_default(),
                body: bytes,
                retry_after: retry_after(&headers),
            }),
        }
    }

    /// Sends a request and deserializes `result` as `T`.
    pub(crate) async fn call<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: CfBody,
    ) -> Result<T, CfFailure> {
        let value = self.send(method, path, body).await?;
        serde_json::from_value(value).map_err(|_| {
            CfFailure::Transport(Error::Protocol("unexpected Cloudflare response".into()))
        })
    }

    /// Fetches up to 20 pages of `per_page` items from a `GET` list endpoint.
    pub(crate) async fn list<T: DeserializeOwned>(
        &self,
        path: &str,
        per_page: usize,
    ) -> Result<Vec<T>, CfFailure> {
        let sep = if path.contains('?') { '&' } else { '?' };
        let mut all = Vec::new();
        for page in 1..=MAX_LIST_PAGES {
            let items: Vec<T> = self
                .call(
                    Method::GET,
                    &format!("{path}{sep}per_page={per_page}&page={page}"),
                    CfBody::Empty,
                )
                .await?;
            let n = items.len();
            all.extend(items);
            if n < per_page {
                break;
            }
        }
        Ok(all)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::sync::http::error_code;

    #[test]
    fn validates_base_url_token_and_debug_redaction() {
        assert!(matches!(
            CfClient::new("not a url", "tok"),
            Err(CfFailure::Transport(Error::InvalidUrl(m))) if m == "invalid API base URL"
        ));
        for non_https in [
            "http://api.cloudflare.com/client/v4",
            "http://127.0.0.1.evil.com/client/v4",
            "ftp://api.cloudflare.com/client/v4",
        ] {
            assert!(matches!(
                CfClient::new(non_https, "tok"),
                Err(CfFailure::Transport(Error::InvalidUrl(m))) if m == "API base must be https"
            ));
        }
        // URL check runs before the empty-token check.
        assert!(matches!(
            CfClient::new("http://example.com", "   "),
            Err(CfFailure::Transport(Error::InvalidUrl(_)))
        ));
        assert!(matches!(
            CfClient::new(CLOUDFLARE_API_BASE, "  \t\n "),
            Err(CfFailure::NoToken)
        ));

        for valid in [
            "  https://api.cloudflare.com/client/v4///  ",
            "http://127.0.0.1:8787/client/v4/",
            "http://localhost:8787",
            "http://[::1]:8787",
        ] {
            let client = CfClient::new(valid, "  super-secret-token  ").unwrap();
            assert!(!client.base.ends_with('/'));
            assert!(!format!("{client:?}").contains("super-secret-token"));
        }
    }

    #[tokio::test]
    async fn forget_and_invalid_header_bytes_fail_before_network() {
        let server = MockServer::start().await;
        let base = format!("{}/client/v4", server.uri());

        let client = CfClient::new(&base, "valid-token").unwrap();
        client.forget();
        assert!(matches!(
            client.send(Method::GET, "/ping", CfBody::Empty).await,
            Err(CfFailure::NoToken)
        ));

        let bad_header = CfClient::new(&base, "bad\ntoken").unwrap();
        assert!(matches!(
            bad_header
                .call::<Value>(Method::GET, "/ping", CfBody::Empty)
                .await,
            Err(CfFailure::NoToken)
        ));
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn sends_empty_json_and_multipart_bodies_with_trimmed_bearer() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/client/v4/item"))
            .and(header("authorization", "Bearer trimmed-tok"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "success": true, "errors": [], "result": null })),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/client/v4/json"))
            .and(header("authorization", "Bearer trimmed-tok"))
            .and(header("content-type", "application/json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "success": true, "result": { "ok": 7 } })),
            )
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/client/v4/upload"))
            .and(header(
                "content-type",
                "multipart/form-data; boundary=hatoba-b1",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true })))
            .mount(&server)
            .await;

        let client = CfClient::new(&format!("{}/client/v4/", server.uri()), "  trimmed-tok\n")
            .expect("client");
        let deleted = client
            .send(Method::DELETE, "/item", CfBody::Empty)
            .await
            .unwrap();
        assert_eq!(deleted, Value::Null);

        let reply: Value = client
            .call(Method::POST, "/json", CfBody::Json(json!({ "x": 1 })))
            .await
            .unwrap();
        assert_eq!(reply, json!({ "ok": 7 }));

        let uploaded = client
            .send(
                Method::PUT,
                "/upload",
                CfBody::Multipart {
                    boundary: "hatoba-b1".into(),
                    bytes: b"--hatoba-b1--\r\n".to_vec(),
                },
            )
            .await
            .unwrap();
        assert_eq!(uploaded, Value::Null);

        let requests = server.received_requests().await.unwrap();
        assert!(requests[0].headers.get("content-type").is_none());
        assert_eq!(requests[1].body, br#"{"x":1}"#);
        assert_eq!(requests[2].body, b"--hatoba-b1--\r\n");
    }

    #[tokio::test]
    async fn maps_2xx_anomalies_and_non_2xx_error_shapes() {
        let server = MockServer::start().await;
        Mock::given(path("/client/v4/html-200"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>ok</html>"))
            .mount(&server)
            .await;
        Mock::given(path("/client/v4/null-200"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "success": true, "result": null })),
            )
            .mount(&server)
            .await;
        Mock::given(path("/client/v4/false-200"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": false,
                "errors": [{ "code": 10000, "message": "Authentication error" }],
                "result": null
            })))
            .mount(&server)
            .await;
        Mock::given(path("/client/v4/rate-429"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "15")
                    .set_body_json(json!({
                        "success": false,
                        "errors": [{ "code": 971, "message": "rate limited" }]
                    })),
            )
            .mount(&server)
            .await;
        Mock::given(path("/client/v4/plain-503"))
            .respond_with(
                ResponseTemplate::new(503).set_body_json(json!({ "error": "upstream_down" })),
            )
            .mount(&server)
            .await;

        let client = CfClient::new(&format!("{}/client/v4", server.uri()), "tok").unwrap();

        assert!(matches!(
            client.send(Method::GET, "/html-200", CfBody::Empty).await,
            Err(CfFailure::Transport(Error::Protocol(m))) if m == "malformed Cloudflare response"
        ));
        assert!(matches!(
            client
                .call::<Vec<u32>>(Method::GET, "/null-200", CfBody::Empty)
                .await,
            Err(CfFailure::Transport(Error::Protocol(m))) if m == "unexpected Cloudflare response"
        ));
        match client.send(Method::GET, "/false-200", CfBody::Empty).await {
            Err(CfFailure::Api { status, errors, .. }) => {
                assert_eq!(status, StatusCode::OK);
                assert_eq!(
                    errors,
                    vec![CfError {
                        code: 10000,
                        message: "Authentication error".into()
                    }]
                );
            }
            other => panic!("expected Api failure, got {other:?}"),
        }
        match client.send(Method::GET, "/rate-429", CfBody::Empty).await {
            Err(CfFailure::Api {
                status,
                retry_after,
                errors,
                ..
            }) => {
                assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
                assert_eq!(retry_after, Some(15));
                assert_eq!(errors[0].code, 971);
            }
            other => panic!("expected 429 Api failure, got {other:?}"),
        }
        match client.send(Method::GET, "/plain-503", CfBody::Empty).await {
            Err(CfFailure::Api {
                status,
                errors,
                body,
                ..
            }) => {
                assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
                assert!(errors.is_empty());
                assert_eq!(error_code(&body).as_deref(), Some("upstream_down"));
            }
            other => panic!("expected 503 Api failure, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_paginates_with_or_without_initial_query() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/client/v4/dbs"))
            .and(query_param("per_page", "2"))
            .and(query_param("page", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "result": ["a", "b"],
                "result_info": { "page": 1, "per_page": 2, "total_pages": 2 }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/client/v4/dbs"))
            .and(query_param("per_page", "2"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "result": ["c"]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/client/v4/accounts"))
            .and(query_param("direction", "asc"))
            .and(query_param("per_page", "50"))
            .and(query_param("page", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "result": ["acc-1"]
            })))
            .mount(&server)
            .await;

        let client = CfClient::new(&format!("{}/client/v4", server.uri()), "tok").unwrap();
        let dbs: Vec<String> = client.list("/dbs", 2).await.unwrap();
        assert_eq!(dbs, ["a", "b", "c"]);
        let accounts: Vec<String> = client.list("/accounts?direction=asc", 50).await.unwrap();
        assert_eq!(accounts, ["acc-1"]);
    }
}
