//! The judge of Auto mode in the shell (AI-47): what it reads, its verdict against a mock System
//! One endpoint, the verdict kept until the call's result is stored, and the `permission_mode`
//! notes of Auto.

use super::*;
use crate::ai::{auto_view, find_conversation, guard, judge_state, open_call_at, screen_tail};
use crate::commands::ai::save_ai_settings;
use crate::dto::{AiAutoVerdict, AiJudgeDecision, AiModelRef, AiSettingsPatch, Clearable};
use crate::mcp::tests::{put_server, set_enabled, stdio_server};
use hatoba_ai::entry::{AutoDecision, AutoVerdict};
use hatoba_ai::judge::{
    self, CHANGES_SYSTEM, DESTROYS_DATA, FORBIDDEN, JudgeState, REQUESTED, SENDS_DATA_OUT,
    TOUCHES_SECRETS,
};
use hatoba_ai::tools::{PermissionMode, permission_mode_block, terminal_state_block};

/// The key of the judge's provider, which nothing may show.
const JUDGE_KEY: &str = "sk-judge-NEVER-SHOWN";

/// Answers to a command's questions that make it run as a change the user asked for.
const ASKED_FOR: [(&str, f64); 6] = [
    (REQUESTED, 0.97),
    (FORBIDDEN, 0.01),
    (CHANGES_SYSTEM, 0.92),
    (DESTROYS_DATA, 0.02),
    (TOUCHES_SECRETS, 0.0),
    (SENDS_DATA_OUT, 0.01),
];

/// A System One endpoint at `{uri}/v1/systemone` that gives every request these Noul answers
/// after `delay`.
async fn system_one(pairs: &[(&str, f64)], delay: Duration) -> MockServer {
    let answers: serde_json::Map<String, Value> = pairs
        .iter()
        .map(|(id, p)| ((*id).to_owned(), json!({"type": "noul", "noul": p})))
        .collect();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({
                    "model": "jev-1.13.0",
                    "answers": answers,
                    "usage": {"input_tokens": 1, "output_tokens": 1}
                }))
                .set_delay(delay),
        )
        .mount(&server)
        .await;
    server
}

/// The bodies of the requests `server` received.
async fn bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| r.body_json().unwrap())
        .collect()
}

/// A response with these calls, `(id, name, arguments as JSON text)`.
fn response(calls: &[(&str, &str, &str)]) -> AiEntry {
    AiEntry::assistant(
        10,
        AssistantEntry {
            provider_id: "p".into(),
            model_id: "m1".into(),
            text: "ASSISTANT-TEXT".into(),
            tool_calls: calls
                .iter()
                .map(|(id, name, arguments)| ToolCall {
                    id: (*id).into(),
                    name: (*name).into(),
                    arguments: (*arguments).into(),
                })
                .collect(),
            finish: if calls.is_empty() {
                Finish::Stop
            } else {
                Finish::ToolCalls
            },
            ..AssistantEntry::default()
        },
    )
}

fn auto(f: &Fixture) -> AiTurnContext {
    AiTurnContext {
        permission_mode: AiPermissionMode::Auto,
        ..f.context()
    }
}

impl Fixture {
    /// Stores a System One provider for `server` with the model `jev-latest` and chooses it as
    /// the judge in Settings → AI. Returns the provider's id.
    fn choose_judge(&self, server: &MockServer) -> String {
        let mut v = lock(&self.vault);
        let id = v
            .put(
                None,
                Item::AiProvider(AiProvider {
                    name: "Jev".into(),
                    protocol: AiProtocol::SystemOne,
                    base_url: format!("{}/v1", server.uri()),
                    api_key: Zeroizing::new(JUDGE_KEY.into()),
                    models: vec![AiModel {
                        id: "jev-latest".into(),
                        name: "Jev".into(),
                        ..AiModel::default()
                    }],
                    ..AiProvider::default()
                }),
            )
            .unwrap();
        let chosen = AiSettingsPatch {
            judge_model: Clearable::Set(AiModelRef {
                provider_id: id.clone(),
                model_id: "jev-latest".into(),
            }),
            ..AiSettingsPatch::default()
        };
        assert!(save_ai_settings(&mut v, &chosen).unwrap());
        id
    }

    async fn judge(
        &self,
        conversation_id: &str,
        call: &str,
        screen: Option<&str>,
    ) -> AiAutoVerdict {
        self.manager
            .judge(&self.vault, conversation_id, call, screen)
            .await
            .unwrap()
    }

    /// What the judge would read about the open call `call`.
    fn state_of(&self, conversation_id: &str, call: &str, screen: Option<&str>) -> Value {
        let v = lock(&self.vault);
        let conversation = find_conversation(&v, conversation_id).unwrap();
        let entries = Entries::load(&v, conversation_id).unwrap();
        let at = open_call_at(&entries, call).unwrap();
        let state: JudgeState = judge_state(
            &v,
            &self.mcp,
            conversation_id,
            &conversation,
            &entries.list,
            at,
            screen,
        );
        serde_json::to_value(state).unwrap()
    }

    /// Stores a result the panel produced for a call Auto decided.
    fn judged_result(
        &self,
        conversation_id: &str,
        call: &str,
        status: AiToolStatus,
        content: &str,
    ) -> AppResult<AiEntryView> {
        self.manager.tool_result(
            &self.vault,
            self.env.as_ref(),
            conversation_id,
            call,
            &AiToolResultInput {
                status,
                content: content.into(),
                edited_arguments: None,
                judged: true,
            },
        )
    }

    /// The verdicts kept for calls that wait.
    fn pending_verdicts(&self) -> usize {
        guard(&self.manager.0.pending_verdicts).len()
    }

    /// The verdict stored with the result of `call`.
    fn stored_verdict(&self, conversation_id: &str, call: &str) -> Option<AutoVerdict> {
        self.entries(conversation_id)
            .into_iter()
            .find_map(|e| match e.body {
                EntryBody::Tool {
                    tool_call_id, auto, ..
                } if tool_call_id == call => Some(auto),
                _ => None,
            })
            .unwrap()
    }
}

fn verdict_of(view: &AiEntryView) -> Option<&AiAutoVerdict> {
    match view {
        AiEntryView::Tool { auto, .. } => auto.as_ref(),
        other => panic!("expected a tool entry, got {other:?}"),
    }
}

#[test]
fn the_judge_reads_what_the_user_typed_and_what_the_calls_do() {
    let f = Fixture::with_base_url(None, "http://127.0.0.1:9/v1".into());
    f.set_instructions("  Never restart nginx on weekdays.\n");
    let host = f.add_host("prod-db", "Primary database.");
    let first = format!(
        "{}{}<pasted_text lines=\"120\">\nPASTED-BODY\n</pasted_text>\n\n\
         <file name=\"app&amp;.log\" lines=\"40\">\nFILE-BODY\n</file>\n\nWhy does the app fail?",
        permission_mode_block(PermissionMode::Auto),
        terminal_state_block("prod-db", false, "SCREEN-TEXT"),
    );
    let second = "<terminal_selection host=\"prod-db\" lines=\"1\">\nSELECTED\n\
                  </terminal_selection>\n\n<connection_diagnostics host=\"prod-db\">\n\
                  DIAGNOSTICS\n</connection_diagnostics>\n\nRestart it";
    let (conv, ids) = f.conversation_of(
        AiConversation {
            host_id: Some(host),
            ..AiConversation::default()
        },
        &[
            AiEntry::user(1, first),
            response(&[
                (
                    "c1",
                    "run_command",
                    r#"{"command": "journalctl -u app -n 50"}"#,
                ),
                (
                    "c2",
                    "call_mcp_tool",
                    r#"{"tool": "mcp__files__echo", "arguments": "{\"text\": \"hi\"}"}"#,
                ),
            ]),
            AiEntry::tool(3, "c1", ToolStatus::Ok, "RESULT-ONE"),
            AiEntry::tool(4, "c2", ToolStatus::Ok, "RESULT-TWO"),
            response(&[]),
            AiEntry::summary(6, "SUMMARY-TEXT"),
            AiEntry::user(7, second),
            // Notes alone: nothing the user typed.
            AiEntry::user(8, permission_mode_block(PermissionMode::Manual)),
            response(&[
                ("c3", "read_terminal", "{}"),
                ("c4", "send_input", r#"{"text": "y\n"}"#),
                // Cut off while it streamed.
                ("c5", "run_command", r#"{"command": "ls"#),
            ]),
        ],
    );
    // AI-21: compacted, which hides nothing the user said from the judge.
    {
        let mut v = lock(&f.vault);
        let mut item = find_conversation(&v, &conv).unwrap();
        item.context_start = Some(ids[5].clone());
        v.put(Some(&conv), Item::AiConversation(item)).unwrap();
    }
    let screen = (1..=15)
        .map(|n| format!("line {n}\u{7}  "))
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n\r\n   \r\n";

    let state = f.state_of(&conv, "c4", Some(&screen));
    assert_eq!(
        state,
        json!({
            "host": "prod-db",
            "user_messages": [
                "[pasted text, 120 lines]\n[file app&.log, 40 lines]\n\nWhy does the app fail?",
                "[terminal selection, 1 line]\n[connection diagnostics]\n\nRestart it"
            ],
            "user_rules": "Custom instructions:\nNever restart nginx on weekdays.\n\n\
                           Notes for this host:\nPrimary database.",
            "earlier_calls": [
                {"tool": "run_command", "arguments": {"command": "journalctl -u app -n 50"}},
                // AI-45: as the MCP tool it names, as the panel shows it.
                {"tool": "mcp__files__echo", "arguments": {"text": "hi"}},
                {"tool": "read_terminal", "arguments": {}}
            ],
            "call": {
                "tool": "send_input",
                "arguments": {"text": "y\n"},
                "screen_tail": (6..=15)
                    .map(|n| format!("line {n}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        })
    );
    // No assistant text, no result, no summary, no note, no attachment's content.
    let text = state.to_string();
    for hidden in [
        "ASSISTANT-TEXT",
        "RESULT-",
        "SUMMARY-TEXT",
        "permission_mode",
        "SCREEN-TEXT",
        "PASTED-BODY",
        "FILE-BODY",
        "SELECTED",
        "DIAGNOSTICS",
    ] {
        assert!(!text.contains(hidden), "{hidden} in {text}");
    }

    // Only `send_input` shows the screen; arguments that are not JSON are kept as text.
    let state = f.state_of(&conv, "c5", Some(&screen));
    assert_eq!(
        state["call"],
        json!({"tool": "run_command", "arguments": r#"{"command": "ls"#})
    );
    assert_eq!(state["earlier_calls"].as_array().unwrap().len(), 4);
}

#[test]
fn the_screen_tail_keeps_the_last_lines_and_characters() {
    assert_eq!(screen_tail("\n \r\n\u{1b}\n"), None);
    assert_eq!(
        screen_tail("$ apt upgrade\nContinue? [Y/n] \u{1b}\n\n").as_deref(),
        Some("$ apt upgrade\nContinue? [Y/n]")
    );
    let long = format!("{}END", "x".repeat(3_000));
    let tail = screen_tail(&format!("first\n{long}")).unwrap();
    assert_eq!(tail.chars().count(), 2_000);
    assert!(tail.ends_with("xEND") && !tail.contains("first"));
}

#[test]
fn a_verdict_shows_its_answers_in_the_order_the_judge_asks() {
    for tool in [
        "run_command",
        "send_input",
        "edit_file",
        "write_file",
        "fetch_url",
        "mcp__files__echo",
    ] {
        let asked = judge::question_ids(tool);
        let mut answers: std::collections::BTreeMap<String, f64> = asked
            .iter()
            .enumerate()
            .map(|(n, id)| ((*id).to_owned(), n as f64 / 10.0))
            .collect();
        // Questions a newer version asks come last, by id.
        answers.insert("zz_newer".into(), 0.5);
        answers.insert("aa_newer".into(), 0.5);
        let view = auto_view(&AutoVerdict {
            provider_id: "p".into(),
            model_id: "jev-latest".into(),
            decision: AutoDecision::Run,
            answers,
            concerns: Vec::new(),
            failure: None,
        });
        let order: Vec<&str> = view.answers.iter().map(|a| a.question.as_str()).collect();
        let expected: Vec<&str> = asked
            .iter()
            .copied()
            .chain(["aa_newer", "zz_newer"])
            .collect();
        assert_eq!(order, expected, "{tool}");
        assert_eq!(view.answers[1].probability, 0.1);
        assert_eq!(view.decision, AiJudgeDecision::Run);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_mcp_call_shows_the_judge_its_server_description_and_annotations() {
    let f = Fixture::new(vec![
        calls(&[
            ("c1", "mcp__files__echo", json!({"text": "hi"})),
            (
                "c2",
                "call_mcp_tool",
                json!({"tool": "mcp__files__echo", "arguments": {"text": "again"}}),
            ),
        ]),
        answer("Done."),
    ])
    .await;
    let files = put_server(&f.vault, stdio_server("files"));
    set_enabled(&f.vault, &files, true);
    let (started, sink) = f.send_with(None, "echo hi", auto(&f)).await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;

    // The server's description and annotations go with the call, as hints.
    let echo = json!({
        "tool": "mcp__files__echo",
        "arguments": {"text": "hi"},
        "server": "files",
        "description": "Echoes text",
        "annotations": {"title": "Echo", "readOnlyHint": true}
    });
    assert_eq!(f.state_of(&conv, "c1", None)["call"], echo);
    // AI-45: a call through `call_mcp_tool` is the tool it calls.
    let state = f.state_of(&conv, "c2", None);
    assert_eq!(
        state["call"],
        json!({
            "tool": "mcp__files__echo",
            "arguments": {"text": "again"},
            "server": "files",
            "description": "Echoes text",
            "annotations": {"title": "Echo", "readOnlyHint": true}
        })
    );
    assert_eq!(
        state["earlier_calls"],
        json!([{"tool": "mcp__files__echo", "arguments": {"text": "hi"}}])
    );
    f.manager.stop(&f.vault, f.env.as_ref(), &conv);
}

#[tokio::test]
async fn without_a_judge_a_call_asks_as_unavailable_and_nothing_is_sent() {
    let f = Fixture::new(vec![
        calls(&[(
            "c1",
            "run_command",
            json!({"command": "systemctl restart nginx"}),
        )]),
        answer("Done."),
    ])
    .await;
    let (started, sink) = f.send_with(None, "restart nginx", auto(&f)).await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;

    let unavailable = |provider_id: &str| AiAutoVerdict {
        provider_id: provider_id.into(),
        model_id: if provider_id.is_empty() {
            String::new()
        } else {
            "jev-latest".into()
        },
        decision: AiJudgeDecision::Ask,
        answers: Vec::new(),
        concerns: Vec::new(),
        failure: Some("unavailable".into()),
    };
    assert_eq!(f.judge(&conv, "c1", None).await, unavailable(""));
    // A judge whose provider is gone, as one deleted on another device, is no judge either.
    {
        let mut v = lock(&f.vault);
        let mut settings = v.settings();
        settings.ai.judge_model = Some(hatoba_core::model::AiModelRef {
            provider_id: "gone".into(),
            model_id: "jev-latest".into(),
        });
        v.put(Some(SETTINGS_ID), Item::Settings(settings)).unwrap();
    }
    let verdict = f.judge(&conv, "c1", None).await;
    assert_eq!(verdict, unavailable("gone"));
    // Only the turn's request was sent.
    assert_eq!(f.requests().await.len(), 1);

    // The card asked, and the user rejected the call: the result keeps why the card asked.
    let rejected = f
        .judged_result(&conv, "c1", AiToolStatus::Rejected, "not now")
        .unwrap();
    assert_eq!(verdict_of(&rejected), Some(&verdict));
    assert_eq!(f.pending_verdicts(), 0);
    // A call with a result, or none at all, is not judged.
    let err = f
        .manager
        .judge(&f.vault, &conv, "c1", None)
        .await
        .unwrap_err();
    assert_eq!(err.field.as_deref(), Some("tool_call_id"));
    let err = f
        .manager
        .judge(&f.vault, &conv, "c9", None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    sink.done_with(AiFinish::Stop).await;
}

#[tokio::test]
async fn a_judge_lets_a_requested_change_run_and_its_result_keeps_the_verdict() {
    let f = Fixture::new(vec![
        calls(&[
            ("c1", "send_input", json!({"text": "y\n"})),
            (
                "c2",
                "run_command",
                json!({"command": "systemctl restart nginx"}),
            ),
        ]),
        answer("Done."),
    ])
    .await;
    let endpoint = system_one(&ASKED_FOR, Duration::ZERO).await;
    let judge_id = f.choose_judge(&endpoint);
    let (started, sink) = f
        .send_with(
            None,
            "I changed the nginx config, please restart nginx",
            auto(&f),
        )
        .await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;

    let first = f
        .judge(&conv, "c1", Some("Restart nginx? [y/N] \u{7}\n\n"))
        .await;
    assert_eq!(first.decision, AiJudgeDecision::Run);
    assert_eq!(first.failure, None);
    assert!(first.concerns.is_empty());
    assert_eq!(
        (first.provider_id.as_str(), first.model_id.as_str()),
        (judge_id.as_str(), "jev-latest")
    );
    let order: Vec<&str> = first.answers.iter().map(|a| a.question.as_str()).collect();
    assert_eq!(order, judge::question_ids("send_input"));
    assert_eq!(first.answers[0].probability, 0.97);
    let second = f.judge(&conv, "c2", None).await;
    assert_eq!(second.decision, AiJudgeDecision::Run);
    assert_eq!(f.pending_verdicts(), 2);

    // The judge read the user's words and the calls, not the notes, and never the key.
    let sent = bodies(&endpoint).await;
    assert_eq!(sent.len(), 2);
    assert_eq!(
        sent[0]["state"],
        json!({
            "user_messages": ["I changed the nginx config, please restart nginx"],
            "earlier_calls": [],
            "call": {
                "tool": "send_input",
                "arguments": {"text": "y\n"},
                "screen_tail": "Restart nginx? [y/N]"
            }
        })
    );
    assert_eq!(
        sent[1]["state"]["earlier_calls"],
        json!([{"tool": "send_input", "arguments": {"text": "y\n"}}])
    );
    assert!(!sent[1].to_string().contains(JUDGE_KEY));

    // The panel ran c1 on the verdict: its result keeps it. c2 ran without Auto deciding it
    // (`judged` false), so its verdict is dropped.
    let typed = f
        .judged_result(&conv, "c1", AiToolStatus::Ok, "sent")
        .unwrap();
    assert_eq!(verdict_of(&typed), Some(&first));
    assert_eq!(f.pending_verdicts(), 1);
    let ran = f.run(&conv, "c2", None, None).await.unwrap();
    assert_eq!(verdict_of(&ran), None);
    assert_eq!(f.pending_verdicts(), 0);
    sink.done_with(AiFinish::Stop).await;

    // Stored and synced with the result, shown by history, and never sent to the model.
    let stored = f.stored_verdict(&conv, "c1").unwrap();
    assert_eq!(stored.decision, AutoDecision::Run);
    assert_eq!(stored.provider_id, judge_id);
    assert_eq!(f.stored_verdict(&conv, "c2"), None);
    let detail = f.detail(&conv);
    let shown = detail
        .entries
        .iter()
        .find(|e| matches!(e, AiEntryView::Tool { tool_call_id, .. } if tool_call_id == "c1"))
        .unwrap();
    assert_eq!(verdict_of(shown), Some(&first));
    let next = f.requests().await.pop().unwrap();
    assert_eq!(tool_message(&next, "c1")["content"], "sent");
    let next = next.to_string();
    assert!(
        !next.contains("jev-latest") && !next.contains(&judge_id),
        "{next}"
    );
}

#[tokio::test]
async fn a_verdict_for_a_call_answered_meanwhile_is_not_kept_and_a_stop_ends_the_judge() {
    let f = Fixture::new(vec![calls(&[
        ("c1", "send_input", json!({"text": "y\n"})),
        ("c2", "send_input", json!({"text": "q"})),
    ])])
    .await;
    let slow = system_one(&ASKED_FOR, Duration::from_millis(500)).await;
    f.choose_judge(&slow);
    let (started, sink) = f.send_with(None, "answer the prompt", auto(&f)).await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;

    let judging = {
        let (manager, vault, conv) = (f.manager.clone(), f.vault.clone(), conv.clone());
        tokio::spawn(async move { manager.judge(&vault, &conv, "c1", None).await })
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    while bodies(&slow).await.is_empty() {
        assert!(Instant::now() < deadline, "the judge was not asked");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // The user answered the call while the judge was asked.
    let answered = f
        .judged_result(&conv, "c1", AiToolStatus::Rejected, "")
        .unwrap();
    assert_eq!(verdict_of(&answered), None);
    let late = judging.await.unwrap().unwrap();
    assert_eq!(late.decision, AiJudgeDecision::Run);
    assert_eq!(f.pending_verdicts(), 0);
    assert_eq!(f.stored_verdict(&conv, "c1"), None);

    // A stop ends the judge's request at once; the call's cancelled result keeps no verdict.
    let hanging = system_one(&ASKED_FOR, Duration::from_secs(30)).await;
    f.choose_judge(&hanging);
    let judging = {
        let (manager, vault, conv) = (f.manager.clone(), f.vault.clone(), conv.clone());
        tokio::spawn(async move { manager.judge(&vault, &conv, "c2", None).await })
    };
    while bodies(&hanging).await.is_empty() {
        assert!(Instant::now() < deadline, "the judge was not asked");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let stopped_at = Instant::now();
    f.manager.stop(&f.vault, f.env.as_ref(), &conv);
    let stopped = judging.await.unwrap().unwrap();
    assert!(stopped_at.elapsed() < Duration::from_secs(5));
    assert_eq!(stopped.decision, AiJudgeDecision::Ask);
    assert_eq!(stopped.failure.as_deref(), Some("cancelled"));
    assert_eq!(f.pending_verdicts(), 0);
    assert_eq!(f.stored_verdict(&conv, "c2"), None);
}

#[tokio::test]
async fn in_auto_the_first_message_and_a_summary_name_the_mode() {
    let f = Fixture::new(vec![
        answer("One."),
        answer("Two."),
        answer("SUMMARY: nginx was restarted."),
        answer("Three."),
        answer("Four."),
    ])
    .await;
    let note = permission_mode_block(PermissionMode::Auto);
    // §13.5: the system prompt starts in manual approval, so a first message in Auto names it.
    let (conv, _) = f.exchange(None, "first", auto(&f)).await;
    f.exchange(Some(&conv), "second", auto(&f)).await;
    // The model sees nothing before a summary, so the summary keeps the mode.
    let summary = f
        .manager
        .compact(&f.vault, f.env.as_ref(), &conv, &auto(&f))
        .await
        .unwrap();
    let AiEntryView::Summary { text, .. } = &summary else {
        panic!("expected a summary, got {summary:?}");
    };
    assert_eq!(text, &format!("{note}SUMMARY: nginx was restarted."));
    f.exchange(Some(&conv), "third", auto(&f)).await;
    // AI-16: from Auto to bypass, a mode of its own.
    let bypass = AiTurnContext {
        permission_mode: AiPermissionMode::Bypass,
        ..f.context()
    };
    f.exchange(Some(&conv), "fourth", bypass).await;
    assert_eq!(
        f.user_texts(&conv),
        [
            format!("{note}first"),
            "second".to_owned(),
            "third".to_owned(),
            format!("{}fourth", permission_mode_block(PermissionMode::Bypass)),
        ]
    );
}
