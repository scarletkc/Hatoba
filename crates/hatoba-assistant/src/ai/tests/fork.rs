//! Forking (AI-43): what a fork copies, when a conversation can be forked, and the host a fork is
//! on.

use super::*;

#[tokio::test]
async fn a_fork_copies_the_entries_up_to_its_point_and_leaves_the_original() {
    let f = Fixture::new(Vec::new()).await;
    let host = f.add_host("prod-db", "");
    // alpha, a response calling c1 and c2, their results, the answer, a summary, beta, its answer.
    let (conv, ids) = f.conversation_of(
        AiConversation {
            title: "Disk usage on prod".into(),
            host_id: Some(host.clone()),
            pinned: true,
            effort: Some(CoreEffort::High),
            created_at: 1,
            ..AiConversation::default()
        },
        &[
            AiEntry::user(10, "alpha"),
            f.response("Checking.", &["c1", "c2"]),
            AiEntry::tool(30, "c1", ToolStatus::Ok, "screen one"),
            AiEntry::tool(31, "c2", ToolStatus::Rejected, "no"),
            f.response("answer alpha", &[]),
            AiEntry::summary(50, "SUMMARY"),
            AiEntry::user(60, "beta"),
            f.response("answer beta", &[]),
        ],
    );
    {
        let mut v = lock(&f.vault);
        let mut c = crate::ai::find_conversation(&v, &conv).unwrap();
        c.context_start = Some(ids[5].clone());
        v.put(Some(&conv), Item::AiConversation(c)).unwrap();
    }
    let original = crate::ai::find_conversation(&lock(&f.vault), &conv).unwrap();
    let before = f.stored(&conv);
    f.manager.0.seen.record(
        &conv,
        &ids[1],
        "ops@prod-db.example.org:22",
        "/etc/hosts",
        b"x",
    );
    let changes = f.env.changes.load(Ordering::SeqCst);

    // From the response that called tools: its results come with it.
    let fork = f
        .fork(&conv, Some(&ids[1]), "  Disk usage on prod (fork)  ")
        .unwrap();
    assert_eq!(f.env.changes.load(Ordering::SeqCst), changes + 1);
    assert_eq!(fork.title, "Disk usage on prod (fork)");
    assert_eq!(fork.host_id.as_deref(), Some(host.as_str()));
    assert_eq!(fork.effort, Some(AiEffort::High));
    assert!(!fork.pinned);
    // The summary the original's context starts at is after the point, and none is before it.
    assert_eq!(fork.context_start, None);
    let copied = f.stored(&fork.id);
    // The JSON as stored (raw and created_at included), under new, increasing ids.
    assert_eq!(
        copied.iter().map(|(_, json)| json).collect::<Vec<_>>(),
        before[..4].iter().map(|(_, json)| json).collect::<Vec<_>>()
    );
    assert!(copied[0].1.contains("\"created_at\":10"));
    assert!(copied[1].1.contains("RAW-ONLY"));
    assert!(copied.windows(2).all(|w| w[0].0 < w[1].0));
    assert!(copied.iter().all(|(id, _)| !ids.contains(id)));
    // What the model saw of files stays with the original (§13.4).
    assert!(f.manager.0.seen.knows(&conv));
    assert!(!f.manager.0.seen.knows(&fork.id));

    // From the finished reply, and from a user message after the summary, where the fork's
    // context starts.
    let reply = f.fork(&conv, Some(&ids[4]), "reply").unwrap();
    assert_eq!(f.stored(&reply.id).len(), 5);
    assert_eq!(reply.context_start, None);
    let beta = f.fork(&conv, Some(&ids[6]), "beta").unwrap();
    let beta_entries = f.stored(&beta.id);
    assert_eq!(beta_entries.len(), 7);
    assert_eq!(beta.context_start.as_ref(), Some(&beta_entries[5].0));
    assert!(beta_entries[6].1.contains("\"beta\""));
    // History's Fork takes every entry.
    let whole = f.fork(&conv, None, "whole").unwrap();
    let whole_entries = f.stored(&whole.id);
    assert_eq!(
        whole_entries.iter().map(|(_, j)| j).collect::<Vec<_>>(),
        before.iter().map(|(_, j)| j).collect::<Vec<_>>()
    );
    assert_eq!(whole.context_start.as_ref(), Some(&whole_entries[5].0));
    // A long title is cut to 200 characters.
    let long = f.fork(&conv, None, &"t".repeat(250)).unwrap();
    assert_eq!(long.title.chars().count(), 200);

    // The original is as it was.
    assert_eq!(f.stored(&conv), before);
    assert_eq!(
        crate::ai::find_conversation(&lock(&f.vault), &conv).unwrap(),
        original
    );

    // Not from a tool result, a summary, or an unknown entry, and not without a title.
    for at in [&ids[2], &ids[5]] {
        let err = f.fork(&conv, Some(at), "x").err().unwrap();
        assert_eq!(err.field.as_deref(), Some("entry_id"));
    }
    let err = f
        .fork(&conv, Some("0190a0a0-0000-7000-8000-000000000000"), "x")
        .err()
        .unwrap();
    assert_eq!(err.code, ErrorCode::NotFound);
    let err = f.fork(&conv, None, " ").err().unwrap();
    assert_eq!(err.field.as_deref(), Some("title"));
    let err = f.fork("missing", None, "x").err().unwrap();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn a_fork_point_needs_every_call_answered_and_takes_late_results() {
    let f = Fixture::new(Vec::new()).await;
    // c1's result arrived after the user's next message (merged from another device); c2 has
    // none yet.
    let (conv, ids) = f.conversation_of(
        AiConversation::default(),
        &[
            AiEntry::user(1, "first"),
            f.response("Checking.", &["c1"]),
            AiEntry::user(3, "second"),
            AiEntry::tool(4, "c1", ToolStatus::Ok, "late"),
            f.response("Again.", &["c2"]),
        ],
    );
    let fork = f.fork(&conv, Some(&ids[1]), "fork").unwrap();
    let copies = f.entries(&fork.id);
    assert_eq!(copies.len(), 3);
    assert!(matches!(&copies[2].body, EntryBody::Tool { content, .. } if content == "late"));
    let err = f.fork(&conv, Some(&ids[4]), "fork").err().unwrap();
    assert_eq!(err.field.as_deref(), Some("entry_id"));
    // A user message is a fork point whatever follows it, and c1's late result comes along.
    let fork = f.fork(&conv, Some(&ids[2]), "fork").unwrap();
    let copies = f.entries(&fork.id);
    assert_eq!(copies.len(), 4);
    assert!(matches!(&copies[2].body, EntryBody::User { text, .. } if text == "second"));
    assert!(matches!(&copies[3].body, EntryBody::Tool { content, .. } if content == "late"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_conversation_is_not_forked_while_it_is_busy() {
    let f = Fixture::new(vec![
        calls(&[("c1", "read_terminal", json!({}))]),
        answer("slow").set_delay(Duration::from_secs(30)),
    ])
    .await;
    let (started, sink) = f.send(None, "look").await;
    let conv = started.conversation.id;
    let user = entry_id(&started.user_entry).to_owned();
    // The turn waits for the result of its call, as it does for an approval.
    sink.done_with(AiFinish::ToolCalls).await;
    let err = f.fork(&conv, Some(&user), "fork").err().unwrap();
    assert_eq!(err.field.as_deref(), Some("conversation_id"));
    f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    f.idle(&conv).await;
    // A compaction counts too.
    let compaction = f.manager.start_compaction(&conv);
    assert!(f.fork(&conv, None, "fork").is_err());
    drop(compaction);
    // A side question (AI-42) does not: it adds nothing to the conversation.
    let side = f.spawn_side(&conv, 1, "what was that?");
    f.wait_for_requests(2).await;
    let fork = f.fork(&conv, None, "fork").unwrap();
    f.manager.side_stop(&conv, 1);
    assert_eq!(side.await.unwrap().unwrap_err().code, ErrorCode::Cancelled);
    // The stopped call's cancelled result comes with it.
    assert_eq!(f.entries(&fork.id).len(), 3);
}

#[tokio::test]
async fn a_fork_sends_the_copied_context_and_goes_on_by_itself() {
    let f = Fixture::new(vec![answer("One."), answer("Two."), answer("Fork.")]).await;
    let (conv, _) = f.exchange(None, "first", f.context()).await;
    f.exchange(Some(&conv), "second", f.context()).await;
    let first_reply = entry_id(&f.detail(&conv).entries[1]).to_owned();
    let fork = f.fork(&conv, Some(&first_reply), "fork").unwrap();

    let (_, reason) = f.exchange(Some(&fork.id), "elsewhere", f.context()).await;
    assert_eq!(reason, AiTurnEndReason::Completed);
    let request = f.requests().await.pop().unwrap();
    assert_eq!(
        user_messages(&request),
        [manual("first").as_str(), "elsewhere"]
    );
    // The copied reply goes as its raw message (§13.7).
    assert!(request.to_string().contains("RAW-ONLY"));
    assert!(!request.to_string().contains("Two."));
    // Neither conversation sees the other's new entries.
    assert_eq!(f.entries(&conv).len(), 4);
    assert_eq!(f.entries(&fork.id).len(), 4);
}

#[tokio::test]
async fn a_fork_is_on_the_host_its_copies_came_from() {
    use hatoba_ai::tools::host_change_block;

    let f = Fixture::new((0..8).map(|i| answer(&format!("Answer {i}."))).collect()).await;
    let staging = f.add_host("staging-web", "");
    let prod = f.add_host("prod-db", "");
    let on = |host: &str| AiTurnContext {
        host_id: Some(host.to_owned()),
        ..f.context()
    };
    let (conv, _) = f.exchange(None, "first", on(&staging)).await;
    f.exchange(Some(&conv), "second", on(&prod)).await;
    let first_reply = entry_id(&f.detail(&conv).entries[1]).to_owned();
    // Each message records where the entries before it came from.
    let came_from: Vec<Option<EntryPlace>> = f
        .entries(&conv)
        .into_iter()
        .filter_map(|e| match e.body {
            EntryBody::User { came_from, .. } => Some(came_from),
            _ => None,
        })
        .collect();
    let on_staging = EntryPlace {
        host_id: Some(staging.clone()),
        ..EntryPlace::default()
    };
    assert_eq!(came_from, [Some(EntryPlace::default()), Some(on_staging)]);

    // The original moved to prod after the first reply. A fork from that reply is on staging,
    // renamed since, and not on the host that took its old name. Its next message on prod notes
    // the move.
    rename_host(&f, &staging, "staging-old");
    add_host_at(&f, "staging-web", "10.0.0.2");
    let fork = f.fork(&conv, Some(&first_reply), "fork").unwrap();
    assert_eq!(fork.host_id.as_deref(), Some(staging.as_str()));
    assert_eq!(fork.origin, None);
    f.exchange(Some(&fork.id), "again", on(&prod)).await;
    assert_eq!(
        f.user_texts(&fork.id),
        [
            manual("first"),
            format!("{}again", host_change_block("staging-old", "prod-db"))
        ]
    );
    // A fork from after the move is on prod.
    let whole = f.fork(&conv, None, "whole").unwrap();
    assert_eq!(whole.host_id.as_deref(), Some(prod.as_str()));

    // A conversation that moved off a quick connection: the fork records the target.
    let (quick, _) = f.exchange(None, "one", f.quick("10.0.0.9")).await;
    f.exchange(Some(&quick), "two", on(&prod)).await;
    let reply = entry_id(&f.detail(&quick).entries[1]).to_owned();
    let fork = f.fork(&quick, Some(&reply), "fork").unwrap();
    assert_eq!(
        f.recorded_place(&fork.id),
        (None, Some("root@10.0.0.9:22".to_owned()))
    );

    // On a host deleted since, the fork's next message notes the move from it, without a name.
    lock(&f.vault).delete(&staging).unwrap();
    let fork = f.fork(&conv, Some(&first_reply), "fork").unwrap();
    assert_eq!(f.recorded_place(&fork.id), (Some(staging.clone()), None));
    f.exchange(Some(&fork.id), "again", on(&prod)).await;
    assert_eq!(
        f.user_texts(&fork.id)[1],
        format!("{}again", host_change_block("", "prod-db"))
    );
}

#[tokio::test]
async fn a_fork_from_before_its_conversation_had_a_host_has_none() {
    let f = Fixture::new((0..6).map(|i| answer(&format!("Answer {i}."))).collect()).await;
    let prod = f.add_host("prod-db", "");
    let staging = f.add_host("staging-web", "");
    let on = |host: &str| AiTurnContext {
        host_id: Some(host.to_owned()),
        ..f.context()
    };
    // A home tab chat that went on in a tab on prod, as after Continue Here: it took the host
    // without a note.
    let (conv, _) = f.exchange(None, "first", f.context()).await;
    f.exchange(Some(&conv), "second", on(&prod)).await;
    assert_eq!(f.recorded_place(&conv), (Some(prod.clone()), None));
    assert_eq!(f.user_texts(&conv), [manual("first").as_str(), "second"]);
    let first_reply = entry_id(&f.detail(&conv).entries[1]).to_owned();

    // A fork from the home tab reply has no host, and takes the next tab's without a note too.
    let fork = f.fork(&conv, Some(&first_reply), "fork").unwrap();
    assert_eq!(f.recorded_place(&fork.id), (None, None));
    f.exchange(Some(&fork.id), "again", on(&staging)).await;
    assert_eq!(f.user_texts(&fork.id), [manual("first").as_str(), "again"]);
    // A fork from after the move is on prod.
    let whole = f.fork(&conv, None, "whole").unwrap();
    assert_eq!(whole.host_id.as_deref(), Some(prod.as_str()));

    // An edit of the message that took the host keeps what it recorded.
    let second = entry_id(&f.detail(&conv).entries[2]).to_owned();
    f.edit_with(&conv, &second, "second, edited", on(&prod))
        .await;
    let fork = f.fork(&conv, Some(&first_reply), "fork").unwrap();
    assert_eq!(f.recorded_place(&fork.id), (None, None));
}

#[tokio::test]
async fn a_fork_of_an_older_conversation_goes_by_the_name_its_note_gives() {
    use hatoba_ai::tools::host_change_block;

    let f = Fixture::new((0..4).map(|i| answer(&format!("Answer {i}."))).collect()).await;
    let web = f.add_host("web", "");
    let twin = f.add_host("twin", "");
    add_host_at(&f, "twin", "10.0.0.3");
    let on = |host: &str| AiTurnContext {
        host_id: Some(host.to_owned()),
        ..f.context()
    };
    // Messages from before they recorded where the entries before them came from: a fork from
    // before a move goes by the name in the move's note.
    let older = |from: &str| {
        let (conv, ids) = f.conversation_of(
            AiConversation {
                host_id: Some(web.clone()),
                ..AiConversation::default()
            },
            &[
                AiEntry::user(1, "first"),
                f.response("One.", &[]),
                AiEntry::user(3, format!("{}second", host_change_block(from, "web"))),
            ],
        );
        let fork = f.fork(&conv, Some(&ids[1]), "fork").unwrap();
        let item = crate::ai::find_conversation(&lock(&f.vault), &fork.id).unwrap();
        let place = (
            item.host_id,
            item.quick_target
                .as_ref()
                .map(crate::ai::places::target_name),
            item.origin,
        );
        (fork.id, place)
    };
    assert_eq!(older("web").1, (Some(web.clone()), None, None));
    assert_eq!(
        older("root@10.0.0.9:22").1,
        (None, Some("root@10.0.0.9:22".into()), None)
    );
    // A name two hosts share, a host renamed since, and one deleted since, whose name the note
    // left empty: the fork has no host but keeps the name.
    assert_eq!(older("twin").1, (None, None, Some("twin".into())));
    assert_eq!(older("").1, (None, None, Some(String::new())));
    let (fork, place) = older("old-name");
    assert_eq!(place, (None, None, Some("old-name".into())));

    // A message on the home tab keeps the name and records it, so a fork of the fork keeps it.
    f.exchange(Some(&fork), "home", f.context()).await;
    let first_reply = entry_id(&f.detail(&fork).entries[1]).to_owned();
    for again in [
        f.fork(&fork, Some(&first_reply), "again").unwrap(),
        f.fork(&fork, None, "whole").unwrap(),
    ] {
        assert_eq!(again.origin.as_deref(), Some("old-name"));
        assert_eq!(again.host_id, None);
    }
    // AI-42: a side question asked in that tab first starts with the note the next message will
    // have, and leaves the fork where it is.
    let input = AiSideQuestionInput {
        context: on(&twin),
        ..side_input(&f, &fork, 0, "where was this?", None)
    };
    f.manager
        .side_question(&f.vault, f.env.as_ref(), input, &|_: AiSideEvent| {})
        .await
        .unwrap();
    assert_eq!(
        last_user_message(f.requests().await.last().unwrap()),
        format!(
            "{}{}",
            host_change_block("old-name", "twin"),
            side_message("where was this?")
        )
    );
    let item = crate::ai::find_conversation(&lock(&f.vault), &fork).unwrap();
    assert_eq!(item.origin.as_deref(), Some("old-name"));
    // The next message in a tab on a host notes the move from it, and the move ends it.
    f.exchange(Some(&fork), "on twin", on(&twin)).await;
    assert_eq!(
        f.user_texts(&fork)[2],
        format!("{}on twin", host_change_block("old-name", "twin"))
    );
    let item = crate::ai::find_conversation(&lock(&f.vault), &fork).unwrap();
    assert_eq!((item.host_id, item.origin), (Some(twin), None));
}

/// Renames a saved host.
fn rename_host(f: &Fixture, host_id: &str, name: &str) {
    let mut v = lock(&f.vault);
    let mut host = v.get(host_id).and_then(Item::as_host).unwrap().clone();
    host.name = name.into();
    v.put(Some(host_id), Item::Host(host)).unwrap();
}

/// Stores a host at `ops@{address}:22`, so two hosts may share a name but not a server.
fn add_host_at(f: &Fixture, name: &str, address: &str) -> String {
    lock(&f.vault)
        .put(
            None,
            Item::Host(hatoba_core::model::Host {
                name: name.into(),
                address: address.into(),
                username: "ops".into(),
                ..hatoba_core::model::Host::default()
            }),
        )
        .unwrap()
}

#[test]
fn a_note_names_a_target_or_a_single_host() {
    use crate::ai::places::{Place, parse_target, place_named, target_name};

    for (label, name) in [
        ("root@10.0.0.9:22", "root@10.0.0.9:22"),
        ("ops@[fe80::1]:2222", "ops@[fe80::1]:2222"),
        ("a@b@Example.ORG:22", "a@b@example.org:22"),
    ] {
        assert_eq!(target_name(&parse_target(label).unwrap()), name);
    }
    for label in [
        "prod-db",
        "",
        "root@host",
        "root@host:x",
        "@host:22",
        "root@:22",
        "root@fe80::1:22",
        "root@host:99999",
        "my server@host:22",
        "root@host:+22",
    ] {
        assert_eq!(parse_target(label), None, "{label}");
    }
    let hosts: Vec<(String, String)> = [
        ("h-web", "web"),
        ("h-twin-1", "twin"),
        ("h-twin-2", "twin"),
        // A display name that looks like a target names the saved host.
        ("h-lookalike", "root@prod:22"),
    ]
    .into_iter()
    .map(|(id, name)| (id.to_owned(), name.to_owned()))
    .collect();
    let place = |from: &str| {
        let (
            Place {
                host_id,
                quick_target,
                ..
            },
            origin,
        ) = place_named(&hosts, from.to_owned());
        (host_id, quick_target.as_ref().map(target_name), origin)
    };
    assert_eq!(place("web"), (Some("h-web".into()), None, None));
    assert_eq!(place("twin"), (None, None, Some("twin".into())));
    assert_eq!(
        place("root@prod:22"),
        (Some("h-lookalike".into()), None, None)
    );
    assert_eq!(place(""), (None, None, Some(String::new())));
    assert_eq!(
        place("root@10.0.0.9:22"),
        (None, Some("root@10.0.0.9:22".into()), None)
    );
}
