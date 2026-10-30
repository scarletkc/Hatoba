//! Memories in turns (AI-53): the `memory` notes and the `memory` tool.

use super::*;

/// AI-53: a conversation's first message and each summary start with the global memories and
/// those of its host, and the `memory` tool saves one without changing the system prompt or the
/// tools, so the provider's cache keeps serving them. The saved memory shows from the summary on.
#[tokio::test]
async fn memories_start_the_first_message_and_each_summary() {
    use crate::ai::memory::{Place, Scope};
    let f = Fixture::new(vec![
        calls(&[(
            "m1",
            "memory",
            json!({"action": "save", "scope": "host", "name": "sudo", "content": "Uses doas."}),
        )]),
        answer("Noted."),
        answer("SUMMARY: noted doas."),
        answer("Next answer."),
    ])
    .await;
    let web = f.add_host("web-1", "");
    let on_web = AiTurnContext {
        host_id: Some(web.clone()),
        ..f.context()
    };
    let place = Place {
        scope: Scope::Host(web.clone()),
        name: "web-1".into(),
    };
    crate::ai::memory::save(
        &mut lock(&f.vault),
        &[],
        &Scope::Global,
        "Replies",
        "Answer in Chinese.",
    )
    .unwrap()
    .unwrap();
    let before = crate::ai::memory::note(&lock(&f.vault), Some(&place));
    assert!(
        before.starts_with(
            "<memory entries=\"1\" conversation_host=\"web-1\">\n<entry name=\"Replies\" \
             scope=\"global\" updated=\"20"
        ),
        "{before}"
    );
    let (started, sink) = f.send_with(None, "web-1 uses doas", on_web.clone()).await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let m1 = f.run(&conv, "m1", None, None).await.unwrap();
    assert_eq!(
        view_content(&m1),
        (
            "m1",
            AiToolStatus::Ok,
            "Saved the host memory \"sudo\" of web-1."
        )
    );
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;

    assert_eq!(
        f.user_texts(&conv),
        [manual(&format!("{before}web-1 uses doas"))]
    );
    let requests = f.requests().await;
    assert_eq!(system_of(&requests[1]), system_of(&requests[0]));
    assert_eq!(tool_names(&requests[1]), tool_names(&requests[0]));
    assert!(system_of(&requests[0]).contains("- Memory: the memory tool keeps notes"));

    let summary = f
        .manager
        .compact(&f.vault, f.env.as_ref(), &conv, &on_web, None)
        .await
        .unwrap();
    let AiEntryView::Summary { text, .. } = &summary else {
        panic!("expected a summary, got {summary:?}");
    };
    let after = crate::ai::memory::note(&lock(&f.vault), Some(&place));
    assert!(
        after.contains("<entry name=\"sudo\" scope=\"host\" host=\"web-1\" updated=\"20"),
        "{after}"
    );
    assert_eq!(text, &manual(&format!("{after}SUMMARY: noted doas.")));

    // The message after the summary has no note of its own, and search skips the note's text.
    let (_, sink) = f.send_with(Some(&conv), "next question", on_web).await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    assert_eq!(f.user_texts(&conv)[1], "next question");
    // The notes start the message that carries the summary, outside its block.
    let requests = f.requests().await;
    let continued = messages(requests.last().unwrap())
        .iter()
        .find_map(|m| {
            m["content"]
                .as_str()
                .filter(|c| c.contains("This conversation continues"))
        })
        .unwrap();
    assert!(
        continued.starts_with(&manual(&format!("{after}This conversation continues"))),
        "{continued}"
    );
    let hits = crate::ai::search(&f.vault, "Chinese").unwrap();
    assert!(hits.is_empty(), "{hits:?}");
}

/// AI-53: with memory off, conversations start without the note and the tool is not offered.
#[tokio::test]
async fn memory_off_sends_no_note_and_offers_no_tool() {
    let f = Fixture::new(vec![answer("Hi.")]).await;
    {
        let mut v = lock(&f.vault);
        crate::ai::memory::save(
            &mut v,
            &[],
            &crate::ai::memory::Scope::Global,
            "Replies",
            "Answer in Chinese.",
        )
        .unwrap()
        .unwrap();
        let mut settings = v.settings();
        settings.ai.memory = false;
        v.put(Some(SETTINGS_ID), Item::Settings(settings)).unwrap();
    }
    let (conv, _) = f.exchange(None, "hello", f.context()).await;
    assert_eq!(f.user_texts(&conv), [manual("hello")]);
    let requests = f.requests().await;
    assert!(!tool_names(&requests[0]).contains(&"memory".to_owned()));
    assert!(!system_of(&requests[0]).contains("- Memory:"));
}

/// AI-53: a message on a host or computer whose memories no note of the context showed starts with
/// a note of just those. A host the context showed already, one without memories and a
/// quick-connect target get none, and the global memories show once.
#[tokio::test]
async fn a_message_on_another_host_shows_its_memories() {
    use crate::ai::memory::{Place, Scope, note, place_note, save};
    use crate::dto::LocalShellKind;
    use hatoba_ai::tools::host_change_block;

    let f = Fixture::new((0..6).map(|i| answer(&format!("Answer {i}."))).collect()).await;
    let web = f.add_host("web-1", "");
    let db = f.add_host("db-1", "");
    let bare = f.add_host("bare", "");
    let at = |scope: Scope, name: &str| Place {
        scope,
        name: name.into(),
    };
    let (on_web, on_db, on_local) = (
        at(Scope::Host(web.clone()), "web-1"),
        at(Scope::Host(db.clone()), "db-1"),
        at(Scope::Local("DEMO-PC".into()), "DEMO-PC (local terminal)"),
    );
    {
        let mut v = lock(&f.vault);
        for (scope, name) in [
            (&Scope::Global, "Replies"),
            (&on_web.scope, "sudo"),
            (&on_db.scope, "postgres"),
            (&on_local.scope, "shell"),
        ] {
            save(&mut v, &[], scope, name, "x").unwrap().unwrap();
        }
    }
    let on = |host: &str| AiTurnContext {
        host_id: Some(host.to_owned()),
        ..f.context()
    };
    let (conv, _) = f.exchange(None, "one", on(&web)).await;
    f.exchange(Some(&conv), "two", on(&db)).await;
    f.exchange(Some(&conv), "three", on(&web)).await;
    f.exchange(Some(&conv), "four", on(&bare)).await;
    let local = f.local("DEMO-PC", "PowerShell", LocalShellKind::Powershell);
    f.exchange(Some(&conv), "five", local).await;
    f.exchange(Some(&conv), "six", f.quick("10.0.0.9")).await;

    let (web_note, db_note, local_note) = {
        let v = lock(&f.vault);
        (
            note(&v, Some(&on_web)),
            place_note(&v, &on_db),
            place_note(&v, &on_local),
        )
    };
    assert!(db_note.starts_with("<memory entries=\"1\" conversation_host=\"db-1\">"));
    assert_eq!(
        f.user_texts(&conv),
        [
            manual(&format!("{web_note}one")),
            format!("{}{db_note}two", host_change_block("web-1", "db-1")),
            format!("{}three", host_change_block("db-1", "web-1")),
            format!("{}four", host_change_block("web-1", "bare")),
            format!(
                "{}{local_note}five",
                host_change_block("bare", "DEMO-PC (local terminal)")
            ),
            format!(
                "{}six",
                host_change_block("DEMO-PC (local terminal)", "root@10.0.0.9:22")
            ),
        ]
    );
}

/// AI-53: a compaction before a message leaves the memories its summary showed, of where the
/// conversation was, so the message brings those of where it goes, in their place among the
/// notes, unless a note since the summary showed them, and drops its own when the summary shows
/// them.
#[test]
fn a_message_after_a_compaction_brings_the_memories_of_its_host() {
    use crate::ai::memory::{Place, Scope, place_note, save};
    use hatoba_ai::tools::{PermissionMode, permission_mode_block};

    let mut v = Vault::open_in_memory().unwrap();
    v.create_with_params("pw", hatoba_core::KdfParams::for_tests())
        .unwrap();
    let web = Place {
        scope: Scope::Host("h1".into()),
        name: "web-1".into(),
    };
    save(&mut v, &[], &web.scope, "sudo", "Uses doas.")
        .unwrap()
        .unwrap();
    let shown = |host: &str| {
        format!(
            "<memory entries=\"1\" conversation_host=\"{host}\">\n<entry name=\"x\" \
             scope=\"global\">\ny\n</entry>\n</memory>\n\n"
        )
    };
    let permission = permission_mode_block(PermissionMode::Manual);
    let screen = "<terminal_state host=\"web-1\" alternate_screen=\"false\" lines=\"1\">\n$\n\
                  </terminal_state>\n\n";
    let notes = format!("{permission}{screen}");
    let summary = |host: &str| AiEntry::summary(1, format!("{}SUMMARY", shown(host)));

    // The first message showed web-1's memories, but the summary those of db-1.
    let earlier = [
        AiEntry::user(0, format!("{}hello", shown("web-1"))),
        summary("db-1"),
    ];
    let note = place_note(&v, &web);
    assert!(!note.is_empty());
    assert_eq!(
        crate::ai::notes::with_place_memories(&v, &earlier, &notes, Some(&web)),
        format!("{permission}{note}{screen}")
    );
    // A note since the summary, or the summary itself, showed them; or the notes have one.
    for earlier in [
        vec![
            summary("db-1"),
            AiEntry::user(2, format!("{}more", shown("web-1"))),
        ],
        vec![summary("web-1")],
    ] {
        assert_eq!(
            crate::ai::notes::with_place_memories(&v, &earlier, &notes, Some(&web)),
            notes
        );
    }
    // Notes stored before conversation_host name the conversation's host in host.
    let old = |host: &str| shown(host).replacen("conversation_host=", "host=", 1);
    for earlier in [
        vec![AiEntry::summary(1, format!("{}SUMMARY", old("web-1")))],
        vec![
            summary("db-1"),
            AiEntry::user(2, format!("{}more", old("web-1"))),
        ],
    ] {
        assert_eq!(
            crate::ai::notes::with_place_memories(&v, &earlier, &notes, Some(&web)),
            notes
        );
    }
    let noted = format!("{permission}{note}{screen}");
    assert_eq!(
        crate::ai::notes::with_place_memories(&v, &[summary("db-1")], &noted, Some(&web)),
        noted
    );
    // A message that showed them before the compaction leaves them to the summary.
    assert_eq!(
        crate::ai::notes::with_place_memories(&v, &[summary("web-1")], &noted, Some(&web)),
        notes
    );
    assert_eq!(
        crate::ai::notes::with_place_memories(&v, &[summary("db-1")], &notes, None),
        notes
    );
}

/// AI-53: the `memory` tool's host scope is the conversation's host or computer, and a
/// conversation on neither keeps only global memories.
#[tokio::test]
async fn the_memory_tool_saves_host_memories_of_the_conversations_host() {
    let save = |id: &str, scope: &str, name: &str| {
        calls(&[(
            id,
            "memory",
            json!({"action": "save", "scope": scope, "name": name, "content": "x"}),
        )])
    };
    let f = Fixture::new(vec![
        save("m1", "host", "sudo"),
        answer("Saved."),
        save("m2", "host", "sudo"),
        answer("Not saved."),
    ])
    .await;
    let web = f.add_host("web-1", "");
    let on_web = AiTurnContext {
        host_id: Some(web.clone()),
        ..f.context()
    };
    let (started, sink) = f.send_with(None, "remember", on_web).await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    f.run(&conv, "m1", None, None).await.unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    let memories = lock(&f.vault).ai_memories();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].1.host_id.as_deref(), Some(web.as_str()));

    let (started, sink) = f.send_with(None, "remember", f.quick("10.0.0.9")).await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let m2 = f.run(&conv, "m2", None, None).await.unwrap();
    assert_eq!(
        view_content(&m2),
        (
            "m2",
            AiToolStatus::Error,
            "This conversation is not on a saved host or a local terminal, so it has no host \
             memories."
        )
    );
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
}
