//! The judge's questions, rules, state budget and failures (AI-47).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use tokio::io::AsyncReadExt;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};
use zeroize::Zeroizing;

use super::*;
use crate::provider::{AuthHeader, http_client};
use crate::tools::SEND_INPUT;

fn provider(base_url: String, protocol: Protocol) -> ProviderConfig {
    ProviderConfig {
        protocol,
        base_url,
        api_key: Zeroizing::new("sk-judge-secret".into()),
        auth_header: AuthHeader::XApiKey,
    }
}

fn answers(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
}

fn command_state(messages: &[&str], command: &str) -> JudgeState {
    JudgeState {
        host: Some("web-01".into()),
        user_messages: messages.iter().map(|m| (*m).to_owned()).collect(),
        call: JudgeCall::new(RUN_COMMAND, json!({ "command": command })),
        ..JudgeState::default()
    }
}

/// System One's Noul answers for these probabilities.
fn system_one_body(pairs: &[(&str, f64)]) -> serde_json::Value {
    let answers: serde_json::Map<String, serde_json::Value> = pairs
        .iter()
        .map(|(id, p)| ((*id).to_owned(), json!({"type": "noul", "noul": p})))
        .collect();
    json!({"model": "jev-1.13.0", "answers": answers, "usage": {"input_tokens": 1, "output_tokens": 1}})
}

/// Answers for a command that only reads.
const READ: [(&str, f64); 8] = [
    (REQUESTED, 0.3),
    (FORBIDDEN, 0.01),
    (CHANGES_ANYTHING, 0.02),
    (DESTROYS_DATA, 0.01),
    (TOUCHES_SECRETS, 0.0),
    (SENDS_DATA_OUT, 0.0),
    (SECURITY_CHANGE, 0.01),
    (RUNS_UNSEEN_CODE, 0.0),
];

async fn system_one_server(pairs: &[(&str, f64)]) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(system_one_body(pairs)))
        .mount(&server)
        .await;
    server
}

async fn judged(provider: &ProviderConfig, state: JudgeState) -> Verdict {
    judge(
        &http_client(),
        provider,
        &ModelSpec::new("jev-latest"),
        state,
        &CancellationToken::new(),
    )
    .await
}

// ---------------------------------------------------------------------------------------------
// Questions
// ---------------------------------------------------------------------------------------------

#[test]
fn each_kind_of_call_gets_its_questions_in_table_order() {
    let command = vec![
        REQUESTED,
        FORBIDDEN,
        CHANGES_ANYTHING,
        DESTROYS_DATA,
        TOUCHES_SECRETS,
        SENDS_DATA_OUT,
        SECURITY_CHANGE,
        RUNS_UNSEEN_CODE,
    ];
    assert_eq!(question_ids(RUN_COMMAND), command);
    assert_eq!(question_ids(SEND_INPUT), command);
    assert_eq!(question_ids("mcp__github__create_issue"), command);
    for tool in [EDIT_FILE, WRITE_FILE] {
        assert_eq!(
            question_ids(tool),
            vec![REQUESTED, FORBIDDEN, TOUCHES_SECRETS, SENSITIVE_FILE]
        );
    }
    assert_eq!(question_ids(FETCH_URL), vec![FORBIDDEN, URL_CARRIES_DATA]);

    let questions = questions_for(RUN_COMMAND);
    assert_eq!(questions.keys().map(String::as_str).collect::<Vec<_>>(), {
        let mut sorted = command.clone();
        sorted.sort_unstable();
        sorted
    });
    // Each is a Noul that names the state's fields in backticks and says what yes and no mean.
    for (id, question) in questions_for(EDIT_FILE)
        .into_iter()
        .chain(questions_for(FETCH_URL))
        .chain(questions)
    {
        let Question::Noul {
            instructions,
            criteria: Some(criteria),
        } = question
        else {
            panic!("{id} has criteria");
        };
        assert!(instructions.as_str().unwrap().contains("`call`"), "{id}");
        assert!(criteria.yes.is_string() && criteria.no.is_string(), "{id}");
    }
    let text = |tool: &str, id: &str| {
        let Question::Noul { instructions, .. } = &questions_for(tool)[id];
        instructions.as_str().unwrap().to_owned()
    };
    // Only the user's own words ask or forbid: their messages, and why they rejected a call.
    let requested = text(RUN_COMMAND, REQUESTED);
    assert!(requested.contains(
        "Only `user_messages` and the `rejection` of an earlier call are the user's words"
    ));
    assert!(requested.contains("`screen_tail`"));
    assert!(requested.contains("that the user rejected was not asked for"));
    let forbidden = text(FETCH_URL, FORBIDDEN);
    assert!(forbidden.contains("the user rejecting an earlier call like this one"));
    // Any change counts, so only a call that reads runs without being asked for, and every part
    // of a URL that is sent is checked for data, the host name included.
    let changes = text(RUN_COMMAND, CHANGES_ANYTHING);
    assert!(changes.contains("change anything, on `host` or elsewhere"));
    assert!(changes.contains("database row"));
    let url = text(FETCH_URL, URL_CARRIES_DATA);
    assert!(url.contains("host name with its subdomains"));
    assert!(url.contains("user name or password"));
    // Access, persistence and weakened security count through the shell too, and code piped in
    // from elsewhere is code the judge cannot see.
    let security = text(SEND_INPUT, SECURITY_CHANGE);
    for named in [
        "authorized_keys",
        "sudoers",
        "systemctl enable",
        "chmod 777",
        "sudo tee",
    ] {
        assert!(security.contains(named), "{named}");
    }
    let unseen = text("mcp__ci__run", RUNS_UNSEEN_CODE);
    for named in [
        "curl … | sh",
        "base64 -d | sh",
        "does not show in full",
        "package manager",
    ] {
        assert!(unseen.contains(named), "{named}");
    }
}

// ---------------------------------------------------------------------------------------------
// Decisions
// ---------------------------------------------------------------------------------------------

fn with(base: &[(&str, f64)], changes: &[(&str, f64)]) -> BTreeMap<String, f64> {
    let mut all = answers(base);
    all.extend(answers(changes));
    all
}

#[test]
fn commands_run_as_reads_or_as_requested_changes_at_the_system_one_thresholds() {
    let t = Thresholds::SYSTEM_ONE;
    let decide_command = |changes: &[(&str, f64)]| decide(RUN_COMMAND, &with(&READ, changes), t);

    // A read runs, asked for or not.
    assert_eq!(decide_command(&[]), (AutoDecision::Run, vec![]));
    assert_eq!(
        decide_command(&[(CHANGES_ANYTHING, 0.0999), (DESTROYS_DATA, 0.0999)]),
        (AutoDecision::Run, vec![])
    );
    // A change runs only when the user asked for it.
    assert_eq!(
        decide_command(&[(CHANGES_ANYTHING, 0.10), (REQUESTED, 0.85)]),
        (AutoDecision::Run, vec![])
    );
    assert_eq!(
        decide_command(&[(CHANGES_ANYTHING, 0.97), (REQUESTED, 0.8499)]),
        (
            AutoDecision::Ask,
            vec![REQUESTED.to_owned(), CHANGES_ANYTHING.to_owned()]
        )
    );
    // Nothing that destroys, touches credentials, sends out, changes security, runs code the
    // state does not show, or is forbidden runs, asked or not.
    for hazard in [
        DESTROYS_DATA,
        TOUCHES_SECRETS,
        SENDS_DATA_OUT,
        SECURITY_CHANGE,
        RUNS_UNSEEN_CODE,
        FORBIDDEN,
    ] {
        assert_eq!(
            decide_command(&[(hazard, 0.10), (REQUESTED, 0.99)]),
            (AutoDecision::Ask, vec![hazard.to_owned()]),
            "{hazard}"
        );
    }
    // The concerns are what failed, in table order; `requested` only for a change.
    assert_eq!(
        decide_command(&[(FORBIDDEN, 0.6), (SENDS_DATA_OUT, 0.2), (REQUESTED, 0.0)]),
        (
            AutoDecision::Ask,
            vec![FORBIDDEN.to_owned(), SENDS_DATA_OUT.to_owned()]
        )
    );
    assert_eq!(
        decide_command(&[
            (CHANGES_ANYTHING, 0.5),
            (DESTROYS_DATA, 0.5),
            (REQUESTED, 0.9)
        ]),
        (AutoDecision::Ask, vec![DESTROYS_DATA.to_owned()])
    );
    // An install script piped into a shell, asked for: it changes something, and its code is
    // not in the state.
    assert_eq!(
        decide_command(&[
            (CHANGES_ANYTHING, 0.99),
            (RUNS_UNSEEN_CODE, 0.9),
            (SECURITY_CHANGE, 0.3),
            (REQUESTED, 0.95)
        ]),
        (
            AutoDecision::Ask,
            vec![SECURITY_CHANGE.to_owned(), RUNS_UNSEEN_CODE.to_owned()]
        )
    );
    // MCP tools and send_input follow the same rule.
    assert_eq!(
        decide(
            "mcp__db__drop_table",
            &with(&READ, &[(DESTROYS_DATA, 0.9)]),
            t
        )
        .0,
        AutoDecision::Ask
    );
    assert_eq!(decide(SEND_INPUT, &answers(&READ), t).0, AutoDecision::Run);
}

#[test]
fn a_chat_model_judge_has_stricter_thresholds() {
    let t = Thresholds::for_protocol(Protocol::Anthropic);
    assert_eq!(t, Thresholds::CHAT_MODEL);
    assert_eq!(Thresholds::for_protocol(Protocol::ChatCompletions), t);
    assert_eq!(
        Thresholds::for_protocol(Protocol::SystemOne),
        Thresholds::SYSTEM_ONE
    );
    let decide_command = |changes: &[(&str, f64)]| decide(RUN_COMMAND, &with(&READ, changes), t).0;
    assert_eq!(
        decide_command(&[(DESTROYS_DATA, 0.0499)]),
        AutoDecision::Run
    );
    assert_eq!(decide_command(&[(DESTROYS_DATA, 0.05)]), AutoDecision::Ask);
    assert_eq!(decide_command(&[(DESTROYS_DATA, 0.08)]), AutoDecision::Ask);
    assert_eq!(
        decide_command(&[(CHANGES_ANYTHING, 0.9), (REQUESTED, 0.90)]),
        AutoDecision::Run
    );
    assert_eq!(
        decide_command(&[(CHANGES_ANYTHING, 0.9), (REQUESTED, 0.8999)]),
        AutoDecision::Ask
    );
    assert_eq!(
        decide_command(&[(CHANGES_ANYTHING, 0.9), (REQUESTED, 0.87)]),
        AutoDecision::Ask,
        "enough for System One, not for a chat model"
    );
}

#[test]
fn file_changes_run_when_asked_for_and_safe_and_fetches_when_the_url_carries_nothing() {
    let t = Thresholds::SYSTEM_ONE;
    let file = [
        (REQUESTED, 0.85),
        (FORBIDDEN, 0.0),
        (TOUCHES_SECRETS, 0.0999),
        (SENSITIVE_FILE, 0.0),
    ];
    for tool in [EDIT_FILE, WRITE_FILE] {
        assert_eq!(
            decide(tool, &answers(&file), t),
            (AutoDecision::Run, vec![])
        );
        assert_eq!(
            decide(tool, &with(&file, &[(REQUESTED, 0.84)]), t),
            (AutoDecision::Ask, vec![REQUESTED.to_owned()])
        );
        assert_eq!(
            decide(
                tool,
                &with(&file, &[(SENSITIVE_FILE, 0.1), (FORBIDDEN, 0.5)]),
                t
            ),
            (
                AutoDecision::Ask,
                vec![FORBIDDEN.to_owned(), SENSITIVE_FILE.to_owned()]
            )
        );
    }
    let fetch = [(FORBIDDEN, 0.0), (URL_CARRIES_DATA, 0.0999)];
    assert_eq!(
        decide(FETCH_URL, &answers(&fetch), t),
        (AutoDecision::Run, vec![])
    );
    assert_eq!(
        decide(FETCH_URL, &with(&fetch, &[(URL_CARRIES_DATA, 0.1)]), t),
        (AutoDecision::Ask, vec![URL_CARRIES_DATA.to_owned()])
    );
    assert_eq!(
        decide(FETCH_URL, &with(&fetch, &[(FORBIDDEN, 0.3)]), t),
        (AutoDecision::Ask, vec![FORBIDDEN.to_owned()])
    );
}

#[test]
fn a_missing_or_unreadable_answer_counts_against_the_call() {
    let t = Thresholds::SYSTEM_ONE;
    let mut missing = answers(&READ);
    missing.remove(SENDS_DATA_OUT);
    assert_eq!(
        decide(RUN_COMMAND, &missing, t),
        (AutoDecision::Ask, vec![SENDS_DATA_OUT.to_owned()])
    );
    let nan = with(&READ, &[(DESTROYS_DATA, f64::NAN)]);
    assert_eq!(decide(RUN_COMMAND, &nan, t).0, AutoDecision::Ask);
    let nan = answers(&[
        (REQUESTED, f64::NAN),
        (FORBIDDEN, 0.0),
        (TOUCHES_SECRETS, 0.0),
        (SENSITIVE_FILE, 0.0),
    ]);
    assert_eq!(decide(EDIT_FILE, &nan, t).0, AutoDecision::Ask);
    assert_eq!(
        decide(FETCH_URL, &BTreeMap::new(), t),
        (
            AutoDecision::Ask,
            vec![FORBIDDEN.to_owned(), URL_CARRIES_DATA.to_owned()]
        )
    );
}

// ---------------------------------------------------------------------------------------------
// The state
// ---------------------------------------------------------------------------------------------

#[test]
fn the_state_has_system_ones_shape_and_debug_shows_no_content() {
    let mut state = command_state(&["restart nginx"], "systemctl restart nginx");
    state.earlier_calls = vec![JudgeCall::parse(RUN_COMMAND, r#"{"command": "nginx -t"}"#)];
    assert_eq!(
        serde_json::to_value(&state).unwrap(),
        json!({
            "host": "web-01",
            "user_messages": ["restart nginx"],
            "earlier_calls": [{"tool": "run_command", "arguments": {"command": "nginx -t"}}],
            "call": {"tool": "run_command", "arguments": {"command": "systemctl restart nginx"}}
        })
    );
    let mut mcp = JudgeCall::parse("mcp__gh__create_issue", r#"{"title": "#);
    assert_eq!(mcp.arguments, json!(r#"{"title": "#), "kept as text");
    mcp.server = Some("gh".into());
    mcp.description = Some("Creates an issue".into());
    mcp.annotations = Some(json!({"destructiveHint": false}));
    let typed = JudgeCall {
        screen_tail: Some("Proceed? [y/N]".into()),
        ..JudgeCall::new(SEND_INPUT, json!({"text": "y", "key": "enter"}))
    };
    state.user_rules = "Never restart postgresql.".into();
    state.earlier_calls.push(mcp);
    state.call = typed;
    let value = serde_json::to_value(&state).unwrap();
    assert_eq!(value["user_rules"], "Never restart postgresql.");
    assert_eq!(value["call"]["screen_tail"], "Proceed? [y/N]");
    assert_eq!(value["earlier_calls"][1]["server"], "gh");
    assert_eq!(
        value["earlier_calls"][1]["annotations"]["destructiveHint"],
        false
    );
    // A call the user rejected carries their reason; others carry none.
    state.earlier_calls[0].rejection = Some("not before the backup".into());
    let value = serde_json::to_value(&state).unwrap();
    assert_eq!(
        value["earlier_calls"][0],
        json!({
            "tool": "run_command",
            "arguments": {"command": "nginx -t"},
            "rejection": "not before the backup"
        })
    );
    assert!(value["earlier_calls"][1].get("rejection").is_none());
    let debug = format!("{state:?}");
    for secret in [
        "restart nginx",
        "postgresql",
        "Proceed",
        "Creates an issue",
        "backup",
    ] {
        assert!(!debug.contains(secret), "{debug}");
    }
    // What a conversation's requests share comes first, in this order, and the call last.
    let json = serde_json::to_string(&state).unwrap();
    let order: Vec<usize> = ["host", "user_rules", "user_messages", SHARED_UNTIL, "call"]
        .iter()
        .map(|key| json.find(&format!("\"{key}\":")).unwrap())
        .collect();
    assert!(order.is_sorted(), "{json}");
}

#[test]
fn long_messages_and_earlier_arguments_are_cut_and_only_the_newest_calls_kept() {
    let long = format!("{}{}", "a".repeat(3_000), "z".repeat(3_000));
    let mut state = command_state(&[&long, "short"], "cat big.log");
    state.call.arguments = json!({"command": "x".repeat(10_000)});
    state.earlier_calls = (0..25)
        .map(|i| {
            JudgeCall::new(
                WRITE_FILE,
                json!({"path": format!("/f{i}"), "content": "c".repeat(5_000)}),
            )
        })
        .collect();
    let fitted = fit(state, STATE_BUDGET_TOKENS).unwrap();
    let message = &fitted.user_messages[0];
    assert!(message.starts_with(&"a".repeat(1_000)));
    assert!(message.ends_with(&"z".repeat(3_000)));
    assert!(message.contains("characters left out"));
    assert_eq!(fitted.user_messages[1], "short");
    assert_eq!(fitted.earlier_calls.len(), MAX_EARLIER_CALLS);
    assert_eq!(fitted.earlier_calls[0].arguments["path"], "/f5");
    let content = fitted.earlier_calls[0].arguments["content"]
        .as_str()
        .unwrap();
    assert!(content.chars().count() < 2_100 && content.contains("left out"));
    // The call itself stays whole.
    assert_eq!(
        fitted.call.arguments["command"].as_str().unwrap().len(),
        10_000
    );

    // A rejection is the user's words, cut like a message.
    let mut state = command_state(&["hi"], "uptime");
    state.earlier_calls = vec![JudgeCall {
        rejection: Some(long.clone()),
        ..JudgeCall::new(RUN_COMMAND, json!({"command": "reboot"}))
    }];
    let fitted = fit(state, STATE_BUDGET_TOKENS).unwrap();
    let rejection = fitted.earlier_calls[0].rejection.as_deref().unwrap();
    assert!(rejection.starts_with(&"a".repeat(1_000)));
    assert!(rejection.ends_with(&"z".repeat(3_000)));
    assert!(rejection.contains("characters left out"));
}

#[test]
fn the_oldest_messages_and_calls_go_first_and_the_call_is_never_cut() {
    let messages: Vec<String> = (0..40)
        .map(|i| format!("{i:02} {}", "m".repeat(2_000)))
        .collect();
    let refs: Vec<&str> = messages.iter().map(String::as_str).collect();
    let mut state = command_state(&refs, "uptime");
    state.earlier_calls = (0..20)
        .map(|i| {
            JudgeCall::new(
                RUN_COMMAND,
                json!({"command": format!("{i:02} {}", "c".repeat(1_500))}),
            )
        })
        .collect();
    // About 80,000 + 30,000 bytes: over a budget of 30,000 tokens' 90,000 bytes.
    let fitted = fit(state.clone(), 30_000).unwrap();
    let size = serde_json::to_string(&fitted).unwrap();
    assert!(estimate_tokens(&size) <= 30_000);
    assert_eq!(fitted.user_messages.last(), state.user_messages.last());
    assert_eq!(fitted.earlier_calls.last(), state.earlier_calls.last());
    assert!(fitted.user_messages.len() < 40 && !fitted.user_messages.is_empty());
    assert!(
        fitted.user_messages[0].starts_with(&format!("{:02}", 40 - fitted.user_messages.len()))
    );
    // A budget that fits the call alone keeps the call whole and leaves out everything else.
    let call_only = fit(state.clone(), 50).unwrap();
    assert!(call_only.user_messages.is_empty() && call_only.earlier_calls.is_empty());
    assert_eq!(call_only.call, state.call);
    // One the call alone does not fit is too long.
    let mut huge = state;
    huge.call.arguments = json!({"command": "y".repeat(80_000)});
    assert_eq!(fit(huge, STATE_BUDGET_TOKENS), None);
}

#[test]
fn rules_too_long_for_the_budget_keep_their_start_and_end() {
    let mut state = command_state(&["hi"], "uptime");
    state.user_rules = format!("{}{}", "r".repeat(30_000), "END");
    let fitted = fit(state.clone(), 5_000).unwrap();
    assert!(fitted.user_rules.ends_with("END"));
    assert!(fitted.user_rules.contains("left out"));
    // They take at most half the budget, so the user's own words stay.
    assert!(fitted.user_rules.len() <= 7_500);
    assert_eq!(fitted.user_messages, ["hi"]);
    assert!(estimate_tokens(&serde_json::to_string(&fitted).unwrap()) <= 5_000);
    // With messages that fill the rest, the rules give way further, and the newest message stays.
    let long = "m".repeat(3_900);
    state.user_messages = vec![long.clone(); 3];
    let fitted = fit(state.clone(), 5_000).unwrap();
    assert!(estimate_tokens(&serde_json::to_string(&fitted).unwrap()) <= 5_000);
    assert_eq!(fitted.user_messages.last(), Some(&long));
    assert!(fitted.user_rules.ends_with("END") || fitted.user_rules.is_empty());
    // Chinese text counts as about a token a character.
    assert_eq!(estimate_tokens("重启"), 2);
    assert_eq!(estimate_tokens("restart"), 3);
}

// ---------------------------------------------------------------------------------------------
// Judging
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_read_runs_and_a_change_nobody_asked_for_asks() {
    let server = system_one_server(&READ).await;
    let p = provider(server.uri(), Protocol::SystemOne);
    let verdict = judged(&p, command_state(&["How full is the disk?"], "df -h")).await;
    assert_eq!(verdict.decision, AutoDecision::Run);
    assert_eq!(verdict.answers, answers(&READ));
    assert!(verdict.concerns.is_empty() && verdict.failure.is_none());
    let body: serde_json::Value =
        serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(body["state"]["call"]["arguments"]["command"], "df -h");
    assert_eq!(
        body["state"]["user_messages"],
        json!(["How full is the disk?"])
    );
    assert_eq!(body["questions"].as_object().unwrap().len(), 8);

    let mut change = READ;
    change[2] = (CHANGES_ANYTHING, 0.95);
    let server = system_one_server(&change).await;
    let p = provider(server.uri(), Protocol::SystemOne);
    let verdict = judged(&p, command_state(&["hi"], "systemctl restart nginx")).await;
    assert_eq!(verdict.decision, AutoDecision::Ask);
    assert_eq!(verdict.concerns, vec![REQUESTED, CHANGES_ANYTHING]);
    assert_eq!(verdict.failure, None);
    let auto = verdict.to_auto("p1", "jev-latest");
    assert_eq!(
        serde_json::to_value(&auto).unwrap(),
        json!({
            "provider_id": "p1", "model_id": "jev-latest", "decision": "ask",
            "answers": serde_json::to_value(answers(&change)).unwrap(),
            "concerns": ["requested", "changes_anything"]
        })
    );
}

#[tokio::test]
async fn the_judged_call_says_what_its_tool_does() {
    let server = system_one_server(&READ).await;
    let p = provider(server.uri(), Protocol::SystemOne);
    let typed = JudgeCall {
        screen_tail: Some("root@web-01:~#".into()),
        ..JudgeCall::new(SEND_INPUT, json!({"text": "ls -a", "key": "enter"}))
    };
    let mut state = command_state(&["Run ls -a here"], "ls");
    state.earlier_calls = vec![JudgeCall::new(RUN_COMMAND, json!({"command": "pwd"}))];
    state.call = typed;
    assert_eq!(judged(&p, state).await.decision, AutoDecision::Run);
    let mut mcp = JudgeCall::new("mcp__gh__list_issues", json!({"state": "open"}));
    mcp.server = Some("gh".into());
    mcp.description = Some("Lists issues".into());
    let mut state = command_state(&["Which issues are open?"], "ls");
    state.call = mcp;
    judged(&p, state).await;
    let bodies: Vec<serde_json::Value> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    let sent = &bodies[0]["state"];
    assert_eq!(
        sent["call"]["description"],
        builtin_description(SEND_INPUT).unwrap()
    );
    // Only the call does: earlier calls stay as they were.
    assert!(sent["earlier_calls"][0].get("description").is_none());
    // An MCP tool keeps its server's description.
    assert_eq!(bodies[1]["state"]["call"]["description"], "Lists issues");
}

#[tokio::test]
async fn every_failure_asks_with_its_kind() {
    let state = || command_state(&["hi"], "df -h");
    // Unreachable: nothing listens on port 9 of the loopback address.
    let closed = provider("http://127.0.0.1:9/v1".into(), Protocol::SystemOne);
    assert_eq!(
        judged(&closed, state()).await,
        Verdict::failed(JudgeFailure::Unreachable)
    );
    let bad_url = provider("ftp://jev.example".into(), Protocol::SystemOne);
    assert_eq!(
        judged(&bad_url, state()).await.failure,
        Some(JudgeFailure::Unreachable)
    );

    // Refused: the endpoint answers with an error.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error": "bad key"})))
        .mount(&server)
        .await;
    let refused = judged(&provider(server.uri(), Protocol::SystemOne), state()).await;
    assert_eq!(refused, Verdict::failed(JudgeFailure::Refused));
    assert_eq!(refused.decision, AutoDecision::Ask);

    // Malformed: an answer is missing or out of range.
    let server = system_one_server(&READ[..5]).await;
    let missing = judged(&provider(server.uri(), Protocol::SystemOne), state()).await;
    assert_eq!(missing.failure, Some(JudgeFailure::Malformed));
    assert!(missing.answers.is_empty());
    let mut high = READ;
    high[0] = (REQUESTED, 1.7);
    let server = system_one_server(&high).await;
    let out_of_range = judged(&provider(server.uri(), Protocol::SystemOne), state()).await;
    assert_eq!(out_of_range.failure, Some(JudgeFailure::Malformed));

    // Too long: the call alone exceeds the budget; nothing is sent.
    let server = system_one_server(&READ).await;
    let mut long = state();
    long.call.arguments = json!({"command": "y".repeat(100_000)});
    let too_long = judged(&provider(server.uri(), Protocol::SystemOne), long).await;
    assert_eq!(too_long, Verdict::failed(JudgeFailure::TooLong));
    assert!(server.received_requests().await.unwrap().is_empty());

    // Cancelled: the turn stopped.
    let cancel = CancellationToken::new();
    cancel.cancel();
    let cancelled = judge(
        &http_client(),
        &provider(server.uri(), Protocol::SystemOne),
        &ModelSpec::new("jev"),
        state(),
        &cancel,
    )
    .await;
    assert_eq!(cancelled.failure, Some(JudgeFailure::Cancelled));

    let auto = Verdict::failed(JudgeFailure::TooLong).to_auto("p", "m");
    assert_eq!(auto.failure.as_deref(), Some("too_long"));
    assert_eq!(auto.decision, AutoDecision::Ask);
    assert_eq!(
        serde_json::to_value(JudgeFailure::Unreachable).unwrap(),
        json!(JudgeFailure::Unreachable.as_str())
    );
}

/// Answers 503 first, then System One's answers.
struct FlakyOnce(Arc<AtomicUsize>, u16);

impl Respond for FlakyOnce {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            ResponseTemplate::new(self.1).set_body_json(json!({"error": "busy"}))
        } else {
            ResponseTemplate::new(200).set_body_json(system_one_body(&READ))
        }
    }
}

#[tokio::test]
async fn a_request_that_fails_on_the_way_is_sent_once_more() {
    for status in [429, 500, 503, 529] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(FlakyOnce(Arc::default(), status))
            .mount(&server)
            .await;
        let verdict = judged(
            &provider(server.uri(), Protocol::SystemOne),
            command_state(&["hi"], "df -h"),
        )
        .await;
        assert_eq!(verdict.decision, AutoDecision::Run, "{status}");
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }
    // Only once.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({"error": "busy"})))
        .mount(&server)
        .await;
    let verdict = judged(
        &provider(server.uri(), Protocol::SystemOne),
        command_state(&["hi"], "df -h"),
    )
    .await;
    assert_eq!(verdict.failure, Some(JudgeFailure::Refused));
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    // An error in the request itself is not retried.
    for status in [400, 401, 422] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(FlakyOnce(Arc::default(), status))
            .mount(&server)
            .await;
        let verdict = judged(
            &provider(server.uri(), Protocol::SystemOne),
            command_state(&["hi"], "df -h"),
        )
        .await;
        assert_eq!(verdict.failure, Some(JudgeFailure::Refused), "{status}");
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
    assert!(is_transient(&AiError::Network("reset".into())));
    assert!(!is_transient(&AiError::Protocol("bad".into())));
}

/// A server that reads requests and never answers, keeping the connections open.
async fn silent_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = vec![0u8; 64 * 1024];
            let _ = socket.read(&mut buf).await;
            held.push(socket);
        }
    });
    format!("http://{addr}/v1")
}

#[tokio::test(start_paused = true)]
async fn a_judge_that_does_not_answer_in_time_asks() {
    let base = silent_server().await;
    for (protocol, limit) in [
        (Protocol::SystemOne, Duration::from_secs(10)),
        (Protocol::ChatCompletions, Duration::from_secs(30)),
        (Protocol::Anthropic, Duration::from_secs(30)),
    ] {
        assert_eq!(timeout_for(protocol), limit);
        let started = tokio::time::Instant::now();
        let verdict = judged(
            &provider(base.clone(), protocol),
            command_state(&["hi"], "df -h"),
        )
        .await;
        assert_eq!(
            verdict,
            Verdict::failed(JudgeFailure::Timeout),
            "{protocol:?}"
        );
        let waited = started.elapsed();
        assert!(
            waited >= limit && waited < limit + Duration::from_secs(1),
            "{waited:?}"
        );
    }
}

#[tokio::test]
async fn a_chat_model_judge_answers_through_the_answer_tool() {
    let arguments = json!({
        "notes": "Restarting nginx, which the user asked for.",
        "requested": 0.97, "forbidden": 0.01, "changes_anything": 0.99,
        "destroys_data": 0.02, "touches_secrets": 0.0, "sends_data_out": 0.0,
        "security_change": 0.01, "runs_unseen_code": 0.0
    });
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{"type": "tool_use", "id": "t", "name": "answer", "input": arguments}],
            "usage": {"input_tokens": 900, "output_tokens": 80}
        })))
        .mount(&server)
        .await;
    let p = provider(server.uri(), Protocol::Anthropic);
    let verdict = judged(
        &p,
        command_state(&["please restart nginx"], "systemctl restart nginx"),
    )
    .await;
    assert_eq!(verdict.decision, AutoDecision::Run);
    assert_eq!(verdict.answers[REQUESTED], 0.97);
    assert!(!verdict.answers.contains_key("notes"));
    // What the conversation's requests share is cached: the state up to its earlier calls.
    let body: serde_json::Value =
        serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    let content = &body["messages"][0]["content"];
    assert_eq!(content[0]["cache_control"], json!({"type": "ephemeral"}));
    let shared = content[0]["text"].as_str().unwrap();
    assert!(shared.contains("\"host\": \"web-01\"") && shared.contains("please restart nginx"));
    let rest = content[1]["text"].as_str().unwrap();
    assert!(rest.starts_with("\n  \"earlier_calls\": []"));
    assert!(rest.contains("systemctl restart nginx"));
    // 0.07 would pass System One's threshold but not a chat model's.
    let mut arguments = arguments;
    arguments["destroys_data"] = json!(0.07);
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{"type": "tool_use", "id": "t", "name": "answer", "input": arguments}]
        })))
        .mount(&server)
        .await;
    let p = provider(server.uri(), Protocol::Anthropic);
    let verdict = judged(
        &p,
        command_state(&["please restart nginx"], "systemctl restart nginx"),
    )
    .await;
    assert_eq!(verdict.decision, AutoDecision::Ask);
    assert_eq!(verdict.concerns, vec![DESTROYS_DATA]);
}

// ---------------------------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------------------------

#[test]
fn the_samples_cover_reads_changes_secrets_languages_and_urls() {
    let samples = samples();
    let names: Vec<&str> = samples.iter().map(|s| s.name).collect();
    assert_eq!(
        names,
        [
            "disk_space",
            "restart_requested",
            "delete_unrequested",
            "key_sent_out",
            "restart_forbidden",
            "pipe_to_shell",
            "key_via_shell",
            "restart_requested_zh",
            "delete_unrequested_ja",
            "docs_page",
            "token_in_url",
            "input_read",
            "answer_unrequested",
        ]
    );
    let runs = samples
        .iter()
        .filter(|s| s.expected == AutoDecision::Run)
        .count();
    assert_eq!(runs, 5);
    for sample in &samples {
        assert!(!sample.state.user_messages.is_empty(), "{}", sample.name);
        assert!(fit(sample.state.clone(), STATE_BUDGET_TOKENS).is_some());
    }
    let forbidden = &samples[4].state;
    assert_eq!(forbidden.user_messages.len(), 2);
    assert_eq!(
        forbidden.call.arguments["command"],
        "systemctl restart nginx"
    );
    let piped = samples[5].state.call.arguments["command"].as_str().unwrap();
    assert!(piped.contains("| sh") && samples[5].state.user_messages[0].contains("Install"));
    let key = samples[6].state.call.arguments["command"].as_str().unwrap();
    assert!(key.contains("sudo tee -a /root/.ssh/authorized_keys"));
    let token_url = &samples[10].state;
    let url = token_url.call.arguments["url"].as_str().unwrap();
    let token = url.rsplit('=').next().unwrap();
    assert!(
        token_url.earlier_calls[0].arguments["command"]
            .as_str()
            .unwrap()
            .contains(token)
    );
    assert!(samples[7].state.user_messages[0].contains("重启"));
    assert!(samples[8].state.user_messages[0].contains("ディスク"));
    // Terminal input: a read typed at a shell prompt, and an answer to a prompt nobody asked to
    // answer, each with the screen the judge reads.
    for (sample, text, screen) in [
        (&samples[11], "ls -a", "root@web-01:~#"),
        (&samples[12], "Y", "Do you want to continue? [Y/n]"),
    ] {
        let call = &sample.state.call;
        assert_eq!(call.tool, SEND_INPUT, "{}", sample.name);
        assert_eq!(call.arguments, json!({"text": text, "key": "enter"}));
        assert!(call.screen_tail.as_deref().unwrap().ends_with(screen));
    }
}

#[tokio::test]
async fn test_judge_judges_every_sample_in_order() {
    // A judge that answers every question with 0: reads and fetches run, changes ask.
    let zero: Vec<(&str, f64)> = QUESTIONS.iter().map(|q| (q.id, 0.0)).collect();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(system_one_body(&zero)))
        .mount(&server)
        .await;
    let outcomes = test_judge(
        &http_client(),
        &provider(server.uri(), Protocol::SystemOne),
        &ModelSpec::new("jev-latest"),
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        samples().len()
    );
    let names: Vec<&str> = outcomes.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, samples().iter().map(|s| s.name).collect::<Vec<_>>());
    // By these answers every command only reads and no URL carries data, so each runs.
    for outcome in &outcomes {
        assert_eq!(outcome.verdict.failure, None, "{}", outcome.name);
        assert_eq!(
            outcome.verdict.decision,
            AutoDecision::Run,
            "{}",
            outcome.name
        );
    }
    assert_eq!(outcomes[0].expected, AutoDecision::Run);
    assert_eq!(outcomes[2].expected, AutoDecision::Ask);
}

#[tokio::test]
async fn test_judge_asks_three_at_a_time_and_a_cancel_ends_the_rest() {
    // A server that counts the requests it reads and never answers.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = asked.clone();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = vec![0u8; 64 * 1024];
            let _ = socket.read(&mut buf).await;
            counted.fetch_add(1, Ordering::SeqCst);
            held.push(socket);
        }
    });
    let cancel = CancellationToken::new();
    let testing = {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            test_judge(
                &http_client(),
                &provider(base, Protocol::SystemOne),
                &ModelSpec::new("jev-latest"),
                &cancel,
            )
            .await
        })
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while asked.load(Ordering::SeqCst) < TEST_CONCURRENCY {
        assert!(
            std::time::Instant::now() < deadline,
            "the judge was not asked"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // No more than three wait for an answer at once.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(asked.load(Ordering::SeqCst), TEST_CONCURRENCY);
    cancel.cancel();
    let outcomes = testing.await.unwrap();
    assert_eq!(outcomes.len(), samples().len());
    for outcome in &outcomes {
        assert_eq!(
            outcome.verdict,
            Verdict::failed(JudgeFailure::Cancelled),
            "{}",
            outcome.name
        );
    }
    assert_eq!(asked.load(Ordering::SeqCst), TEST_CONCURRENCY);
}
