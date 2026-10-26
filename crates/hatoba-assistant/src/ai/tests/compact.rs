//! Compaction (AI-21, AI-22): the Compact request and the prefix it shares, compaction before a
//! message, between tool calls and at a retry, and messages stopped or replaced meanwhile.

use super::*;

#[tokio::test]
async fn compaction_moves_the_context_start_to_the_summary() {
    let f = Fixture::new(vec![
        answer("First answer."),
        answer("SUMMARY: the user checked the disk."),
        answer("Next answer."),
    ])
    .await;
    let (started, sink) = f.send(None, "first question").await;
    let conv = started.conversation.id;
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    // What the model saw of files (AI-38…40) is in the entries the summary replaces, and no
    // approval card waits on a read once the context is summarized.
    remember_a_file(&f, &conv, "call_x");

    let summary = f
        .manager
        .compact(&f.vault, f.env.as_ref(), &conv, &f.context(), None)
        .await
        .unwrap();
    assert!(!f.manager.0.seen.knows(&conv));
    assert!(!holds_a_file_base(&f, &conv));
    let AiEntryView::Summary { entry_id, text, .. } = &summary else {
        panic!("expected a summary, got {summary:?}");
    };
    assert_eq!(text, &manual("SUMMARY: the user checked the disk."));
    let detail = f.detail(&conv);
    assert_eq!(detail.conversation.context_start.as_ref(), Some(entry_id));
    assert_eq!(detail.entries.len(), 3);

    let requests = f.requests().await;
    let compact = &requests[1];
    // The request a turn would send, with its tools and system prompt.
    assert_eq!(tool_names(compact), tool_names(&requests[0]));
    assert_eq!(system_of(compact), system_of(&requests[0]));
    let last = messages(compact).last().unwrap();
    assert_eq!(last["role"], "user");
    assert_eq!(last["content"], COMPACT_INSTRUCTION);
    assert!(compact.to_string().contains("first question"));

    let (_, sink) = f.send(Some(&conv), "next question").await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    let requests = f.requests().await;
    let next = requests[2].to_string();
    assert!(!next.contains("first question"));
    assert!(next.contains("This conversation continues from a summary"));
    assert!(next.contains("<summary>\\nSUMMARY: the user checked the disk.\\n</summary>"));
    assert!(next.contains("next question"));
}

/// AI-21: a Compact request is the request the next turn would send with the instruction after
/// it: the same system prompt, built-in and MCP tools (without the server the conversation
/// switched off) and messages, `raw` replayed, and no `tool_choice`. So the provider's prompt
/// cache serves everything before the instruction.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_compact_request_is_the_next_request_with_the_instruction_after_it() {
    use crate::mcp::tests::{put_server, set_enabled, stdio_server};

    let f = Fixture::new(vec![
        calls(&[("c1", "read_terminal", json!({}))]),
        answer("The disk is fine."),
        answer("SUMMARY: the disk is fine."),
    ])
    .await;
    let files = put_server(&f.vault, stdio_server("files"));
    let other = put_server(&f.vault, stdio_server("other"));
    set_enabled(&f.vault, &files, true);
    set_enabled(&f.vault, &other, true);
    let context = AiTurnContext {
        disabled_mcp_servers: vec![other],
        ..f.context()
    };
    let (started, sink) = f.send_with(None, "check the disk", context.clone()).await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    f.result(&conv, "c1", AiToolStatus::Ok, "Use% 42%", None)
        .await
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;

    f.manager
        .compact(&f.vault, f.env.as_ref(), &conv, &context, None)
        .await
        .unwrap();
    let requests = f.requests().await;
    assert_eq!(requests.len(), 3);
    let (last_turn, compact) = (&requests[1], &requests[2]);
    let names = tool_names(compact);
    assert!(names.contains(&"mcp__files__echo".to_owned()), "{names:?}");
    assert!(
        !names.iter().any(|n| n.starts_with("mcp__other__")),
        "{names:?}"
    );
    assert!(compact.get("tool_choice").is_none());
    assert_eq!(without_messages(compact), without_messages(last_turn));
    let (before, after) = messages(compact).split_at(messages(last_turn).len());
    assert_eq!(before, &messages(last_turn)[..]);
    // The answer, replayed from `raw`, then the instruction.
    assert_eq!(after.len(), 2);
    assert_eq!(after[0]["content"], "The disk is fine.");
    assert_eq!(after[0]["provider_note"], "RAW-ONLY");
    assert_eq!(
        after[1],
        json!({"role": "user", "content": COMPACT_INSTRUCTION})
    );
    f.mcp.stop_all(&f.vault).await;
}

/// AI-21 on Anthropic: the Compact request's body is the next request's with the instruction
/// after its messages. Only `max_tokens` differs, which is not part of the cached prefix: 16,000
/// instead of the model's whole output limit, so that it fits beside a nearly full context.
#[tokio::test]
async fn an_anthropic_compact_request_keeps_the_prefix_and_asks_for_less_output() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(Script(Mutex::new(
            vec![
                anthropic_answer("First answer."),
                anthropic_answer("Second answer."),
                anthropic_answer("SUMMARY: two questions."),
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
        provider.models[0].context_window = Some(200_000);
        provider.models[0].max_output_tokens = Some(64_000);
        v.put(Some(&f.provider_id), Item::AiProvider(provider))
            .unwrap();
    }
    let (conv, _) = f.exchange(None, "first question", f.context()).await;
    f.exchange(Some(&conv), "second question", f.context())
        .await;
    let summary = f
        .manager
        .compact(&f.vault, f.env.as_ref(), &conv, &f.context(), None)
        .await
        .unwrap();
    assert!(matches!(summary, AiEntryView::Summary { .. }));

    let requests = f.requests().await;
    let (last_turn, compact) = (&requests[1], &requests[2]);
    assert_eq!(last_turn["max_tokens"], 64_000);
    assert_eq!(compact["max_tokens"], 16_000);
    let rest = |body: &Value| {
        let mut body = without_messages(body);
        body.as_object_mut().unwrap().remove("max_tokens");
        body
    };
    assert_eq!(rest(compact), rest(last_turn));
    assert!(!compact["tools"].as_array().unwrap().is_empty());
    let (before, after) = messages(compact).split_at(messages(last_turn).len());
    assert_eq!(before, &messages(last_turn)[..]);
    assert_eq!(
        after,
        [
            json!({"role": "assistant", "content": [{"type": "text", "text": "Second answer."}]}),
            json!({"role": "user", "content": [{"type": "text", "text": COMPACT_INSTRUCTION}]}),
        ]
    );
}

/// AI-22 on Anthropic: the compaction between tool calls asks for the output of a Compact
/// request too, 9% of the window here, not the model's whole limit as the turn's requests do.
#[tokio::test]
async fn an_anthropic_compaction_between_tool_calls_asks_for_less_output() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(Script(Mutex::new(
            vec![
                anthropic_call("c1", "read_terminal"),
                anthropic_answer("SUMMARY: the screen."),
                anthropic_answer("Done."),
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
        provider.models[0].context_window = Some(1_000);
        provider.models[0].max_output_tokens = Some(64_000);
        v.put(Some(&f.provider_id), Item::AiProvider(provider))
            .unwrap();
    }
    let (started, sink) = f.send(None, "look at the screen").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    // The screen alone is about 1,000 tokens.
    f.result(&conv, "c1", AiToolStatus::Ok, &"z".repeat(4_000), None)
        .await
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    assert_eq!(summaries(&sink).len(), 1);

    let requests = f.requests().await;
    let max_tokens: Vec<&Value> = requests.iter().map(|r| &r["max_tokens"]).collect();
    assert_eq!(max_tokens, [&json!(64_000), &json!(90), &json!(64_000)]);
}

/// AI-21: an answer to a Compact request that calls tools, or has no text, is dropped (nothing
/// runs and nothing is stored) and the request goes once more without tools, with the system
/// prompt that says why. Only once: when that answer is no summary either (an endpoint that
/// still calls tools, its text only the preamble), the compaction fails.
#[tokio::test]
async fn a_compact_answer_without_a_summary_is_asked_again_without_tools() {
    let f = Fixture::new(vec![
        answer("First answer."),
        answer("   "),
        // "Checking." and a call, from a request without tools.
        calls(&[("x0", "read_terminal", json!({}))]),
        calls(&[("x1", "run_command", json!({"command": "rm -rf /"}))]),
        answer("SUMMARY from the request without tools."),
    ])
    .await;
    let (started, sink) = f.send(None, "first question").await;
    let conv = started.conversation.id;
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;

    let err = f
        .manager
        .compact(&f.vault, f.env.as_ref(), &conv, &f.context(), None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Ai);
    assert_eq!(err.detail, "the model returned no summary");
    assert_eq!(f.requests().await.len(), 3);
    assert_eq!(f.entries(&conv).len(), 2);

    let summary = f
        .manager
        .compact(&f.vault, f.env.as_ref(), &conv, &f.context(), None)
        .await
        .unwrap();
    let AiEntryView::Summary { text, .. } = &summary else {
        panic!("expected a summary, got {summary:?}");
    };
    assert_eq!(text, &manual("SUMMARY from the request without tools."));
    let requests = f.requests().await;
    assert_eq!(requests.len(), 5);
    for (with_tools, plain) in [(&requests[1], &requests[2]), (&requests[3], &requests[4])] {
        assert!(!tool_names(with_tools).is_empty());
        assert!(plain.get("tools").is_none());
        assert!(
            system_of(plain).contains("only asks for a summary of the conversation"),
            "{}",
            system_of(plain)
        );
        assert_eq!(messages(plain)[1..], messages(with_tools)[1..]);
        assert_eq!(
            messages(plain).last().unwrap()["content"],
            COMPACT_INSTRUCTION
        );
    }
    // The dropped calls never ran and were never stored.
    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 3);
    assert!(matches!(&entries[2].body, EntryBody::Summary { .. }));
}

#[test]
fn a_compact_request_asks_for_output_that_fits_beside_the_context() {
    use hatoba_ai::provider::ModelSpec;

    for (window, output, expected) in [
        (Some(200_000), Some(64_000), 16_000),
        (Some(128_000), Some(16_384), 11_520),
        (Some(1_000_000), Some(8_000), 8_000),
        (None, Some(4_096), 4_096),
        (None, None, 16_000),
        (Some(1_000), None, 90),
    ] {
        let model = ModelSpec {
            context_window: window,
            max_output_tokens: output,
            ..ModelSpec::new("m1")
        };
        assert_eq!(
            crate::ai::compact::compact_output_tokens(&model),
            expected,
            "window {window:?}, output {output:?}"
        );
    }
}

#[tokio::test]
async fn compaction_is_refused_while_a_turn_runs() {
    let f = Fixture::new(vec![calls(&[("call_1", "read_terminal", json!({}))])]).await;
    let (started, sink) = f.send(None, "look").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let err = f
        .manager
        .compact(&f.vault, f.env.as_ref(), &conv, &f.context(), None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
}

#[tokio::test]
async fn a_new_message_that_would_fill_the_context_is_sent_after_a_compaction() {
    let f = Fixture::new(vec![
        answer("First answer."),
        answer("SUMMARY: the user asked a first question."),
        answer("Second answer."),
    ])
    .await;
    // 90% of 1,000 tokens: the first answer used 105, and the long message adds 900 more.
    f.set_context_window(1_000);
    let (started, sink) = f.send(None, "first question").await;
    let conv = started.conversation.id;
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    assert_eq!(f.requests().await.len(), 1);

    let long = "disk ".repeat(720);
    let (started, sink) = f.send(Some(&conv), &long).await;
    // The summary was stored and sent on the turn's channel before the message was stored.
    let summary = summaries(&sink);
    assert_eq!(summary.len(), 1);
    let summary_id = entry_id(&summary[0]).to_owned();
    assert!(entry_id(&started.user_entry) > summary_id.as_str());
    assert_eq!(
        started.conversation.context_start.as_deref(),
        Some(summary_id.as_str())
    );
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;

    let requests = f.requests().await;
    assert_eq!(requests.len(), 3);
    let compact = &requests[1];
    assert_eq!(tool_names(compact), tool_names(&requests[0]));
    assert_eq!(
        messages(compact).last().unwrap()["content"],
        COMPACT_INSTRUCTION
    );
    assert!(compact.to_string().contains("first question"));
    assert!(!compact.to_string().contains("disk disk"));
    // The request carries the summary and the new message, not what came before.
    let next = requests[2].to_string();
    assert!(next.contains("SUMMARY: the user asked a first question."));
    assert!(next.contains("disk disk"));
    assert!(!next.contains("First answer."));
    let entries = f.entries(&conv);
    assert!(matches!(&entries[2].body, EntryBody::Summary { .. }));
    assert!(matches!(&entries[3].body, EntryBody::User { text, .. } if *text == long));
}

#[tokio::test]
async fn a_full_context_between_tool_calls_is_compacted_once() {
    let long_summary = format!("SUMMARY-2 {}", "y".repeat(4_000));
    let f = Fixture::new(vec![
        calls(&[("c1", "read_terminal", json!({}))]),
        answer(&long_summary),
        answer("Done."),
    ])
    .await;
    f.set_context_window(1_000);
    let (started, sink) = f.send(None, "look at the screen").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    remember_a_file(&f, &conv, "c1");
    // The screen alone is about 1,000 tokens.
    f.result(&conv, "c1", AiToolStatus::Ok, &"z".repeat(4_000), None)
        .await
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    // AI-22 forgets what the model saw of files, as Compact does (AI-38…40), and the next request
    // drops what approval cards read, since no call waits for one.
    assert!(!f.manager.0.seen.knows(&conv));
    assert!(!holds_a_file_base(&f, &conv));

    let requests = f.requests().await;
    assert_eq!(requests.len(), 3);
    // The request the turn was about to send, asking where the task stands.
    let compact = &requests[1];
    assert_eq!(tool_names(compact), tool_names(&requests[0]));
    let roles: Vec<&Value> = messages(compact).iter().map(|m| &m["role"]).collect();
    assert_eq!(roles, ["system", "user", "assistant", "tool", "user"]);
    assert_eq!(
        messages(compact).last().unwrap()["content"],
        COMPACT_MID_TURN_INSTRUCTION
    );
    // The summary is over 90% too, but it is never compacted again right away.
    let next = requests[2].to_string();
    assert!(next.contains("SUMMARY-2"));
    assert!(!next.contains("look at the screen"));
    assert!(!tool_names(&requests[2]).is_empty());
    let summary = summaries(&sink);
    assert_eq!(summary.len(), 1);
    assert_eq!(
        f.context_start(&conv).as_deref(),
        Some(entry_id(&summary[0]))
    );
}

#[tokio::test]
async fn a_message_stopped_while_the_context_is_compacted_is_not_stored() {
    let f = Fixture::new(vec![
        answer("First answer."),
        answer("SUMMARY").set_delay(Duration::from_secs(30)),
    ])
    .await;
    let conv = f.nearly_full().await;

    let (send, sink) = spawn_send(&f, &conv, &"disk ".repeat(720));
    // The Compact request is on its way when the user stops.
    f.wait_for_requests(2).await;
    f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
    let err = send.await.unwrap().unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled);
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert!(sink.entries().is_empty());
    // Neither the message nor a summary was stored, and nothing runs.
    assert_eq!(f.entries(&conv).len(), 2);
    assert!(!f.manager.is_running(&conv));
    assert_eq!(f.context_start(&conv), None);
}

#[tokio::test]
async fn a_move_stopped_while_the_context_is_compacted_is_not_stored() {
    use hatoba_ai::tools::host_change_block;

    let f = Fixture::new(vec![
        answer("First answer."),
        answer("SUMMARY").set_delay(Duration::from_secs(30)),
        answer("Short answer."),
    ])
    .await;
    f.set_context_window(1_000);
    let (conv, _) = f
        .exchange(None, "first question", f.quick("10.0.0.9"))
        .await;

    // AI-09: the move to another target goes with the message, which the stop keeps from being
    // stored, and so does the thinking level (AI-05).
    let elsewhere = AiTurnContext {
        effort: Some(AiEffort::High),
        ..f.quick("10.0.0.10")
    };
    let (send, sink) = spawn_send_with(&f, &conv, &"disk ".repeat(720), elsewhere);
    f.wait_for_requests(2).await;
    f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
    assert_eq!(send.await.unwrap().unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    let first = "root@10.0.0.9:22";
    assert_eq!(f.recorded_place(&conv), (None, Some(first.to_owned())));
    assert_eq!(f.detail(&conv).conversation.effort, None);

    // So the next message from that target still notes the move.
    f.exchange(Some(&conv), "short", f.quick("10.0.0.10")).await;
    assert_eq!(
        f.user_texts(&conv),
        [
            manual("first question"),
            format!("{}short", host_change_block(first, "root@10.0.0.10:22"))
        ]
    );
    assert_eq!(
        f.recorded_place(&conv),
        (None, Some("root@10.0.0.10:22".to_owned()))
    );
}

#[tokio::test]
async fn a_message_replaced_while_the_context_is_compacted_is_not_stored() {
    let f = Fixture::new(vec![
        answer("First answer."),
        answer("SUMMARY").set_delay(Duration::from_secs(30)),
        answer("Short answer."),
    ])
    .await;
    let conv = f.nearly_full().await;

    let (send, first) = spawn_send(&f, &conv, &"disk ".repeat(720));
    f.wait_for_requests(2).await;
    // A newer message stops the turn that is compacting; only the newer one is stored.
    let (again, second) = f.send(Some(&conv), "short").await;
    assert_eq!(again.conversation.id, conv);
    assert_eq!(send.await.unwrap().unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(first.ended().await, AiTurnEndReason::Stopped);
    assert_eq!(second.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 4);
    assert!(matches!(&entries[2].body, EntryBody::User { text, .. } if text == "short"));
    assert!(matches!(&entries[3].body, EntryBody::Assistant(a) if a.text == "Short answer."));
}

#[tokio::test]
async fn a_retry_compacts_a_context_that_would_fill_the_window_and_only_once() {
    let f = Fixture::new(vec![
        ResponseTemplate::new(400)
            .set_body_json(json!({"error": {"message": "prompt is too long"}})),
        answer("SUMMARY: the user asked about the disk."),
        ResponseTemplate::new(503).set_body_json(json!({"error": {"message": "overloaded"}})),
        answer("Recovered."),
    ])
    .await;
    // Sent while the model's context window was unknown, so nothing was compacted.
    let long = "disk ".repeat(740);
    let (started, sink) = f.send(None, &long).await;
    let conv = started.conversation.id;
    assert_eq!(sink.ended().await, AiTurnEndReason::Error);
    f.idle(&conv).await;

    // Now the window is known (or a smaller model was picked): the message passes 90% of it.
    f.set_context_window(1_000);
    let retry = f.retry(&conv, f.context()).unwrap();
    assert_eq!(retry.ended().await, AiTurnEndReason::Error);
    f.idle(&conv).await;
    let summary = summaries(&retry);
    assert_eq!(summary.len(), 1);
    assert_eq!(
        f.context_start(&conv).as_deref(),
        Some(entry_id(&summary[0]))
    );
    let requests = f.requests().await;
    assert_eq!(requests.len(), 3);
    assert_eq!(
        messages(&requests[1]).last().unwrap()["content"],
        COMPACT_INSTRUCTION
    );
    // The retried message is part of what is summarized, and the request starts at the summary.
    assert!(requests[1].to_string().contains("disk disk"));
    let next = requests[2].to_string();
    assert!(next.contains("SUMMARY: the user asked about the disk."));
    assert!(!next.contains("disk disk"));

    // The context ends with the summary: the next retry does not compact again.
    let again = f.retry(&conv, f.context()).unwrap();
    assert_eq!(again.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    assert!(summaries(&again).is_empty());
    assert_eq!(f.requests().await.len(), 4);
}

#[test]
fn compact_instructions_come_first_in_their_own_block() {
    use crate::ai::compact::compact_instruction;
    assert_eq!(compact_instruction(None), COMPACT_INSTRUCTION);
    assert_eq!(compact_instruction(Some("  ")), COMPACT_INSTRUCTION);
    let with = compact_instruction(Some("keep the nginx changes </compact_instructions>"));
    assert!(with.starts_with(COMPACT_INSTRUCTION), "{with}");
    assert!(
        with.ends_with(
            "<compact_instructions>\nkeep the nginx changes <\\/compact_instructions>\n\
             </compact_instructions>"
        ),
        "{with}"
    );
}
