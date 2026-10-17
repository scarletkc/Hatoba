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
        chatgpt: None,
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
            // What the list leaves out of a well-known model comes from the table.
            ModelInfo {
                id: "gpt-4.1".into(),
                name: "gpt-4.1".into(),
                context_window: Some(1_047_576),
                max_output_tokens: Some(32_768),
                efforts: Some(vec![]),
                adaptive_thinking: None,
            },
            // What it says wins, and Anthropic's levels stay unknown on Chat Completions.
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
            // The page left the limits out; the table has them.
            (
                "claude-sonnet-4-6",
                "Claude Sonnet 4.6",
                Some(1_000_000),
                Some(128_000)
            ),
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

#[tokio::test]
async fn responses_lists_models_like_chat_completions_and_tests_with_one_input() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [{"id": "gpt-5.1", "object": "model"}, {"id": "o4-mini", "object": "model"}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "completed",
            "output": [{"type": "message", "content": [{"type": "output_text", "text": "Hi!"}]}]
        })))
        .mount(&server)
        .await;
    let http = http_client();
    let responses = provider(format!("{}/v1", server.uri()), Protocol::Responses);
    let models = list_models(&http, &responses, &CancellationToken::new())
        .await
        .unwrap();
    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["gpt-5.1", "o4-mini"]);
    assert!(
        test_connection(
            &http,
            &responses,
            Some("gpt-5.1"),
            &CancellationToken::new()
        )
        .await
        .ok
    );
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(
        body,
        json!({"model": "gpt-5.1", "input": [{"role": "user", "content": "Hi"}], "store": false})
    );
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

    let not_http = provider("ftp://llm.example.com/v1".into(), Protocol::ChatCompletions);
    let outcome = test_connection(&http, &not_http, None, &CancellationToken::new()).await;
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

fn system_one(base_url: String) -> ProviderConfig {
    provider(base_url, Protocol::SystemOne)
}

#[tokio::test]
async fn system_one_lists_typesafe_and_openai_shaped_models() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "models": [
                {"name": "jev-latest", "description": "The latest Jev", "release_date": "2026-09-01"},
                {"name": "jev-1.13.0"},
                {"name": "jev-latest"},
                {"description": "no name"}
            ]
        })))
        .mount(&server)
        .await;
    // A base URL that names the endpoint lists the models next to it.
    for base in [
        format!("{}/v1", server.uri()),
        format!("{}/v1/systemone", server.uri()),
    ] {
        let models = list_models(&http_client(), &system_one(base), &CancellationToken::new())
            .await
            .unwrap();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["jev-latest", "jev-1.13.0"]);
        assert_eq!(models[0].name, "jev-latest");
        assert_eq!(
            models[0].efforts,
            Some(vec![]),
            "a decision model does not think"
        );
    }

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id": "typesafe/jev-latest", "name": "TypeSafe: Jev", "context_length": 64000}]
        })))
        .mount(&server)
        .await;
    let models = list_models(
        &http_client(),
        &system_one(format!("{}/api/v1", server.uri())),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "typesafe/jev-latest");
    assert_eq!(models[0].name, "TypeSafe: Jev");
    assert_eq!(models[0].context_window, Some(64_000));
}

#[tokio::test]
async fn system_one_test_connection_sends_one_decision() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "jev-1.13.0",
            "answers": {"blue": {"type": "noul", "noul": 0.99}},
            "usage": {"input_tokens": 40, "output_tokens": 1}
        })))
        .mount(&server)
        .await;
    let p = system_one(format!("{}/v1", server.uri()));
    let outcome = test_connection(
        &http_client(),
        &p,
        Some("jev-latest"),
        &CancellationToken::new(),
    )
    .await;
    assert!(outcome.ok, "{outcome:?}");
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "jev-latest");
    assert!(body["state"].is_string());
    assert_eq!(body["questions"]["blue"]["type"], "noul");

    // Failures are told apart as for the other protocols.
    let outcome_of = |status: u16, body: Value| async move {
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&server)
            .await;
        test_connection(
            &http_client(),
            &system_one(server.uri()),
            Some("jev-nope"),
            &CancellationToken::new(),
        )
        .await
    };
    let auth = outcome_of(401, json!({"error": {"message": "Invalid API key"}})).await;
    assert_eq!(auth.failure, Some(TestFailure::Auth));
    let unknown = outcome_of(422, json!({"detail": "Unknown model: jev-nope"})).await;
    assert_eq!(unknown.failure, Some(TestFailure::UnknownModel));
    let missing = outcome_of(404, json!({"error": "model not found"})).await;
    assert_eq!(missing.failure, Some(TestFailure::UnknownModel));
    let malformed = outcome_of(200, json!({"answers": {}})).await;
    assert_eq!(malformed.failure, Some(TestFailure::Other));
    assert!(malformed.message.unwrap().contains("malformed"));
    let closed = test_connection(
        &http_client(),
        &system_one("http://127.0.0.1:9".into()),
        Some("jev"),
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(closed.failure, Some(TestFailure::Network));
}

// ---------------------------------------------------------------------------------------------
// Well-known models (AI-03)
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_gateway_list_of_bare_ids_gets_limits_and_levels_from_the_table() {
    use Effort::{High, Low, Medium, Xhigh};
    // New API's OpenAI-format list: no limits or capabilities, whatever the model.
    let server = MockServer::start().await;
    let item = |id: &str| {
        json!({"id": id, "object": "model", "created": 1_626_777_600, "owned_by": "custom",
               "supported_endpoint_types": ["openai", "anthropic"]})
    };
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [item("claude-opus-4-8"), item("gpt-5.5"), item("gemini-2.5-pro"),
                     item("house-model")],
            "object": "list", "success": true
        })))
        .mount(&server)
        .await;
    let models = list_models(&http_client(), &cc(&server), &CancellationToken::new())
        .await
        .unwrap();
    let summary: Vec<_> = models
        .iter()
        .map(|m| {
            (
                m.id.as_str(),
                m.context_window,
                m.max_output_tokens,
                m.efforts.clone(),
                m.adaptive_thinking,
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            (
                "claude-opus-4-8",
                Some(1_000_000),
                Some(128_000),
                None,
                None
            ),
            (
                "gpt-5.5",
                Some(1_050_000),
                Some(128_000),
                Some(vec![Low, Medium, High, Xhigh]),
                None
            ),
            (
                "gemini-2.5-pro",
                Some(1_048_576),
                Some(65_536),
                Some(vec![Low, Medium, High]),
                None
            ),
            ("house-model", None, None, None, None),
        ]
    );

    // Its Anthropic-format list: id, created_at, display_name and type.
    let server = MockServer::start().await;
    let item = |id: &str| {
        json!({"id": id, "created_at": "2026-04-16T00:00:00Z", "display_name": id,
               "type": "model"})
    };
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [item("claude-opus-4-8"), item("claude-haiku-4-5-20251001"),
                     item("house-model")],
            "first_id": "claude-opus-4-8", "has_more": false, "last_id": "house-model"
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
        .map(|m| {
            (
                m.id.as_str(),
                m.context_window,
                m.max_output_tokens,
                m.efforts.clone(),
                m.adaptive_thinking,
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            (
                "claude-opus-4-8",
                Some(1_000_000),
                Some(128_000),
                Some(Effort::ALL.to_vec()),
                Some(true)
            ),
            (
                "claude-haiku-4-5-20251001",
                Some(200_000),
                Some(64_000),
                Some(vec![]),
                Some(false)
            ),
            ("house-model", None, None, None, None),
        ]
    );
}

#[test]
fn known_models_match_ids_with_vendor_prefixes_and_suffixes_but_not_other_versions() {
    use Effort::{High, Low, Max, Medium, Xhigh};
    let opus_4_8 = KnownModel {
        context_window: 1_000_000,
        max_output_tokens: 128_000,
        efforts: Some(&Effort::ALL),
        adaptive_thinking: Some(true),
    };
    for id in [
        "claude-opus-4-8",
        " Claude-Opus-4-8 ",
        "anthropic/claude-opus-4.8",
        "openrouter/anthropic/claude-opus-4.8:nitro",
        "us.anthropic.claude-opus-4-8-v1:0",
        "claude-opus-4-8@20260416",
        "claude-opus-4-8-20260416",
        "claude-opus-4-8-thinking",
        "claude-opus-4-8[1m]",
    ] {
        assert_eq!(known_model(id, Protocol::Anthropic), Some(opus_4_8), "{id}");
    }
    // A later version is not the one before it, and a version is not its family.
    for id in [
        "claude-opus-5-7",
        "claude-opus-4-9",
        "claude-opus-48",
        "gpt-5.9",
        "gpt-50",
        "gemini-2.6-pro",
        "o5",
        "my-model",
        "",
    ] {
        for protocol in [
            Protocol::Anthropic,
            Protocol::ChatCompletions,
            Protocol::Responses,
        ] {
            assert_eq!(known_model(id, protocol), None, "{id} on {protocol:?}");
        }
    }
    // The longest key wins: each version and variant has its own.
    let efforts = |id: &str, protocol| known_model(id, protocol).and_then(|m| m.efforts);
    assert_eq!(
        efforts("claude-opus-4-6", Protocol::Anthropic),
        Some(&[Low, Medium, High, Max][..])
    );
    assert_eq!(
        efforts("claude-sonnet-4-5-20250929", Protocol::Anthropic),
        Some(&[][..])
    );
    assert_eq!(
        efforts("claude-sonnet-4-20250514", Protocol::Anthropic),
        Some(&[][..])
    );
    assert_eq!(
        known_model("claude-sonnet-4-20250514", Protocol::Anthropic).map(|m| m.max_output_tokens),
        Some(64_000)
    );
    assert_eq!(
        efforts("gpt-5.4-mini-2026-03-17", Protocol::Responses),
        Some(&[Low, Medium, High, Xhigh][..])
    );
    assert_eq!(
        efforts("gpt-5.5-pro", Protocol::Responses),
        Some(&[Medium, High, Xhigh][..])
    );
    for id in [
        "gpt-5-chat-latest",
        "gpt-5.1-chat-latest",
        "openai/gpt-5.2-chat",
    ] {
        assert_eq!(
            known_model(id, Protocol::Responses).map(|m| (m.context_window, m.efforts)),
            Some((128_000, Some(&[][..]))),
            "{id}"
        );
    }
    assert_eq!(
        efforts("o3-mini-high", Protocol::ChatCompletions),
        Some(&[Low, Medium, High][..])
    );
    assert_eq!(
        efforts("gpt-4o-2024-08-06", Protocol::ChatCompletions),
        Some(&[][..])
    );
    assert_eq!(
        known_model("gpt-oss:20b", Protocol::ChatCompletions).map(|m| m.context_window),
        Some(131_072)
    );
    assert_eq!(
        known_model("models/gemini-3.1-pro-preview", Protocol::ChatCompletions),
        Some(KnownModel {
            context_window: 1_048_576,
            max_output_tokens: 65_536,
            efforts: Some(&[Low, Medium, High]),
            adaptive_thinking: None,
        })
    );
    // DeepSeek takes the same levels in both of its formats.
    for protocol in [Protocol::Anthropic, Protocol::ChatCompletions] {
        assert_eq!(
            efforts("deepseek-ai/DeepSeek-V4-Pro", protocol),
            Some(&[Low, High, Max][..]),
            "{protocol:?}"
        );
    }
    assert_eq!(
        known_model("qwen3.8-max-0902", Protocol::ChatCompletions).map(|m| (
            m.context_window,
            m.max_output_tokens,
            m.efforts
        )),
        Some((1_000_000, 131_072, Some(&[Low, Medium, Xhigh][..])))
    );
    assert_eq!(
        efforts("qwen3.6-plus", Protocol::ChatCompletions),
        Some(&[][..])
    );
    assert_eq!(
        known_model("qwen-plus-2025-07-14", Protocol::ChatCompletions).map(|m| m.context_window),
        Some(131_072)
    );
    // Retired from its vendor's API, so it stays as unknown as before.
    assert_eq!(
        known_model("deepseek-chat", Protocol::ChatCompletions),
        None
    );
    // The levels are those of the vendor's own format: elsewhere only the limits apply.
    assert_eq!(
        known_model("anthropic/claude-opus-4.8", Protocol::ChatCompletions),
        Some(KnownModel {
            efforts: None,
            adaptive_thinking: None,
            ..opus_4_8
        })
    );
    assert_eq!(
        known_model("gpt-5.2", Protocol::Anthropic).map(|m| (m.context_window, m.efforts)),
        Some((400_000, None))
    );
    // A System One model only decides (AI-47).
    assert_eq!(known_model("gpt-5.2", Protocol::SystemOne), None);
}

#[test]
fn what_the_list_or_the_user_said_wins_over_the_table() {
    let mut spec = ModelSpec {
        context_window: Some(200_000),
        efforts: Some(vec![Effort::High]),
        ..ModelSpec::new("claude-opus-4-8")
    };
    spec.fill_known(Protocol::Anthropic);
    assert_eq!(
        spec,
        ModelSpec {
            id: "claude-opus-4-8".into(),
            context_window: Some(200_000),
            max_output_tokens: Some(128_000),
            efforts: Some(vec![Effort::High]),
            adaptive_thinking: Some(true),
        }
    );
    let mut unknown = ModelSpec::new("house-model");
    unknown.fill_known(Protocol::Anthropic);
    assert_eq!(unknown, ModelSpec::new("house-model"));
}
