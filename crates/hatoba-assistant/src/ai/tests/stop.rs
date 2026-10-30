//! Stopping turns and the lock (§13.1, AI-18): what a stop keeps of a streamed reply, the
//! commands a stop waits for, and the results of the calls a stop or the lock leaves.

use super::*;

#[tokio::test]
async fn stopping_mid_stream_keeps_the_text_that_streamed() {
    let (base_url, _) = stalling_server(&[
        json!({"choices": [{"index": 0, "delta": {"reasoning_content": "Thinking"}}]}),
        text_delta("Hel"),
    ])
    .await;
    let f = Fixture::with_base_url(None, base_url);
    let (started, sink) = f.send(None, "hello").await;
    let conv = started.conversation.id;
    streamed(&sink, "Hel").await;

    let at = Instant::now();
    f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert!(at.elapsed() < Duration::from_secs(5));
    assert!(!f.manager.is_running(&conv));
    // The task notices the cancellation and sends nothing more.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);

    // What the user saw is stored, marked, without the reasoning or a `raw` to replay.
    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 2);
    assert!(matches!(&entries[0].body, EntryBody::User { .. }));
    assert_eq!(
        entries[1].body,
        EntryBody::Assistant(AssistantEntry {
            provider_id: f.provider_id.clone(),
            model_id: "m1".into(),
            text: "Hel".into(),
            terminal: Some(true),
            // No MCP server is on, and a reply records that too.
            mcp_servers: Some(Vec::new()),
            interrupted: true,
            ..AssistantEntry::default()
        })
    );
    // It reaches the panel before the end of the turn.
    let events = sink.events();
    assert!(matches!(
        &events[events.len() - 2],
        AiTurnEvent::Entry { entry: AiEntryView::Assistant { text, .. } } if text == "Hel"
    ));
    assert_eq!(f.detail(&conv).entries.len(), 2);
}

#[tokio::test]
async fn a_stopped_reply_on_a_local_tab_keeps_its_shell() {
    let (base_url, _) = stalling_server(&[text_delta("Clon")]).await;
    let f = Fixture::with_base_url(None, base_url);
    let wsl = f.local("DEMO-PC", "Ubuntu", crate::dto::LocalShellKind::Wsl);
    let (started, sink) = f.send_with(None, "clone it", wsl).await;
    let conv = started.conversation.id;
    streamed(&sink, "Clon").await;
    f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    // §7.5: the next message on another shell is noted from it, as after a whole reply.
    match &f.entries(&conv)[1].body {
        EntryBody::Assistant(a) => {
            assert!(a.interrupted);
            assert_eq!(a.shell.as_deref(), Some("Ubuntu"));
        }
        other => panic!("not a reply: {other:?}"),
    }
}

#[test]
fn a_stop_keeps_the_text_and_the_complete_calls_that_streamed() {
    let streamed = |text: &str, tool_calls: Vec<ToolCall>| Streamed {
        provider_id: "p".into(),
        model_id: "m".into(),
        text: text.into(),
        tool_calls,
        shell: Some("Ubuntu".into()),
        mcp_servers: Some(vec!["Context7".into()]),
    };
    let call = ToolCall {
        id: "c1".into(),
        name: "run_command".into(),
        arguments: r#"{"command":"df -h"}"#.into(),
    };
    let entry = streamed("Checking.", vec![call.clone()])
        .into_entry(false)
        .unwrap();
    assert_eq!(
        entry,
        AssistantEntry {
            provider_id: "p".into(),
            model_id: "m".into(),
            text: "Checking.".into(),
            tool_calls: vec![call.clone()],
            finish: Finish::ToolCalls,
            terminal: Some(false),
            shell: Some("Ubuntu".into()),
            mcp_servers: Some(vec!["Context7".into()]),
            interrupted: true,
            ..AssistantEntry::default()
        }
    );
    // A call alone was shown too; its result is the cancelled one the stop stores.
    let entry = streamed("", vec![call]).into_entry(true).unwrap();
    assert!(entry.text.is_empty() && entry.finish == Finish::ToolCalls);
    assert_eq!(
        streamed("Hi", vec![]).into_entry(true).unwrap().finish,
        Finish::Stop
    );
    // Nothing but white space: nothing to keep.
    assert_eq!(streamed(" \n", vec![]).into_entry(true), None);
}

/// The turn's task runs on another thread and acts the moment a stop cancels it: its request ends,
/// it drops what streamed and ends the turn. The test makes that happen after the stop's cancel
/// and before the stop stores anything, the order that lost the text or its `entry` event. The
/// stop still stores the text, and sends it before the one `turn_ended`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::await_holding_lock,
    reason = "the lock holds the stop while the task, which never takes it, runs on another thread"
)]
async fn a_stop_keeps_the_text_when_the_task_acts_first() {
    let f = Fixture::with_base_url(None, "http://127.0.0.1:9/v1".into());
    let conv = {
        let mut v = lock(&f.vault);
        let conv = v
            .put(
                None,
                Item::AiConversation(AiConversation {
                    title: "race".into(),
                    ..AiConversation::default()
                }),
            )
            .unwrap();
        append(&mut v, &conv, &AiEntry::user(1, "df?")).unwrap();
        conv
    };
    let sink = Arc::new(Sink::default());
    let turn = f.manager.register(&conv, f.context(), None, sink.clone());
    *turn.streamed.lock().unwrap() = Some(Streamed {
        provider_id: f.provider_id.clone(),
        model_id: "m1".into(),
        text: "The disk is 4".into(),
        tool_calls: Vec::new(),
        shell: None,
        mcp_servers: None,
    });
    // What the turn loop and `task` do when the request returns cancelled.
    let task = {
        let turn = turn.clone();
        tokio::spawn(async move {
            turn.cancel.cancelled().await;
            turn.streamed.lock().unwrap().take();
            turn.finish(AiTurnEndReason::Stopped);
        })
    };

    // Right after its cancel the stop forgets the conversation's file bases; holding their lock
    // keeps it there until the task has acted.
    let file_bases = f.manager.0.file_bases.lock().unwrap();
    let stop = {
        let (manager, vault, env) = (f.manager.clone(), f.vault.clone(), f.env.clone());
        let conv = conv.clone();
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            runtime.block_on(manager.stop(&vault, env.as_ref(), &conv));
        })
    };
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("the stop cancels the turn before it forgets the file bases")
        .unwrap();
    assert!(sink.events().is_empty());
    drop(file_bases);
    stop.await.unwrap();

    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 2);
    assert!(
        matches!(&entries[1].body, EntryBody::Assistant(a) if a.text == "The disk is 4" && a.interrupted)
    );
    assert_eq!(kinds(&sink.events()), ["entry", "turn_ended"]);
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
}

#[tokio::test]
async fn stopping_before_any_text_stores_nothing() {
    let (base_url, _) = stalling_server(&[json!(
        {"choices": [{"index": 0, "delta": {"reasoning_content": "Thinking"}}]}
    )])
    .await;
    let f = Fixture::with_base_url(None, base_url);
    let (started, sink) = f.send(None, "hello").await;
    let conv = started.conversation.id;
    sink.until("reasoning", |e| {
        e.iter().any(|e| matches!(e, AiTurnEvent::Reasoning { .. }))
    })
    .await;

    f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    f.idle(&conv).await;
    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 1);
    assert!(matches!(&entries[0].body, EntryBody::User { .. }));
    assert!(sink.entries().is_empty());
}

#[tokio::test]
async fn the_message_after_a_stopped_reply_says_it_was_interrupted() {
    use hatoba_ai::tools::interrupted_block;

    let (base_url, bodies) = stalling_server(&[text_delta("The disk is 4")]).await;
    let f = Fixture::with_base_url(None, base_url);
    let (started, sink) = f.send(None, "df?").await;
    let conv = started.conversation.id;
    streamed(&sink, "The disk is 4").await;
    f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    f.idle(&conv).await;

    let context = f.context();
    assert_eq!(
        f.exchange(Some(&conv), "never mind", context.clone())
            .await
            .1,
        AiTurnEndReason::Completed
    );
    // Noted once: the reply after it was not stopped.
    f.exchange(Some(&conv), "thanks", context).await;
    let noted = format!("{}never mind", interrupted_block());
    assert_eq!(
        f.user_texts(&conv),
        [manual("df?"), noted.clone(), "thanks".to_owned()]
    );

    // The model gets the part it wrote, rebuilt, then the note with the message.
    let bodies = bodies.lock().unwrap().clone();
    assert_eq!(bodies.len(), 3);
    let sent = messages(&bodies[1]);
    assert_eq!(sent.len(), 4);
    assert_eq!(
        sent[2],
        json!({"role": "assistant", "content": "The disk is 4"})
    );
    assert_eq!(sent[3], json!({"role": "user", "content": noted}));
}

#[tokio::test]
async fn sending_while_a_reply_streams_keeps_the_part_that_streamed() {
    use hatoba_ai::tools::interrupted_block;

    let (base_url, _) = stalling_server(&[text_delta("Hel")]).await;
    let f = Fixture::with_base_url(None, base_url);
    let (started, first) = f.send(None, "one").await;
    let conv = started.conversation.id;
    streamed(&first, "Hel").await;

    let (_, second) = f.send(Some(&conv), "two").await;
    assert_eq!(first.ended().await, AiTurnEndReason::Stopped);
    assert_eq!(second.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;

    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 4);
    assert!(
        matches!(&entries[1].body, EntryBody::Assistant(a) if a.text == "Hel" && a.interrupted)
    );
    assert!(
        matches!(&entries[2].body, EntryBody::User { text, .. } if *text == format!("{}two", interrupted_block()))
    );
    assert!(
        matches!(&entries[3].body, EntryBody::Assistant(a) if a.text == "Done." && !a.interrupted)
    );
    // The panel listens to the new turn's channel by then, so the stopped reply goes there too.
    assert!(matches!(
        second.entries().first(),
        Some(AiEntryView::Assistant { text, .. }) if text == "Hel"
    ));
}

#[tokio::test]
async fn a_stop_noted_on_a_message_without_a_reply_is_not_noted_again() {
    use hatoba_ai::tools::interrupted_block;

    let f = Fixture::new(vec![
        ResponseTemplate::new(500).set_body_json(json!({"error": {"message": "overloaded"}})),
        answer("Two."),
        answer("Three."),
    ])
    .await;
    let conv = {
        let mut v = lock(&f.vault);
        let conv = v
            .put(
                None,
                Item::AiConversation(AiConversation {
                    title: "stopped".into(),
                    ..AiConversation::default()
                }),
            )
            .unwrap();
        append(&mut v, &conv, &AiEntry::user(1, "one")).unwrap();
        let stopped = AssistantEntry {
            provider_id: f.provider_id.clone(),
            model_id: "m1".into(),
            text: "Partly".into(),
            terminal: Some(true),
            interrupted: true,
            ..AssistantEntry::default()
        };
        append(&mut v, &conv, &AiEntry::assistant(2, stopped)).unwrap();
        conv
    };
    let (_, failed) = f.exchange(Some(&conv), "two", f.context()).await;
    assert_eq!(failed, AiTurnEndReason::Error);
    // A retry stores no message, so nothing is noted.
    assert_eq!(
        f.retry(&conv, f.context()).unwrap().ended().await,
        AiTurnEndReason::Completed
    );
    f.idle(&conv).await;
    f.exchange(Some(&conv), "three", f.context()).await;
    // "one" named no mode, as in a conversation from before every first message did, so "two"
    // names it.
    assert_eq!(
        f.user_texts(&conv),
        [
            "one".to_owned(),
            format!("{}{}", interrupted_block(), manual("two")),
            "three".to_owned()
        ]
    );
}

/// A response with a batch of a read and a `run_command`, whose calls the panel runs at once.
async fn a_read_and_a_command() -> (Fixture, String, Arc<Sink>) {
    let f = Fixture::new(vec![
        calls(&[
            ("c1", "read_file", json!({"path": "/etc/hosts"})),
            ("c2", "run_command", json!({"command": "ls /etc"})),
        ]),
        answer("Fine."),
    ])
    .await;
    let (started, sink) = f.send(None, "look around").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    (f, conv, sink)
}

/// The command has run (without a session, it reports that) and its result waits for the read's:
/// it no longer counts as a running command.
async fn until_the_command_waits(f: &Fixture, conv: &str) {
    tokio::time::sleep(Duration::from_millis(200)).await;
    let entries = Entries::load(&lock(&f.vault), conv).unwrap();
    assert!(open_call(&entries, "c2").is_ok());
    assert!(f.manager.running_commands(conv).is_empty());
}

/// A stop that does not wait for commands, such as a message sent while the turn runs, stores
/// what a command that has ended returned while its result waits for an earlier call's, rather
/// than a result that says it may still run (AI-18).
#[tokio::test]
async fn a_message_keeps_the_result_of_a_command_that_ended_and_waits() {
    let (f, conv, sink) = a_read_and_a_command().await;
    // The read still runs.
    let read = f.manager.start_call(&conv, "c1", OpKind::Other).unwrap();
    let send = async {
        until_the_command_waits(&f, &conv).await;
        f.send(Some(&conv), "never mind").await
    };
    let (command, (_, next)) = tokio::join!(f.run(&conv, "c2", None, None), send);
    drop(read);
    assert_eq!(
        view_content(&command.unwrap()),
        ("c2", AiToolStatus::Error, DISCONNECTED)
    );
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert_eq!(next.ended().await, AiTurnEndReason::Completed);
    assert_eq!(
        tool_results(&f.entries(&conv)),
        [
            ("c1", ToolStatus::Cancelled, ""),
            ("c2", ToolStatus::Error, DISCONNECTED),
        ]
    );
    assert_eq!(sent_results(&sink), ["c1", "c2"]);
    f.idle(&conv).await;
}

/// The lock, like a message, stores what a command that has ended returned (AI-18).
#[tokio::test]
async fn the_lock_keeps_the_result_of_a_command_that_ended_and_waits() {
    let (f, conv, sink) = a_read_and_a_command().await;
    let read = f.manager.start_call(&conv, "c1", OpKind::Other).unwrap();
    let lock_now = async {
        until_the_command_waits(&f, &conv).await;
        let mut v = lock(&f.vault);
        f.manager.stop_all(&mut v);
        v.lock();
    };
    let (command, ()) = tokio::join!(f.run(&conv, "c2", None, None), lock_now);
    drop(read);
    assert_eq!(command.unwrap_err().code, ErrorCode::Locked);
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert_eq!(sent_results(&sink), ["c1", "c2"]);
    lock(&f.vault).unlock(PW).unwrap();
    assert_eq!(
        tool_results(&f.entries(&conv)),
        [
            ("c1", ToolStatus::Cancelled, ""),
            ("c2", ToolStatus::Error, DISCONNECTED),
        ]
    );
}

/// A stop that waits for a `run_command` of a batch (AI-18) does not wait on the calls before
/// it: the command's result, which waits for an earlier read's, is stored as the stop cancels
/// the turn, within [`COMMAND_STOP_WAIT`], and the read gets a cancelled result.
#[tokio::test]
async fn a_stopped_command_of_a_batch_stores_its_result_without_the_earlier_calls() {
    let f = Fixture::new(vec![calls(&[
        ("c1", "read_file", json!({"path": "/etc/hosts"})),
        ("c2", "run_command", json!({"command": "ls /etc"})),
    ])])
    .await;
    let (started, sink) = f.send(None, "look around").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    // The read still runs.
    let read = f.manager.start_call(&conv, "c1", OpKind::Other).unwrap();

    let stop = async {
        // The command has run (without a session, it reports that) and waits for the read.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(open_call(&Entries::load(&lock(&f.vault), &conv).unwrap(), "c2").is_ok());
        let started = Instant::now();
        f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
        started.elapsed()
    };
    let (command, took) = tokio::join!(f.run(&conv, "c2", None, None), stop);
    drop(read);
    assert!(took < COMMAND_STOP_WAIT, "the stop took {took:?}");
    assert_eq!(
        view_content(&command.unwrap()),
        ("c2", AiToolStatus::Error, DISCONNECTED)
    );
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert_eq!(
        tool_results(&f.entries(&conv)),
        [
            ("c2", ToolStatus::Error, DISCONNECTED),
            ("c1", ToolStatus::Cancelled, ""),
        ]
    );
    assert_eq!(sent_results(&sink), ["c2", "c1"]);
    f.idle(&conv).await;
}

#[tokio::test]
async fn stopping_while_a_tool_waits_cancels_the_calls_without_a_result() {
    let f = Fixture::new(vec![calls(&[
        ("call_1", "read_terminal", json!({})),
        ("call_2", "run_command", json!({"command": "df -h"})),
    ])])
    .await;
    let (started, sink) = f.send(None, "check").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    f.result(&conv, "call_1", AiToolStatus::Ok, "screen", None)
        .await
        .unwrap();

    f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);

    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 4);
    assert_eq!(
        tool_entry(&entries[2]),
        ("call_1", ToolStatus::Ok, "screen")
    );
    assert_eq!(
        tool_entry(&entries[3]),
        ("call_2", ToolStatus::Cancelled, "")
    );
    // The cancelled result reaches the panel before the end of the turn.
    let events = sink.events();
    let cancelled_at = events
        .iter()
        .position(|e| {
            matches!(e, AiTurnEvent::Entry { entry: AiEntryView::Tool { tool_call_id, status: AiToolStatus::Cancelled, .. } } if tool_call_id == "call_2")
        })
        .expect("cancelled result event");
    assert!(cancelled_at < events.len() - 1);
    assert_eq!(
        ended(events.last().unwrap()),
        Some(AiTurnEndReason::Stopped)
    );

    // The call has its result now, so a late report or run is refused.
    assert_eq!(
        f.result(&conv, "call_2", AiToolStatus::Ok, "late", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );
    assert!(f.run(&conv, "call_2", Some("s1"), None).await.is_err());
    assert_eq!(f.requests().await.len(), 1);
    f.idle(&conv).await;
}

#[tokio::test]
async fn locking_stops_every_turn() {
    let f = Fixture::new(vec![calls(&[(
        "call_1",
        "run_command",
        json!({"command": "uptime"}),
    )])])
    .await;
    let (started, sink) = f.send(None, "uptime?").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;

    {
        let mut v = lock(&f.vault);
        f.manager.stop_all(&mut v);
        v.lock();
    }
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert!(!f.manager.is_running(&conv));
    assert_eq!(
        f.result(&conv, "call_1", AiToolStatus::Ok, "late", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Locked
    );

    lock(&f.vault).unlock(PW).unwrap();
    let entries = f.entries(&conv);
    assert_eq!(
        tool_entry(entries.last().unwrap()),
        ("call_1", ToolStatus::Cancelled, "")
    );
    assert_eq!(f.requests().await.len(), 1);
}

#[tokio::test]
async fn sending_while_a_turn_runs_stops_it_first() {
    let f = Fixture::new(vec![
        answer("slow").set_delay(Duration::from_secs(30)),
        answer("fast"),
    ])
    .await;
    let (started, first) = f.send(None, "one").await;
    let conv = started.conversation.id;
    let deadline = Instant::now() + Duration::from_secs(20);
    while f.requests().await.is_empty() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let (again, second) = f.send(Some(&conv), "two").await;
    assert_eq!(again.conversation.id, conv);
    assert_eq!(first.ended().await, AiTurnEndReason::Stopped);
    assert_eq!(second.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;

    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 3);
    assert!(matches!(&entries[0].body, EntryBody::User { text, .. } if *text == manual("one")));
    assert!(matches!(&entries[1].body, EntryBody::User { text, .. } if text == "two"));
    assert!(matches!(&entries[2].body, EntryBody::Assistant(a) if a.text == "fast"));
    assert!(
        !first
            .events()
            .iter()
            .any(|e| matches!(e, AiTurnEvent::Entry { .. }))
    );
}

/// §13.4, "File tools": a stop does not cut off a write that has begun, so the cancelled result of
/// an edit or write that was running tells the model the file may have changed.
#[tokio::test]
async fn a_stop_while_a_file_tool_runs_says_the_file_may_have_changed() {
    let f = Fixture::new(vec![calls(&[
        (
            "w1",
            "write_file",
            json!({"path": "/tmp/x", "content": "y"}),
        ),
        ("r1", "read_file", json!({"path": "/tmp/x"})),
        ("w2", "edit_file", json!({"path": "/tmp/x"})),
    ])])
    .await;
    let (started, sink) = f.send(None, "write it").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    // w1 and r1 run; w2 has not started.
    let _write = f.manager.start_call(&conv, "w1", OpKind::Other).unwrap();
    let _read = f.manager.start_call(&conv, "r1", OpKind::Other).unwrap();
    f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);

    let results: Vec<(String, ToolStatus, String)> = f
        .entries(&conv)
        .into_iter()
        .filter_map(|e| match e.body {
            EntryBody::Tool {
                tool_call_id,
                status,
                content,
                ..
            } => Some((tool_call_id, status, content)),
            _ => None,
        })
        .collect();
    assert_eq!(
        results,
        [
            (
                "w1".into(),
                ToolStatus::Cancelled,
                crate::ai::files::WRITE_STOPPED.into()
            ),
            ("r1".into(), ToolStatus::Cancelled, String::new()),
            ("w2".into(), ToolStatus::Cancelled, String::new()),
        ]
    );
}

/// What `tool_run` does for a `run_command` call that a stop cancels: the command takes `takes`
/// to end, then the call stores the result that says so, unless the call has one by then, and
/// lets go of its op.
async fn command_ends(f: &Fixture, conv: &str, op: OpGuard<'_>, takes: Duration) {
    op.token().cancelled().await;
    tokio::time::sleep(takes).await;
    let mut v = lock(&f.vault);
    if open_call(&Entries::load(&v, conv).unwrap(), "c1").is_ok() {
        f.manager
            .store_result(
                &mut v,
                conv,
                "c1",
                ToolStatus::Cancelled,
                "ended".into(),
                false,
            )
            .unwrap();
    }
    drop(v);
    drop(op);
}

/// AI-18: a stop waits for a running command to end, so the call keeps the result that says how
/// it ended, and the stop sends it with the cancelled results of the other calls.
#[tokio::test]
async fn a_stop_waits_for_a_running_command_and_keeps_its_result() {
    let f = Fixture::new(vec![calls(&[
        ("c1", "run_command", json!({"command": "sleep 300"})),
        ("c2", "run_command", json!({"command": "ls"})),
    ])])
    .await;
    let (started, sink) = f.send(None, "run it").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let op = f.manager.start_call(&conv, "c1", OpKind::Command).unwrap();

    // Nothing new starts in the turn while the stop waits.
    let meanwhile = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(f.manager.is_running(&conv));
        assert!(f.manager.start_op(&conv).token().is_cancelled());
    };
    tokio::join!(
        f.manager.stop(&f.vault, f.env.as_ref(), &conv),
        command_ends(&f, &conv, op, Duration::from_millis(100)),
        meanwhile,
    );
    assert!(!f.manager.is_running(&conv));
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert_eq!(
        tool_results(&f.entries(&conv)),
        [
            ("c1", ToolStatus::Cancelled, "ended"),
            ("c2", ToolStatus::Cancelled, ""),
        ]
    );
    assert_eq!(sent_results(&sink), ["c1", "c2"]);
}

/// AI-18: while a stop waits for a command, the conversation counts as running, and a message
/// sent meanwhile settles the stop at once: the command's result says it may still run, and comes
/// before the message, which the new turn's channel sends it with.
#[tokio::test]
async fn a_message_sent_while_a_stop_waits_settles_it() {
    let f = Fixture::new(vec![
        calls(&[("c1", "run_command", json!({"command": "sleep 300"}))]),
        answer("Stopped it."),
    ])
    .await;
    let (started, sink) = f.send(None, "run it").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let op = f.manager.start_call(&conv, "c1", OpKind::Command).unwrap();

    let send = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(f.manager.is_running(&conv));
        f.send(Some(&conv), "never mind").await
    };
    let ((), (_, next), ()) = tokio::join!(
        f.manager.stop(&f.vault, f.env.as_ref(), &conv),
        send,
        command_ends(&f, &conv, op, Duration::from_millis(300)),
    );
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert_eq!(next.ended().await, AiTurnEndReason::Completed);
    let entries = f.entries(&conv);
    assert_eq!(
        tool_results(&entries),
        [(
            "c1",
            ToolStatus::Cancelled,
            hatoba_ai::tools::COMMAND_STOP_UNCONFIRMED
        )]
    );
    let result = entries
        .iter()
        .position(|e| matches!(e.body, EntryBody::Tool { .. }))
        .unwrap();
    assert!(
        matches!(&entries[result + 1].body, EntryBody::User { text, .. } if text.ends_with("never mind"))
    );
    assert_eq!(sent_results(&next), ["c1"]);
}

/// AI-18: the lock settles a stop that waits for a command, so the command's call never stays
/// without a result behind the lock screen.
#[tokio::test]
async fn the_lock_settles_a_stop_that_waits() {
    let f = Fixture::new(vec![calls(&[
        ("c1", "run_command", json!({"command": "sleep 300"})),
        ("c2", "run_command", json!({"command": "ls"})),
    ])])
    .await;
    let (started, sink) = f.send(None, "run it").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let op = f.manager.start_call(&conv, "c1", OpKind::Command).unwrap();
    let lock_now = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        f.manager.stop_all(&mut lock(&f.vault));
    };
    tokio::join!(
        f.manager.stop(&f.vault, f.env.as_ref(), &conv),
        lock_now,
        command_ends(&f, &conv, op, Duration::from_millis(300)),
    );
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert!(!f.manager.is_running(&conv));
    assert_eq!(
        tool_results(&f.entries(&conv)),
        [
            (
                "c1",
                ToolStatus::Cancelled,
                hatoba_ai::tools::COMMAND_STOP_UNCONFIRMED
            ),
            ("c2", ToolStatus::Cancelled, ""),
        ]
    );
    assert_eq!(sent_results(&sink), ["c1", "c2"]);
}

/// AI-18: a stop that something else settled while it waited leaves alone a later stop of the
/// same conversation when its own wait ends.
#[tokio::test]
async fn a_settled_stop_leaves_a_later_stop_alone() {
    let f = Fixture::new(vec![calls(&[(
        "c1",
        "run_command",
        json!({"command": "sleep 300"}),
    )])])
    .await;
    let (started, sink) = f.send(None, "run it").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let first_command = f.manager.start_call(&conv, "c1", OpKind::Command).unwrap();
    let later = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        // A message settles the first stop at once, and its turn runs a command of its own.
        f.manager.stop_locked(&mut lock(&f.vault), &conv);
        let next = Arc::new(Sink::default());
        let _turn = f.manager.register(&conv, f.context(), None, next.clone());
        let second_command = f.manager.start_call(&conv, "c2", OpKind::Command).unwrap();
        let ends = async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            // The first stop's command ends, which ends its wait.
            drop(first_command);
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(f.manager.is_running(&conv), "the second stop still waits");
            assert!(next.events().is_empty());
            drop(second_command);
        };
        tokio::join!(f.manager.stop(&f.vault, f.env.as_ref(), &conv), ends);
        assert_eq!(next.ended().await, AiTurnEndReason::Stopped);
    };
    tokio::join!(f.manager.stop(&f.vault, f.env.as_ref(), &conv), later);
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert!(!f.manager.is_running(&conv));
}

/// AI-18: locking the vault stops the turn without waiting for a running command, so its result
/// says the command may still run.
#[tokio::test]
async fn a_lock_while_a_command_runs_says_it_may_still_run() {
    let f = Fixture::new(vec![calls(&[(
        "c1",
        "run_command",
        json!({"command": "sleep 300"}),
    )])])
    .await;
    let (started, sink) = f.send(None, "run it").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let op = f.manager.start_call(&conv, "c1", OpKind::Command).unwrap();
    f.manager.stop_all(&mut lock(&f.vault));
    assert!(op.token().is_cancelled());
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert_eq!(
        tool_results(&f.entries(&conv)),
        [(
            "c1",
            ToolStatus::Cancelled,
            hatoba_ai::tools::COMMAND_STOP_UNCONFIRMED
        )]
    );
}

#[tokio::test]
async fn deleting_a_conversation_stops_its_turn() {
    let f = Fixture::new(vec![calls(&[("call_1", "read_terminal", json!({}))])]).await;
    let (started, sink) = f.send(None, "look").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    remember_a_file(&f, &conv, "call_1");
    f.manager
        .delete_conversation(&f.vault, f.env.as_ref(), &conv)
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    // What the conversation read of files (AI-38…40) goes with it.
    assert!(!holds_a_file_base(&f, &conv));
    assert!(!f.manager.0.seen.knows(&conv));
    assert!(!f.manager.is_running(&conv));
    assert_eq!(
        f.manager
            .conversation_detail(&f.vault, &conv)
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert!(lock(&f.vault).ai_entries(&conv).unwrap().is_empty());
}

#[test]
fn the_lock_cancels_provider_requests_outside_a_conversation() {
    let manager = AiManager::default();
    let op = manager.background_op();
    drop(manager.background_op());
    // A conversation's stop does not reach it.
    let _ = manager.cancel("0190a0a0-0000-7000-8000-000000000000");
    assert!(!op.token().is_cancelled());
    let mut vault = Vault::open_in_memory().unwrap();
    manager.stop_all(&mut vault);
    assert!(op.token().is_cancelled());
    drop(op);
    assert!(crate::ai::guard(&manager.0.ops).is_empty());
}

// ────────────────── the prompt's context, notes, instructions (§13.1, §13.3) ──────────────────
