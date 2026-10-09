//! Model listing and Test Connection against a mock server.

use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zeroize::Zeroizing;

use super::*;
use crate::provider::{AuthHeader, http_client};

const KEY: &str = "sk-models-secret";

fn provider(base_url: String, protocol: Protocol) -> ProviderConfig {
    ProviderConfig {
        protocol,
        base_url,
        api_key: Zeroizing::new(KEY.into()),
        auth_header: AuthHeader::XApiKey,
    }
}

fn cc(server: &MockServer) -> ProviderConfig {
    provider(format!("{}/v1", server.uri()), Protocol::ChatCompletions)
}

fn anthropic(server: &MockServer) -> ProviderConfig {
    provider(server.uri(), Protocol::Anthropic)
}

#[tokio::test]
async fn chat_completions_list_reads_ids_names_and_limits() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [
                {"id": "gpt-4.1", "object": "model", "owned_by": "openai"},
                {"id": "anthropic/claude-sonnet-4.5", "name": "Anthropic: Claude Sonnet 4.5",
                 "context_length": 1_000_000, "top_provider": {"max_completion_tokens": 64_000}},
                {"id": "llama-3.3-70b", "context_window": 131_072, "max_completion_tokens": 32_768},
                {"id": "gpt-4.1", "object": "model"},
                {"id": ""},
                {"object": "model"}
            ]
        })))
        .mount(&server)
        .await;
    let models = list_models(&http_client(), &cc(&server), &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(
        models,
        vec![
            ModelInfo {
                id: "gpt-4.1".into(),
                name: "gpt-4.1".into(),
                context_window: None,
                max_output_tokens: None,
                ..ModelInfo::default()
            },
            ModelInfo {
                id: "anthropic/claude-sonnet-4.5".into(),
                name: "Anthropic: Claude Sonnet 4.5".into(),
                context_window: Some(1_000_000),
                max_output_tokens: Some(64_000),
                ..ModelInfo::default()
            },
            ModelInfo {
                id: "llama-3.3-70b".into(),
                name: "llama-3.3-70b".into(),
                context_window: Some(131_072),
                max_output_tokens: Some(32_768),
                ..ModelInfo::default()
            },
        ]
    );
}

#[tokio::test]
async fn anthropic_list_follows_pages() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(query_param("after_id", "claude-sonnet-4-6"))
        .and(query_param("limit", "1000"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"type": "model", "id": "claude-haiku-4-5", "display_name": "Claude Haiku 4.5",
                      "created_at": "2025-10-01T00:00:00Z", "max_input_tokens": 200_000,
                      "max_tokens": 64_000}],
            "has_more": false, "first_id": "claude-haiku-4-5", "last_id": "claude-haiku-4-5"
        })))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(query_param("limit", "1000"))
        .and(header("x-api-key", KEY))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                {"type": "model", "id": "claude-opus-4-8", "display_name": "Claude Opus 4.8",
                 "max_input_tokens": 1_000_000, "max_tokens": 128_000},
                {"type": "model", "id": "claude-sonnet-4-6", "display_name": "Claude Sonnet 4.6"}
            ],
            "has_more": true, "first_id": "claude-opus-4-8", "last_id": "claude-sonnet-4-6"
        })))
        .mount(&server)
        .await;
    let models = list_models(
        &http_client(),
        &anthropic(&server),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let summary: Vec<(&str, &str, Option<u64>, Option<u64>)> = models
        .iter()
        .map(|m| {
            (
                m.id.as_str(),
                m.name.as_str(),
                m.context_window,
                m.max_output_tokens,
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            (
                "claude-opus-4-8",
                "Claude Opus 4.8",
                Some(1_000_000),
                Some(128_000)
            ),
            ("claude-sonnet-4-6", "Claude Sonnet 4.6", None, None),
            (
                "claude-haiku-4-5",
                "Claude Haiku 4.5",
                Some(200_000),
                Some(64_000)
            ),
        ]
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].url.query().unwrap().contains("limit=1000"));
    assert!(!requests[0].url.query().unwrap().contains("after_id"));
}

#[tokio::test]
async fn anthropic_list_reads_thinking_levels_and_adaptive_thinking() {
    let level = |on: bool| json!({"supported": on});
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                {"id": "claude-opus-5-5", "max_tokens": 128_000, "capabilities": {
                    "image_input": {"supported": true},
                    "thinking": {"supported": true, "types": {
                        "enabled": level(false), "adaptive": level(true)}},
                    "effort": {"supported": true, "low": level(true), "medium": level(true),
                               "high": level(true), "xhigh": level(true), "max": level(true)}}},
                // Opus 4.6: no `xhigh`.
                {"id": "claude-opus-4-6", "capabilities": {
                    "thinking": {"supported": true, "types": {"adaptive": level(true)}},
                    "effort": {"supported": true, "low": level(true), "medium": level(true),
                               "high": level(true), "xhigh": level(false), "max": level(true)}}},
                // Haiku 4.5: budgeted thinking only, no effort.
                {"id": "claude-haiku-4-5", "capabilities": {
                    "thinking": {"supported": true, "types": {
                        "enabled": level(true), "adaptive": level(false)}},
                    "effort": {"supported": false, "low": level(false)}}},
                {"id": "claude-3-haiku", "capabilities": {"thinking": level(false)}},
                // An Anthropic-compatible vendor: no capabilities, or ones without these trees.
                {"id": "vendor-model"},
                {"id": "vendor-other", "capabilities": {"effort": {"supported": true}}}
            ],
            "has_more": false
        })))
        .mount(&server)
        .await;
    let models = list_models(
        &http_client(),
        &anthropic(&server),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let summary: Vec<_> = models
        .iter()
        .map(|m| (m.id.as_str(), m.efforts.clone(), m.adaptive_thinking))
        .collect();
    use Effort::{High, Low, Max, Medium, Xhigh};
    assert_eq!(
        summary,
        vec![
            (
                "claude-opus-5-5",
                Some(vec![Low, Medium, High, Xhigh, Max]),
                Some(true)
            ),
            (
                "claude-opus-4-6",
                Some(vec![Low, Medium, High, Max]),
                Some(true)
            ),
            ("claude-haiku-4-5", Some(vec![]), Some(false)),
            ("claude-3-haiku", None, Some(false)),
            ("vendor-model", None, None),
            ("vendor-other", None, None),
        ]
    );
    assert_eq!(models[0].max_output_tokens, Some(128_000));
}

#[tokio::test]
async fn a_page_that_repeats_its_cursor_ends_the_listing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id": "m1"}], "has_more": true, "last_id": "m1"
        })))
        .mount(&server)
        .await;
    let models = list_models(
        &http_client(),
        &anthropic(&server),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn test_connection_with_a_model_sends_a_minimal_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "Hi!"}}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "type": "message", "content": [{"type": "text", "text": "Hi"}]
        })))
        .mount(&server)
        .await;
    let http = http_client();
    let outcome = test_connection(
        &http,
        &cc(&server),
        Some("gpt-4.1"),
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(
        outcome,
        TestOutcome {
            ok: true,
            failure: None,
            status: None,
            message: None
        }
    );
    assert!(
        test_connection(
            &http,
            &anthropic(&server),
            Some("claude-opus-4-8"),
            &CancellationToken::new()
        )
        .await
        .ok
    );
    let bodies: Vec<Value> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    assert_eq!(
        bodies[0],
        json!({"model": "gpt-4.1", "messages": [{"role": "user", "content": "Hi"}]})
    );
    assert_eq!(bodies[1]["max_tokens"], 16);
    assert!(bodies[1].get("stream").is_none());
}

async fn outcome_for(status: u16, body: Value, model: Option<&str>) -> TestOutcome {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(&server)
        .await;
    test_connection(
        &http_client(),
        &cc(&server),
        model,
        &CancellationToken::new(),
    )
    .await
}

#[tokio::test]
async fn test_connection_tells_failures_apart() {
    let auth = outcome_for(
        401,
        json!({"error": {"message": "Incorrect API key provided"}}),
        Some("m"),
    )
    .await;
    assert_eq!(
        auth,
        TestOutcome {
            ok: false,
            failure: Some(TestFailure::Auth),
            status: Some(401),
            message: Some("Incorrect API key provided".into())
        }
    );
    let forbidden = outcome_for(403, json!({"message": "Forbidden"}), None).await;
    assert_eq!(forbidden.failure, Some(TestFailure::Auth));

    for (status, body) in [
        (
            404,
            json!({"error": {"message": "The model `gpt-9` does not exist or you do not have access to it.",
                             "code": "model_not_found"}}),
        ),
        (
            404,
            json!({"type": "error", "error": {"type": "not_found_error", "message": "model: claude-9"}}),
        ),
        (
            400,
            json!({"error": {"message": "Model Not Exist", "type": "invalid_request_error"}}),
        ),
        (
            400,
            json!([{"error": {"code": 400, "message": "models/gemini-9 is not found for API version v1beta"}}]),
        ),
    ] {
        let outcome = outcome_for(status, body, Some("x")).await;
        assert_eq!(
            outcome.failure,
            Some(TestFailure::UnknownModel),
            "{outcome:?}"
        );
        assert_eq!(outcome.status, Some(status));
    }

    // A 404 that names no model is a wrong path, and listing has no model to be unknown.
    let wrong_path = outcome_for(
        404,
        json!({"error": {"message": "Invalid URL (POST /v1/chat/completion)"}}),
        Some("x"),
    )
    .await;
    assert_eq!(wrong_path.failure, Some(TestFailure::Other));
    let list_404 = outcome_for(
        404,
        json!({"error": {"message": "model list not found"}}),
        None,
    )
    .await;
    assert_eq!(list_404.failure, Some(TestFailure::Other));
    let other = outcome_for(
        400,
        json!({"error": {"message": "messages: too short"}}),
        Some("x"),
    )
    .await;
    assert_eq!(other.failure, Some(TestFailure::Other));
    let server_error = outcome_for(500, json!({"error": {"message": "boom"}}), Some("x")).await;
    assert_eq!(server_error.failure, Some(TestFailure::Other));
}

#[tokio::test]
async fn test_connection_network_and_url_failures() {
    let http = http_client();
    // Nothing listens on port 9 of the loopback address.
    let closed = provider("http://127.0.0.1:9/v1".into(), Protocol::ChatCompletions);
    let outcome = test_connection(&http, &closed, Some("m"), &CancellationToken::new()).await;
    assert_eq!(outcome.failure, Some(TestFailure::Network), "{outcome:?}");
    assert_eq!(outcome.status, None);

    let public_http = provider("http://8.8.8.8/v1".into(), Protocol::ChatCompletions);
    let outcome = test_connection(&http, &public_http, None, &CancellationToken::new()).await;
    assert_eq!(outcome.failure, Some(TestFailure::InvalidUrl));
    let outcome = test_connection(
        &http,
        &provider("nonsense".into(), Protocol::Anthropic),
        None,
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(outcome.failure, Some(TestFailure::InvalidUrl));

    let mut bad_key = closed.clone();
    bad_key.api_key = Zeroizing::new("line\nbreak".into());
    let outcome = test_connection(&http, &bad_key, Some("m"), &CancellationToken::new()).await;
    assert_eq!(outcome.failure, Some(TestFailure::Auth));
    assert!(!outcome.message.unwrap().contains("line"));
}

#[tokio::test]
async fn test_connection_without_a_model_lists_models() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
        .mount(&server)
        .await;
    assert!(
        test_connection(
            &http_client(),
            &cc(&server),
            Some("  "),
            &CancellationToken::new()
        )
        .await
        .ok
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests[0].method.as_str(), "GET");
}

#[test]
fn classification_of_errors() {
    let http = |status, message: &str| AiError::Http {
        status,
        message: message.into(),
    };
    assert_eq!(
        classify(&http(401, "x"), true).failure,
        Some(TestFailure::Auth)
    );
    assert_eq!(
        classify(&AiError::Network("timed out".into()), true).failure,
        Some(TestFailure::Network)
    );
    assert_eq!(
        classify(&AiError::Protocol("bad".into()), true).failure,
        Some(TestFailure::Other)
    );
    assert_eq!(
        classify(&http(400, "Unknown model: foo"), true).failure,
        Some(TestFailure::UnknownModel)
    );
    assert_eq!(
        classify(&http(400, "Unknown model: foo"), false).failure,
        Some(TestFailure::Other)
    );
    assert_eq!(
        serde_json::to_string(&TestFailure::UnknownModel).unwrap(),
        "\"unknown_model\""
    );
}

/// A server that takes 30 s to answer anything.
async fn slow_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(30))
                .set_body_json(json!({"data": []})),
        )
        .mount(&server)
        .await;
    server
}

/// A token that fires after 200 ms, like a lock arriving while a request is in flight.
fn cancel_soon() -> CancellationToken {
    let cancel = CancellationToken::new();
    let later = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        later.cancel();
    });
    cancel
}

#[tokio::test]
async fn cancelling_aborts_the_model_list_and_the_test_connection() {
    let server = slow_server().await;
    let http = http_client();
    let started = std::time::Instant::now();

    let err = list_models(&http, &cc(&server), &cancel_soon())
        .await
        .unwrap_err();
    assert_eq!(err, AiError::Cancelled);
    let err = list_models(&http, &anthropic(&server), &cancel_soon())
        .await
        .unwrap_err();
    assert_eq!(err, AiError::Cancelled);

    for model in [None, Some("m")] {
        let outcome = test_connection(&http, &cc(&server), model, &cancel_soon()).await;
        assert!(!outcome.ok);
        assert_eq!(outcome.message.as_deref(), Some("cancelled"), "{model:?}");
    }
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "each request was dropped when the token fired, not after the 30 s the server takes"
    );

    // A token that has fired already: nothing is sent.
    let before = server.received_requests().await.unwrap().len();
    let fired = CancellationToken::new();
    fired.cancel();
    assert_eq!(
        list_models(&http, &cc(&server), &fired).await,
        Err(AiError::Cancelled)
    );
    let outcome = test_connection(&http, &cc(&server), Some("m"), &fired).await;
    assert_eq!(outcome.message.as_deref(), Some("cancelled"));
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}

#[tokio::test]
async fn an_http_base_url_with_a_host_name_goes_through_the_guarded_client() {
    // `localhost` is a host name, so these requests use the client whose resolver keeps to local
    // addresses; it resolves to loopback, which the mock server listens on.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"id": "m1"}]})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"choices": []})))
        .mount(&server)
        .await;
    let provider = provider(
        format!("http://localhost:{}/v1", server.address().port()),
        Protocol::ChatCompletions,
    );
    let http = http_client();
    let models = list_models(&http, &provider, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "m1");
    let outcome = test_connection(&http, &provider, Some("m1"), &CancellationToken::new()).await;
    assert!(outcome.ok, "{outcome:?}");
}
