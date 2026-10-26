//! The notes before a message (spec §13.3) in turns: the screen, moves to another host, a
//! quick connection or a local terminal, terminals that come or go, and switches of model.

use super::*;

#[tokio::test]
async fn each_message_shows_the_screen_it_was_sent_with() {
    use hatoba_ai::tools::{
        PermissionMode, permission_mode_block, terminal_change_block, terminal_state_block,
    };

    let f = Fixture::new((0..6).map(|i| answer(&format!("Answer {i}."))).collect()).await;
    let prod = f.add_host("prod-db", "");
    let on_prod = |text: &str, alternate| AiTurnContext {
        host_id: Some(prod.clone()),
        screen: Some(AiScreen {
            text: text.into(),
            alternate,
        }),
        ..f.context()
    };

    // The first message shows it too, and each one the screen it was sent with (AI-46).
    let (conv, _) = f
        .exchange(None, "first", on_prod("$ make\nerror", false))
        .await;
    f.exchange(Some(&conv), "second", on_prod("~\n\"a.conf\" 3L", true))
        .await;
    // A blank screen shows nothing.
    f.exchange(Some(&conv), "third", on_prod("\n  \n", false))
        .await;
    // Nor does a tab that is not connected, whatever the panel sent.
    let detached = AiTurnContext {
        tab: false,
        ..on_prod("$ stale", false)
    };
    f.exchange(Some(&conv), "fourth", detached).await;
    // Nor a message the panel sends without one, as while **Send Terminal Screen** is off.
    let quiet = AiTurnContext {
        screen: None,
        ..on_prod("", false)
    };
    f.exchange(Some(&conv), "fifth", quiet).await;
    // The screen comes after the other notes.
    let bypass = AiTurnContext {
        permission_mode: AiPermissionMode::Bypass,
        ..on_prod("$ back", false)
    };
    f.exchange(Some(&conv), "sixth", bypass).await;

    assert_eq!(
        f.user_texts(&conv),
        [
            manual(&format!(
                "{}first",
                terminal_state_block("prod-db", false, "$ make\nerror")
            )),
            format!(
                "{}second",
                terminal_state_block("prod-db", true, "~\n\"a.conf\" 3L")
            ),
            "third".to_owned(),
            format!("{}fourth", terminal_change_block(false)),
            format!("{}fifth", terminal_change_block(true)),
            format!(
                "{}{}sixth",
                permission_mode_block(PermissionMode::Bypass),
                terminal_state_block("prod-db", false, "$ back")
            ),
        ]
    );
    // The note goes to the model as part of the message, and never titles the conversation.
    let last = f.requests().await.pop().unwrap();
    assert!(user_messages(&last).last().unwrap().contains(
        "\n\n<terminal_state host=\"prod-db\" alternate_screen=\"false\" lines=\"1\">\n$ back\n"
    ));
    assert_eq!(title_of(&f.user_texts(&conv)[0]), "first");
}

#[tokio::test]
async fn a_move_to_another_host_and_a_switch_of_model_are_noted_once() {
    use hatoba_ai::tools::{host_change_block, model_change_block};

    let f = Fixture::new((0..5).map(|i| answer(&format!("Answer {i}."))).collect()).await;
    f.add_model("m2", "Model 2");
    let staging = f.add_host("staging-web", "");
    let prod = f.add_host("prod-db", "");
    let on = |host: &str, model: &str| AiTurnContext {
        host_id: Some(host.to_owned()),
        model_id: model.into(),
        ..f.context()
    };

    let (conv, _) = f.exchange(None, "first", on(&staging, "m1")).await;
    // AI-09: the message that moves the conversation carries the note, once.
    f.exchange(Some(&conv), "second", on(&prod, "m1")).await;
    // AI-05: so does the first message to another model.
    f.exchange(Some(&conv), "third", on(&prod, "m2")).await;
    f.exchange(Some(&conv), "fourth", on(&prod, "m2")).await;
    // Both at once: the host's note first.
    f.exchange(Some(&conv), "fifth", on(&staging, "m1")).await;

    let moved = host_change_block("staging-web", "prod-db");
    let switched = model_change_block("Model 1 (m1)", "Model 2 (m2)");
    let both = format!(
        "{}{}fifth",
        host_change_block("prod-db", "staging-web"),
        model_change_block("Model 2 (m2)", "Model 1 (m1)")
    );
    let expected = [
        manual("first"),
        format!("{moved}second"),
        format!("{switched}third"),
        "fourth".to_owned(),
        both,
    ];
    assert_eq!(f.user_texts(&conv), expected);
    // The notes go to the model with the message, as stored.
    let last = f.requests().await.pop().unwrap();
    assert_eq!(user_messages(&last), expected);
    // They never title the conversation.
    assert_eq!(f.detail(&conv).conversation.title, "first");
}

#[tokio::test]
async fn a_quick_connection_takes_the_conversation_off_its_host() {
    use hatoba_ai::tools::host_change_block;

    let f = Fixture::new(vec![answer("One."), answer("Two.")]).await;
    let prod = f.add_host("prod-db", "Primary.");
    let on_prod = AiTurnContext {
        host_id: Some(prod.clone()),
        ..f.context()
    };
    let quick = AiTurnContext {
        target: Some(QuickTarget {
            address: "10.0.0.9".into(),
            port: 22,
            username: "root".into(),
        }),
        ..f.context()
    };
    let (conv, _) = f.exchange(None, "first", on_prod).await;
    f.exchange(Some(&conv), "second", quick).await;
    // HOST-12: the target is named where a host would be, and the saved host is left.
    assert_eq!(
        f.user_texts(&conv),
        [
            manual("first"),
            format!("{}second", host_change_block("prod-db", "root@10.0.0.9:22"))
        ]
    );
    assert_eq!(f.detail(&conv).conversation.host_id, None);
    let system = system_of(&f.requests().await[1]);
    assert!(
        system.contains("connected to the host \"root@10.0.0.9:22\" as the user \"root\""),
        "{system}"
    );
    assert!(!system.contains("<host_notes"), "{system}");
}

#[tokio::test]
async fn a_move_off_a_quick_connection_is_noted_from_its_target() {
    use hatoba_ai::tools::{host_change_block, terminal_change_block};

    let f = Fixture::new((0..5).map(|i| answer(&format!("Answer {i}."))).collect()).await;
    let prod = f.add_host("prod-db", "");
    let first = "root@10.0.0.9:22";
    let second = "root@10.0.0.10:22";
    let home = AiTurnContext {
        tab: false,
        ..f.context()
    };

    // HOST-12: a conversation started on a target records it in place of a host.
    let (conv, _) = f.exchange(None, "one", f.quick("10.0.0.9")).await;
    assert_eq!(f.recorded_place(&conv), (None, Some(first.to_owned())));
    f.exchange(Some(&conv), "two", f.quick("10.0.0.9")).await;
    // The home tab leaves it there.
    f.exchange(Some(&conv), "three", home).await;
    assert_eq!(f.recorded_place(&conv), (None, Some(first.to_owned())));
    // AI-09: another target is a move, and so is a saved host.
    f.exchange(Some(&conv), "four", f.quick("10.0.0.10")).await;
    assert_eq!(f.recorded_place(&conv), (None, Some(second.to_owned())));
    let on_prod = AiTurnContext {
        host_id: Some(prod.clone()),
        ..f.context()
    };
    f.exchange(Some(&conv), "five", on_prod).await;
    assert_eq!(f.recorded_place(&conv), (Some(prod), None));

    let expected = [
        manual("one"),
        "two".to_owned(),
        format!("{}three", terminal_change_block(false)),
        format!(
            "{}{}four",
            host_change_block(first, second),
            terminal_change_block(true)
        ),
        format!("{}five", host_change_block(second, "prod-db")),
    ];
    assert_eq!(f.user_texts(&conv), expected);
    let last = f.requests().await.pop().unwrap();
    assert_eq!(user_messages(&last), expected);
}

#[tokio::test]
async fn the_home_tab_names_the_target_its_conversation_is_on() {
    use hatoba_ai::tools::terminal_change_block;

    let f = Fixture::new(vec![answer("One."), answer("Two.")]).await;
    let (conv, _) = f.exchange(None, "one", f.quick("10.0.0.9")).await;
    // The panel shows the target (AI-09), and the home tab sends it, as it sends a saved host.
    assert_eq!(
        f.detail(&conv).conversation.quick_target,
        Some(QuickTarget {
            address: "10.0.0.9".into(),
            port: 22,
            username: "root".into(),
        })
    );
    let home = AiTurnContext {
        tab: false,
        ..f.quick("10.0.0.9")
    };
    f.exchange(Some(&conv), "two", home).await;
    assert_eq!(
        f.user_texts(&conv),
        [
            manual("one"),
            format!("{}two", terminal_change_block(false))
        ]
    );
    assert_eq!(
        f.recorded_place(&conv),
        (None, Some("root@10.0.0.9:22".to_owned()))
    );
    let system = system_of(&f.requests().await[1]);
    assert!(
        system.contains("This conversation is about the host \"root@10.0.0.9:22\"."),
        "{system}"
    );
}

#[tokio::test]
async fn a_quick_connection_and_the_host_saved_from_it_are_one_server() {
    let f = Fixture::new((0..4).map(|i| answer(&format!("Answer {i}."))).collect()).await;
    // Save as Host… (HOST-12): the same address, port, and user.
    let saved = f.add_saved_target("web-1", "10.0.0.9");
    let on_saved = AiTurnContext {
        host_id: Some(saved.clone()),
        ..f.context()
    };
    let (conv, _) = f.exchange(None, "one", f.quick("10.0.0.9")).await;
    f.exchange(Some(&conv), "two", on_saved.clone()).await;
    assert_eq!(f.recorded_place(&conv), (Some(saved.clone()), None));
    // Back on a tab that has not reconnected as the saved host yet.
    f.exchange(Some(&conv), "three", f.quick("10.0.0.9")).await;
    assert_eq!(
        f.user_texts(&conv),
        [manual("one").as_str(), "two", "three"]
    );

    // A conversation on a target from before targets were recorded came from no known place.
    {
        let mut v = lock(&f.vault);
        let mut item = crate::ai::find_conversation(&v, &conv).unwrap();
        item.quick_target = None;
        v.put(Some(&conv), Item::AiConversation(item)).unwrap();
    }
    let prod = f.add_host("prod-db", "");
    let on_prod = AiTurnContext {
        host_id: Some(prod),
        ..f.context()
    };
    f.exchange(Some(&conv), "four", on_prod).await;
    assert_eq!(
        f.user_texts(&conv),
        [manual("one").as_str(), "two", "three", "four"]
    );
}

#[tokio::test]
async fn a_target_in_another_letter_case_is_the_same_server() {
    let f = Fixture::new((0..3).map(|i| answer(&format!("Answer {i}."))).collect()).await;
    // The panel compares addresses in any letter case and without brackets (HOST-12), so the
    // conversation records the address in lowercase.
    let saved = f.add_saved_target("db", "db.example.org");
    let (conv, _) = f.exchange(None, "one", f.quick("DB.Example.org")).await;
    f.exchange(Some(&conv), "two", f.quick("db.example.ORG"))
        .await;
    assert_eq!(
        f.recorded_place(&conv),
        (None, Some("root@db.example.org:22".to_owned()))
    );
    let on_saved = AiTurnContext {
        host_id: Some(saved),
        ..f.context()
    };
    f.exchange(Some(&conv), "three", on_saved).await;
    assert_eq!(
        f.user_texts(&conv),
        [manual("one").as_str(), "two", "three"]
    );
}

#[tokio::test]
async fn an_edit_drops_the_note_of_a_move_from_the_server_it_goes_from() {
    use hatoba_ai::tools::host_change_block;

    let f = Fixture::new((0..7).map(|i| answer(&format!("Answer {i}."))).collect()).await;
    let prod = f.add_host("prod-db", "");
    let on = |host: &str| AiTurnContext {
        host_id: Some(host.to_owned()),
        ..f.context()
    };
    let (conv, _) = f.exchange(None, "one", f.quick("srv.example")).await;
    f.exchange(Some(&conv), "two", on(&prod)).await;
    let moved = host_change_block("root@srv.example:22", "prod-db");
    assert_eq!(f.user_texts(&conv)[1], format!("{moved}two"));
    let edit = |text: &'static str, context: AiTurnContext| {
        let f = &f;
        let conv = &conv;
        async move {
            let two = f.user_ids(conv)[1].clone();
            f.edit_with(conv, &two, text, context).await;
        }
    };

    // AI-26: edited from the target in another letter case, the message goes from the server
    // the earlier screens came from.
    edit("two, same target", f.quick("SRV.Example")).await;
    assert_eq!(
        f.user_texts(&conv),
        [manual("one").as_str(), "two, same target"]
    );
    // So does one from the host the target was saved as (Save as Host…, HOST-12).
    f.exchange(Some(&conv), "three", on(&prod)).await;
    let srv = f.add_saved_target("srv", "srv.example");
    let three = f.user_ids(&conv)[2].clone();
    f.edit_with(&conv, &three, "three, saved", on(&srv)).await;
    assert_eq!(
        f.user_texts(&conv),
        [manual("one").as_str(), "two, same target", "three, saved"]
    );

    // And the other way round: a note from the saved host, edited from its target.
    f.exchange(Some(&conv), "four", on(&prod)).await;
    assert_eq!(
        f.user_texts(&conv)[3],
        format!("{}four", host_change_block("srv", "prod-db"))
    );
    let four = f.user_ids(&conv)[3].clone();
    f.edit_with(&conv, &four, "four, quick", f.quick("srv.example"))
        .await;
    assert_eq!(f.user_texts(&conv)[3], "four, quick");
    assert_eq!(
        f.recorded_place(&conv),
        (None, Some("root@srv.example:22".to_owned()))
    );
}

#[tokio::test]
async fn a_switch_noted_on_a_message_without_a_reply_is_not_noted_again() {
    use hatoba_ai::tools::model_change_block;

    let f = Fixture::new(vec![
        answer("One."),
        ResponseTemplate::new(500).set_body_json(json!({"error": {"message": "overloaded"}})),
        answer("Two."),
    ])
    .await;
    f.add_model("m2", "Model 2");
    let to = |model: &str| AiTurnContext {
        model_id: model.into(),
        ..f.context()
    };
    let (conv, _) = f.exchange(None, "one", to("m1")).await;
    let (_, failed) = f.exchange(Some(&conv), "two", to("m2")).await;
    assert_eq!(failed, AiTurnEndReason::Error);
    f.exchange(Some(&conv), "two again", to("m2")).await;
    assert_eq!(
        f.user_texts(&conv),
        [
            manual("one"),
            format!("{}two", model_change_block("Model 1 (m1)", "Model 2 (m2)")),
            "two again".to_owned()
        ]
    );
}

#[tokio::test]
async fn editing_a_message_keeps_the_note_of_the_move_it_made() {
    use hatoba_ai::tools::host_change_block;

    let f = Fixture::new((0..5).map(|i| answer(&format!("Answer {i}."))).collect()).await;
    let staging = f.add_host("staging-web", "");
    let prod = f.add_host("prod-db", "");
    let db = f.add_host("db-2", "");
    let on = |host: &str| AiTurnContext {
        host_id: Some(host.to_owned()),
        ..f.context()
    };
    let (conv, _) = f.exchange(None, "first", on(&staging)).await;
    f.exchange(Some(&conv), "second", on(&prod)).await;
    let second = |f: &Fixture| {
        let v = lock(&f.vault);
        let entries = Entries::load(&v, &conv).unwrap();
        entries.ids[2].clone()
    };

    // AI-26: the edited message replaces one that moved the conversation, so it keeps the note.
    let id = second(&f);
    f.edit_with(&conv, &id, "second, edited", on(&prod)).await;
    let moved = host_change_block("staging-web", "prod-db");
    assert_eq!(
        f.user_texts(&conv),
        [manual("first"), format!("{moved}second, edited")]
    );

    // Edited from a tab on a third host: the earlier screens still came from the first one.
    let id = second(&f);
    f.edit_with(&conv, &id, "second, from db-2", on(&db)).await;
    assert_eq!(
        f.user_texts(&conv)[1],
        format!(
            "{}second, from db-2",
            host_change_block("staging-web", "db-2")
        )
    );

    // The first message has nothing before it, so it gets no note of a move, only its mode.
    let first = lock(&f.vault).ai_entries(&conv).unwrap()[0]
        .entry_id
        .clone();
    f.edit_with(&conv, &first, "first, edited", on(&prod)).await;
    assert_eq!(f.user_texts(&conv), [manual("first, edited")]);
    assert_eq!(
        f.detail(&conv).conversation.host_id.as_deref(),
        Some(prod.as_str())
    );
}

#[tokio::test]
async fn editing_a_message_keeps_the_note_of_a_move_off_a_quick_connection() {
    use hatoba_ai::tools::{host_change_block, terminal_change_block};

    let f = Fixture::new((0..7).map(|i| answer(&format!("Answer {i}."))).collect()).await;
    let prod = f.add_host("prod-db", "");
    let on_prod = AiTurnContext {
        host_id: Some(prod.clone()),
        ..f.context()
    };
    let first = "root@10.0.0.9:22";
    let second = "root@10.0.0.10:22";
    let (conv, _) = f.exchange(None, "one", f.quick("10.0.0.9")).await;
    f.exchange(Some(&conv), "two", on_prod.clone()).await;
    let edit = |text: &'static str, context: AiTurnContext| {
        let f = &f;
        let conv = &conv;
        async move {
            let two = f.user_ids(conv)[1].clone();
            f.edit_with(conv, &two, text, context).await;
        }
    };

    // AI-26: the edited message replaces the one that moved the conversation off the target.
    edit("two, edited", on_prod.clone()).await;
    let moved = host_change_block(first, "prod-db");
    assert_eq!(
        f.user_texts(&conv),
        [manual("one"), format!("{moved}two, edited")]
    );

    // Edited from another target: the earlier screens still came from the first one.
    edit("two, elsewhere", f.quick("10.0.0.10")).await;
    let elsewhere = host_change_block(first, second);
    assert_eq!(f.user_texts(&conv)[1], format!("{elsewhere}two, elsewhere"));
    assert_eq!(f.recorded_place(&conv), (None, Some(second.to_owned())));

    // From the home tab, the conversation stays on the target it records.
    let home = AiTurnContext {
        tab: false,
        ..f.context()
    };
    edit("two, from home", home).await;
    assert_eq!(
        f.user_texts(&conv)[1],
        format!("{elsewhere}{}two, from home", terminal_change_block(false))
    );
    assert_eq!(f.recorded_place(&conv), (None, Some(second.to_owned())));

    // From the first target, nothing moved.
    edit("two, back", f.quick("10.0.0.9")).await;
    assert_eq!(f.user_texts(&conv), [manual("one").as_str(), "two, back"]);
    assert_eq!(f.recorded_place(&conv), (None, Some(first.to_owned())));

    // The first message has nothing before it, so it only names its mode.
    let one = f.user_ids(&conv)[0].clone();
    f.edit_with(&conv, &one, "one, edited", on_prod).await;
    assert_eq!(f.user_texts(&conv), [manual("one, edited")]);
    assert_eq!(f.recorded_place(&conv), (Some(prod), None));
}

#[tokio::test]
async fn a_terminal_that_comes_or_goes_between_replies_is_noted() {
    use hatoba_ai::tools::terminal_change_block;

    let f = Fixture::new((0..5).map(|i| answer(&format!("Answer {i}."))).collect()).await;
    let vmiss = f.add_host("VMISS", "");
    let home = AiTurnContext {
        tab: false,
        ..f.context()
    };
    let connected = AiTurnContext {
        host_id: Some(vmiss.clone()),
        ..f.context()
    };
    let disconnected = AiTurnContext {
        tab: false,
        ..connected.clone()
    };

    // A home tab chat moves to a connected tab: no host to come from, but a terminal now.
    let (conv, _) = f.exchange(None, "can you run df?", home).await;
    f.exchange(Some(&conv), "connected", connected.clone())
        .await;
    // Nothing changed since.
    f.exchange(Some(&conv), "and now?", connected.clone()).await;
    // The tab disconnects, and connects again.
    f.exchange(Some(&conv), "still there?", disconnected).await;
    f.exchange(Some(&conv), "back", connected).await;
    let attached = terminal_change_block(true);
    let expected = [
        manual("can you run df?"),
        format!("{attached}connected"),
        "and now?".to_owned(),
        format!("{}still there?", terminal_change_block(false)),
        format!("{attached}back"),
    ];
    assert_eq!(f.user_texts(&conv), expected);
    // Each reply records what its request offered.
    assert_eq!(
        f.reply_terminals(&conv),
        [Some(false), Some(true), Some(true), Some(false), Some(true)]
    );
    // The notes go to the model with the message, as stored, and never title the conversation.
    let last = f.requests().await.pop().unwrap();
    assert_eq!(user_messages(&last), expected);
    assert_eq!(f.detail(&conv).conversation.title, "can you run df?");
}

#[tokio::test]
async fn the_newest_reply_decides_whatever_device_or_version_wrote_it() {
    use hatoba_ai::tools::terminal_change_block;

    let f = Fixture::new((0..3).map(|i| answer(&format!("Answer {i}."))).collect()).await;
    let vmiss = f.add_host("VMISS", "");
    let connected = AiTurnContext {
        host_id: Some(vmiss),
        ..f.context()
    };
    let disconnected = AiTurnContext {
        tab: false,
        ..connected.clone()
    };
    let (conv, _) = f.exchange(None, "first", connected.clone()).await;
    // Another device went on without a terminal; its entries merged after this device's.
    let detached = format!("{}on the laptop", terminal_change_block(false));
    f.append_exchange(&conv, &detached, Some(false));
    f.exchange(Some(&conv), "still without", disconnected.clone())
        .await;
    // A reply from an earlier version did not record it, so nothing is noted after it.
    f.append_exchange(&conv, "from an older version", None);
    f.exchange(Some(&conv), "connected again", connected).await;
    assert_eq!(
        f.user_texts(&conv),
        [
            manual("first"),
            detached,
            "still without".to_owned(),
            "from an older version".to_owned(),
            "connected again".to_owned(),
        ]
    );
}

#[tokio::test]
async fn notes_of_a_move_a_terminal_and_a_model_come_in_that_order() {
    use hatoba_ai::tools::{host_change_block, model_change_block, terminal_change_block};

    let f = Fixture::new((0..3).map(|i| answer(&format!("Answer {i}."))).collect()).await;
    f.add_model("m2", "Model 2");
    let staging = f.add_host("staging-web", "");
    let prod = f.add_host("prod-db", "");
    let (conv, _) = f
        .exchange(
            None,
            "first",
            AiTurnContext {
                host_id: Some(staging),
                ..f.context()
            },
        )
        .await;
    // Opened on the home tab: the conversation keeps its host, without a terminal.
    let home = AiTurnContext {
        tab: false,
        ..f.context()
    };
    f.exchange(Some(&conv), "second", home).await;
    // Then in a connected tab on another host, with another model.
    let prod_tab = AiTurnContext {
        host_id: Some(prod),
        model_id: "m2".into(),
        ..f.context()
    };
    f.exchange(Some(&conv), "third", prod_tab).await;
    assert_eq!(
        f.user_texts(&conv),
        [
            manual("first"),
            format!("{}second", terminal_change_block(false)),
            format!(
                "{}{}{}third",
                host_change_block("staging-web", "prod-db"),
                terminal_change_block(true),
                model_change_block("Model 1 (m1)", "Model 2 (m2)")
            ),
        ]
    );
}

#[tokio::test]
async fn an_edit_compares_the_terminal_with_the_reply_before_it() {
    use hatoba_ai::tools::terminal_change_block;

    let f = Fixture::new((0..7).map(|i| answer(&format!("Answer {i}."))).collect()).await;
    let vmiss = f.add_host("VMISS", "");
    let home = AiTurnContext {
        tab: false,
        ..f.context()
    };
    let connected = AiTurnContext {
        host_id: Some(vmiss),
        ..f.context()
    };
    let (conv, _) = f.exchange(None, "first", home.clone()).await;
    f.exchange(Some(&conv), "second", connected.clone()).await;
    f.exchange(Some(&conv), "third", connected.clone()).await;
    let attached = terminal_change_block(true);

    // AI-26: the reply before the third message had the terminal, so editing it from home notes
    // that the terminal went away.
    let third = f.user_ids(&conv)[2].clone();
    f.edit_with(&conv, &third, "third, from home", home.clone())
        .await;
    assert_eq!(
        f.user_texts(&conv),
        [
            manual("first"),
            format!("{attached}second"),
            format!("{}third, from home", terminal_change_block(false)),
        ]
    );

    // The second message brought the terminal: before it there was none, whatever came after.
    let second = f.user_ids(&conv)[1].clone();
    f.edit_with(&conv, &second, "second, edited", connected.clone())
        .await;
    assert_eq!(
        f.user_texts(&conv),
        [manual("first"), format!("{attached}second, edited")]
    );
    // Edited from home, nothing changed since the first reply.
    let second = f.user_ids(&conv)[1].clone();
    f.edit_with(&conv, &second, "second, from home", home).await;
    assert_eq!(
        f.user_texts(&conv),
        [manual("first").as_str(), "second, from home"]
    );

    // The first message has nothing before it, so it only names its mode.
    let first = f.user_ids(&conv)[0].clone();
    f.edit_with(&conv, &first, "first, edited", connected).await;
    assert_eq!(f.user_texts(&conv), [manual("first, edited")]);
}

#[tokio::test]
async fn a_change_at_a_retry_is_not_noted_and_the_next_message_compares_with_its_reply() {
    use hatoba_ai::tools::terminal_change_block;

    let overloaded =
        || ResponseTemplate::new(500).set_body_json(json!({"error": {"message": "overloaded"}}));
    let f = Fixture::new(vec![
        answer("One."),
        overloaded(),
        overloaded(),
        answer("Retried."),
        answer("Three."),
        answer("Edited."),
    ])
    .await;
    let vmiss = f.add_host("VMISS", "");
    let connected = AiTurnContext {
        host_id: Some(vmiss),
        ..f.context()
    };
    let disconnected = AiTurnContext {
        tab: false,
        ..connected.clone()
    };
    let (conv, _) = f.exchange(None, "one", connected.clone()).await;
    let (_, failed) = f.exchange(Some(&conv), "two", disconnected.clone()).await;
    assert_eq!(failed, AiTurnEndReason::Error);
    // A change noted on a message without a reply is not noted again.
    let (_, failed) = f
        .exchange(Some(&conv), "two again", disconnected.clone())
        .await;
    assert_eq!(failed, AiTurnEndReason::Error);
    // The retry stores no message to carry a note; its reply records the terminal it had.
    let sink = f.retry(&conv, connected.clone()).unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    f.exchange(Some(&conv), "three", connected).await;
    let detached = format!("{}two", terminal_change_block(false));
    assert_eq!(
        f.user_texts(&conv),
        [
            manual("one"),
            detached.clone(),
            "two again".to_owned(),
            "three".to_owned()
        ]
    );
    assert_eq!(
        f.reply_terminals(&conv),
        [Some(true), Some(true), Some(true)]
    );

    // Editing a message from before the retry compares with what came before that message,
    // not with the retry: the note on "two" already said the terminal went away.
    let again = f.user_ids(&conv)[2].clone();
    f.edit_with(&conv, &again, "two, edited", disconnected)
        .await;
    assert_eq!(
        f.user_texts(&conv),
        [manual("one"), detached, "two, edited".to_owned()]
    );
}

/// The description of the request's tool `name`, in Chat Completions' shape.
fn tool_description(body: &Value, name: &str) -> String {
    body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["function"]["name"] == name)
        .map(|t| t["function"]["description"].as_str().unwrap().to_owned())
        .unwrap_or_default()
}

#[tokio::test]
async fn a_local_tab_moves_the_conversation_to_its_computer() {
    use crate::dto::LocalShellKind;
    use hatoba_ai::tools::{host_change_block, terminal_change_block};

    let f = Fixture::new((0..4).map(|i| answer(&format!("Answer {i}."))).collect()).await;
    let prod = f.add_host("prod-db", "Primary.");
    let on_prod = AiTurnContext {
        host_id: Some(prod.clone()),
        ..f.context()
    };
    let (conv, _) = f.exchange(None, "one", on_prod.clone()).await;
    // §7.5: the computer is the conversation's place, named like a host in notes.
    let here = f.local("DEMO-PC", "PowerShell", LocalShellKind::Powershell);
    f.exchange(Some(&conv), "two", here).await;
    assert_eq!(
        f.detail(&conv).conversation.local.as_deref(),
        Some("DEMO-PC")
    );
    assert_eq!(f.recorded_place(&conv), (None, None));
    // The home tab sends the computer it recorded, which leaves it there.
    let home = AiTurnContext {
        tab: false,
        local: Some(crate::dto::AiLocalTab {
            device: "DEMO-PC".into(),
            shell: None,
            kind: None,
        }),
        ..f.context()
    };
    f.exchange(Some(&conv), "three", home).await;
    f.exchange(Some(&conv), "four", on_prod).await;
    assert_eq!(f.detail(&conv).conversation.local, None);
    assert_eq!(
        f.user_texts(&conv),
        [
            manual("one"),
            format!(
                "{}two",
                host_change_block("prod-db", "DEMO-PC (local terminal)")
            ),
            format!("{}three", terminal_change_block(false)),
            format!(
                "{}{}four",
                host_change_block("DEMO-PC (local terminal)", "prod-db"),
                terminal_change_block(true)
            ),
        ]
    );

    let requests = f.requests().await;
    let local = system_of(&requests[1]);
    assert!(
        local.contains("local terminal on the user's own computer \"DEMO-PC\""),
        "{local}"
    );
    assert!(local.contains("It runs PowerShell"), "{local}");
    assert!(local.contains("Windows paths"), "{local}");
    assert!(!local.contains("SSH server"), "{local}");
    assert!(
        tool_description(&requests[1], "run_command").contains("on this computer"),
        "{}",
        tool_description(&requests[1], "run_command")
    );
    let home = system_of(&requests[2]);
    assert!(
        home.contains("This conversation is about the host \"DEMO-PC (local terminal)\"."),
        "{home}"
    );
    assert!(tool_description(&requests[3], "run_command").contains("SSH connection"));
}

#[tokio::test]
async fn another_computers_local_tab_is_a_move() {
    use crate::dto::LocalShellKind;
    use hatoba_ai::tools::host_change_block;

    let f = Fixture::new(vec![answer("One."), answer("Two.")]).await;
    let (conv, _) = f
        .exchange(None, "one", f.local("DEMO-PC", "zsh", LocalShellKind::Unix))
        .await;
    let wsl = f.local("LAPTOP", "Ubuntu", LocalShellKind::Wsl);
    f.exchange(Some(&conv), "two", wsl).await;
    assert_eq!(
        f.detail(&conv).conversation.local.as_deref(),
        Some("LAPTOP")
    );
    assert_eq!(
        f.user_texts(&conv)[1],
        format!(
            "{}two",
            host_change_block("DEMO-PC (local terminal)", "LAPTOP (local terminal)")
        )
    );
    let system = system_of(&f.requests().await[1]);
    assert!(system.contains("Ubuntu is a WSL distribution"), "{system}");
}

#[test]
fn a_note_from_a_local_terminal_names_its_computer() {
    let (place, origin) =
        crate::ai::places::place_named(&[], "DEMO-PC (local terminal)".to_owned());
    assert_eq!(place.local.as_deref(), Some("DEMO-PC"));
    assert!(place.host_id.is_none() && place.quick_target.is_none() && origin.is_none());
}
