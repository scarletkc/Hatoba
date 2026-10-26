//! Editing a message to send it again (AI-26).

use super::*;

#[tokio::test]
async fn editing_a_message_deletes_it_and_what_followed_and_repairs_the_context_start() {
    let f = Fixture::new(vec![
        answer("Answer to gamma-edited."),
        answer("Answer to beta-edited."),
        answer("Answer to alpha-edited."),
    ])
    .await;
    let assistant = |text: &str| {
        AiEntry::assistant(
            2,
            AssistantEntry {
                provider_id: f.provider_id.clone(),
                model_id: "m1".into(),
                text: text.into(),
                finish: Finish::Stop,
                ..AssistantEntry::default()
            },
        )
    };
    // alpha, its answer, summary 1, beta, its answer, summary 2, gamma, its answer.
    let (conv, ids) = {
        let mut v = lock(&f.vault);
        let conv = v
            .put(
                None,
                Item::AiConversation(AiConversation {
                    title: "edit".into(),
                    ..AiConversation::default()
                }),
            )
            .unwrap();
        let entries = [
            AiEntry::user(1, "alpha"),
            assistant("answer alpha"),
            AiEntry::summary(3, "SUMMARY ONE"),
            AiEntry::user(4, "beta"),
            assistant("answer beta"),
            AiEntry::summary(6, "SUMMARY TWO"),
            AiEntry::user(7, "gamma"),
            assistant("answer gamma"),
        ];
        let ids: Vec<String> = entries
            .iter()
            .map(|e| append(&mut v, &conv, e).unwrap())
            .collect();
        let mut c = v
            .get(&conv)
            .and_then(Item::as_ai_conversation)
            .cloned()
            .unwrap();
        c.context_start = Some(ids[5].clone());
        v.put(Some(&conv), Item::AiConversation(c)).unwrap();
        (conv, ids)
    };

    // Only the user's own messages can be edited.
    let err = f.edit(&conv, &ids[1], "x").await.err().unwrap();
    assert_eq!(err.field.as_deref(), Some("entry_id"));
    let err = f
        .edit(&conv, "0190a0a0-0000-7000-8000-000000000000", "x")
        .await;
    assert_eq!(err.err().unwrap().code, ErrorCode::NotFound);
    let err = f.edit(&conv, &ids[6], "  ").await.err().unwrap();
    assert_eq!(err.field.as_deref(), Some("text"));
    assert_eq!(f.entries(&conv).len(), 8);

    // gamma comes after the context start (summary 2): the start stays. What the model saw of
    // files (AI-38…40) may be in the deleted entries, so it is forgotten.
    f.manager
        .0
        .seen
        .record(&conv, "response", "ops@db:22", "/etc/hosts", b"x");
    let (started, sink) = f.edit(&conv, &ids[6], "gamma-edited").await.unwrap();
    assert!(!f.manager.0.seen.knows(&conv));
    assert_eq!(
        started.conversation.context_start.as_deref(),
        Some(ids[5].as_str())
    );
    assert!(entry_id(&started.user_entry) > ids[7].as_str());
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    let texts: Vec<String> = f
        .entries(&conv)
        .iter()
        .map(|e| match &e.body {
            EntryBody::User { text, .. } | EntryBody::Summary { text } => text.clone(),
            EntryBody::Assistant(a) => a.text.clone(),
            EntryBody::Tool { content, .. } => content.clone(),
        })
        .collect();
    // The stored summaries name no mode, as before every summary did, so the edit names it.
    assert_eq!(
        texts,
        [
            "alpha",
            "answer alpha",
            "SUMMARY ONE",
            "beta",
            "answer beta",
            "SUMMARY TWO",
            manual("gamma-edited").as_str(),
            "Answer to gamma-edited."
        ]
    );
    let request = f.requests().await[0].to_string();
    assert!(request.contains("SUMMARY TWO") && request.contains("gamma-edited"));
    assert!(!request.contains("answer gamma") && !request.contains("beta"));

    // beta comes before it: summary 2 is deleted, so the start moves to summary 1.
    let (started, sink) = f.edit(&conv, &ids[3], "beta-edited").await.unwrap();
    assert_eq!(
        started.conversation.context_start.as_deref(),
        Some(ids[2].as_str())
    );
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    assert_eq!(f.entries(&conv).len(), 5);
    assert_eq!(f.context_start(&conv).as_deref(), Some(ids[2].as_str()));

    // alpha comes before every summary: the start is cleared.
    let (started, sink) = f.edit(&conv, &ids[0], "alpha-edited").await.unwrap();
    assert_eq!(started.conversation.context_start, None);
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 2);
    assert!(
        matches!(&entries[0].body, EntryBody::User { text, .. } if *text == manual("alpha-edited"))
    );
    let request = f.requests().await[2].to_string();
    assert!(!request.contains("SUMMARY ONE"));
}

#[tokio::test]
async fn editing_while_a_turn_runs_stops_it_first() {
    let f = Fixture::new(vec![
        calls(&[("c1", "read_terminal", json!({}))]),
        answer("Edited answer."),
    ])
    .await;
    let (started, first) = f.send(None, "original").await;
    let conv = started.conversation.id;
    let user = entry_id(&started.user_entry).to_owned();
    first.done_with(AiFinish::ToolCalls).await;
    let (_, second) = f.edit(&conv, &user, "edited").await.unwrap();
    assert_eq!(first.ended().await, AiTurnEndReason::Stopped);
    assert_eq!(second.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    // The open call and its cancelled result went with the edited message.
    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 2);
    assert!(matches!(&entries[0].body, EntryBody::User { text, .. } if *text == manual("edited")));
    assert!(
        !second
            .entries()
            .iter()
            .any(|e| matches!(e, AiEntryView::Tool { .. }))
    );
}
