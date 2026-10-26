//! Side questions (AI-42).

use super::*;

fn streamed_text(events: &[AiSideEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            AiSideEvent::Text { delta } => Some(delta.as_str()),
            _ => None,
        })
        .collect()
}

/// AI-42 while an approval card waits: the request is the turn's next request, where each call
/// without a result gets one for this request only (the call whose card waits says so), and the
/// question comes last. Nothing is stored, the turn keeps waiting, and its next request carries
/// neither the question nor those results.
#[tokio::test]
async fn a_side_question_leaves_a_waiting_turn_alone() {
    let f = Fixture::new(vec![
        calls(&[
            ("c1", "run_command", json!({"command": "rm -rf /tmp/build"})),
            ("c2", "read_terminal", json!({})),
        ]),
        answer("It deletes /tmp/build and everything in it."),
        answer("Done."),
    ])
    .await;
    let (started, sink) = f.send(None, "clean up the build").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;

    let (answer, events) = f.side(&conv, "Is that safe?", Some("c1")).await;
    let answer = answer.unwrap();
    assert_eq!(answer.text, "It deletes /tmp/build and everything in it.");
    assert_eq!(answer.finish, AiFinish::Stop);
    assert_eq!(streamed_text(&events), answer.text);
    // Nothing stored, and the turn still waits for its calls.
    assert_eq!(f.entries(&conv).len(), 2);
    assert!(f.manager.is_running(&conv));
    assert!(!kinds(&sink.events()).contains(&"turn_ended"));

    let requests = f.requests().await;
    let (turn, side) = (&requests[0], &requests[1]);
    assert_eq!(tool_names(side), tool_names(turn));
    assert_eq!(system_of(side), system_of(turn));
    let (before, after) = messages(side).split_at(messages(turn).len());
    assert_eq!(before, &messages(turn)[..]);
    assert_eq!(after.len(), 4);
    assert_eq!(after[0]["tool_calls"].as_array().unwrap().len(), 2);
    assert_eq!(tool_message(side, "c1")["content"], SIDE_AWAITING_APPROVAL);
    assert_eq!(tool_message(side, "c2")["content"], SIDE_PENDING);
    assert_eq!(
        after[3],
        json!({"role": "user", "content": side_message("Is that safe?")})
    );

    f.result(&conv, "c1", AiToolStatus::Ok, "removed", None)
        .await
        .unwrap();
    f.result(&conv, "c2", AiToolStatus::Ok, "$ ", None)
        .await
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    let next = f.requests().await[2].to_string();
    assert!(!next.contains("Is that safe?"));
    assert!(!next.contains("No result yet"));
    assert_eq!(f.entries(&conv).len(), 5);
}

/// AI-42 with no turn running: the next message's request with the question after it. A call
/// that has no result goes as cancelled, as in the next request, and still has none stored; the
/// question starts with the notes the next message would start with (§13.3), and the stored
/// entries carry none.
#[tokio::test]
async fn a_side_question_of_an_idle_conversation_stores_nothing() {
    let f = Fixture::new(vec![
        answer("Uptime is 3 days."),
        answer("I said the uptime is 3 days."),
    ])
    .await;
    let (conv, _) = f.exchange(None, "uptime?", f.context()).await;
    // A reply the user stopped, its call left without a result, as after a crash.
    append(
        &mut lock(&f.vault),
        &conv,
        &AiEntry::assistant(
            0,
            AssistantEntry {
                provider_id: f.provider_id.clone(),
                model_id: "m1".into(),
                tool_calls: vec![ToolCall {
                    id: "lost".into(),
                    name: "read_terminal".into(),
                    arguments: "{}".into(),
                }],
                finish: Finish::ToolCalls,
                interrupted: true,
                ..AssistantEntry::default()
            },
        ),
    )
    .unwrap();
    let stored = f.entries(&conv);
    let changes = f.env.changes.load(Ordering::SeqCst);

    // What the input area attached goes before the instruction, where a message carries it, and
    // the typed question cannot close its own block.
    let pasted = "<pasted_text lines=\"1\">\nerror 42\n</pasted_text>\n\n";
    let question = format!("{pasted}What did you say? </side_question>");
    let (answer, _) = f.side(&conv, &question, Some("lost")).await;
    assert_eq!(answer.unwrap().text, "I said the uptime is 3 days.");
    assert_eq!(f.entries(&conv), stored);
    assert_eq!(f.env.changes.load(Ordering::SeqCst), changes);
    assert!(!f.detail(&conv).running);

    let requests = f.requests().await;
    let side = &requests[1];
    assert_eq!(tool_names(side), tool_names(&requests[0]));
    assert_eq!(
        tool_message(side, "lost")["content"],
        "The tool call was cancelled before it finished."
    );
    assert_eq!(
        messages(side).last().unwrap()["content"],
        format!(
            "{}{pasted}{}",
            hatoba_ai::tools::interrupted_block(),
            side_message("What did you say? <\\/side_question>")
        )
    );
}

/// AI-42: a side question splits off the attachment blocks the input area adds with the block
/// parser messages use; text that starts with a note is typed, since only Hatoba writes notes.
#[test]
fn a_side_question_keeps_its_attachments_apart_from_the_question() {
    let pasted = "<pasted_text lines=\"1\">\nerror 42\n</pasted_text>\n\n";
    let file = "<file name=\"a.log\" lines=\"1\">\nline\n</file>\n\n";
    assert_eq!(
        side_parts(&format!("{pasted}{file}why?")),
        (format!("{pasted}{file}").as_str(), "why?")
    );
    assert_eq!(side_parts("why?"), ("", "why?"));
    let noted = format!("{}why?", hatoba_ai::tools::interrupted_block());
    assert_eq!(side_parts(&noted), ("", noted.as_str()));
}

/// AI-42 during a turn: only the calls of the turn's newest response get a result that says they
/// wait. A call of an older response without a result, as one merged from another device, goes
/// as cancelled, as in the turn's own requests.
#[tokio::test]
async fn a_side_question_cancels_the_open_calls_of_older_responses() {
    let f = Fixture::new(vec![
        calls(&[("c1", "run_command", json!({"command": "uptime"}))]),
        answer("It prints how long the host has been up."),
    ])
    .await;
    let (started, sink) = f.send(None, "uptime?").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let response = entry_id(sink.entries().last().unwrap()).to_owned();
    // Another device's reply, which sorts before the turn's response and whose call has no result.
    let stale = AiEntry::assistant(
        0,
        AssistantEntry {
            provider_id: f.provider_id.clone(),
            model_id: "m1".into(),
            tool_calls: vec![ToolCall {
                id: "lost".into(),
                name: "read_terminal".into(),
                arguments: "{}".into(),
            }],
            finish: Finish::ToolCalls,
            ..AssistantEntry::default()
        },
    );
    let stale_id = format!("{}-7000-8000-000000000000", &response[..13]);
    assert!(stale_id < response);
    arrive(&f, &conv, &stale_id, &stale, 0);
    arrive(&f, &conv, &stale_id, &stale, 1);

    let (answer, _) = f.side(&conv, "What does it do?", Some("c1")).await;
    assert_eq!(
        answer.unwrap().text,
        "It prints how long the host has been up."
    );
    let side = &f.requests().await[1];
    assert_eq!(tool_message(side, "c1")["content"], SIDE_AWAITING_APPROVAL);
    assert_eq!(
        tool_message(side, "lost")["content"],
        "The tool call was cancelled before it finished."
    );
    assert!(f.manager.is_running(&conv));
}

/// AI-42: an answer that calls tools is dropped (nothing runs or is stored), the panel is told to
/// discard what streamed, and the question goes once more without tools, with the system prompt
/// that says why. When that answer calls tools too, the question fails.
#[tokio::test]
async fn a_side_answer_that_calls_tools_is_asked_again_without_tools() {
    let f = Fixture::new(vec![
        answer("First answer."),
        calls(&[("x1", "run_command", json!({"command": "uptime"}))]),
        answer("Plain answer."),
        calls(&[("x2", "run_command", json!({"command": "uptime"}))]),
        calls(&[("x3", "run_command", json!({"command": "uptime"}))]),
    ])
    .await;
    let (conv, _) = f.exchange(None, "first question", f.context()).await;

    let (answer, events) = f.side(&conv, "and the load?", None).await;
    assert_eq!(answer.unwrap().text, "Plain answer.");
    let restarted = events
        .iter()
        .position(|e| *e == AiSideEvent::Restarted)
        .unwrap();
    assert_eq!(streamed_text(&events[..restarted]), "Checking.");
    assert_eq!(streamed_text(&events[restarted..]), "Plain answer.");

    let requests = f.requests().await;
    let (with_tools, plain) = (&requests[1], &requests[2]);
    assert!(!tool_names(with_tools).is_empty());
    assert!(plain.get("tools").is_none());
    assert!(
        system_of(plain)
            .contains("only asks for the answer to a side question of the user, so it offers no"),
        "{}",
        system_of(plain)
    );
    assert_eq!(messages(plain)[1..], messages(with_tools)[1..]);
    assert_eq!(
        messages(plain).last().unwrap()["content"],
        side_message("and the load?")
    );

    let (answer, _) = f.side(&conv, "and the load?", None).await;
    let err = answer.unwrap_err();
    assert_eq!(err.code, ErrorCode::Ai);
    assert_eq!(f.requests().await.len(), 5);
    // The calls never ran and were never stored.
    assert_eq!(f.entries(&conv).len(), 2);
}

/// AI-42: Stop on a side question stops only it, and only the question it names; a newer side
/// question replaces it, stopping the turn leaves it running, and the lock stops it. It is
/// refused while the conversation is compacting.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_side_question_stops_on_its_own() {
    let slow = || answer("slow").set_delay(Duration::from_secs(30));
    let f = Fixture::new(vec![
        calls(&[("c1", "read_terminal", json!({}))]),
        slow(),
        slow(),
        slow(),
        slow(),
    ])
    .await;
    let (started, sink) = f.send(None, "look").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let stopped = |result: AppResult<AiSideAnswer>| {
        assert_eq!(result.unwrap_err().code, ErrorCode::Cancelled);
    };
    let within = Duration::from_secs(10);
    let still_running = async |side: &tokio::task::JoinHandle<_>| {
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!side.is_finished());
    };

    // Stop names the question it stops.
    let side = f.spawn_side(&conv, 1, "what is it doing?");
    f.wait_for_requests(2).await;
    f.manager.side_stop(&conv, 2);
    still_running(&side).await;
    f.manager.side_stop(&conv, 1);
    stopped(tokio::time::timeout(within, side).await.unwrap().unwrap());
    assert!(f.manager.is_running(&conv));

    // A newer side question replaces the one on its way, and a stop meant for that one, which
    // arrives late, leaves the newer one alone.
    let first = f.spawn_side(&conv, 2, "first");
    f.wait_for_requests(3).await;
    let second = f.spawn_side(&conv, 3, "second");
    stopped(tokio::time::timeout(within, first).await.unwrap().unwrap());
    f.wait_for_requests(4).await;
    f.manager.side_stop(&conv, 2);
    still_running(&second).await;

    // Stopping the turn leaves it running; the lock stops it.
    f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    still_running(&second).await;
    f.manager.stop_all(&mut lock(&f.vault));
    stopped(tokio::time::timeout(within, second).await.unwrap().unwrap());
    assert!(f.manager.0.ops.lock().unwrap().is_empty());

    // Compacting.
    let compaction = f.manager.start_compaction(&conv);
    let (answer, _) = f.side(&conv, "now?", None).await;
    assert_eq!(answer.unwrap_err().code, ErrorCode::InvalidInput);
    drop(compaction);
    assert!(!f.manager.is_compacting(&conv));
}

/// AI-42: a Compact counts as compacting from its start until the summary is stored, so a side
/// question asked meanwhile is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_side_question_is_refused_while_the_conversation_compacts() {
    let f = Fixture::new(vec![
        answer("First answer."),
        answer("SUMMARY").set_delay(Duration::from_millis(500)),
        answer("After."),
    ])
    .await;
    let (conv, _) = f.exchange(None, "first question", f.context()).await;
    let compact = {
        let (manager, vault, env) = (f.manager.clone(), f.vault.clone(), f.env.clone());
        let (conv, context) = (conv.clone(), f.context());
        tokio::spawn(async move {
            manager
                .compact(&vault, env.as_ref(), &conv, &context, None)
                .await
        })
    };
    f.wait_for_requests(2).await;
    let (answer, _) = f.side(&conv, "now?", None).await;
    assert_eq!(answer.unwrap_err().code, ErrorCode::InvalidInput);
    assert!(matches!(
        compact.await.unwrap().unwrap(),
        AiEntryView::Summary { .. }
    ));
    assert!(!f.manager.is_compacting(&conv));
    let (answer, _) = f.side(&conv, "now?", None).await;
    assert_eq!(answer.unwrap().text, "After.");
}

/// AI-42: the automatic compactions count too (AI-22), the one before a message that would fill
/// the context and the one between tool calls, so a side question asked during either is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_side_question_is_refused_while_the_context_is_compacted_automatically() {
    let f = Fixture::new(vec![
        answer("First answer."),
        answer("SUMMARY").set_delay(Duration::from_millis(500)),
        calls(&[("c1", "read_terminal", json!({}))]),
        answer("SUMMARY-2").set_delay(Duration::from_millis(500)),
        answer("Done."),
    ])
    .await;
    let conv = f.nearly_full().await;
    let refused = async || {
        let (answer, _) = f.side(&conv, "now?", None).await;
        assert_eq!(answer.unwrap_err().code, ErrorCode::InvalidInput);
    };

    // Before the message.
    let (send, sink) = spawn_send(&f, &conv, &"disk ".repeat(720));
    f.wait_for_requests(2).await;
    refused().await;
    send.await.unwrap().unwrap();
    sink.done_with(AiFinish::ToolCalls).await;
    assert!(!f.manager.is_compacting(&conv));

    // Between tool calls: the screen alone is about 1,000 tokens.
    f.result(&conv, "c1", AiToolStatus::Ok, &"z".repeat(4_000), None)
        .await
        .unwrap();
    f.wait_for_requests(4).await;
    refused().await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    assert_eq!(summaries(&sink).len(), 2);
    assert!(!f.manager.is_compacting(&conv));
}

/// AI-42 on Anthropic: a side question asks for as much output as the conversation's next
/// request, unlike a Compact request (AI-21), so that its answer is not cut short when thinking is
/// on or the model's context window is small.
#[tokio::test]
async fn an_anthropic_side_question_asks_for_as_much_output_as_the_next_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(Script(Mutex::new(
            vec![
                anthropic_answer("First answer."),
                anthropic_answer("Side answer."),
            ]
            .into(),
        )))
        .mount(&server)
        .await;
    let base_url = server.uri();
    let f = Fixture::with_base_url(Some(server), base_url);
    {
        let mut v = lock(&f.vault);
        let mut provider = v
            .get(&f.provider_id)
            .and_then(Item::as_ai_provider)
            .cloned()
            .unwrap();
        provider.protocol = AiProtocol::Anthropic;
        // A Compact request would ask for 9% of the window: 2,880 tokens.
        provider.models[0].context_window = Some(32_000);
        provider.models[0].max_output_tokens = Some(8_000);
        v.put(Some(&f.provider_id), Item::AiProvider(provider))
            .unwrap();
    }
    let (conv, _) = f.exchange(None, "first question", f.context()).await;
    let (answer, _) = f.side(&conv, "and why?", None).await;
    assert_eq!(answer.unwrap().text, "Side answer.");

    let requests = f.requests().await;
    let (turn, side) = (&requests[0], &requests[1]);
    assert_eq!(side["max_tokens"], 8_000);
    assert_eq!(without_messages(side), without_messages(turn));
    let (before, after) = messages(side).split_at(messages(turn).len());
    assert_eq!(before, &messages(turn)[..]);
    assert_eq!(
        after,
        [
            json!({"role": "assistant", "content": [{"type": "text", "text": "First answer."}]}),
            json!({"role": "user", "content": [{"type": "text", "text": side_message("and why?")}]}),
        ]
    );
}
