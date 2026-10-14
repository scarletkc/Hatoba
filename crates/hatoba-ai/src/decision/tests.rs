//! Decision requests against a mock server: System One, Workers AI, and chat models in its place.

use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zeroize::Zeroizing;

use super::*;
use crate::provider::{AuthHeader, http_client};

const KEY: &str = "sk-decision-secret";

fn provider(base_url: String, protocol: Protocol) -> ProviderConfig {
    ProviderConfig {
        protocol,
        base_url,
        api_key: Zeroizing::new(KEY.into()),
        auth_header: AuthHeader::XApiKey,
    }
}

/// Two yes-or-no questions.
fn questions() -> BTreeMap<String, Question> {
    BTreeMap::from([
        (
            "angry".to_owned(),
            Question::noul("Is the customer in `state` angry?", "Angry.", "Not angry."),
        ),
        (
            "urgent".to_owned(),
            Question::noul(
                "Does `state` ask for help at once?",
                "It does.",
                "It does not.",
            ),
        ),
    ])
}

fn state() -> Value {
    json!({"message": "I was charged twice!"})
}

/// System One's answers to [`questions`], as TypeSafe documents them.
fn system_one_answers() -> Value {
    json!({
        "angry": {"type": "noul", "noul": 0.95},
        "urgent": {"type": "noul", "noul": 0.3}
    })
}

async fn bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

async fn raw_bodies(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| String::from_utf8(r.body.clone()).unwrap())
        .collect()
}

async fn ask(provider: &ProviderConfig, model: &str) -> Result<Evaluation, AiError> {
    evaluate(
        &http_client(),
        provider,
        &ModelSpec::new(model),
        &state(),
        None,
        &questions(),
        &CancellationToken::new(),
    )
    .await
}

/// A state whose members are in the order its caller wants, unlike the sorted keys of a
/// [`Value`]: what stays the same first, then what changes.
#[derive(Serialize)]
struct GrowingState {
    zeta_stays: &'static str,
    middle_grows: Vec<&'static str>,
    alpha_changes: &'static str,
}

fn ordered_state() -> GrowingState {
    GrowingState {
        zeta_stays: "the same in every request",
        middle_grows: vec!["one", "two"],
        alpha_changes: "this request's own",
    }
}

#[test]
fn questions_serialize_in_system_ones_format() {
    assert_eq!(
        serde_json::to_value(questions()).unwrap(),
        json!({
            "angry": {"type": "noul", "instructions": "Is the customer in `state` angry?",
                      "criteria": {"true": "Angry.", "false": "Not angry."}},
            "urgent": {"type": "noul", "instructions": "Does `state` ask for help at once?",
                       "criteria": {"true": "It does.", "false": "It does not."}}
        })
    );
    let bare: Question =
        serde_json::from_value(json!({"type": "noul", "instructions": {"ask": "x"}})).unwrap();
    assert_eq!(
        serde_json::to_value(&bare).unwrap(),
        json!({"type": "noul", "instructions": {"ask": "x"}})
    );
    // Only yes-or-no questions are asked.
    assert!(
        serde_json::from_value::<Question>(
            json!({"type": "score", "instructions": "x", "criteria": ["a", "b"]})
        )
        .is_err()
    );
}

// ---------------------------------------------------------------------------------------------
// System One
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn system_one_posts_state_and_questions_and_reads_the_answers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "jev-1.13.0",
            "answers": system_one_answers(),
            "usage": {"input_tokens": 120, "output_tokens": 3}
        })))
        .mount(&server)
        .await;
    let p = provider(format!("{}/v1/", server.uri()), Protocol::SystemOne);
    let evaluation = ask(&p, "jev-latest").await.unwrap();
    assert_eq!(
        bodies(&server).await,
        vec![json!({
            "model": "jev-latest",
            "state": state(),
            "questions": serde_json::to_value(questions()).unwrap()
        })]
    );
    assert_eq!(evaluation.answers["angry"], Answer::Noul { noul: 0.95 });
    assert_eq!(evaluation.answers["urgent"].noul(), 0.3);
    assert_eq!(
        evaluation.usage,
        Some(DecisionUsage {
            input_tokens: 120,
            output_tokens: 3
        })
    );
}

#[tokio::test]
async fn the_state_keeps_the_order_of_its_members() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"answers": system_one_answers()})),
        )
        .mount(&server)
        .await;
    let p = provider(server.uri(), Protocol::SystemOne);
    evaluate(
        &http_client(),
        &p,
        &ModelSpec::new("jev-latest"),
        &ordered_state(),
        None,
        &questions(),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let raw = &raw_bodies(&server).await[0];
    // The body as TypeSafe documents it, and the state as its type orders it, not sorted.
    let at = |text: &str| raw.find(text).unwrap();
    assert!(at("\"model\"") < at("\"state\"") && at("\"state\"") < at("\"questions\""));
    assert!(at("\"zeta_stays\"") < at("\"middle_grows\""));
    assert!(at("\"middle_grows\"") < at("\"alpha_changes\""));
}

#[tokio::test]
async fn a_base_url_that_names_the_endpoint_is_used_as_it_is() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"answers": system_one_answers()})),
        )
        .mount(&server)
        .await;
    let p = provider(
        format!("{}/api/v1/systemone", server.uri()),
        Protocol::SystemOne,
    );
    let evaluation = ask(&p, "jev-latest").await.unwrap();
    assert_eq!(evaluation.usage, None);
    assert_eq!(bodies(&server).await[0]["model"], "jev-latest");
    let base = Url::parse("https://api.typesafe.ai/v1/systemone/").unwrap();
    assert_eq!(
        system_one_base(&base).as_str(),
        "https://api.typesafe.ai/v1"
    );
}

#[tokio::test]
async fn workers_ai_takes_the_model_in_the_url_and_wraps_the_answers_in_result() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/client/v4/accounts/0123abcd/ai/run/@cf/cloudflare/clef-flash",
        ))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "result": {
                "model": "clef-flash",
                "answers": system_one_answers(),
                "usage": {"input_tokens": 90, "output_tokens": 3}
            },
            "success": true, "errors": [], "messages": []
        })))
        .mount(&server)
        .await;
    let p = provider(
        format!("{}/client/v4/accounts/0123abcd/ai/run/", server.uri()),
        Protocol::SystemOne,
    );
    let evaluation = ask(&p, "@cf/cloudflare/clef-flash").await.unwrap();
    assert_eq!(bodies(&server).await[0]["model"], "clef-flash");
    assert_eq!(evaluation.answers["angry"].noul(), 0.95);
    assert_eq!(evaluation.usage.unwrap().input_tokens, 90);

    let base = Url::parse("https://api.cloudflare.com/client/v4/accounts/a/ai/run").unwrap();
    let (url, model) = system_one_target(&base, "@cf/cloudflare/clef");
    assert_eq!(
        url.as_str(),
        "https://api.cloudflare.com/client/v4/accounts/a/ai/run/@cf/cloudflare/clef"
    );
    assert_eq!(model, "clef");
    // A base URL pasted with a model after `/ai/run` runs the provider's model.
    let pasted = Url::parse(
        "https://api.cloudflare.com/client/v4/accounts/a/ai/run/@cf/cloudflare/clef-flash",
    )
    .unwrap();
    let (url, model) = system_one_target(&pasted, "@cf/cloudflare/clef");
    assert_eq!(
        url.as_str(),
        "https://api.cloudflare.com/client/v4/accounts/a/ai/run/@cf/cloudflare/clef"
    );
    assert_eq!(model, "clef");
    // `/ai/running` is no Workers AI path.
    let (url, _) = system_one_target(&Url::parse("http://x.local/ai/running").unwrap(), "jev");
    assert_eq!(url.as_str(), "http://x.local/ai/running/systemone");
    let (url, model) = system_one_target(&Url::parse("http://jev.local:8080").unwrap(), "jev");
    assert_eq!(url.as_str(), "http://jev.local:8080/systemone");
    assert_eq!(model, "jev");
}

#[tokio::test]
async fn system_one_answers_that_are_missing_mistyped_or_out_of_range_are_malformed() {
    let mut cases = Vec::new();
    let mut missing = system_one_answers();
    missing.as_object_mut().unwrap().remove("urgent");
    cases.push(json!({"answers": missing}));
    let mut mistyped = system_one_answers();
    mistyped["angry"] =
        json!({"type": "score", "score": 1.0, "probabilities": {}, "confidence": 1.0});
    cases.push(json!({"answers": mistyped}));
    let mut out_of_range = system_one_answers();
    out_of_range["angry"] = json!({"type": "noul", "noul": 1.5});
    cases.push(json!({"answers": out_of_range}));
    let mut negative = system_one_answers();
    negative["urgent"] = json!({"type": "noul", "noul": -0.2});
    cases.push(json!({"answers": negative}));
    let mut text = system_one_answers();
    text["urgent"] = json!({"type": "noul", "noul": "likely"});
    cases.push(json!({"answers": text}));
    cases.push(json!({"result": {"answers": []}}));
    cases.push(json!({"model": "jev"}));
    for body in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body.clone()))
            .mount(&server)
            .await;
        let p = provider(server.uri(), Protocol::SystemOne);
        let err = ask(&p, "jev-latest").await.unwrap_err();
        assert!(matches!(err, AiError::Protocol(_)), "{body}: {err:?}");
    }

    // A probability that rounding puts just outside 0 to 1 counts, clamped.
    let mut rounded = system_one_answers();
    rounded["angry"] = json!({"type": "noul", "noul": 1.000_000_1});
    let evaluation = read_system_one(&json!({"answers": rounded}), &questions()).unwrap();
    assert_eq!(evaluation.answers["angry"].noul(), 1.0);
}

#[tokio::test]
async fn system_one_errors_carry_the_providers_message() {
    for status in [401, 422, 429, 529] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(status).set_body_json(json!({"error": {"message": "nope"}})),
            )
            .mount(&server)
            .await;
        let p = provider(server.uri(), Protocol::SystemOne);
        assert_eq!(
            ask(&p, "jev-latest").await.unwrap_err(),
            AiError::Http {
                status,
                message: "nope".into()
            }
        );
    }
}

#[tokio::test]
async fn questions_that_system_one_does_not_allow_are_refused_without_a_request() {
    let server = MockServer::start().await;
    let p = provider(server.uri(), Protocol::SystemOne);
    let http = http_client();
    let cancel = CancellationToken::new();
    let model = ModelSpec::new("jev-latest");
    let noul = || Question::noul("x", "y", "n");
    let cases = [
        BTreeMap::new(),
        BTreeMap::from([("notes".to_owned(), noul())]),
        BTreeMap::from([(String::new(), noul())]),
    ];
    for questions in cases {
        let err = evaluate(&http, &p, &model, &state(), None, &questions, &cancel)
            .await
            .unwrap_err();
        assert!(matches!(err, AiError::Config(_)), "{err:?}");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

// ---------------------------------------------------------------------------------------------
// A chat model in System One's place
// ---------------------------------------------------------------------------------------------

/// What a chat model answers for [`questions`].
fn model_arguments() -> Value {
    json!({
        "notes": "A customer complains about a double charge.",
        "angry": 0.9,
        "urgent": 0.2
    })
}

fn chat_reply(arguments: &Value) -> Value {
    json!({
        "id": "chatcmpl-1",
        "choices": [{"index": 0, "finish_reason": "tool_calls", "message": {
            "role": "assistant", "content": null,
            "tool_calls": [{"id": "call_1", "type": "function", "function": {
                "name": "answer", "arguments": arguments.to_string()
            }}]
        }}],
        "usage": {"prompt_tokens": 400, "completion_tokens": 60, "total_tokens": 460}
    })
}

fn anthropic_reply(input: &Value) -> Value {
    json!({
        "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
        "content": [
            {"type": "thinking", "thinking": "Hmm.", "signature": "sig"},
            {"type": "tool_use", "id": "toolu_1", "name": "answer", "input": input}
        ],
        "stop_reason": "tool_use",
        "usage": {"input_tokens": 300, "cache_read_input_tokens": 100, "output_tokens": 50}
    })
}

fn assert_answered(evaluation: &Evaluation) {
    assert_eq!(evaluation.answers.len(), 2);
    assert_eq!(evaluation.answers["angry"], Answer::Noul { noul: 0.9 });
    assert_eq!(evaluation.answers["urgent"], Answer::Noul { noul: 0.2 });
}

/// The tool's properties as the request body orders them: each is the only `"id":{` in it.
fn property_order(raw: &str) -> Vec<String> {
    ["notes", "angry", "urgent"]
        .iter()
        .map(|key| {
            let pattern = format!("\"{key}\":{{");
            assert_eq!(raw.matches(&pattern).count(), 1, "{pattern}");
            (raw.find(&pattern).unwrap(), (*key).to_owned())
        })
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect()
}

#[tokio::test]
async fn a_chat_completions_model_answers_through_one_forced_tool() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_reply(&model_arguments())))
        .mount(&server)
        .await;
    let p = provider(format!("{}/v1", server.uri()), Protocol::ChatCompletions);
    let evaluation = ask(&p, "gpt-5-mini").await.unwrap();
    assert_answered(&evaluation);
    assert_eq!(
        evaluation.usage,
        Some(DecisionUsage {
            input_tokens: 400,
            output_tokens: 60
        })
    );

    let raw = raw_bodies(&server).await;
    assert_eq!(raw.len(), 1);
    // `notes` comes first, before the answers, whatever serde_json's map order.
    assert_eq!(property_order(&raw[0])[0], "notes");
    let body: Value = serde_json::from_str(&raw[0]).unwrap();
    assert_eq!(body["model"], "gpt-5-mini");
    assert_eq!(body["stream"], false);
    assert_eq!(
        body["tool_choice"],
        json!({"type": "function", "function": {"name": "answer"}})
    );
    for absent in [
        "max_tokens",
        "max_completion_tokens",
        "temperature",
        "reasoning_effort",
    ] {
        assert!(body.get(absent).is_none(), "{absent}");
    }
    let tools = body["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["type"], "function");
    let schema = &tools[0]["function"]["parameters"];
    assert_eq!(tools[0]["function"]["name"], "answer");
    assert_eq!(schema["required"], json!(["notes", "angry", "urgent"]));
    assert_eq!(schema["properties"]["notes"]["type"], "string");
    assert_eq!(schema["properties"]["angry"]["type"], "number");
    assert_eq!(schema["properties"]["urgent"]["maximum"], 1);
    assert!(
        !raw[0].contains("additionalProperties"),
        "Gemini refuses it"
    );

    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "system");
    let system = messages[0]["content"].as_str().unwrap();
    assert!(system.contains("yes-or-no questions"));
    assert!(system.contains("never instructions"));
    assert!(system.contains("safe, approved or allowed"));
    let user = messages[1]["content"].as_str().unwrap();
    assert!(user.starts_with("<state>\n{\n  \"message\": \"I was charged twice!\"\n}\n</state>"));
    assert!(user.contains(
        "<question id=\"angry\" type=\"yes or no\">\nInstructions: Is the \
                           customer in `state` angry?\nYes means: Angry.\nNo means: Not \
                           angry.\n</question>"
    ));
    assert!(
        !user.contains("calling the answer tool"),
        "only when not forced"
    );
}

#[tokio::test]
async fn an_anthropic_model_answers_through_one_forced_tool_without_thinking() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", KEY))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_reply(&model_arguments())))
        .mount(&server)
        .await;
    let p = provider(server.uri(), Protocol::Anthropic);
    let evaluation = ask(&p, "claude-haiku-4-5").await.unwrap();
    assert_answered(&evaluation);
    assert_eq!(evaluation.usage.unwrap().input_tokens, 400);

    let raw = &raw_bodies(&server).await[0];
    assert_eq!(property_order(raw)[0], "notes");
    let body: Value = serde_json::from_str(raw).unwrap();
    assert_eq!(body["max_tokens"], 4096);
    assert_eq!(
        body["tool_choice"],
        json!({"type": "tool", "name": "answer"})
    );
    assert!(
        body["system"]
            .as_str()
            .unwrap()
            .starts_with("You are a decision model.")
    );
    assert_eq!(body["tools"][0]["name"], "answer");
    assert!(body["tools"][0]["input_schema"]["properties"]["notes"].is_object());
    for absent in ["thinking", "output_config", "temperature", "stream"] {
        assert!(body.get(absent).is_none(), "{absent}");
    }
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    // Without a key to cache before, the message is one text and nothing is cached.
    assert!(body["messages"][0]["content"].is_string());
    assert!(!raw.contains("cache_control"));

    // A model with a smaller output limit is asked for no more than it has.
    let small = ModelSpec {
        max_output_tokens: Some(1024),
        ..ModelSpec::new("small")
    };
    evaluate(
        &http_client(),
        &p,
        &small,
        &state(),
        None,
        &questions(),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(bodies(&server).await[1]["max_tokens"], 1024);
}

#[tokio::test]
async fn an_anthropic_request_caches_the_state_up_to_the_key_given() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_reply(&model_arguments())))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_reply(&model_arguments())))
        .mount(&server)
        .await;
    let send = |protocol, key: &'static str| {
        let p = provider(server.uri(), protocol);
        async move {
            let model = ModelSpec::new("claude-haiku-4-5");
            let state = ordered_state();
            let questions = questions();
            let cancel = CancellationToken::new();
            evaluate(
                &http_client(),
                &p,
                &model,
                &state,
                Some(key),
                &questions,
                &cancel,
            )
            .await
            .unwrap()
        }
    };
    assert_answered(&send(Protocol::Anthropic, "middle_grows").await);
    let body = &bodies(&server).await[0];
    let content = body["messages"][0]["content"].as_array().unwrap();
    assert_eq!(content.len(), 2);
    // The first block is the start that stays the same, ending in a cache breakpoint.
    assert_eq!(
        content[0],
        json!({
            "type": "text",
            "text": "<state>\n{\n  \"zeta_stays\": \"the same in every request\",",
            "cache_control": {"type": "ephemeral"}
        })
    );
    let rest = content[1]["text"].as_str().unwrap();
    assert!(rest.starts_with("\n  \"middle_grows\": [\n    \"one\","));
    assert!(rest.contains("\"alpha_changes\": \"this request's own\"\n}\n</state>"));
    assert!(rest.contains("<question id=\"urgent\""));
    assert!(content[1].get("cache_control").is_none());

    // Together they are the message a request without a breakpoint sends, as Chat Completions
    // does, whose provider caches the start of a prompt by itself.
    let whole = format!("{}{rest}", content[0]["text"].as_str().unwrap());
    assert_answered(&send(Protocol::ChatCompletions, "middle_grows").await);
    let body = &bodies(&server).await[1];
    assert_eq!(body["messages"][1]["content"], json!(whole));
    assert!(!body.to_string().contains("cache_control"));

    // A key the state does not have at its top level splits nothing.
    assert_answered(&send(Protocol::Anthropic, "one").await);
    assert_eq!(
        bodies(&server).await[2]["messages"][0]["content"],
        json!(whole)
    );
}

fn responses_reply(arguments: &Value) -> Value {
    json!({
        "id": "resp_1", "object": "response", "status": "completed",
        "output": [
            {"id": "rs_1", "type": "reasoning", "summary": []},
            {"id": "fc_1", "type": "function_call", "status": "completed", "call_id": "call_1",
             "name": "answer", "arguments": arguments.to_string()}
        ],
        "usage": {"input_tokens": 350, "input_tokens_details": {"cached_tokens": 0},
                  "output_tokens": 70, "total_tokens": 420}
    })
}

#[tokio::test]
async fn a_responses_model_answers_through_one_forced_tool_without_storing_it() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .and(body_partial_json(
            json!({"tool_choice": {"type": "function", "name": "answer"}}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(responses_reply(&model_arguments())))
        .mount(&server)
        .await;
    let p = provider(format!("{}/v1", server.uri()), Protocol::Responses);
    let evaluation = ask(&p, "gpt-5-mini").await.unwrap();
    assert_answered(&evaluation);
    assert_eq!(
        evaluation.usage,
        Some(DecisionUsage {
            input_tokens: 350,
            output_tokens: 70
        })
    );

    let raw = raw_bodies(&server).await;
    assert_eq!(raw.len(), 1);
    assert_eq!(property_order(&raw[0])[0], "notes");
    let body: Value = serde_json::from_str(&raw[0]).unwrap();
    assert_eq!(body["model"], "gpt-5-mini");
    assert_eq!(body["store"], false);
    for absent in [
        "stream",
        "max_output_tokens",
        "temperature",
        "reasoning",
        "include",
        "messages",
    ] {
        assert!(body.get(absent).is_none(), "{absent}");
    }
    let tools = body["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["type"], "function");
    assert_eq!(tools[0]["name"], "answer");
    assert_eq!(tools[0]["strict"], false);
    assert_eq!(
        tools[0]["parameters"]["required"],
        json!(["notes", "angry", "urgent"])
    );
    let instructions = body["instructions"].as_str().unwrap();
    assert!(instructions.contains("yes-or-no questions"));
    let input = body["input"].as_array().unwrap();
    assert_eq!(input.len(), 1);
    assert_eq!(input[0]["role"], "user");
    assert!(
        input[0]["content"]
            .as_str()
            .unwrap()
            .starts_with("<state>\n{\n  \"message\": \"I was charged twice!\"\n}\n</state>")
    );

    // A server that does not take a named tool gets auto, and an answer in text still counts.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(
            json!({"tool_choice": {"type": "function"}}),
        ))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {"message": "tool_choice must be \"auto\"", "code": "invalid_value"}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"tool_choice": "auto"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "completed",
            "output": [{"type": "message", "role": "assistant", "content": [
                {"type": "output_text", "text": format!("Answers: {}", model_arguments())}
            ]}]
        })))
        .mount(&server)
        .await;
    let p = provider(server.uri(), Protocol::Responses);
    let evaluation = ask(&p, "local-model").await.unwrap();
    assert_answered(&evaluation);
    assert_eq!(evaluation.usage, None);
    let sent = bodies(&server).await;
    assert_eq!(sent.len(), 2);
    let user = sent[1]["input"][0]["content"].as_str().unwrap();
    assert!(user.ends_with("\n\nAnswer by calling the answer tool."));
}

#[tokio::test]
async fn a_provider_that_refuses_a_forced_tool_gets_auto_and_a_request_to_call_it() {
    // Anthropic: thinking does not allow a forced tool.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"tool_choice": {"type": "tool"}})))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "type": "error",
            "error": {"type": "invalid_request_error",
                      "message": "Thinking may not be enabled when tool_choice forces tool use."}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"tool_choice": {"type": "auto"}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_reply(&model_arguments())))
        .mount(&server)
        .await;
    let p = provider(server.uri(), Protocol::Anthropic);
    assert_answered(&ask(&p, "claude-opus-5-5").await.unwrap());
    let sent = bodies(&server).await;
    assert_eq!(sent.len(), 2);
    let user = sent[1]["messages"][0]["content"].as_str().unwrap();
    assert!(user.ends_with("\n\nAnswer by calling the answer tool."));

    // Chat Completions: a server that does not take a named tool_choice.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(
            json!({"tool_choice": {"type": "function"}}),
        ))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {"message": "`tool_choice` only supports \"auto\" and \"none\""}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"tool_choice": "auto"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_reply(&model_arguments())))
        .mount(&server)
        .await;
    let p = provider(server.uri(), Protocol::ChatCompletions);
    assert_answered(&ask(&p, "deepseek-reasoner").await.unwrap());
    assert_eq!(bodies(&server).await.len(), 2);

    // Any other 4xx is not retried.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(400).set_body_json(json!({"error": {"message": "bad model"}})),
        )
        .mount(&server)
        .await;
    let p = provider(server.uri(), Protocol::ChatCompletions);
    assert!(matches!(
        ask(&p, "x").await.unwrap_err(),
        AiError::Http { status: 400, .. }
    ));
    assert_eq!(bodies(&server).await.len(), 1);
}

#[tokio::test]
async fn an_answer_written_as_text_is_read_from_its_first_json_object() {
    let text = format!(
        "Sure. Here are my answers:\n```json\n{}\n```\nLet me know {{if}} that helps.",
        model_arguments()
    );
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{"message": {"role": "assistant", "content": text}}]
        })))
        .mount(&server)
        .await;
    let p = provider(server.uri(), Protocol::ChatCompletions);
    let evaluation = ask(&p, "local").await.unwrap();
    assert_answered(&evaluation);
    assert_eq!(evaluation.usage, None);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{"type": "text", "text": format!("{{not json}} {}", model_arguments())}],
            "usage": {"input_tokens": 1, "output_tokens": 2}
        })))
        .mount(&server)
        .await;
    let p = provider(server.uri(), Protocol::Anthropic);
    assert_answered(&ask(&p, "claude").await.unwrap());

    assert_eq!(
        first_json_object("a { b } {\"x\": 1} {\"y\": 2}"),
        Some(Map::from_iter([("x".to_owned(), json!(1))]))
    );
    assert_eq!(first_json_object("no object [1, 2]"), None);
}

#[tokio::test]
async fn chat_model_answers_out_of_range_or_missing_are_malformed() {
    let mut cases = Vec::new();
    for value in [
        json!(1.2),
        json!(95),
        json!(-0.1),
        json!("likely"),
        json!({"yes": 0.9}),
        json!([0.9]),
    ] {
        let mut arguments = model_arguments();
        arguments["angry"] = value;
        cases.push(arguments);
    }
    let mut missing = model_arguments();
    missing.as_object_mut().unwrap().remove("urgent");
    cases.push(missing);
    for arguments in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(chat_reply(&arguments)))
            .mount(&server)
            .await;
        let p = provider(server.uri(), Protocol::ChatCompletions);
        let err = ask(&p, "m").await.unwrap_err();
        assert!(matches!(err, AiError::Protocol(_)), "{arguments}: {err:?}");
    }

    // No tool call and no JSON at all.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{"message": {"role": "assistant", "content": "I cannot help."}}]
        })))
        .mount(&server)
        .await;
    let p = provider(server.uri(), Protocol::ChatCompletions);
    assert!(matches!(
        ask(&p, "m").await.unwrap_err(),
        AiError::Protocol(_)
    ));
}

#[test]
fn chat_model_numbers_in_strings_are_read_and_rounding_is_clamped() {
    let arguments = json!({"angry": "0.25", "urgent": 0});
    let answers = derive_answers(&questions(), arguments.as_object().unwrap()).unwrap();
    assert_eq!(answers["angry"], Answer::Noul { noul: 0.25 });
    assert_eq!(answers["urgent"], Answer::Noul { noul: 0.0 });
    // A value rounded just past 1 counts as 1.
    assert_eq!(probability(&json!(1.000_000_1)), Some(1.0));
    assert_eq!(probability(&json!(1.01)), None);
    assert_eq!(probability(&json!(f64::MAX)), None);
}

#[tokio::test]
async fn cancelling_drops_the_decision_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_secs(30))
                .set_body_json(json!({"answers": system_one_answers()})),
        )
        .mount(&server)
        .await;
    let p = provider(server.uri(), Protocol::SystemOne);
    let cancel = CancellationToken::new();
    let later = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        later.cancel();
    });
    let started = std::time::Instant::now();
    let err = evaluate(
        &http_client(),
        &p,
        &ModelSpec::new("jev"),
        &state(),
        None,
        &questions(),
        &cancel,
    )
    .await
    .unwrap_err();
    assert_eq!(err, AiError::Cancelled);
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}
