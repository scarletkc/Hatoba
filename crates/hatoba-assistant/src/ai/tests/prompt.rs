//! The system prompt (§13.1) and the `permission_mode` notes (AI-16).

use super::*;

#[tokio::test]
async fn the_prompt_names_the_model_and_the_server_and_carries_instructions_and_host_notes() {
    let f = Fixture::new(vec![answer("One."), answer("Two."), answer("Three.")]).await;
    let host = f.add_host(
        "prod-db",
        "PostgreSQL 16 primary.\nAsk before <restarting> it.",
    );
    f.set_instructions("Answer in English.");
    let context = AiTurnContext {
        host_id: Some(host.clone()),
        session_id: Some(LIVE_SESSION.into()),
        ..f.context()
    };
    let (conv, reason) = f.exchange(None, "status?", context.clone()).await;
    assert_eq!(reason, AiTurnEndReason::Completed);
    let system = system_of(&f.requests().await[0]);
    for needle in [
        "You are the model \"Model 1\" (m1), which this request asks for through the provider \
         \"Mock\".\n",
        "The terminal tab is connected to the host \"prod-db\" as the user \"ops\".\n\
         The SSH server identifies itself as \"SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13.5\".\n\
         </context>",
        "- Follow the user's instructions and the host notes below; where they differ",
        "</rules>\n\n<user_instructions>\nAnswer in English.\n</user_instructions>\n\n\
         <host_notes host=\"prod-db\">\nPostgreSQL 16 primary.\nAsk before <restarting> it.\n\
         </host_notes>\n\n<attachments>",
    ] {
        assert!(system.contains(needle), "missing {needle:?} in {system}");
    }

    // Without a connected tab there is no server to name, and the session is not asked.
    let detached = AiTurnContext {
        tab: false,
        ..context.clone()
    };
    f.exchange(Some(&conv), "and now?", detached).await;
    let system = system_of(&f.requests().await[1]);
    assert!(!system.contains("SSH server"), "{system}");
    assert!(system.contains("<host_notes host=\"prod-db\">"), "{system}");

    // A session that is not open names no server; nothing else changes.
    let closed = AiTurnContext {
        session_id: Some("gone".into()),
        ..context
    };
    f.exchange(Some(&conv), "again", closed).await;
    let system = system_of(&f.requests().await[2]);
    assert!(!system.contains("SSH server"), "{system}");
    assert!(system.contains("<user_instructions>"), "{system}");
}

#[tokio::test]
async fn a_switch_of_permission_mode_is_noted_once_and_keeps_the_system_prompt() {
    use hatoba_ai::tools::{PermissionMode, permission_mode_block};

    let f = Fixture::new((0..5).map(|i| answer(&format!("Answer {i}."))).collect()).await;
    let bypass = || AiTurnContext {
        permission_mode: AiPermissionMode::Bypass,
        ..f.context()
    };
    // §13.5: the first message names its mode, manual approval too, since the default is the
    // user's setting, and only a switch adds another note.
    let (manual, _) = f.exchange(None, "hello", f.context()).await;
    assert_eq!(
        f.user_texts(&manual),
        [format!(
            "{}hello",
            permission_mode_block(PermissionMode::Manual)
        )]
    );
    let (conv, _) = f.exchange(None, "first", bypass()).await;
    f.exchange(Some(&conv), "second", bypass()).await;
    f.exchange(Some(&conv), "third", f.context()).await;
    f.exchange(Some(&conv), "fourth", f.context()).await;
    assert_eq!(
        f.user_texts(&conv),
        [
            format!("{}first", permission_mode_block(PermissionMode::Bypass)),
            "second".to_owned(),
            format!("{}third", permission_mode_block(PermissionMode::Manual)),
            "fourth".to_owned(),
        ]
    );
    // The system prompt never changes with the mode, so the provider's cache keeps serving it.
    let requests = f.requests().await;
    let first = system_of(&requests[0]);
    assert!(requests.iter().all(|r| system_of(r) == first));
    assert!(
        first.contains(
            "\n- Permission mode: the newest <permission_mode> note says which mode applies.\n"
        ),
        "{first}"
    );
}

#[tokio::test]
async fn a_summary_carries_the_permission_mode_past_a_compaction() {
    use hatoba_ai::tools::{PermissionMode, permission_mode_block};

    let f = Fixture::new(vec![
        answer("First answer."),
        answer("SUMMARY: the disk is full."),
        answer("Next answer."),
    ])
    .await;
    let bypass = AiTurnContext {
        permission_mode: AiPermissionMode::Bypass,
        ..f.context()
    };
    let (conv, _) = f.exchange(None, "first question", bypass.clone()).await;
    let summary = f
        .manager
        .compact(&f.vault, f.env.as_ref(), &conv, &bypass, None)
        .await
        .unwrap();
    // The model sees nothing before a summary, so the summary keeps the mode it was told of.
    let note = permission_mode_block(PermissionMode::Bypass);
    let AiEntryView::Summary { text, .. } = &summary else {
        panic!("expected a summary, got {summary:?}");
    };
    assert_eq!(text, &format!("{note}SUMMARY: the disk is full."));
    // So the next message in bypass needs no note of its own, and the request starts the summary's
    // message with it, outside the summary's block.
    f.exchange(Some(&conv), "next question", bypass).await;
    assert_eq!(f.user_texts(&conv).last().unwrap(), "next question");
    let last = f.requests().await.pop().unwrap();
    let carried = user_messages(&last);
    assert!(
        carried[0].starts_with(&format!("{note}This conversation continues")),
        "{carried:?}"
    );
}

#[test]
fn the_mode_last_noted_reads_back_auto_too() {
    use hatoba_ai::tools::{PermissionMode, permission_mode_block};

    for mode in PermissionMode::ALL {
        let note = permission_mode_block(mode);
        let noted = vec![
            AiEntry::user(0, "first"),
            AiEntry::user(1, format!("{note}second")),
        ];
        assert_eq!(crate::ai::notes::noted_permission(&noted), Some(mode));
        // AI-47: a summary carries the mode, whichever it is.
        let summarized = vec![AiEntry::summary(2, format!("{note}SUMMARY"))];
        assert_eq!(crate::ai::notes::noted_permission(&summarized), Some(mode));
    }
    // None before any note, and after a summary without one, as in a conversation from before
    // every first message named its mode: its next message names it.
    assert_eq!(crate::ai::notes::noted_permission(&[]), None);
    let old = vec![AiEntry::user(0, "first"), AiEntry::summary(1, "SUMMARY")];
    assert_eq!(crate::ai::notes::noted_permission(&old), None);
}
