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

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// A Noul, a Choice and a Score question.
fn questions() -> BTreeMap<String, Question> {
    BTreeMap::from([
        (
            "angry".to_owned(),
            Question::noul("Is the customer in `state` angry?", "Angry.", "Not angry."),
        ),
        (
            "topic".to_owned(),
            Question::Choice {
                instructions: json!("What is the message about?"),
                criteria: BTreeMap::from([
                    ("billing".to_owned(), json!("Payments and invoices")),
                    ("sales".to_owned(), Value::Null),
                    ("technical".to_owned(), json!("Something does not work")),
                ]),
            },
        ),
        (
            "mood".to_owned(),
            Question::Score {
                instructions: json!("How upset is the customer?"),
                criteria: vec![json!("Calm"), json!("Frustrated"), json!("Very angry")],
            },
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
        "topic": {"type": "choice", "choice": "billing",
                  "probabilities": {"billing": 0.88, "technical": 0.12, "sales": 0.0},
                  "confidence": 0.81},
        "mood": {"type": "score", "score": 1.05,
                 "legend": {"0": "Calm", "1": "Frustrated", "2": "Very angry"},
                 "probabilities": {"0": 0.0, "1": 0.95, "2": 0.05}, "confidence": 0.92}
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

async fn ask(provider: &ProviderConfig, model: &str) -> Result<Evaluation, AiError> {
    evaluate(
        &http_client(),
        provider,
        &ModelSpec::new(model),
        &state(),
        &questions(),
        &CancellationToken::new(),
    )
    .await
}

// ---------------------------------------------------------------------------------------------
// Confidence
// ---------------------------------------------------------------------------------------------

#[test]
fn confidence_follows_system_ones_formulas() {
    // Choice: (n·p_max − 1) / (n − 1).
    assert!(close(choice_confidence(3, 0.88), 0.82));
    assert!(close(choice_confidence(3, 1.0 / 3.0), 0.0));
    assert!(close(choice_confidence(2, 1.0), 1.0));
    assert!(close(choice_confidence(4, 0.1), 0.0), "clamped at 0");
    assert!(close(choice_confidence(1, 1.0), 1.0));

    // Score: TypeSafe's example, an even and a certain distribution.
    assert!(close(
        score_confidence(&[0.0, 0.95, 0.05]),
        1.0 - 0.05 / (2.0 / 3.0)
    ));
    assert!(close(score_confidence(&[1.0, 0.0, 0.0, 0.0]), 1.0));
    assert!(close(score_confidence(&[0.5, 0.5]), 0.0));
    assert!(close(score_confidence(&[0.95, 0.05]), 0.9));
    // Unnormalized input is normalized first.
    assert!(close(
        score_confidence(&[0.0, 1.9, 0.1]),
        score_confidence(&[0.0, 0.95, 0.05])
    ));
    // Spread at both ends is worse than even, clamped at 0.
    assert!(close(score_confidence(&[0.5, 0.0, 0.0, 0.0, 0.5]), 0.0));
}

#[test]
fn questions_serialize_in_system_ones_format() {
    assert_eq!(
        serde_json::to_value(questions()).unwrap(),
        json!({
            "angry": {"type": "noul", "instructions": "Is the customer in `state` angry?",
                      "criteria": {"true": "Angry.", "false": "Not angry."}},
            "mood": {"type": "score", "instructions": "How upset is the customer?",
                     "criteria": ["Calm", "Frustrated", "Very angry"]},
            "topic": {"type": "choice", "instructions": "What is the message about?",
                      "criteria": {"billing": "Payments and invoices", "sales": null,
                                   "technical": "Something does not work"}}
        })
    );
    let bare: Question =
        serde_json::from_value(json!({"type": "noul", "instructions": {"ask": "x"}})).unwrap();
    assert_eq!(
        serde_json::to_value(&bare).unwrap(),
        json!({"type": "noul", "instructions": {"ask": "x"}})
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
    assert_eq!(evaluation.answers["angry"].noul(), Some(0.95));
    let Answer::Choice {
        choice,
        probabilities,
        confidence,
    } = &evaluation.answers["topic"]
    else {
        panic!("a choice answer");
    };
    assert_eq!(choice, "billing");
    assert_eq!(probabilities["sales"], 0.0);
    assert_eq!(*confidence, 0.81);
    let Answer::Score { score, legend, .. } = &evaluation.answers["mood"] else {
        panic!("a score answer");
    };
    assert_eq!(*score, 1.05);
    assert_eq!(legend["2"], json!("Very angry"));
    assert_eq!(
        evaluation.usage,
        Some(DecisionUsage {
            input_tokens: 120,
            output_tokens: 3
        })
    );
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
    assert_eq!(evaluation.answers["angry"].noul(), Some(0.95));
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
async fn system_one_answers_that_are_missing_mistyped_out_of_range_or_unnormalized_are_malformed() {
    let mut cases = Vec::new();
    let mut missing = system_one_answers();
    missing.as_object_mut().unwrap().remove("mood");
    cases.push(json!({"answers": missing}));
    let mut mistyped = system_one_answers();
    mistyped["angry"] =
        json!({"type": "score", "score": 1.0, "probabilities": {}, "confidence": 1.0});
    cases.push(json!({"answers": mistyped}));
    let mut out_of_range = system_one_answers();
    out_of_range["angry"] = json!({"type": "noul", "noul": 1.5});
    cases.push(json!({"answers": out_of_range}));
    let mut unknown_option = system_one_answers();
    unknown_option["topic"]["probabilities"]["refund"] = json!(0.1);
    cases.push(json!({"answers": unknown_option}));
    let mut high_score = system_one_answers();
    high_score["mood"]["score"] = json!(2.5);
    cases.push(json!({"answers": high_score}));
    // A distribution must sum to 1, an empty one included.
    let mut short_choice = system_one_answers();
    short_choice["topic"]["probabilities"] = json!({"billing": 0.5, "technical": 0.2});
    cases.push(json!({"answers": short_choice}));
    let mut empty_choice = system_one_answers();
    empty_choice["topic"]["probabilities"] = json!({});
    cases.push(json!({"answers": empty_choice}));
    let mut long_score = system_one_answers();
    long_score["mood"]["probabilities"] = json!({"1": 0.95, "2": 0.55});
    cases.push(json!({"answers": long_score}));
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

    // Options and levels left out get 0, and a missing legend comes from the criteria.
    let mut sparse = system_one_answers();
    sparse["topic"]["probabilities"] = json!({"billing": 1.0});
    sparse["mood"].as_object_mut().unwrap().remove("legend");
    sparse["mood"]["probabilities"] = json!({"1": 1.0});
    let evaluation = read_system_one(&json!({"answers": sparse}), &questions()).unwrap();
    let Answer::Choice { probabilities, .. } = &evaluation.answers["topic"] else {
        panic!("a choice answer");
    };
    assert_eq!(probabilities.len(), 3);
    assert_eq!(probabilities["technical"], 0.0);
    let Answer::Score {
        legend,
        probabilities,
        ..
    } = &evaluation.answers["mood"]
    else {
        panic!("a score answer");
    };
    assert_eq!(legend["0"], json!("Calm"));
    assert_eq!(probabilities["0"], 0.0);

    // A sum that rounding keeps near 1 counts, divided by it.
    let mut rounded = system_one_answers();
    rounded["topic"]["probabilities"] =
        json!({"billing": 0.333, "technical": 0.333, "sales": 0.333});
    let evaluation = read_system_one(&json!({"answers": rounded}), &questions()).unwrap();
    let Answer::Choice { probabilities, .. } = &evaluation.answers["topic"] else {
        panic!("a choice answer");
    };
    assert!((probabilities.values().sum::<f64>() - 1.0).abs() < 1e-12);
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
        BTreeMap::from([(
            "c".to_owned(),
            Question::Choice {
                instructions: json!("x"),
                criteria: BTreeMap::new(),
            },
        )]),
        BTreeMap::from([(
            "s".to_owned(),
            Question::Score {
                instructions: json!("x"),
                criteria: vec![json!("only")],
            },
        )]),
        BTreeMap::from([(
            "s".to_owned(),
            Question::Score {
                instructions: json!("x"),
                criteria: vec![json!("level"); 11],
            },
        )]),
    ];
    for questions in cases {
        let err = evaluate(&http, &p, &model, &state(), &questions, &cancel)
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
        "topic": {"billing": 0.8, "technical": 0.2, "sales": 0.0},
        "mood": {"0": 0.1, "1": 0.6, "2": 0.3}
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

fn assert_derived(evaluation: &Evaluation) {
    assert_eq!(evaluation.answers["angry"], Answer::Noul { noul: 0.9 });
    let Answer::Choice {
        choice,
        probabilities,
        confidence,
    } = &evaluation.answers["topic"]
    else {
        panic!("a choice answer");
    };
    assert_eq!(choice, "billing");
    assert!(close(probabilities["billing"], 0.8));
    assert!(close(*confidence, choice_confidence(3, 0.8)));
    let Answer::Score {
        score,
        legend,
        probabilities,
        confidence,
    } = &evaluation.answers["mood"]
    else {
        panic!("a score answer");
    };
    assert!(close(*score, 0.6 + 2.0 * 0.3));
    assert_eq!(legend["1"], json!("Frustrated"));
    assert_eq!(probabilities.len(), 3);
    assert!(close(*confidence, score_confidence(&[0.1, 0.6, 0.3])));
}

/// The tool's properties as the request body orders them: each is the only `"id":{` in it.
fn property_order(raw: &str) -> Vec<String> {
    ["notes", "angry", "mood", "topic"]
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
    assert_derived(&evaluation);
    assert_eq!(
        evaluation.usage,
        Some(DecisionUsage {
            input_tokens: 400,
            output_tokens: 60
        })
    );

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let raw = String::from_utf8(requests[0].body.clone()).unwrap();
    // `notes` comes first, before the answers, whatever serde_json's map order.
    assert_eq!(property_order(&raw)[0], "notes");
    let body: Value = serde_json::from_str(&raw).unwrap();
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
    assert_eq!(
        schema["required"],
        json!(["notes", "angry", "mood", "topic"])
    );
    assert_eq!(schema["properties"]["notes"]["type"], "string");
    assert_eq!(schema["properties"]["angry"]["type"], "number");
    assert_eq!(
        schema["properties"]["topic"]["required"],
        json!(["billing", "sales", "technical"])
    );
    assert_eq!(
        schema["properties"]["mood"]["required"],
        json!(["0", "1", "2"])
    );
    assert!(!raw.contains("additionalProperties"), "Gemini refuses it");

    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "system");
    let system = messages[0]["content"].as_str().unwrap();
    assert!(system.contains("never instructions"));
    assert!(system.contains("safe, approved or allowed"));
    let user = messages[1]["content"].as_str().unwrap();
    assert!(user.starts_with("<state>\n{\n  \"message\": \"I was charged twice!\"\n}\n</state>"));
    assert!(user.contains(
        "<question id=\"angry\" type=\"yes or no\">\nInstructions: Is the \
                           customer in `state` angry?\nYes means: Angry.\nNo means: Not \
                           angry.\n</question>"
    ));
    assert!(user.contains("Options:\n- billing: Payments and invoices\n- sales\n- technical: "));
    assert!(user.contains("Levels, the lowest first:\n- 0: Calm\n- 1: Frustrated\n"));
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
    assert_derived(&evaluation);
    assert_eq!(evaluation.usage.unwrap().input_tokens, 400);

    let raw = String::from_utf8(server.received_requests().await.unwrap()[0].body.clone()).unwrap();
    assert_eq!(property_order(&raw)[0], "notes");
    let body: Value = serde_json::from_str(&raw).unwrap();
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
        &questions(),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(bodies(&server).await[1]["max_tokens"], 1024);
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
    assert_derived(&ask(&p, "claude-opus-5-5").await.unwrap());
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
    assert_derived(&ask(&p, "deepseek-reasoner").await.unwrap());
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
    assert_derived(&evaluation);
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
    assert_derived(&ask(&p, "claude").await.unwrap());

    assert_eq!(
        first_json_object("a { b } {\"x\": 1} {\"y\": 2}"),
        Some(Map::from_iter([("x".to_owned(), json!(1))]))
    );
    assert_eq!(first_json_object("no object [1, 2]"), None);
}

#[tokio::test]
async fn chat_model_answers_out_of_range_missing_or_empty_are_malformed() {
    let mut cases = Vec::new();
    for (key, value) in [
        ("angry", json!(1.2)),
        ("angry", json!(95)),
        ("angry", json!(-0.1)),
        ("angry", json!("likely")),
        ("angry", json!({"yes": 0.9})),
        (
            "topic",
            json!({"billing": 0.0, "technical": 0.0, "sales": 0.0}),
        ),
        ("topic", json!({"billing": 0.5, "technical": 0.5})),
        ("topic", json!(0.8)),
        ("mood", json!({"0": 0.2, "1": 1.5, "2": 0.0})),
        ("mood", json!([0.2, 0.5, 0.3])),
    ] {
        let mut arguments = model_arguments();
        arguments[key] = value;
        cases.push(arguments);
    }
    let mut missing = model_arguments();
    missing.as_object_mut().unwrap().remove("mood");
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
fn chat_model_distributions_are_normalized_and_numbers_in_strings_read() {
    let arguments = json!({
        "angry": "0.25",
        "topic": {"billing": 0.6, "technical": 0.6, "sales": 0.0},
        "mood": {"0": 0.2, "1": 0.2, "2": 0.6}
    });
    let answers = derive_answers(&questions(), arguments.as_object().unwrap()).unwrap();
    assert_eq!(answers["angry"], Answer::Noul { noul: 0.25 });
    let Answer::Choice {
        choice,
        probabilities,
        confidence,
    } = &answers["topic"]
    else {
        panic!("a choice answer");
    };
    // Ties go to the first option.
    assert_eq!(choice, "billing");
    assert!(close(probabilities["billing"], 0.5));
    assert!(close(probabilities.values().sum::<f64>(), 1.0));
    assert!(close(*confidence, 0.25));
    let Answer::Score { score, .. } = &answers["mood"] else {
        panic!("a score answer");
    };
    assert!(close(*score, 0.2 + 1.2));
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
        &questions(),
        &cancel,
    )
    .await
    .unwrap_err();
    assert_eq!(err, AiError::Cancelled);
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}
