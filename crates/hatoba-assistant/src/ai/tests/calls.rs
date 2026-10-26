//! Tool calls in turns: results stored in the order of the calls, edits and rejections, calls
//! left without a result, the tools that run in Rust, the tools offered without a tab, native
//! search (AI-14), MCP tools (AI-30), and calls that share an id.

use super::*;

#[tokio::test]
async fn a_tool_call_waits_for_its_result_and_the_next_request_carries_it() {
    let f = Fixture::new(vec![
        calls(&[("call_1", "read_terminal", json!({"lines": 50}))]),
        answer("Disk usage is fine."),
    ])
    .await;
    let (started, sink) = f.send(None, "  check the disk\nplease").await;
    let conv = started.conversation.id.clone();
    assert_eq!(started.conversation.title, "check the disk");
    assert!(
        matches!(&started.user_entry, AiEntryView::User { text, .. } if *text == manual("  check the disk\nplease"))
    );

    sink.done_with(AiFinish::ToolCalls).await;
    assert!(f.manager.is_running(&conv));
    assert!(f.detail(&conv).running);
    // Nothing goes out until the call has a result.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(f.requests().await.len(), 1);

    let stored = f
        .result(&conv, "call_1", AiToolStatus::Ok, "$ df -h\n42%", None)
        .await
        .unwrap();
    assert_eq!(
        view_content(&stored),
        ("call_1", AiToolStatus::Ok, "$ df -h\n42%")
    );
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;

    let events = sink.events();
    let milestones: Vec<&str> = kinds(&events)
        .into_iter()
        .filter(|k| matches!(*k, "request_started" | "entry" | "done" | "turn_ended"))
        .collect();
    assert_eq!(
        milestones,
        [
            "request_started",
            "entry",
            "done",
            "entry",
            "request_started",
            "entry",
            "done",
            "turn_ended"
        ]
    );
    assert!(events.contains(&AiTurnEvent::ToolCall {
        id: "call_1".into(),
        name: "read_terminal".into(),
        arguments: r#"{"lines":50}"#.into(),
    }));
    assert!(events.contains(&AiTurnEvent::Usage {
        input_tokens: 90,
        output_tokens: 12,
        estimated: false,
    }));
    assert!(events.contains(&AiTurnEvent::Text {
        delta: "Disk usage is fine.".into()
    }));

    let requests = f.requests().await;
    assert_eq!(requests.len(), 2);
    let first = &requests[0];
    assert_eq!(first["model"], "m1");
    assert_eq!(messages(first)[0]["role"], "system");
    assert!(
        messages(first)[0]["content"]
            .as_str()
            .unwrap()
            .contains("Hatoba")
    );
    // No search provider: no web_search. No skill of the user's, but the built-in one is on
    // (AI-34), so read_skill is offered.
    assert_eq!(
        tool_names(first),
        [
            "read_terminal",
            "run_command",
            "send_input",
            "read_file",
            "edit_file",
            "write_file",
            "grep",
            "glob",
            "fetch_url",
            "read_skill",
            "memory"
        ]
    );
    assert_eq!(
        tool_message(&requests[1], "call_1")["content"],
        "$ df -h\n42%"
    );

    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 4);
    assert!(matches!(&entries[0].body, EntryBody::User { .. }));
    assert!(
        matches!(&entries[1].body, EntryBody::Assistant(a) if a.tool_calls.len() == 1 && a.finish == Finish::ToolCalls)
    );
    assert_eq!(tool_entry(&entries[2]).1, ToolStatus::Ok);
    assert!(matches!(&entries[3].body, EntryBody::Assistant(a) if a.text == "Disk usage is fine."));

    // The raw message is stored for replay but never reaches the WebView.
    let stored_json: String = lock(&f.vault)
        .ai_entries(&conv)
        .unwrap()
        .iter()
        .map(|e| e.json.as_str().to_owned())
        .collect();
    assert!(stored_json.contains("RAW-ONLY"));
    let detail = f.detail(&conv);
    assert!(!detail.running);
    assert_eq!(detail.entries.len(), 4);
    let shown = serde_json::to_string(&detail).unwrap();
    assert!(!shown.contains("RAW-ONLY") && !shown.contains("\"raw\""));
    assert!(!shown.contains(KEY));
    assert!(!serde_json::to_string(&events).unwrap().contains("RAW-ONLY"));

    // Results answer the newest response only.
    let again = f
        .result(&conv, "call_1", AiToolStatus::Ok, "twice", None)
        .await
        .unwrap_err();
    assert_eq!(again.code, ErrorCode::NotFound);
    assert_eq!(
        f.result(&conv, "nope", AiToolStatus::Ok, "", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert!(f.env.changes.load(Ordering::SeqCst) >= 4);
}

#[tokio::test]
async fn an_edited_call_tells_the_model_what_ran() {
    let f = Fixture::new(vec![
        calls(&[(
            "call_1",
            "send_input",
            json!({"text": "rm -rf /tmp/x", "key": "enter"}),
        )]),
        answer("Done."),
    ])
    .await;
    let (started, sink) = f.send(None, "clean up").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;

    let edited = r#"{"text":"ls /tmp/x","key":"enter"}"#;
    let stored = f
        .result(&conv, "call_1", AiToolStatus::Ok, "a b c", Some(edited))
        .await
        .unwrap();
    let (_, status, content) = view_content(&stored);
    assert_eq!(status, AiToolStatus::Ok);
    assert_eq!(content, format!("{EDIT_NOTE} {edited}\n\na b c"));
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);

    let requests = f.requests().await;
    assert_eq!(tool_message(&requests[1], "call_1")["content"], content);
}

#[tokio::test]
async fn a_rejection_sends_the_users_reason() {
    let f = Fixture::new(vec![
        calls(&[("call_1", "run_command", json!({"command": "reboot"}))]),
        answer("Understood, I will not reboot."),
    ])
    .await;
    let (started, sink) = f.send(None, "fix it").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;

    let stored = f
        .result(
            &conv,
            "call_1",
            AiToolStatus::Rejected,
            "  not on prod  ",
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        view_content(&stored),
        ("call_1", AiToolStatus::Rejected, "not on prod")
    );
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);

    let requests = f.requests().await;
    let content = tool_message(&requests[1], "call_1")["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(content.contains("rejected"), "{content}");
    assert!(content.contains("not on prod"), "{content}");
}

/// The panel runs the calls of a response that only read at once (§13.1); their results are
/// stored in the order of the calls, whichever finishes first, so the conversation reads as the
/// model made the calls.
#[tokio::test]
async fn results_are_stored_in_the_order_of_the_calls() {
    let f = Fixture::new(vec![
        calls(&[
            ("call_1", "read_terminal", json!({})),
            ("call_2", "read_skill", json!({"name": "hatoba"})),
            ("call_3", "read_terminal", json!({"lines": 5})),
        ]),
        answer("All read."),
    ])
    .await;
    let (started, sink) = f.send(None, "look around").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;

    let first = async {
        // The later calls finish first, and wait.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let kinds: Vec<&str> = f
            .entries(&conv)
            .iter()
            .map(|e| match e.body {
                EntryBody::Tool { .. } => "tool",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["other", "other"], "nothing stored before call_1");
        f.result(&conv, "call_1", AiToolStatus::Ok, "first", None)
            .await
    };
    let (third, second, first) = tokio::join!(
        f.result(&conv, "call_3", AiToolStatus::Ok, "third", None),
        f.run(&conv, "call_2", None, None),
        first,
    );
    for stored in [first, second, third] {
        stored.unwrap();
    }
    let entries = f.entries(&conv);
    let stored: Vec<&str> = entries[2..5].iter().map(|e| tool_entry(e).0).collect();
    assert_eq!(stored, ["call_1", "call_2", "call_3"]);
    let shown: Vec<String> = sink
        .entries()
        .into_iter()
        .filter_map(|e| match e {
            AiEntryView::Tool { tool_call_id, .. } => Some(tool_call_id),
            _ => None,
        })
        .collect();
    assert_eq!(shown, ["call_1", "call_2", "call_3"]);
    // read_skill's run said it ended before its result waited for call_1's, so the panel could
    // count its channel as free meanwhile.
    let events = sink.events();
    let ran = events
        .iter()
        .position(
            |e| matches!(e, AiTurnEvent::ToolRan { tool_call_id } if tool_call_id == "call_2"),
        )
        .expect("call_2 ran");
    let first = events
        .iter()
        .position(|e| matches!(e, AiTurnEvent::Entry { entry: AiEntryView::Tool { tool_call_id, .. } } if tool_call_id == "call_1"))
        .expect("call_1 stored");
    assert!(ran < first);
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
}

/// A result that waits for an earlier call's stops waiting on a stop, which stores what the tools
/// that ended returned and gives every other call without a result a cancelled one (AI-18).
#[tokio::test]
async fn stopping_ends_the_wait_of_results_for_earlier_calls() {
    let f = Fixture::new(vec![calls(&[
        ("call_1", "run_command", json!({"command": "uptime"})),
        ("call_2", "read_skill", json!({"name": "hatoba"})),
        ("call_3", "read_terminal", json!({})),
    ])])
    .await;
    let (started, sink) = f.send(None, "look around").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;

    let stop = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
    };
    let (run, result, ()) = tokio::join!(
        f.run(&conv, "call_2", None, None),
        f.result(&conv, "call_3", AiToolStatus::Ok, "late", None),
        stop,
    );
    // read_skill had ended, so the stop stored what it returned.
    let run = run.unwrap();
    assert_eq!(view_content(&run).1, AiToolStatus::Ok);
    assert_eq!(result.unwrap_err().code, ErrorCode::InvalidInput);
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    let entries = f.entries(&conv);
    let stored: Vec<(&str, ToolStatus)> = entries[2..]
        .iter()
        .map(|e| {
            let (id, status, _) = tool_entry(e);
            (id, status)
        })
        .collect();
    assert_eq!(
        stored,
        [
            ("call_1", ToolStatus::Cancelled),
            ("call_2", ToolStatus::Ok),
            ("call_3", ToolStatus::Cancelled),
        ]
    );
    assert_eq!(tool_entry(&entries[3]).2, view_content(&run).2);
    assert_eq!(sent_results(&sink), ["call_1", "call_2", "call_3"]);
    f.idle(&conv).await;
}

#[tokio::test]
async fn a_call_left_without_a_result_is_cancelled_before_the_next_request() {
    let f = Fixture::new(vec![answer("Back again.")]).await;
    // The app crashed while a call waited: the conversation ends with an unanswered call.
    let conv = {
        let mut v = lock(&f.vault);
        let conv = v
            .put(
                None,
                Item::AiConversation(AiConversation {
                    title: "crash".into(),
                    ..AiConversation::default()
                }),
            )
            .unwrap();
        append(&mut v, &conv, &AiEntry::user(1, "run it")).unwrap();
        let call = AssistantEntry {
            provider_id: f.provider_id.clone(),
            model_id: "m1".into(),
            tool_calls: vec![ToolCall {
                id: "call_9".into(),
                name: "run_command".into(),
                arguments: r#"{"command":"make"}"#.into(),
            }],
            finish: Finish::ToolCalls,
            ..AssistantEntry::default()
        };
        append(&mut v, &conv, &AiEntry::assistant(2, call)).unwrap();
        conv
    };

    // No tab now: the request offers no terminal tools (AI-09).
    let context = AiTurnContext {
        tab: false,
        ..f.context()
    };
    let sink = f.retry(&conv, context).unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    let first = sink.events();
    assert!(matches!(
        &first[0],
        AiTurnEvent::Entry { entry: AiEntryView::Tool { tool_call_id, status: AiToolStatus::Cancelled, .. } } if tool_call_id == "call_9"
    ));

    let requests = f.requests().await;
    assert_eq!(requests.len(), 1);
    // The built-in skill and memory are on by default (AI-34, AI-53), so read_skill and memory
    // are offered too.
    assert_eq!(
        tool_names(&requests[0]),
        ["fetch_url", "read_skill", "memory"]
    );
    let system = messages(&requests[0])[0]["content"].as_str().unwrap();
    assert!(system.contains("No terminal is attached"), "{system}");
    // The call of a tool this request does not offer still goes with its cancelled result.
    let cancelled = tool_message(&requests[0], "call_9")["content"]
        .as_str()
        .unwrap();
    assert!(cancelled.contains("cancelled"), "{cancelled}");

    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 4);
    assert_eq!(
        tool_entry(&entries[2]),
        ("call_9", ToolStatus::Cancelled, "")
    );
    assert!(matches!(&entries[3].body, EntryBody::Assistant(a) if a.text == "Back again."));
}

/// AI-38…40 without a live session: the calls and the approval card's preview say why, and a
/// preview is only for an open edit or write.
#[tokio::test]
async fn file_tools_need_a_live_session() {
    let f = Fixture::new(vec![
        calls(&[
            (
                "f1",
                "edit_file",
                json!({"path": "/etc/hosts", "old_string": "a", "new_string": "b"}),
            ),
            ("f2", "read_file", json!({"path": "/etc/hosts"})),
            ("f3", "write_file", json!({"path": "/tmp/x"})),
        ]),
        answer("Done."),
    ])
    .await;
    let (started, sink) = f.send(None, "fix the hosts file").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let system = messages(&f.requests().await[0])[0]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        system.contains("over the same work with find, grep, cat, sed or heredocs in the shell"),
        "{system}"
    );

    let preview = |call: &'static str| {
        f.manager
            .file_preview(&f.vault, f.env.as_ref(), &conv, call, Some("tab-1"), None)
    };
    let card = preview("f1").await.unwrap();
    assert_eq!(card.path, "/etc/hosts");
    assert!(card.error.unwrap().contains("disconnected"));
    assert!(
        preview("f2")
            .await
            .unwrap()
            .error
            .unwrap()
            .contains("shows no diff")
    );
    assert_eq!(preview("nope").await.unwrap_err().code, ErrorCode::NotFound);

    for call in ["f1", "f2"] {
        let result = f.run(&conv, call, Some("tab-1"), None).await.unwrap();
        assert_eq!(view_content(&result).1, AiToolStatus::Error);
        assert!(
            view_content(&result)
                .2
                .contains("disconnected, so the tool did not run")
        );
    }
    let f3 = f.run(&conv, "f3", Some("tab-1"), None).await.unwrap();
    assert!(
        view_content(&f3)
            .2
            .starts_with("The arguments are not valid")
    );
    // The call has its result, so its card is gone.
    assert_eq!(
        preview("f1").await.unwrap_err().code,
        ErrorCode::InvalidInput
    );
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
}

#[tokio::test]
async fn tools_that_run_in_rust_report_failures_as_error_results() {
    let f = Fixture::new(vec![
        calls(&[
            ("c1", "run_command", json!({"command": "rm -rf /"})),
            ("c2", "mcp__files__read", json!({"path": "/etc"})),
            ("c3", "web_search", json!({"query": "nginx 502"})),
            ("c4", "read_skill", json!({"name": "nginx"})),
            (
                "c5",
                "read_skill",
                json!({"name": "nginx", "path": "./references\\tls.md"}),
            ),
            ("c6", "read_skill", json!({"name": "apache"})),
            ("c7", "teleport", json!({})),
        ]),
        answer("All done."),
    ])
    .await;
    f.add_skill(
        "nginx",
        "Debug nginx",
        &[
            ("SKILL.md", "Run nginx -t first."),
            ("references/tls.md", "TLS notes"),
        ],
    );
    let (started, sink) = f.send(None, "help").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;

    let requests = f.requests().await;
    assert_eq!(
        tool_names(&requests[0]),
        [
            "read_terminal",
            "run_command",
            "send_input",
            "read_file",
            "edit_file",
            "write_file",
            "grep",
            "glob",
            "fetch_url",
            "read_skill",
            "memory"
        ]
    );
    let system = messages(&requests[0])[0]["content"].as_str().unwrap();
    assert!(system.contains("nginx: Debug nginx"), "{system}");

    // AI-08: no live session, so the command does not run; the edit is still reported.
    let edited = r#"{"command":"ls /"}"#;
    let c1 = f
        .run(&conv, "c1", Some("tab-1"), Some(edited))
        .await
        .unwrap();
    assert_eq!(
        view_content(&c1),
        (
            "c1",
            AiToolStatus::Error,
            format!("{EDIT_NOTE} {edited}\n\n{DISCONNECTED}").as_str()
        )
    );
    let c2 = f.run(&conv, "c2", None, None).await.unwrap();
    assert_eq!(view_content(&c2).1, AiToolStatus::Error);
    assert!(view_content(&c2).2.contains("There is no MCP tool named"));
    let c3 = f.run(&conv, "c3", None, None).await.unwrap();
    assert_eq!(view_content(&c3).1, AiToolStatus::Error);
    assert!(view_content(&c3).2.contains("No search provider"));
    let c4 = f.run(&conv, "c4", None, None).await.unwrap();
    assert_eq!(
        view_content(&c4),
        ("c4", AiToolStatus::Ok, "Run nginx -t first.")
    );
    let c5 = f.run(&conv, "c5", None, None).await.unwrap();
    assert_eq!(view_content(&c5), ("c5", AiToolStatus::Ok, "TLS notes"));
    let c6 = f.run(&conv, "c6", None, None).await.unwrap();
    assert_eq!(view_content(&c6).1, AiToolStatus::Error);
    assert!(
        view_content(&c6)
            .2
            .contains("Enabled skills: hatoba, nginx.")
    );
    assert_eq!(
        f.run(&conv, "missing", None, None).await.unwrap_err().code,
        ErrorCode::NotFound
    );
    let c7 = f.run(&conv, "c7", None, None).await.unwrap();
    assert!(view_content(&c7).2.contains("no tool named"));

    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    let tool_results: Vec<String> = sink
        .entries()
        .iter()
        .filter_map(|e| match e {
            AiEntryView::Tool { tool_call_id, .. } => Some(tool_call_id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(tool_results, ["c1", "c2", "c3", "c4", "c5", "c6", "c7"]);
    assert_eq!(f.requests().await.len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_tab_every_tool_but_the_terminal_ones_is_offered_and_runs() {
    use crate::mcp::tests::{put_server, set_enabled, stdio_server};

    let f = Fixture::new(vec![
        calls(&[
            ("c1", "web_search", json!({"query": "zfs"})),
            (
                "c2",
                "fetch_url",
                json!({"url": "http://127.0.0.1:9/private"}),
            ),
            ("c3", "read_skill", json!({"name": "nginx"})),
            ("c4", "mcp__files__echo", json!({"text": "hi"})),
            ("c5", "run_command", json!({"command": "ls"})),
            ("c6", "read_terminal", json!({})),
            ("c7", "read_skill", json!({"name": "hatoba"})),
        ]),
        answer("Done."),
    ])
    .await;
    let search = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("q", "zfs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"title": "ZFS", "url": "https://openzfs.org", "content": "File system"}]
        })))
        .mount(&search)
        .await;
    {
        let mut v = lock(&f.vault);
        let provider = v
            .put(
                None,
                Item::SearchProvider(SearchProvider {
                    kind: SearchKind::Searxng,
                    base_url: Some(search.uri()),
                    ..SearchProvider::default()
                }),
            )
            .unwrap();
        let mut settings = v.settings();
        settings.ai.search_provider_id = Some(provider);
        v.put(Some(SETTINGS_ID), Item::Settings(settings)).unwrap();
    }
    f.add_skill(
        "nginx",
        "Debug nginx",
        &[("SKILL.md", "Run nginx -t first.")],
    );
    let files = put_server(&f.vault, stdio_server("files"));
    set_enabled(&f.vault, &files, true);

    let context = AiTurnContext {
        tab: false,
        ..f.context()
    };
    let (started, sink) = f.send_with(None, "look it up", context).await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;

    // AI-09: web_search, fetch_url, read_skill and MCP tools; no terminal tools.
    let requests = f.requests().await;
    assert_eq!(
        tool_names(&requests[0]),
        [
            "web_search",
            "fetch_url",
            "read_skill",
            "memory",
            "mcp__files__echo",
            "mcp__files__fail",
            "mcp__files__notify",
            "mcp__files__pid",
            "mcp__files__env"
        ]
    );
    let system = messages(&requests[0])[0]["content"].as_str().unwrap();
    assert!(system.contains("No terminal is attached"), "{system}");
    assert!(system.contains("- nginx: Debug nginx"), "{system}");
    assert!(system.contains("- hatoba: "), "{system}");
    assert!(!system.contains("run_command"), "{system}");

    // They run without a session.
    let c1 = f.run(&conv, "c1", None, None).await.unwrap();
    assert_eq!(view_content(&c1).1, AiToolStatus::Ok);
    assert!(view_content(&c1).2.contains("https://openzfs.org"));
    // fetch_url runs too; this address is refused like any private one (AI-15).
    let c2 = f.run(&conv, "c2", None, None).await.unwrap();
    assert_eq!(view_content(&c2).1, AiToolStatus::Error);
    assert!(view_content(&c2).2.contains("could not be fetched"));
    let c3 = f.run(&conv, "c3", None, None).await.unwrap();
    assert_eq!(
        view_content(&c3),
        ("c3", AiToolStatus::Ok, "Run nginx -t first.")
    );
    let c4 = f.run(&conv, "c4", None, None).await.unwrap();
    assert_eq!(view_content(&c4), ("c4", AiToolStatus::Ok, "hi"));
    // A terminal tool without a session is an error result, not a failed command.
    let c5 = f.run(&conv, "c5", None, None).await.unwrap();
    assert_eq!(view_content(&c5), ("c5", AiToolStatus::Error, DISCONNECTED));
    let c6 = f.run(&conv, "c6", None, None).await.unwrap();
    assert_eq!(view_content(&c6).1, AiToolStatus::Error);
    assert!(view_content(&c6).2.contains("runs in the terminal tab"));
    // AI-34: the built-in skill, with the app's version.
    let c7 = f.run(&conv, "c7", None, None).await.unwrap();
    assert_eq!(view_content(&c7).1, AiToolStatus::Ok);
    assert!(view_content(&c7).2.contains(VERSION));

    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    let requests = f.requests().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(tool_message(&requests[1], "c4")["content"], "hi");
    f.mcp.shutdown().await;
}

impl Fixture {
    /// Turns Native Search (AI-14) on or off.
    fn native_search(&self, on: bool) {
        let mut v = lock(&self.vault);
        let mut settings = v.settings();
        settings.ai.native_search = on;
        v.put(Some(SETTINGS_ID), Item::Settings(settings)).unwrap();
    }
}

/// AI-14: with no search provider chosen, `web_search` is offered, and the model that made the
/// call searches with its own provider's search.
#[tokio::test]
async fn without_a_search_provider_the_model_searches_with_its_own_provider() {
    let f = Fixture::new(vec![
        calls(&[
            ("c1", "web_search", json!({"query": "zfs"})),
            ("c2", "web_search", json!({"query": "btrfs"})),
        ]),
        answer("Done."),
    ])
    .await;
    f.native_search(true);
    let (started, sink) = f.send(None, "look it up").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    // A Chat Completions server Hatoba does not know is tried as OpenAI is.
    assert!(tool_names(&f.requests().await[0]).contains(&"web_search".to_owned()));

    let native = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_partial_json(json!({
            "model": "m1",
            "tools": [{"type": "web_search_20250305", "name": "web_search"}],
            "messages": [{"role": "user", "content": "Search the web for: zfs"}],
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [
                {"type": "server_tool_use", "id": "s1", "name": "web_search", "input": {"query": "zfs"}},
                {"type": "web_search_tool_result", "tool_use_id": "s1", "content": [
                    {"type": "web_search_result", "url": "https://openzfs.org", "title": "OpenZFS", "encrypted_content": "e"}
                ]},
                {"type": "text", "text": "ZFS is a file system."}
            ],
            "stop_reason": "end_turn"
        })))
        .mount(&native)
        .await;
    let (protocol, base_url) = {
        let mut v = lock(&f.vault);
        let mut provider = v
            .get(&f.provider_id)
            .and_then(Item::as_ai_provider)
            .unwrap()
            .clone();
        let before = (provider.protocol, provider.base_url.clone());
        provider.protocol = AiProtocol::Anthropic;
        provider.base_url = native.uri();
        v.put(Some(&f.provider_id), Item::AiProvider(provider))
            .unwrap();
        before
    };
    let c1 = f.run(&conv, "c1", None, None).await.unwrap();
    assert_eq!(
        view_content(&c1),
        (
            "c1",
            AiToolStatus::Ok,
            "1. OpenZFS\n   https://openzfs.org\n\nWhat the results say:\nZFS is a file system.\n"
        )
    );

    // Turned off, nothing searches.
    f.native_search(false);
    {
        let mut v = lock(&f.vault);
        let offered = crate::ai::request::builtin_tools(&v, &f.context());
        assert!(offered.iter().all(|t| t.name != "web_search"));
        let mut provider = v
            .get(&f.provider_id)
            .and_then(Item::as_ai_provider)
            .unwrap()
            .clone();
        (provider.protocol, provider.base_url) = (protocol, base_url);
        v.put(Some(&f.provider_id), Item::AiProvider(provider))
            .unwrap();
    }
    let c2 = f.run(&conv, "c2", None, None).await.unwrap();
    assert_eq!(
        view_content(&c2),
        (
            "c2",
            AiToolStatus::Error,
            "No search provider is chosen in Settings → AI, so web_search cannot run."
        )
    );
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
}

/// AI-14: a provider Hatoba does not know that turns the native search away gets the same result
/// for the next call without a request, and `web_search` stays offered: a conversation's tools
/// never change under the model.
#[tokio::test]
async fn a_provider_that_turns_native_search_away_is_not_asked_again() {
    let f = Fixture::new(vec![
        calls(&[
            ("c1", "web_search", json!({"query": "zfs"})),
            ("c2", "web_search", json!({"query": "btrfs"})),
        ]),
        answer("Done."),
    ])
    .await;
    let server = f.server.as_ref().unwrap();
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(json!({"error": {"message": "Not Found"}})),
        )
        .mount(server)
        .await;
    f.native_search(true);
    let (started, sink) = f.send(None, "look it up").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;

    let c1 = f.run(&conv, "c1", None, None).await.unwrap();
    let (_, status, text) = view_content(&c1);
    assert_eq!(status, AiToolStatus::Error);
    assert!(
        text.starts_with("The search failed: HTTP 404: Not Found."),
        "{text}"
    );
    assert!(text.contains("Settings → AI"), "{text}");
    let c2 = f.run(&conv, "c2", None, None).await.unwrap();
    assert_eq!(view_content(&c2), ("c2", AiToolStatus::Error, text));
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);

    let received = server.received_requests().await.unwrap();
    let to = |suffix: &str| {
        received
            .iter()
            .filter(|r| r.url.path().ends_with(suffix))
            .map(|r| r.body_json::<Value>().unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(to("/responses").len(), 1);
    let chats = to("/chat/completions");
    assert_eq!(chats.len(), 2);
    for chat in &chats {
        assert!(tool_names(chat).contains(&"web_search".to_owned()));
    }
}

// ───────────────────────── phase 2: MCP, AI-22, AI-26, AI-24 ─────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_turn_offers_mcp_tools_and_runs_their_calls() {
    use crate::dto::McpServerState;
    use crate::mcp::tests::{
        http_mock, http_server, missing_server, put_server, set_enabled, stdio_server,
    };

    let f = Fixture::new(vec![
        calls(&[
            ("c1", "mcp__files__echo", json!({"text": "hi"})),
            ("c2", "mcp__web__lookup", json!({"q": "dns"})),
            ("c3", "mcp__files__fail", json!({})),
            ("c4", "mcp__files__notify", json!({})),
            ("c5", "mcp__broken__x", json!({})),
        ]),
        answer("All done."),
    ])
    .await;
    let mock = http_mock().await;
    let files = put_server(&f.vault, stdio_server("files"));
    let web = put_server(&f.vault, http_server("web", &mock));
    let broken = put_server(&f.vault, missing_server("broken"));
    // `files` and `broken` were turned on here; `web` (http) is on by default (AI-29).
    set_enabled(&f.vault, &files, true);
    set_enabled(&f.vault, &broken, true);

    let (started, sink) = f.send(None, "use the servers").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let requests = f.requests().await;
    assert_eq!(
        tool_names(&requests[0]),
        [
            "read_terminal",
            "run_command",
            "send_input",
            "read_file",
            "edit_file",
            "write_file",
            "grep",
            "glob",
            "fetch_url",
            "read_skill",
            "memory",
            "mcp__files__echo",
            "mcp__files__fail",
            "mcp__files__notify",
            "mcp__files__pid",
            "mcp__files__env",
            "mcp__web__lookup"
        ]
    );
    // The server's description, as it wrote it (AI-30).
    let echo = requests[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["function"]["name"] == "mcp__files__echo")
        .unwrap();
    assert_eq!(echo["function"]["description"], "Echoes text");
    // The server that cannot start offers nothing, and says why.
    assert_eq!(
        f.mcp_events.states(&broken),
        [McpServerState::Starting, McpServerState::Failed]
    );

    // AI-31: the approval card's view of a call, with Always allow and Always ask.
    let info = |name: &str| {
        crate::mcp::tool_info(&lock(&f.vault), &f.mcp, &conv, name).expect("tool info")
    };
    let echo = info("mcp__files__echo");
    assert_eq!(
        (echo.server_id.as_str(), echo.server_name.as_str()),
        (files.as_str(), "files")
    );
    assert!(!echo.always_ask && !echo.tool.always_allow);
    assert_eq!(echo.tool.tool, "echo");
    assert_eq!(echo.tool.annotations.read_only_hint, Some(true));
    crate::mcp::set_always_allow(&mut lock(&f.vault), &files, Some("echo"), true).unwrap();
    assert!(info("mcp__files__echo").tool.always_allow);
    assert!(!info("mcp__files__fail").tool.always_allow);
    crate::mcp::set_always_allow(&mut lock(&f.vault), &web, None, true).unwrap();
    assert!(info("mcp__web__lookup").tool.always_allow);
    {
        let mut v = lock(&f.vault);
        let mut server = v.get(&web).and_then(Item::as_mcp_server).cloned().unwrap();
        server.always_ask = true;
        v.put(Some(&web), Item::McpServer(server)).unwrap();
    }
    assert!(info("mcp__web__lookup").always_ask);
    assert!(crate::mcp::tool_info(&lock(&f.vault), &f.mcp, &conv, "mcp__nope__x").is_none());

    let c1 = f.run(&conv, "c1", None, None).await.unwrap();
    assert_eq!(view_content(&c1), ("c1", AiToolStatus::Ok, "hi"));
    let c2 = f.run(&conv, "c2", None, None).await.unwrap();
    assert_eq!(view_content(&c2), ("c2", AiToolStatus::Ok, "found dns"));
    let c3 = f.run(&conv, "c3", None, None).await.unwrap();
    assert_eq!(view_content(&c3), ("c3", AiToolStatus::Error, "boom"));
    let c4 = f.run(&conv, "c4", None, None).await.unwrap();
    assert_eq!(view_content(&c4), ("c4", AiToolStatus::Ok, "notified"));
    let c5 = f.run(&conv, "c5", None, None).await.unwrap();
    assert_eq!(view_content(&c5).1, AiToolStatus::Error);
    assert!(view_content(&c5).2.contains("There is no MCP tool named"));

    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    let requests = f.requests().await;
    assert_eq!(requests.len(), 2);
    // `tools/list_changed` refreshed the list before the next request (AI-30).
    assert!(tool_names(&requests[1]).contains(&"mcp__files__added".to_owned()));
    assert_eq!(tool_message(&requests[1], "c1")["content"], "hi");

    // AI-32: locking stops every server.
    lock(&f.vault).lock();
    f.mcp.stop_all(&f.vault).await;
    for id in [&files, &web] {
        assert_eq!(
            f.mcp_events.states(id).last(),
            Some(&McpServerState::Stopped)
        );
    }
}

#[tokio::test]
async fn calls_that_share_an_id_get_error_results_and_never_run() {
    let f = Fixture::new(vec![
        calls(&[
            ("dup", "run_command", json!({"command": "rm -rf /tmp/x"})),
            ("dup", "read_terminal", json!({})),
            ("c3", "read_terminal", json!({})),
        ]),
        answer("Done."),
    ])
    .await;
    let (started, sink) = f.send(None, "go").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    // Their results come before `done`, so the panel finds only c3 open.
    let events = sink.events();
    let done = events
        .iter()
        .position(|e| matches!(e, AiTurnEvent::Done { .. }))
        .unwrap();
    let results: Vec<(String, AiToolStatus, String)> = events[..done]
        .iter()
        .filter_map(|e| match e {
            AiTurnEvent::Entry {
                entry:
                    AiEntryView::Tool {
                        tool_call_id,
                        status,
                        content,
                        ..
                    },
            } => Some((tool_call_id.clone(), *status, content.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(results.len(), 2);
    for (id, status, content) in &results {
        assert_eq!((id.as_str(), *status), ("dup", AiToolStatus::Error));
        assert!(content.contains("2 tool calls"), "{content}");
    }
    let err = f.run(&conv, "dup", Some("s1"), None).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    f.result(&conv, "c3", AiToolStatus::Ok, "the screen", None)
        .await
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    let requests = f.requests().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(tool_message(&requests[1], "c3")["content"], "the screen");
}

#[tokio::test]
async fn calls_stored_with_a_shared_id_run_each_with_its_own_arguments() {
    let f = Fixture::new(Vec::new()).await;
    // Stored before such calls got their results at once (or by another device).
    let conv = {
        let mut v = lock(&f.vault);
        let conv = v
            .put(
                None,
                Item::AiConversation(AiConversation {
                    title: "old".into(),
                    ..AiConversation::default()
                }),
            )
            .unwrap();
        append(&mut v, &conv, &AiEntry::user(1, "read both")).unwrap();
        let skill = |name: &str| ToolCall {
            id: "dup".into(),
            name: "read_skill".into(),
            arguments: json!({ "name": name }).to_string(),
        };
        let response = AssistantEntry {
            tool_calls: vec![skill("first-skill"), skill("second-skill")],
            finish: Finish::ToolCalls,
            ..AssistantEntry::default()
        };
        append(&mut v, &conv, &AiEntry::assistant(2, response)).unwrap();
        conv
    };
    let first = f.run(&conv, "dup", None, None).await.unwrap();
    assert!(
        view_content(&first).2.contains("\"first-skill\""),
        "{first:?}"
    );
    let second = f.run(&conv, "dup", None, None).await.unwrap();
    assert!(
        view_content(&second).2.contains("\"second-skill\""),
        "{second:?}"
    );
    let err = f.run(&conv, "dup", None, None).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_running_call_is_not_run_or_answered_again() {
    use crate::mcp::tests::{put_server, set_enabled, stdio_server};

    // The server holds its answer to c1 until the test releases it. The turn waits for the
    // panel's result of c2, so the response with c1 stays the newest until then.
    let f = Fixture::new(vec![
        calls(&[
            (
                "c1",
                "mcp__files__echo",
                json!({"text": "once", "hold": true}),
            ),
            ("c2", "read_terminal", json!({})),
        ]),
        answer("Done."),
    ])
    .await;
    let files = put_server(&f.vault, stdio_server("files"));
    set_enabled(&f.vault, &files, true);
    let (started, sink) = f.send(None, "echo").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;

    // While the first run waits for the server, a second run of the call is refused before it
    // runs anything, and so is a result from the panel. Nothing polls the first run meanwhile,
    // so it cannot finish before they are checked.
    let mut first = Box::pin(f.run(&conv, "c1", None, None));
    assert!(poll_once(&mut first).await.is_pending());
    let second = f.run(&conv, "c1", None, None).await;
    let result = f
        .result(&conv, "c1", AiToolStatus::Error, "The tool failed", None)
        .await;
    for err in [second.unwrap_err(), result.unwrap_err()] {
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert_eq!(err.detail, "the tool call is already running");
    }
    let echo = f.mcp.offered(&conv, "mcp__files__echo").unwrap();
    let release = json!({"release": true}).as_object().cloned().unwrap();
    f.mcp
        .call(&f.vault, &echo, release, &CancellationToken::new())
        .await;
    assert_eq!(
        view_content(&first.await.unwrap()),
        ("c1", AiToolStatus::Ok, "once")
    );
    // Once it finished, the call has its result.
    let err = f.run(&conv, "c1", None, None).await.unwrap_err();
    assert_eq!(err.detail, "the tool call already has a result");
    f.result(&conv, "c2", AiToolStatus::Ok, "the screen", None)
        .await
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.mcp.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_of_a_server_deleted_elsewhere_does_not_run() {
    use crate::mcp::tests::{put_server, set_enabled, stdio_server};

    let f = Fixture::new(vec![
        calls(&[("c1", "mcp__files__pid", json!({}))]),
        answer("Done."),
    ])
    .await;
    let files = put_server(&f.vault, stdio_server("files"));
    set_enabled(&f.vault, &files, true);
    let (started, sink) = f.send(None, "use files").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    // A sync pull took the item away; the server keeps running until the next request.
    lock(&f.vault).delete(&files).unwrap();
    assert!(crate::mcp::tool_info(&lock(&f.vault), &f.mcp, &conv, "mcp__files__pid").is_none());
    let c1 = f.run(&conv, "c1", None, None).await.unwrap();
    assert_eq!(view_content(&c1).1, AiToolStatus::Error);
    assert!(view_content(&c1).2.contains("was deleted"), "{c1:?}");
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.mcp.shutdown().await;
}
