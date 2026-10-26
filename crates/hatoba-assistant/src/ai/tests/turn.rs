//! Failed turns, retries, the thinking level (AI-05) and unknown models.

use super::*;

#[tokio::test]
async fn a_provider_error_ends_the_turn_and_retry_goes_on() {
    let f = Fixture::new(vec![
        ResponseTemplate::new(503)
            .set_body_json(json!({"error": {"message": "overloaded, try later"}})),
        answer("Recovered."),
    ])
    .await;
    let (started, sink) = f.send(None, "hi").await;
    let conv = started.conversation.id;
    assert_eq!(sink.ended().await, AiTurnEndReason::Error);
    assert_eq!(
        sink.errors(),
        [(Some(503), "overloaded, try later".to_owned())]
    );
    f.idle(&conv).await;
    assert_eq!(f.entries(&conv).len(), 1);

    let retry = f.retry(&conv, f.context()).unwrap();
    assert_eq!(retry.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 2);
    assert!(matches!(&entries[1].body, EntryBody::Assistant(a) if a.text == "Recovered."));

    // The conversation ends with the model's answer: nothing to retry.
    let refused = f.retry(&conv, f.context()).unwrap_err();
    assert_eq!(refused.code, ErrorCode::InvalidInput);
    assert_eq!(f.requests().await.len(), 2);
}

#[tokio::test]
async fn a_reply_without_an_answer_ends_the_turn_as_empty_and_retry_leaves_it_out() {
    let finish =
        |reason: &str| json!({"choices": [{"index": 0, "delta": {}, "finish_reason": reason}]});
    let f = Fixture::new(vec![
        // Reasoning only.
        sse(&[
            json!({"choices": [{"index": 0, "delta": {"role": "assistant", "reasoning_content": "Thinking it over."}}]}),
            finish("stop"),
        ]),
        // Blank text.
        sse(&[text_delta(" \n "), finish("stop")]),
        // Cut off before any text: the output limit is what the panel names.
        sse(&[
            json!({"choices": [{"index": 0, "delta": {"reasoning_content": "Still thinking."}}]}),
            finish("length"),
        ]),
        answer("Recovered."),
    ])
    .await;
    let (started, sink) = f.send(None, "hi").await;
    let conv = started.conversation.id;
    assert_eq!(sink.ended().await, AiTurnEndReason::Empty);
    assert!(sink.errors().is_empty());
    f.idle(&conv).await;
    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 2);
    assert!(matches!(&entries[1].body, EntryBody::Assistant(a)
        if a.text.is_empty() && a.reasoning.as_deref() == Some("Thinking it over.")));

    // The empty reply is no answer, so there is something to retry.
    let retry = f.retry(&conv, f.context()).unwrap();
    assert_eq!(retry.ended().await, AiTurnEndReason::Empty);
    f.idle(&conv).await;
    let retry = f.retry(&conv, f.context()).unwrap();
    assert_eq!(retry.ended().await, AiTurnEndReason::Length);
    f.idle(&conv).await;
    let retry = f.retry(&conv, f.context()).unwrap();
    assert_eq!(retry.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 5);
    assert!(matches!(&entries[4].body, EntryBody::Assistant(a) if a.text == "Recovered."));

    // Every request leaves the empty replies out.
    let requests = f.requests().await;
    assert_eq!(requests.len(), 4);
    for body in &requests {
        let roles: Vec<&str> = messages(body)
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["system", "user"]);
    }
}

/// The level the stored conversation keeps (AI-05).
fn stored_effort(f: &Fixture, conversation_id: &str) -> Option<CoreEffort> {
    lock(&f.vault)
        .get(conversation_id)
        .and_then(Item::as_ai_conversation)
        .unwrap()
        .effort
}

#[tokio::test]
async fn the_thinking_level_goes_with_every_request_and_stays_with_the_conversation() {
    let f = Fixture::new(vec![
        calls(&[("call_1", "web_search", json!({"query": "x"}))]),
        answer("Done."),
        ResponseTemplate::new(400).set_body_json(json!({"error": {
            "message": "Unsupported parameter: 'reasoning_effort' is not supported with this model."
        }})),
        answer("Without the level."),
        answer("At Default."),
    ])
    .await;
    let at = |effort| AiTurnContext {
        effort,
        ..f.context()
    };

    // A new conversation stores the level, and each request of the turn carries it. The model
    // offers Low to High (nothing is known about it), so Max is sent as High.
    let (started, sink) = f.send_with(None, "hi", at(Some(AiEffort::Max))).await;
    let conv = started.conversation.id;
    assert_eq!(started.conversation.effort, Some(AiEffort::Max));
    assert_eq!(stored_effort(&f, &conv), Some(CoreEffort::Max));
    sink.done_with(AiFinish::ToolCalls).await;
    f.result(&conv, "call_1", AiToolStatus::Ok, "found", None)
        .await
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;

    // A refused level: the request goes again without it, and the panel hears about it.
    let (_, sink) = f
        .send_with(Some(&conv), "again", at(Some(AiEffort::Low)))
        .await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    assert_eq!(
        kinds(&sink.events())
            .iter()
            .filter(|k| **k == "effort_ignored")
            .count(),
        1
    );
    assert_eq!(stored_effort(&f, &conv), Some(CoreEffort::Low));

    // Default sends nothing and is stored as no level.
    let (started, sink) = f.send_with(Some(&conv), "plain", at(None)).await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    assert_eq!(started.conversation.effort, None);
    assert_eq!(stored_effort(&f, &conv), None);
    assert!(!kinds(&sink.events()).contains(&"effort_ignored"));

    let requests = f.requests().await;
    let efforts: Vec<Option<&str>> = requests
        .iter()
        .map(|body| body.get("reasoning_effort").and_then(Value::as_str))
        .collect();
    assert_eq!(
        efforts,
        [Some("high"), Some("high"), Some("low"), None, None]
    );

    // A retry at another level is the one the conversation keeps.
    let retry = f.retry(&conv, at(Some(AiEffort::Medium))).unwrap_err();
    assert_eq!(retry.code, ErrorCode::InvalidInput, "nothing to retry");
    assert_eq!(stored_effort(&f, &conv), None);
}

#[tokio::test]
async fn a_retry_keeps_the_level_it_was_sent_with() {
    let f = Fixture::new(vec![
        ResponseTemplate::new(503).set_body_json(json!({"error": {"message": "busy"}})),
        answer("Recovered."),
    ])
    .await;
    let (started, sink) = f.send(None, "hi").await;
    let conv = started.conversation.id;
    assert_eq!(sink.ended().await, AiTurnEndReason::Error);
    f.idle(&conv).await;
    assert_eq!(stored_effort(&f, &conv), None);

    let context = AiTurnContext {
        effort: Some(AiEffort::Medium),
        ..f.context()
    };
    let retry = f.retry(&conv, context).unwrap();
    assert_eq!(retry.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    assert_eq!(stored_effort(&f, &conv), Some(CoreEffort::Medium));
    assert_eq!(f.requests().await[1]["reasoning_effort"], "medium");
    assert_eq!(f.detail(&conv).conversation.effort, Some(AiEffort::Medium));
}

#[tokio::test]
async fn an_unknown_model_is_an_error_of_the_turn() {
    let f = Fixture::new(Vec::new()).await;
    let sink = Arc::new(Sink::default());
    let (started, task) = f
        .manager
        .send(
            &f.vault,
            f.env(),
            AiSendInput {
                conversation_id: None,
                text: "hi".into(),
                context: AiTurnContext {
                    model_id: "gone".into(),
                    ..f.context()
                },
            },
            sink.clone(),
        )
        .await
        .unwrap();
    tokio::spawn(task);
    assert_eq!(sink.ended().await, AiTurnEndReason::Error);
    assert_eq!(sink.errors().len(), 1);
    assert!(sink.errors()[0].1.contains("gone"));
    assert_eq!(f.entries(&started.conversation.id).len(), 1);
    assert!(f.requests().await.is_empty());

    let empty = f
        .manager
        .send(
            &f.vault,
            f.env(),
            AiSendInput {
                conversation_id: None,
                text: " \n ".into(),
                context: f.context(),
            },
            sink,
        )
        .await;
    assert_eq!(empty.err().unwrap().field.as_deref(), Some("text"));
}
