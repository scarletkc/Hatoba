//! File changes (AI-41): their snapshots, reverting them, and the `file_revert` notes of the next
//! message.

use super::*;

/// The change `call` of the response `response_id` makes, as the file tools build it.
fn a_change(path: &str, tool: &str) -> crate::ai::files::Change {
    let created = tool == "write_file";
    crate::ai::files::Change {
        meta: crate::ai::files::ChangeMeta {
            server: "ops@db:22".into(),
            path: path.into(),
            tool: tool.into(),
            created,
            mode: None,
        },
        content: crate::ai::files::ChangeContent {
            before: (!created).then(|| "old\n".to_owned()),
            after: "new\n".into(),
        },
    }
}

fn keeper<'a>(
    f: &'a Fixture,
    conv: &'a str,
    response_id: &'a str,
    call: &'a str,
) -> crate::ai::changes::KeepSnapshot<'a> {
    crate::ai::changes::KeepSnapshot::new(&f.vault, revert_key(conv, response_id, call))
}

/// A change `call` of the response `response_id` made, kept as its snapshot (AI-41): the file tools
/// keep one before they write, which needs a live SSH session, and mark it written after.
fn keep_a_change(f: &Fixture, conv: &str, response_id: &str, call: &str, path: &str, tool: &str) {
    use crate::ai::files::Keep as _;
    let keep = keeper(f, conv, response_id, call);
    keep.keep(&a_change(path, tool)).unwrap();
    keep.ended(Some(path));
}

fn revert_key<'a>(
    conv: &'a str,
    response_id: &'a str,
    call: &'a str,
) -> hatoba_core::SnapshotKey<'a> {
    hatoba_core::SnapshotKey {
        conversation_id: conv,
        response_id,
        tool_call_id: call,
    }
}

/// AI-41: the changes of the file tools are listed for Revert, which waits until the conversation
/// is idle and needs the tab connected; the next message notes each revert once, oldest call
/// first, and an edit of that message notes them again. A side question (AI-42) carries the notes
/// without using them up.
#[tokio::test]
async fn reverted_file_changes_are_noted_once_on_the_next_message() {
    let f = Fixture::new(vec![
        calls(&[
            (
                "e1",
                "edit_file",
                json!({"path": "app.conf", "old_string": "old", "new_string": "new"}),
            ),
            (
                "w1",
                "write_file",
                json!({"path": "/srv/new.conf", "content": "new\n"}),
            ),
        ]),
        answer("Changed both."),
        answer("You reverted both."),
        answer("Noted."),
        answer("Fine."),
        answer("Noted again."),
    ])
    .await;
    let (started, sink) = f.send(None, "change them").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let response = f
        .detail(&conv)
        .entries
        .iter()
        .find(|e| matches!(e, AiEntryView::Assistant { .. }))
        .map(|e| entry_id(e).to_owned())
        .unwrap();
    keep_a_change(&f, &conv, &response, "w1", "/srv/new.conf", "write_file");
    keep_a_change(
        &f,
        &conv,
        &response,
        "e1",
        "/home/ops/app.conf",
        "edit_file",
    );
    let revert = |call: &'static str| {
        let response = response.clone();
        let conv = conv.clone();
        let f = &f;
        async move {
            f.manager
                .file_revert(
                    &f.vault,
                    f.env.as_ref(),
                    revert_key(&conv, &response, call),
                    Some("tab-1"),
                    false,
                )
                .await
        }
    };

    // Not while the turn runs.
    assert_eq!(
        revert("e1").await.unwrap_err().code,
        ErrorCode::InvalidInput
    );
    f.result(&conv, "e1", AiToolStatus::Ok, "Edited app.conf", None)
        .await
        .unwrap();
    f.result(&conv, "w1", AiToolStatus::Ok, "Created /srv/new.conf", None)
        .await
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;

    let listed = |f: &Fixture| {
        f.detail(&conv)
            .file_changes
            .into_iter()
            .map(|c| (c.tool_call_id, c.path, c.created, c.state))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        listed(&f),
        [
            (
                "w1".into(),
                "/srv/new.conf".into(),
                true,
                AiFileChangeState::Written
            ),
            (
                "e1".into(),
                "/home/ops/app.conf".into(),
                false,
                AiFileChangeState::Written
            ),
        ]
    );
    assert_eq!(f.manager.file_changes(&f.vault, &conv).unwrap().len(), 2);
    // No connected tab: nothing happens, and the change can still be reverted later.
    assert_eq!(
        revert("e1").await.unwrap(),
        AiFileRevert::Failed {
            reason: AiRevertFailure::Disconnected,
            detail: None
        }
    );
    assert_eq!(revert("nope").await.unwrap_err().code, ErrorCode::NotFound);
    assert_eq!(listed(&f)[1].3, AiFileChangeState::Written);

    // Reverted on the host (the scripts' part needs SSH): recorded, and not offered again.
    for call in ["e1", "w1"] {
        assert!(
            lock(&f.vault)
                .ai_mark_file_reverted(&revert_key(&conv, &response, call))
                .unwrap()
        );
    }
    assert!(
        listed(&f)
            .iter()
            .all(|c| c.3 == AiFileChangeState::Reverted)
    );
    assert_eq!(
        revert("e1").await.unwrap_err().code,
        ErrorCode::InvalidInput
    );

    // The next message notes both, oldest call first, before what the user typed.
    let notes = format!(
        "{}{}",
        hatoba_ai::tools::file_revert_block("/home/ops/app.conf", "e1", "edit_file", false),
        hatoba_ai::tools::file_revert_block("/srv/new.conf", "w1", "write_file", true),
    );
    // AI-42: a side question asked meanwhile starts with them too, and leaves them for the message.
    let (answer, _) = f.side(&conv, "what did I undo?", None).await;
    assert_eq!(answer.unwrap().text, "You reverted both.");
    assert_eq!(
        last_user_message(&f.requests().await[2]),
        format!("{notes}{}", side_message("what did I undo?"))
    );
    let (second, sink) = f.send(Some(&conv), "why did you change them?").await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    let requests = f.requests().await;
    assert_eq!(
        last_user_message(&requests[3]),
        format!("{notes}why did you change them?")
    );
    assert_eq!(
        crate::ai::entries::title_of(&format!("{notes}typed")),
        "typed"
    );
    assert_eq!(
        crate::ai::notes::noted_reverts(&format!("{notes}typed")),
        ["e1", "w1"]
    );

    // Noted once.
    let (_, sink) = f.send(Some(&conv), "ok").await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    assert_eq!(last_user_message(&f.requests().await[4]), "ok");

    // An edit of the message that noted them deletes the note with it, so the new one notes
    // them again.
    let (_, sink) = f
        .edit(&conv, entry_id(&second.user_entry), "why?")
        .await
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    assert_eq!(
        last_user_message(&f.requests().await[5]),
        format!("{notes}why?")
    );
}

/// AI-26, AI-41: deleting the response that made a call, by an edit, deletes the call's snapshot,
/// and the edited message notes no revert of a call it deleted, as when the user reverted the
/// change before sending the edit.
#[tokio::test]
async fn an_edit_before_a_call_deletes_its_snapshot() {
    let f = Fixture::new(vec![
        calls(&[
            (
                "e1",
                "edit_file",
                json!({"path": "app.conf", "old_string": "old", "new_string": "new"}),
            ),
            (
                "w1",
                "write_file",
                json!({"path": "/srv/new.conf", "content": "new\n"}),
            ),
        ]),
        answer("Edited."),
        answer("Fresh start."),
    ])
    .await;
    let (started, sink) = f.send(None, "edit it").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let response = f
        .detail(&conv)
        .entries
        .iter()
        .find(|e| matches!(e, AiEntryView::Assistant { .. }))
        .map(|e| entry_id(e).to_owned())
        .unwrap();
    keep_a_change(
        &f,
        &conv,
        &response,
        "e1",
        "/home/ops/app.conf",
        "edit_file",
    );
    keep_a_change(&f, &conv, &response, "w1", "/srv/new.conf", "write_file");
    f.result(&conv, "e1", AiToolStatus::Ok, "Edited", None)
        .await
        .unwrap();
    f.result(&conv, "w1", AiToolStatus::Ok, "Created", None)
        .await
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    let listed = f.detail(&conv).file_changes;
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().all(|c| c.server == "ops@db:22"));
    // The panel reverted one before it sent the edit (the scripts' part needs SSH).
    assert!(
        lock(&f.vault)
            .ai_mark_file_reverted(&revert_key(&conv, &response, "w1"))
            .unwrap()
    );

    let (_, sink) = f
        .edit(&conv, entry_id(&started.user_entry), "never mind")
        .await
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    assert!(f.detail(&conv).file_changes.is_empty());
    let requests = f.requests().await;
    assert_eq!(
        last_user_message(requests.last().unwrap()),
        manual("never mind")
    );
}

/// AI-41: a write keeps its snapshot before it begins. It is written once the write ends (with the
/// path as the write resolved it), deleted when the write never began, and stays unfinished when
/// the write failed after it may have begun, or the lock came first: those can be reverted too.
/// A snapshot the size limit dropped says so, and cannot be reverted.
#[tokio::test]
async fn writes_keep_their_snapshot_before_they_begin() {
    use crate::ai::files::Keep as _;
    let f = Fixture::new(vec![
        calls(&[
            (
                "w1",
                "write_file",
                json!({"path": "new.conf", "content": "x"}),
            ),
            (
                "w2",
                "write_file",
                json!({"path": "/srv/a", "content": "x"}),
            ),
            (
                "w3",
                "write_file",
                json!({"path": "/srv/b", "content": "x"}),
            ),
            ("e1", "edit_file", json!({"path": "/srv/b"})),
        ]),
        answer("Done."),
    ])
    .await;
    let (started, sink) = f.send(None, "write them").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let response = f
        .detail(&conv)
        .entries
        .iter()
        .find(|e| matches!(e, AiEntryView::Assistant { .. }))
        .map(|e| entry_id(e).to_owned())
        .unwrap();
    let states = |f: &Fixture| {
        f.detail(&conv)
            .file_changes
            .into_iter()
            .map(|c| (c.tool_call_id, c.path, c.state))
            .collect::<Vec<_>>()
    };

    // Kept before the write: unfinished until it ends, then written at the resolved path.
    let w1 = keeper(&f, &conv, &response, "w1");
    w1.keep(&a_change("new.conf", "write_file")).unwrap();
    assert_eq!(
        states(&f),
        [(
            "w1".into(),
            "new.conf".into(),
            AiFileChangeState::Unfinished
        )]
    );
    w1.ended(Some("/home/ops/new.conf"));
    // A write that never began (refused, or the file changed before it) deletes it again; one that
    // failed after it may have begun leaves it unfinished.
    let w2 = keeper(&f, &conv, &response, "w2");
    w2.keep(&a_change("/srv/a", "write_file")).unwrap();
    w2.ended(None);
    keeper(&f, &conv, &response, "w3")
        .keep(&a_change("/srv/b", "write_file"))
        .unwrap();
    assert_eq!(
        states(&f),
        [
            (
                "w1".into(),
                "/home/ops/new.conf".into(),
                AiFileChangeState::Written
            ),
            ("w3".into(), "/srv/b".into(), AiFileChangeState::Unfinished),
        ]
    );

    // A later call wrote the same file: a revert of the earlier one says so.
    keep_a_change(&f, &conv, &response, "e1", "/srv/b", "edit_file");
    let meta = |path: &str| crate::ai::files::ChangeMeta {
        server: "ops@db:22".into(),
        path: path.into(),
        tool: "write_file".into(),
        created: true,
        mode: None,
    };
    {
        let v = lock(&f.vault);
        assert!(crate::ai::changes::later_writes(
            &v,
            &revert_key(&conv, &response, "w3"),
            &meta("/srv/b")
        ));
        assert!(!crate::ai::changes::later_writes(
            &v,
            &revert_key(&conv, &response, "e1"),
            &meta("/srv/b")
        ));
        assert!(!crate::ai::changes::later_writes(
            &v,
            &revert_key(&conv, &response, "w1"),
            &meta("/home/ops/new.conf")
        ));
    }
    // Reverted, the later change no longer stands in the way.
    assert!(
        lock(&f.vault)
            .ai_mark_file_reverted(&revert_key(&conv, &response, "e1"))
            .unwrap()
    );
    assert!(!crate::ai::changes::later_writes(
        &lock(&f.vault),
        &revert_key(&conv, &response, "w3"),
        &meta("/srv/b")
    ));

    // The lock before the write begins stops the call; nothing is kept.
    lock(&f.vault).lock();
    assert!(matches!(
        keeper(&f, &conv, &response, "w2").keep(&a_change("/srv/a", "write_file")),
        Err(crate::ai::files::Stop::Cancelled)
    ));
    lock(&f.vault).unlock(PW).unwrap();
    assert_eq!(states(&f).len(), 3);
    // Within the limit, nothing is dropped, so the panel reads no other conversation again.
    let small = keeper(&f, &conv, &response, "w5");
    small.keep(&a_change("/srv/d", "write_file")).unwrap();
    assert!(!small.dropped());
    small.ended(None);

    // The size limit dropped the oldest: listed as such, and not revertible. (Each change above
    // holds about 30 bytes.)
    let big = "x".repeat(usize::try_from(hatoba_core::AI_SNAPSHOTS_MAX_BYTES - 40).unwrap());
    lock(&f.vault)
        .ai_put_file_snapshot(&revert_key(&conv, &response, "big"), "{}", &big)
        .unwrap();
    let now: Vec<_> = states(&f)
        .into_iter()
        .map(|(call, _, state)| (call, state))
        .collect();
    assert_eq!(
        now[..3],
        [
            ("w1".to_owned(), AiFileChangeState::Dropped),
            ("w3".to_owned(), AiFileChangeState::Unfinished),
            ("e1".to_owned(), AiFileChangeState::Reverted),
        ]
    );
    f.result(&conv, "w1", AiToolStatus::Ok, "Created", None)
        .await
        .unwrap();
    for call in ["w2", "w3", "e1"] {
        f.result(&conv, call, AiToolStatus::Error, "Failed", None)
            .await
            .unwrap();
    }
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    let error = f
        .manager
        .file_revert(
            &f.vault,
            f.env.as_ref(),
            revert_key(&conv, &response, "w1"),
            Some("tab-1"),
            false,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert!(
        error.detail.contains("no longer be reverted"),
        "{}",
        error.detail
    );
    // The block and the confirmation show what a kept change did; a dropped one has nothing.
    let diff = |call: &str| {
        f.manager
            .file_change_diff(&f.vault, revert_key(&conv, &response, call))
            .unwrap()
    };
    let w3 = diff("w3").unwrap();
    assert_eq!(
        (w3.path.as_str(), w3.before, w3.after.as_deref()),
        ("/srv/b", None, Some("new\n"))
    );
    assert!(diff("w1").is_none());
    assert_eq!(
        f.manager
            .file_change_diff(&f.vault, revert_key(&conv, &response, "nope"))
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    // An unfinished one can be: here it gets as far as the missing session.
    assert_eq!(
        f.manager
            .file_revert(
                &f.vault,
                f.env.as_ref(),
                revert_key(&conv, &response, "w3"),
                None,
                true,
            )
            .await
            .unwrap(),
        AiFileRevert::Failed {
            reason: AiRevertFailure::Disconnected,
            detail: None
        }
    );
    // Keeping a change that goes over the limit drops the oldest kept, and says so.
    let mut large = a_change("/srv/c", "write_file");
    large.content.after = "y".repeat(100);
    let keep = keeper(&f, &conv, &response, "w4");
    keep.keep(&large).unwrap();
    assert!(keep.dropped());
}

/// AI-41: no message is sent and no request retried while a revert of the conversation runs, so
/// the model never acts while the file changes under it, and the next message notes the revert.
/// A call whose id another call of its response shares keeps no snapshot.
#[tokio::test]
async fn no_turn_starts_while_a_revert_runs() {
    let f = Fixture::new(vec![answer("Hello."), answer("Again.")]).await;
    let (started, sink) = f.send(None, "hi").await;
    let conv = started.conversation.id;
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;

    let reverting = crate::ai::changes::Reverting::new(&f.manager.0, &conv);
    let refused = f
        .manager
        .send(
            &f.vault,
            f.env(),
            AiSendInput {
                conversation_id: Some(conv.clone()),
                text: "next".into(),
                context: f.context(),
            },
            Arc::new(Sink::default()),
        )
        .await
        .err()
        .unwrap();
    assert_eq!(refused.code, ErrorCode::InvalidInput);
    assert!(
        refused.detail.contains("being reverted"),
        "{}",
        refused.detail
    );
    assert_eq!(
        f.retry(&conv, f.context()).err().unwrap().code,
        ErrorCode::InvalidInput
    );
    // Nor is the conversation compacted: the summary would come before the revert is recorded.
    let compact = f
        .manager
        .compact(&f.vault, f.env.as_ref(), &conv, &f.context(), None)
        .await
        .unwrap_err();
    assert!(
        compact.detail.contains("being reverted"),
        "{}",
        compact.detail
    );
    // Nothing was stored meanwhile.
    assert_eq!(f.detail(&conv).entries.len(), 2);
    drop(reverting);
    let (_, sink) = f.send(Some(&conv), "next").await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);

    let entries = |calls: &[&str]| crate::ai::entries::Entries {
        ids: vec!["e1".into()],
        list: vec![AiEntry::assistant(
            1,
            AssistantEntry {
                tool_calls: calls
                    .iter()
                    .map(|id| ToolCall {
                        id: (*id).into(),
                        name: "write_file".into(),
                        arguments: "{}".into(),
                    })
                    .collect(),
                finish: Finish::ToolCalls,
                ..AssistantEntry::default()
            },
        )],
    };
    assert!(crate::ai::calls::shares_id(
        &entries(&["dup", "dup", "c3"]),
        "dup"
    ));
    assert!(!crate::ai::calls::shares_id(
        &entries(&["dup", "dup", "c3"]),
        "c3"
    ));

    // A revert note is about the call of the newest response before it with that id, also when
    // an older response used the same id.
    let response = |calls: &[&str]| entries(calls).list.remove(0);
    let list = [
        response(&["call_1"]),
        AiEntry::user(2, "next"),
        response(&["call_1", "call_2"]),
        AiEntry::user(4, "noted"),
    ];
    assert_eq!(
        crate::ai::notes::response_with_call(&list[..3], "call_1"),
        Some(2)
    );
    assert_eq!(
        crate::ai::notes::response_with_call(&list[..2], "call_1"),
        Some(0)
    );
    assert_eq!(
        crate::ai::notes::response_with_call(&list[..3], "call_2"),
        Some(2)
    );
    assert_eq!(
        crate::ai::notes::response_with_call(&list[..1], "call_2"),
        None
    );
}
