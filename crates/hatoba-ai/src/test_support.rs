//! Shared HTTP mock and provider fixtures for tests in `hatoba-ai` and downstream crates.
//!
//! Exposed only under `#[cfg(test)]` or the `test-util` feature; never enable in a shipping
//! build.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use wiremock::{MockServer, ResponseTemplate};
use zeroize::Zeroizing;

use crate::provider::{AuthHeader, Protocol, ProviderConfig};

/// A [`ProviderConfig`] with `AuthHeader::XApiKey` and no ChatGPT OAuth session.
#[must_use]
pub fn provider(protocol: Protocol, base_url: impl Into<String>, api_key: &str) -> ProviderConfig {
    ProviderConfig {
        protocol,
        base_url: base_url.into(),
        api_key: Zeroizing::new(api_key.into()),
        auth_header: AuthHeader::XApiKey,
        chatgpt: None,
    }
}

/// The JSON bodies of every request `server` received, in order.
pub async fn bodies(server: &MockServer) -> Vec<Value> {
    bodies_where(server, |_| true).await
}

/// The JSON bodies of the requests `server` received at `path`, in order.
pub async fn bodies_at(server: &MockServer, path: &str) -> Vec<Value> {
    bodies_where(server, |r| r.url.path() == path).await
}

async fn bodies_where(
    server: &MockServer,
    keep: impl Fn(&wiremock::Request) -> bool,
) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| keep(r))
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

/// An HTTP 200 `text/event-stream` response with `body`.
#[must_use]
pub fn sse(body: impl Into<String>) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream; charset=utf-8")
        .set_body_string(body.into())
}

/// An HTTP 200 `text/event-stream` response from named `(event, data)` pairs.
#[must_use]
pub fn sse_events(events: &[(&str, Value)]) -> ResponseTemplate {
    let body: String = events
        .iter()
        .map(|(name, data)| format!("event: {name}\ndata: {data}\n\n"))
        .collect();
    sse(body)
}

/// An HTTP 200 `text/event-stream` response from `data:` JSON values followed by `[DONE]`.
#[must_use]
pub fn sse_data(events: &[Value]) -> ResponseTemplate {
    let mut body = String::new();
    for event in events {
        body.push_str(&format!("data: {event}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    sse(body)
}

async fn request_body(socket: &mut TcpStream) -> Vec<u8> {
    let mut data = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = socket.read(&mut buf).await.unwrap();
        assert!(n > 0, "the request ended early");
        data.extend_from_slice(&buf[..n]);
        let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&data[..end]).to_ascii_lowercase();
        let length: usize = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .map_or(0, |value| value.trim().parse().unwrap());
        if data.len() >= end + 4 + length {
            return data[end + 4..end + 4 + length].to_vec();
        }
    }
}

/// A server whose first response sends the headers and `events`, then nothing (the stream stays
/// open), and whose later responses answer `"Done."`. Returns its `/v1` base URL and the JSON
/// body of every request it received.
pub async fn stalling_server(events: &[Value]) -> (String, Arc<Mutex<Vec<Value>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let received = bodies.clone();
    let first: String = events.iter().map(|e| format!("data: {e}\n\n")).collect();
    tokio::spawn(async move {
        let mut stalled = Vec::new();
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let body = request_body(&mut socket).await;
            received
                .lock()
                .unwrap()
                .push(serde_json::from_slice::<Value>(&body).unwrap());
            if stalled.is_empty() {
                let mut head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                                transfer-encoding: chunked\r\n\r\n"
                    .to_owned();
                if !first.is_empty() {
                    head.push_str(&format!("{:x}\r\n{first}\r\n", first.len()));
                }
                socket.write_all(head.as_bytes()).await.unwrap();
                stalled.push(socket);
            } else {
                let body = [
                    json!({"choices": [{"index": 0, "delta": {"content": "Done."}}]}),
                    json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
                ]
                .iter()
                .map(|e| format!("data: {e}\n\n"))
                .collect::<String>()
                    + "data: [DONE]\n\n";
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\
                     connection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        }
    });
    (format!("http://{addr}/v1"), bodies)
}
