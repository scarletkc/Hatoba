//! Waiting for the results of a response's calls: what the wait reads, and how it stops and
//! fails.

use super::*;

/// A turn that no manager runs: the test wakes it itself, as a stored result would.
fn bare_turn(f: &Fixture) -> Turn {
    Turn {
        cancel: CancellationToken::new(),
        sink: Arc::new(Sink::default()),
        context: f.context(),
        server_id: None,
        results: Notify::new(),
        waiting: Notify::new(),
        ended: AtomicBool::new(false),
        streamed: Mutex::new(None),
    }
}

/// A response that calls tools with these ids.
fn response_calling(ids: &[&str]) -> AiEntry {
    AiEntry::assistant(
        0,
        AssistantEntry {
            tool_calls: ids
                .iter()
                .map(|id| ToolCall {
                    id: (*id).into(),
                    name: "read_terminal".into(),
                    arguments: "{}".into(),
                })
                .collect(),
            finish: Finish::ToolCalls,
            ..AssistantEntry::default()
        },
    )
}

/// An entry id after `entry_id`, in the same millisecond, for an entry another device wrote.
fn later_id(entry_id: &str) -> String {
    format!("{}-7fff-bfff-ffffffffffff", &entry_id[..13])
}

/// Starts waiting for the results of `entry_id` and polls the wait once.
async fn wait_once(
    f: &Fixture,
    conversation_id: &str,
    turn: &Turn,
    entry_id: &str,
) -> Poll<Result<(), Halt>> {
    poll_once(&mut Box::pin(wait_for_results(
        &f.vault,
        conversation_id,
        turn,
        entry_id,
    )))
    .await
}

fn parts_read(f: &Fixture) -> usize {
    lock(&f.vault).ai_parts_read()
}

#[tokio::test]
async fn waiting_for_results_reads_only_the_response_and_what_follows_it() {
    let f = Fixture::with_base_url(None, "http://127.0.0.1:9/v1".into());
    let conv = lock(&f.vault)
        .put(None, Item::AiConversation(AiConversation::default()))
        .unwrap();
    // A long history, every tenth answer in several parts. Its last response called `dup` too;
    // that result must not answer the new call.
    {
        let mut v = lock(&f.vault);
        let long = "x".repeat(100_000);
        for i in 0..100 {
            append(&mut v, &conv, &AiEntry::user(0, format!("question {i}"))).unwrap();
            let text = if i % 10 == 0 {
                long.clone()
            } else {
                format!("answer {i}")
            };
            let reply = AssistantEntry {
                text,
                ..AssistantEntry::default()
            };
            append(&mut v, &conv, &AiEntry::assistant(0, reply)).unwrap();
        }
        append(&mut v, &conv, &response_calling(&["dup"])).unwrap();
        append(
            &mut v,
            &conv,
            &AiEntry::tool(0, "dup", ToolStatus::Ok, "old"),
        )
        .unwrap();
    }
    // Two calls that share an id (stored before such calls got error results), and one more.
    let entry_id = append(
        &mut lock(&f.vault),
        &conv,
        &response_calling(&["dup", "dup", "other"]),
    )
    .unwrap();
    let history = lock(&f.vault)
        .items()
        .filter(|(_, i)| i.as_ai_message().is_some())
        .count();
    assert!(history > 200, "{history} parts");

    let turn = bare_turn(&f);
    let mut wait = Box::pin(wait_for_results(&f.vault, &conv, &turn, &entry_id));
    // Every call is open: the check reads the response and nothing before it.
    let before = parts_read(&f);
    assert!(poll_once(&mut wait).await.is_pending());
    assert_eq!(parts_read(&f) - before, 1);

    // Another device's result for `dup` arrives in part, then this device stores one: a `dup`
    // and `other` have no complete result yet, and the partial entry is not read.
    let late = later_id(&entry_id);
    let late_result = AiEntry::tool(0, "dup", ToolStatus::Ok, "from elsewhere");
    arrive(&f, &conv, &late, &late_result, 0);
    append(
        &mut lock(&f.vault),
        &conv,
        &AiEntry::tool(0, "dup", ToolStatus::Ok, "here"),
    )
    .unwrap();
    turn.results.notify_one();
    let before = parts_read(&f);
    assert!(poll_once(&mut wait).await.is_pending());
    assert_eq!(parts_read(&f) - before, 2);

    // The rest arrives and `other` gets its result: every call is answered.
    arrive(&f, &conv, &late, &late_result, 1);
    append(
        &mut lock(&f.vault),
        &conv,
        &AiEntry::tool(0, "other", ToolStatus::Ok, "done"),
    )
    .unwrap();
    turn.results.notify_one();
    let before = parts_read(&f);
    assert!(matches!(poll_once(&mut wait).await, Poll::Ready(Ok(()))));
    assert_eq!(parts_read(&f) - before, 5);
}

#[tokio::test]
async fn waiting_for_results_stops_and_fails_as_before() {
    const GONE: &str = "The model's response is no longer stored.";
    let f = Fixture::with_base_url(None, "http://127.0.0.1:9/v1".into());
    let conv = lock(&f.vault)
        .put(None, Item::AiConversation(AiConversation::default()))
        .unwrap();
    let question = append(&mut lock(&f.vault), &conv, &AiEntry::user(0, "q")).unwrap();
    let entry_id = append(&mut lock(&f.vault), &conv, &response_calling(&["call_1"])).unwrap();

    // Stopped before the check, and while it waits.
    let turn = bare_turn(&f);
    turn.cancel.cancel();
    assert!(matches!(
        wait_once(&f, &conv, &turn, &entry_id).await,
        Poll::Ready(Err(Halt::Stopped))
    ));
    let turn = bare_turn(&f);
    let mut wait = Box::pin(wait_for_results(&f.vault, &conv, &turn, &entry_id));
    assert!(poll_once(&mut wait).await.is_pending());
    turn.cancel.cancel();
    assert!(matches!(
        poll_once(&mut wait).await,
        Poll::Ready(Err(Halt::Stopped))
    ));
    drop(wait);

    // A response with a part missing is not stored.
    let turn = bare_turn(&f);
    let partial = later_id(&entry_id);
    arrive(&f, &conv, &partial, &response_calling(&["call_2"]), 0);
    assert!(matches!(
        wait_once(&f, &conv, &turn, &partial).await,
        Poll::Ready(Err(Halt::Failed(m))) if m == GONE
    ));

    // A result stored before the first check is found by it.
    append(
        &mut lock(&f.vault),
        &conv,
        &AiEntry::tool(0, "call_1", ToolStatus::Ok, "ok"),
    )
    .unwrap();
    assert!(matches!(
        wait_once(&f, &conv, &turn, &entry_id).await,
        Poll::Ready(Ok(()))
    ));

    // AI-26: an edit of the question deleted the response.
    lock(&f.vault)
        .ai_delete_entries_from(&conv, &question)
        .unwrap();
    assert!(matches!(
        wait_once(&f, &conv, &turn, &entry_id).await,
        Poll::Ready(Err(Halt::Failed(m))) if m == GONE
    ));

    lock(&f.vault).lock();
    assert!(matches!(
        wait_once(&f, &conv, &turn, &entry_id).await,
        Poll::Ready(Err(Halt::Stopped))
    ));
}
