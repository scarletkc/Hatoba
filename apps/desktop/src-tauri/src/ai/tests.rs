//! Turn tests (spec §12, AI bullet): the turn engine against a mock Chat Completions provider,
//! without Tauri. Approval, edit, rejection, stop, lock and the cancelled results of calls left
//! without one; the tool call limit is the frontend's (AI-18).

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::{Duration, Instant};

use hatoba_ai::entry::{AiEntry, AssistantEntry, EntryBody, Finish, ToolCall, ToolStatus};
use hatoba_core::model::{
    AiConversation, AiEffort as CoreEffort, AiMessage, AiModel, AiProtocol, AiProvider, Item,
    SETTINGS_ID, SearchKind, SearchProvider, Skill, SkillFile,
};
use hatoba_core::sync::SharedVault;
use hatoba_core::{KdfParams, Vault};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};
use zeroize::Zeroizing;

use super::{
    AiEnv, AiManager, COMPACT_INSTRUCTION, COMPACT_MID_TURN_INSTRUCTION, DISCONNECTED, EDIT_NOTE,
    Entries, EventSink,
};
use super::{Halt, Turn, append, lock, title_of, wait_for_results};
use crate::dto::{
    AiConversationDetail, AiEffort, AiEntryView, AiFinish, AiSendInput, AiSendStarted,
    AiToolResultInput, AiToolStatus, AiTurnContext, AiTurnEndReason, AiTurnEvent, QuickTarget,
};
use crate::error::{AppResult, ErrorCode};
use crate::mcp::McpManager;
use crate::mcp::tests::Events as McpEvents;

const PW: &str = "correct horse battery staple";
/// The app version the test environment reports (AI-34).
const VERSION: &str = "9.8.7-test";
const KEY: &str = "sk-test-turns-NEVER-SHOWN";
/// The one session the test environment has open, and what its server sent (§13.1).
const LIVE_SESSION: &str = "live";
const SERVER_ID: &str = "SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13.5";

/// Collects a turn's events.
#[derive(Debug, Default)]
struct Sink(Mutex<Vec<AiTurnEvent>>);

impl EventSink for Sink {
    fn send(&self, event: AiTurnEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Sink {
    fn events(&self) -> Vec<AiTurnEvent> {
        self.0.lock().unwrap().clone()
    }

    async fn until(&self, what: &str, done: impl Fn(&[AiTurnEvent]) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done(&self.events()) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: {:?}",
                kinds(&self.events())
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn done_with(&self, finish: AiFinish) {
        self.until("done", |e| e.contains(&AiTurnEvent::Done { finish }))
            .await;
    }

    async fn ended(&self) -> AiTurnEndReason {
        self.until("turn_ended", |e| e.iter().any(|e| ended(e).is_some()))
            .await;
        let ends: Vec<AiTurnEndReason> = self.events().iter().filter_map(ended).collect();
        assert_eq!(ends.len(), 1, "one turn_ended: {:?}", kinds(&self.events()));
        ends[0]
    }

    fn errors(&self) -> Vec<(Option<u16>, String)> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                AiTurnEvent::Error { status, message } => Some((status, message)),
                _ => None,
            })
            .collect()
    }

    fn entries(&self) -> Vec<AiEntryView> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                AiTurnEvent::Entry { entry } => Some(entry),
                _ => None,
            })
            .collect()
    }
}

fn ended(e: &AiTurnEvent) -> Option<AiTurnEndReason> {
    match e {
        AiTurnEvent::TurnEnded { reason } => Some(*reason),
        _ => None,
    }
}

fn kind(e: &AiTurnEvent) -> &'static str {
    match e {
        AiTurnEvent::RequestStarted => "request_started",
        AiTurnEvent::Text { .. } => "text",
        AiTurnEvent::Reasoning { .. } => "reasoning",
        AiTurnEvent::ToolCall { .. } => "tool_call",
        AiTurnEvent::Usage { .. } => "usage",
        AiTurnEvent::Entry { .. } => "entry",
        AiTurnEvent::Done { .. } => "done",
        AiTurnEvent::Error { .. } => "error",
        AiTurnEvent::EffortIgnored => "effort_ignored",
        AiTurnEvent::TurnEnded { .. } => "turn_ended",
    }
}

fn kinds(events: &[AiTurnEvent]) -> Vec<&'static str> {
    events.iter().map(kind).collect()
}

#[derive(Default)]
struct Env {
    changes: AtomicUsize,
}

impl AiEnv for Env {
    fn changed(&self) {
        self.changes.fetch_add(1, Ordering::SeqCst);
    }

    fn session(&self, _session_id: &str) -> Option<hatoba_ssh::SshSession> {
        None
    }

    /// The session `live` is open, and its server identifies itself as [`SERVER_ID`].
    fn server_id(&self, session_id: &str) -> Option<String> {
        (session_id == LIVE_SESSION).then(|| SERVER_ID.to_owned())
    }

    fn app_version(&self) -> String {
        VERSION.to_owned()
    }
}

/// Scripted provider responses, one per request.
struct Script(Mutex<VecDeque<ResponseTemplate>>);

impl Respond for Script {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        self.0.lock().unwrap().pop_front().unwrap_or_else(|| {
            ResponseTemplate::new(500)
                .set_body_json(json!({"error": {"message": "no scripted response left"}}))
        })
    }
}

fn sse(events: &[Value]) -> ResponseTemplate {
    let mut body = String::new();
    for event in events {
        body.push_str(&format!("data: {event}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body)
}

/// A response without tool calls. `provider_note` is a field only `raw` keeps.
fn answer(text: &str) -> ResponseTemplate {
    sse(&[
        json!({"choices": [{"index": 0, "delta": {"role": "assistant", "content": text, "provider_note": "RAW-ONLY"}}]}),
        json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
        json!({"choices": [], "usage": {"prompt_tokens": 100, "completion_tokens": 5}}),
    ])
}

/// A response that calls tools: `(id, name, arguments)`.
fn calls(calls: &[(&str, &str, Value)]) -> ResponseTemplate {
    let tool_calls: Vec<Value> = calls
        .iter()
        .enumerate()
        .map(|(index, (id, name, arguments))| {
            json!({"index": index, "id": id, "type": "function",
                   "function": {"name": name, "arguments": arguments.to_string()}})
        })
        .collect();
    sse(&[
        json!({"choices": [{"index": 0, "delta": {"role": "assistant", "content": "Checking.", "tool_calls": tool_calls}}]}),
        json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}),
        json!({"choices": [], "usage": {"prompt_tokens": 90, "completion_tokens": 12}}),
    ])
}

/// A server that sends the headers and one text event, then nothing (the stream stays open).
async fn stalling_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 64 * 1024];
        let _ = socket.read(&mut buf).await;
        let event = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hel\"}}]}\n\n";
        let head = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n{:x}\r\n{event}\r\n",
            event.len()
        );
        socket.write_all(head.as_bytes()).await.unwrap();
        tokio::time::sleep(Duration::from_secs(60)).await;
    });
    format!("http://{addr}/v1")
}

struct Fixture {
    server: Option<MockServer>,
    vault: SharedVault,
    manager: AiManager,
    mcp: McpManager,
    mcp_events: Arc<McpEvents>,
    env: Arc<Env>,
    provider_id: String,
}

impl Fixture {
    async fn new(responses: Vec<ResponseTemplate>) -> Self {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(Script(Mutex::new(responses.into())))
            .mount(&server)
            .await;
        let base_url = format!("{}/v1", server.uri());
        Self::with_base_url(Some(server), base_url)
    }

    fn with_base_url(server: Option<MockServer>, base_url: String) -> Self {
        let mut vault = Vault::open_in_memory().unwrap();
        vault
            .create_with_params(PW, KdfParams::for_tests())
            .unwrap();
        let provider_id = vault
            .put(
                None,
                Item::AiProvider(AiProvider {
                    name: "Mock".into(),
                    protocol: AiProtocol::ChatCompletions,
                    base_url,
                    api_key: Zeroizing::new(KEY.into()),
                    models: vec![AiModel {
                        id: "m1".into(),
                        name: "Model 1".into(),
                        ..AiModel::default()
                    }],
                    ..AiProvider::default()
                }),
            )
            .unwrap();
        let (mcp, mcp_events) = crate::mcp::tests::manager();
        Self {
            server,
            vault: Arc::new(Mutex::new(vault)),
            manager: AiManager::with_mcp(mcp.clone()),
            mcp,
            mcp_events,
            env: Arc::new(Env::default()),
            provider_id,
        }
    }

    fn context(&self) -> AiTurnContext {
        AiTurnContext {
            provider_id: self.provider_id.clone(),
            model_id: "m1".into(),
            effort: None,
            host_id: None,
            target: None,
            tab: true,
            session_id: None,
            disabled_mcp_servers: Vec::new(),
        }
    }

    fn env(&self) -> Arc<dyn AiEnv> {
        self.env.clone()
    }

    /// Stores an enabled skill with its files (`SKILL.md` among them).
    fn add_skill(&self, name: &str, description: &str, files: &[(&str, &str)]) -> String {
        let mut v = lock(&self.vault);
        let skill = v
            .put(
                None,
                Item::Skill(Skill {
                    name: name.into(),
                    description: description.into(),
                    enabled: true,
                    ..Skill::default()
                }),
            )
            .unwrap();
        for (path, content) in files {
            v.put(
                None,
                Item::SkillFile(SkillFile {
                    skill_id: skill.clone(),
                    path: (*path).into(),
                    content: (*content).into(),
                    updated_at: 0,
                }),
            )
            .unwrap();
        }
        skill
    }

    async fn send(&self, conversation_id: Option<&str>, text: &str) -> (AiSendStarted, Arc<Sink>) {
        self.send_with(conversation_id, text, self.context()).await
    }

    async fn send_with(
        &self,
        conversation_id: Option<&str>,
        text: &str,
        context: AiTurnContext,
    ) -> (AiSendStarted, Arc<Sink>) {
        let sink = Arc::new(Sink::default());
        let (started, task) = self
            .manager
            .send(
                &self.vault,
                self.env(),
                AiSendInput {
                    conversation_id: conversation_id.map(str::to_owned),
                    text: text.into(),
                    context,
                },
                sink.clone(),
            )
            .await
            .unwrap();
        tokio::spawn(task);
        (started, sink)
    }

    fn retry(&self, conversation_id: &str, context: AiTurnContext) -> AppResult<Arc<Sink>> {
        let sink = Arc::new(Sink::default());
        let task = self.manager.retry(
            &self.vault,
            self.env(),
            conversation_id.to_owned(),
            context,
            sink.clone(),
        )?;
        tokio::spawn(task);
        Ok(sink)
    }

    fn result(
        &self,
        conversation_id: &str,
        tool_call_id: &str,
        status: AiToolStatus,
        content: &str,
        edited_arguments: Option<&str>,
    ) -> AppResult<AiEntryView> {
        self.manager.tool_result(
            &self.vault,
            self.env.as_ref(),
            conversation_id,
            tool_call_id,
            &AiToolResultInput {
                status,
                content: content.into(),
                edited_arguments: edited_arguments.map(str::to_owned),
            },
        )
    }

    async fn run(
        &self,
        conversation_id: &str,
        tool_call_id: &str,
        session_id: Option<&str>,
        edited_arguments: Option<&str>,
    ) -> AppResult<AiEntryView> {
        self.manager
            .tool_run(
                &self.vault,
                self.env.as_ref(),
                conversation_id,
                tool_call_id,
                session_id,
                edited_arguments,
            )
            .await
    }

    fn entries(&self, conversation_id: &str) -> Vec<AiEntry> {
        Entries::load(&lock(&self.vault), conversation_id)
            .unwrap()
            .list
    }

    fn detail(&self, conversation_id: &str) -> AiConversationDetail {
        self.manager
            .conversation_detail(&self.vault, conversation_id)
            .unwrap()
    }

    async fn requests(&self) -> Vec<Value> {
        self.server
            .as_ref()
            .unwrap()
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| r.body_json().unwrap())
            .collect()
    }

    /// Waits until the turn's task has finished and let go of the conversation.
    async fn idle(&self, conversation_id: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while self.manager.is_running(conversation_id) {
            assert!(Instant::now() < deadline, "the turn did not finish");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

/// The messages of a Chat Completions request body.
fn messages(body: &Value) -> &Vec<Value> {
    body["messages"].as_array().unwrap()
}

/// The `tool` message answering `id` in a request body.
fn tool_message<'a>(body: &'a Value, id: &str) -> &'a Value {
    messages(body)
        .iter()
        .find(|m| m["role"] == "tool" && m["tool_call_id"] == id)
        .unwrap_or_else(|| panic!("no tool message for {id}"))
}

fn tool_names(body: &Value) -> Vec<String> {
    body["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .map(|t| t["function"]["name"].as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn tool_entry(entry: &AiEntry) -> (&str, ToolStatus, &str) {
    match &entry.body {
        EntryBody::Tool {
            tool_call_id,
            status,
            content,
        } => (tool_call_id, *status, content),
        other => panic!("expected a tool entry, got {other:?}"),
    }
}

fn view_content(view: &AiEntryView) -> (&str, AiToolStatus, &str) {
    match view {
        AiEntryView::Tool {
            tool_call_id,
            status,
            content,
            ..
        } => (tool_call_id, *status, content),
        other => panic!("expected a tool entry, got {other:?}"),
    }
}

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
        matches!(&started.user_entry, AiEntryView::User { text, .. } if text == "  check the disk\nplease")
    );

    sink.done_with(AiFinish::ToolCalls).await;
    assert!(f.manager.is_running(&conv));
    assert!(f.detail(&conv).running);
    // Nothing goes out until the call has a result.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(f.requests().await.len(), 1);

    let stored = f
        .result(&conv, "call_1", AiToolStatus::Ok, "$ df -h\n42%", None)
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
            "fetch_url",
            "read_skill"
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
        .unwrap_err();
    assert_eq!(again.code, ErrorCode::NotFound);
    assert_eq!(
        f.result(&conv, "nope", AiToolStatus::Ok, "", None)
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

#[tokio::test]
async fn stopping_mid_stream_ends_the_turn_and_stores_nothing() {
    let base_url = stalling_server().await;
    let f = Fixture::with_base_url(None, base_url);
    let (started, sink) = f.send(None, "hello").await;
    let conv = started.conversation.id;
    sink.until("text", |e| {
        e.contains(&AiTurnEvent::Text {
            delta: "Hel".into(),
        })
    })
    .await;

    let at = Instant::now();
    f.manager.stop(&f.vault, f.env.as_ref(), &conv);
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert!(at.elapsed() < Duration::from_secs(5));
    assert!(!f.manager.is_running(&conv));
    // The task notices the cancellation and sends nothing more.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 1);
    assert!(matches!(&entries[0].body, EntryBody::User { .. }));
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
        .unwrap();

    f.manager.stop(&f.vault, f.env.as_ref(), &conv);
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
    // The built-in skill is on by default (AI-34), so read_skill is offered too.
    assert_eq!(tool_names(&requests[0]), ["fetch_url", "read_skill"]);
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
    assert!(matches!(&entries[0].body, EntryBody::User { text } if text == "one"));
    assert!(matches!(&entries[1].body, EntryBody::User { text } if text == "two"));
    assert!(matches!(&entries[2].body, EntryBody::Assistant(a) if a.text == "fast"));
    assert!(
        !first
            .events()
            .iter()
            .any(|e| matches!(e, AiTurnEvent::Entry { .. }))
    );
}

#[tokio::test]
async fn a_provider_error_ends_the_turn_and_retry_goes_on() {
    let f = Fixture::new(vec![
        ResponseTemplate::new(503)
            .set_body_json(json!({"error": {"message": "overloaded, try later"}})),
        answer("Recovered."),
    ])
    .await;
    let (started, sink) = f.send(None, "hi").await;
    let conv = started.conversation.id;
    assert_eq!(sink.ended().await, AiTurnEndReason::Error);
    assert_eq!(
        sink.errors(),
        [(Some(503), "overloaded, try later".to_owned())]
    );
    f.idle(&conv).await;
    assert_eq!(f.entries(&conv).len(), 1);

    let retry = f.retry(&conv, f.context()).unwrap();
    assert_eq!(retry.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 2);
    assert!(matches!(&entries[1].body, EntryBody::Assistant(a) if a.text == "Recovered."));

    // The conversation ends with the model's answer: nothing to retry.
    let refused = f.retry(&conv, f.context()).unwrap_err();
    assert_eq!(refused.code, ErrorCode::InvalidInput);
    assert_eq!(f.requests().await.len(), 2);
}

/// The level the stored conversation keeps (AI-05).
fn stored_effort(f: &Fixture, conversation_id: &str) -> Option<CoreEffort> {
    lock(&f.vault)
        .get(conversation_id)
        .and_then(Item::as_ai_conversation)
        .unwrap()
        .effort
}

#[tokio::test]
async fn the_thinking_level_goes_with_every_request_and_stays_with_the_conversation() {
    let f = Fixture::new(vec![
        calls(&[("call_1", "web_search", json!({"query": "x"}))]),
        answer("Done."),
        ResponseTemplate::new(400).set_body_json(json!({"error": {
            "message": "Unsupported parameter: 'reasoning_effort' is not supported with this model."
        }})),
        answer("Without the level."),
        answer("At Default."),
    ])
    .await;
    let at = |effort| AiTurnContext {
        effort,
        ..f.context()
    };

    // A new conversation stores the level, and each request of the turn carries it. The model
    // offers Low to High (nothing is known about it), so Max is sent as High.
    let (started, sink) = f.send_with(None, "hi", at(Some(AiEffort::Max))).await;
    let conv = started.conversation.id;
    assert_eq!(started.conversation.effort, Some(AiEffort::Max));
    assert_eq!(stored_effort(&f, &conv), Some(CoreEffort::Max));
    sink.done_with(AiFinish::ToolCalls).await;
    f.result(&conv, "call_1", AiToolStatus::Ok, "found", None)
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;

    // A refused level: the request goes again without it, and the panel hears about it.
    let (_, sink) = f
        .send_with(Some(&conv), "again", at(Some(AiEffort::Low)))
        .await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    assert_eq!(
        kinds(&sink.events())
            .iter()
            .filter(|k| **k == "effort_ignored")
            .count(),
        1
    );
    assert_eq!(stored_effort(&f, &conv), Some(CoreEffort::Low));

    // Default sends nothing and is stored as no level.
    let (started, sink) = f.send_with(Some(&conv), "plain", at(None)).await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    assert_eq!(started.conversation.effort, None);
    assert_eq!(stored_effort(&f, &conv), None);
    assert!(!kinds(&sink.events()).contains(&"effort_ignored"));

    let requests = f.requests().await;
    let efforts: Vec<Option<&str>> = requests
        .iter()
        .map(|body| body.get("reasoning_effort").and_then(Value::as_str))
        .collect();
    assert_eq!(
        efforts,
        [Some("high"), Some("high"), Some("low"), None, None]
    );

    // A retry at another level is the one the conversation keeps.
    let retry = f.retry(&conv, at(Some(AiEffort::Medium))).unwrap_err();
    assert_eq!(retry.code, ErrorCode::InvalidInput, "nothing to retry");
    assert_eq!(stored_effort(&f, &conv), None);
}

#[tokio::test]
async fn a_retry_keeps_the_level_it_was_sent_with() {
    let f = Fixture::new(vec![
        ResponseTemplate::new(503).set_body_json(json!({"error": {"message": "busy"}})),
        answer("Recovered."),
    ])
    .await;
    let (started, sink) = f.send(None, "hi").await;
    let conv = started.conversation.id;
    assert_eq!(sink.ended().await, AiTurnEndReason::Error);
    f.idle(&conv).await;
    assert_eq!(stored_effort(&f, &conv), None);

    let context = AiTurnContext {
        effort: Some(AiEffort::Medium),
        ..f.context()
    };
    let retry = f.retry(&conv, context).unwrap();
    assert_eq!(retry.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    assert_eq!(stored_effort(&f, &conv), Some(CoreEffort::Medium));
    assert_eq!(f.requests().await[1]["reasoning_effort"], "medium");
    assert_eq!(f.detail(&conv).conversation.effort, Some(AiEffort::Medium));
}

#[tokio::test]
async fn an_unknown_model_is_an_error_of_the_turn() {
    let f = Fixture::new(Vec::new()).await;
    let sink = Arc::new(Sink::default());
    let (started, task) = f
        .manager
        .send(
            &f.vault,
            f.env(),
            AiSendInput {
                conversation_id: None,
                text: "hi".into(),
                context: AiTurnContext {
                    model_id: "gone".into(),
                    ..f.context()
                },
            },
            sink.clone(),
        )
        .await
        .unwrap();
    tokio::spawn(task);
    assert_eq!(sink.ended().await, AiTurnEndReason::Error);
    assert_eq!(sink.errors().len(), 1);
    assert!(sink.errors()[0].1.contains("gone"));
    assert_eq!(f.entries(&started.conversation.id).len(), 1);
    assert!(f.requests().await.is_empty());

    let empty = f
        .manager
        .send(
            &f.vault,
            f.env(),
            AiSendInput {
                conversation_id: None,
                text: " \n ".into(),
                context: f.context(),
            },
            sink,
        )
        .await;
    assert_eq!(empty.err().unwrap().field.as_deref(), Some("text"));
}

#[tokio::test]
async fn compaction_moves_the_context_start_to_the_summary() {
    let f = Fixture::new(vec![
        answer("First answer."),
        answer("SUMMARY: the user checked the disk."),
        answer("Next answer."),
    ])
    .await;
    let (started, sink) = f.send(None, "first question").await;
    let conv = started.conversation.id;
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    // What the model saw of files (AI-38…40) is in the entries the summary replaces.
    f.manager
        .0
        .seen
        .record(&conv, "ops@db:22", "/etc/hosts", b"x");

    let summary = f
        .manager
        .compact(&f.vault, f.env.as_ref(), &conv, &f.context())
        .await
        .unwrap();
    assert!(!f.manager.0.seen.knows(&conv));
    let AiEntryView::Summary { entry_id, text, .. } = &summary else {
        panic!("expected a summary, got {summary:?}");
    };
    assert_eq!(text, "SUMMARY: the user checked the disk.");
    let detail = f.detail(&conv);
    assert_eq!(detail.conversation.context_start.as_ref(), Some(entry_id));
    assert_eq!(detail.entries.len(), 3);

    let requests = f.requests().await;
    let compact = &requests[1];
    // The request a turn would send, with its tools and system prompt.
    assert_eq!(tool_names(compact), tool_names(&requests[0]));
    assert_eq!(system_of(compact), system_of(&requests[0]));
    let last = messages(compact).last().unwrap();
    assert_eq!(last["role"], "user");
    assert_eq!(last["content"], COMPACT_INSTRUCTION);
    assert!(compact.to_string().contains("first question"));

    let (_, sink) = f.send(Some(&conv), "next question").await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    let requests = f.requests().await;
    let next = requests[2].to_string();
    assert!(!next.contains("first question"));
    assert!(next.contains("This conversation continues from a summary"));
    assert!(next.contains("<summary>\\nSUMMARY: the user checked the disk.\\n</summary>"));
    assert!(next.contains("next question"));
}

fn without_messages(body: &Value) -> Value {
    let mut body = body.clone();
    body.as_object_mut().unwrap().remove("messages");
    body
}

/// AI-21: a Compact request is the request the next turn would send with the instruction after
/// it: the same system prompt, built-in and MCP tools (without the server the conversation
/// switched off) and messages, `raw` replayed, and no `tool_choice`. So the provider's prompt
/// cache serves everything before the instruction.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_compact_request_is_the_next_request_with_the_instruction_after_it() {
    use crate::mcp::tests::{put_server, set_enabled, stdio_server};

    let f = Fixture::new(vec![
        calls(&[("c1", "read_terminal", json!({}))]),
        answer("The disk is fine."),
        answer("SUMMARY: the disk is fine."),
    ])
    .await;
    let files = put_server(&f.vault, stdio_server("files"));
    let other = put_server(&f.vault, stdio_server("other"));
    set_enabled(&f.vault, &files, true);
    set_enabled(&f.vault, &other, true);
    let context = AiTurnContext {
        disabled_mcp_servers: vec![other],
        ..f.context()
    };
    let (started, sink) = f.send_with(None, "check the disk", context.clone()).await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    f.result(&conv, "c1", AiToolStatus::Ok, "Use% 42%", None)
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;

    f.manager
        .compact(&f.vault, f.env.as_ref(), &conv, &context)
        .await
        .unwrap();
    let requests = f.requests().await;
    assert_eq!(requests.len(), 3);
    let (last_turn, compact) = (&requests[1], &requests[2]);
    let names = tool_names(compact);
    assert!(names.contains(&"mcp__files__echo".to_owned()), "{names:?}");
    assert!(
        !names.iter().any(|n| n.starts_with("mcp__other__")),
        "{names:?}"
    );
    assert!(compact.get("tool_choice").is_none());
    assert_eq!(without_messages(compact), without_messages(last_turn));
    let (before, after) = messages(compact).split_at(messages(last_turn).len());
    assert_eq!(before, &messages(last_turn)[..]);
    // The answer, replayed from `raw`, then the instruction.
    assert_eq!(after.len(), 2);
    assert_eq!(after[0]["content"], "The disk is fine.");
    assert_eq!(after[0]["provider_note"], "RAW-ONLY");
    assert_eq!(
        after[1],
        json!({"role": "user", "content": COMPACT_INSTRUCTION})
    );
    f.mcp.stop_all(&f.vault).await;
}

/// An Anthropic response with `text`, a part of its input read from the prompt cache.
fn anthropic_answer(text: &str) -> ResponseTemplate {
    let events = [
        json!({"type": "message_start", "message": {"id": "msg_1", "type": "message",
               "role": "assistant", "model": "m1", "content": [], "stop_reason": null,
               "usage": {"input_tokens": 20, "cache_read_input_tokens": 80, "output_tokens": 1}}}),
        json!({"type": "content_block_start", "index": 0,
               "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0,
               "delta": {"type": "text_delta", "text": text}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"},
               "usage": {"output_tokens": 5}}),
        json!({"type": "message_stop"}),
    ];
    let mut body = String::new();
    for event in events {
        body.push_str(&format!(
            "event: {}\ndata: {event}\n\n",
            event["type"].as_str().unwrap()
        ));
    }
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body)
}

/// AI-21 on Anthropic: the Compact request's body is the next request's with the instruction
/// after its messages. Only `max_tokens` differs, which is not part of the cached prefix: 16,000
/// instead of the model's whole output limit, so that it fits beside a nearly full context.
#[tokio::test]
async fn an_anthropic_compact_request_keeps_the_prefix_and_asks_for_less_output() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(Script(Mutex::new(
            vec![
                anthropic_answer("First answer."),
                anthropic_answer("Second answer."),
                anthropic_answer("SUMMARY: two questions."),
            ]
            .into(),
        )))
        .mount(&server)
        .await;
    let base_url = server.uri();
    let f = Fixture::with_base_url(Some(server), base_url);
    {
        let mut v = lock(&f.vault);
        let mut provider = v
            .get(&f.provider_id)
            .and_then(Item::as_ai_provider)
            .cloned()
            .unwrap();
        provider.protocol = AiProtocol::Anthropic;
        provider.models[0].context_window = Some(200_000);
        provider.models[0].max_output_tokens = Some(64_000);
        v.put(Some(&f.provider_id), Item::AiProvider(provider))
            .unwrap();
    }
    let (conv, _) = f.exchange(None, "first question", f.context()).await;
    f.exchange(Some(&conv), "second question", f.context())
        .await;
    let summary = f
        .manager
        .compact(&f.vault, f.env.as_ref(), &conv, &f.context())
        .await
        .unwrap();
    assert!(matches!(summary, AiEntryView::Summary { .. }));

    let requests = f.requests().await;
    let (last_turn, compact) = (&requests[1], &requests[2]);
    assert_eq!(last_turn["max_tokens"], 64_000);
    assert_eq!(compact["max_tokens"], 16_000);
    let rest = |body: &Value| {
        let mut body = without_messages(body);
        body.as_object_mut().unwrap().remove("max_tokens");
        body
    };
    assert_eq!(rest(compact), rest(last_turn));
    assert!(!compact["tools"].as_array().unwrap().is_empty());
    let (before, after) = messages(compact).split_at(messages(last_turn).len());
    assert_eq!(before, &messages(last_turn)[..]);
    assert_eq!(
        after,
        [
            json!({"role": "assistant", "content": [{"type": "text", "text": "Second answer."}]}),
            json!({"role": "user", "content": [{"type": "text", "text": COMPACT_INSTRUCTION}]}),
        ]
    );
}

/// AI-21: an answer to a Compact request that calls tools, or has no text, is dropped (nothing
/// runs and nothing is stored) and the request goes once more without tools, with the system
/// prompt that says why. Only once: when that answer is no summary either (an endpoint that
/// still calls tools, its text only the preamble), the compaction fails.
#[tokio::test]
async fn a_compact_answer_without_a_summary_is_asked_again_without_tools() {
    let f = Fixture::new(vec![
        answer("First answer."),
        answer("   "),
        // "Checking." and a call, from a request without tools.
        calls(&[("x0", "read_terminal", json!({}))]),
        calls(&[("x1", "run_command", json!({"command": "rm -rf /"}))]),
        answer("SUMMARY from the request without tools."),
    ])
    .await;
    let (started, sink) = f.send(None, "first question").await;
    let conv = started.conversation.id;
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;

    let err = f
        .manager
        .compact(&f.vault, f.env.as_ref(), &conv, &f.context())
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Ai);
    assert_eq!(err.detail, "the model returned no summary");
    assert_eq!(f.requests().await.len(), 3);
    assert_eq!(f.entries(&conv).len(), 2);

    let summary = f
        .manager
        .compact(&f.vault, f.env.as_ref(), &conv, &f.context())
        .await
        .unwrap();
    let AiEntryView::Summary { text, .. } = &summary else {
        panic!("expected a summary, got {summary:?}");
    };
    assert_eq!(text, "SUMMARY from the request without tools.");
    let requests = f.requests().await;
    assert_eq!(requests.len(), 5);
    for (fork, plain) in [(&requests[1], &requests[2]), (&requests[3], &requests[4])] {
        assert!(!tool_names(fork).is_empty());
        assert!(plain.get("tools").is_none());
        assert!(
            system_of(plain).contains("only asks for a summary of the conversation"),
            "{}",
            system_of(plain)
        );
        assert_eq!(messages(plain)[1..], messages(fork)[1..]);
        assert_eq!(
            messages(plain).last().unwrap()["content"],
            COMPACT_INSTRUCTION
        );
    }
    // The dropped calls never ran and were never stored.
    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 3);
    assert!(matches!(&entries[2].body, EntryBody::Summary { .. }));
}

#[test]
fn a_compact_request_asks_for_output_that_fits_beside_the_context() {
    use hatoba_ai::provider::ModelSpec;

    for (window, output, expected) in [
        (Some(200_000), Some(64_000), 16_000),
        (Some(128_000), Some(16_384), 11_520),
        (Some(1_000_000), Some(8_000), 8_000),
        (None, Some(4_096), 4_096),
        (None, None, 16_000),
        (Some(1_000), None, 90),
    ] {
        let model = ModelSpec {
            context_window: window,
            max_output_tokens: output,
            ..ModelSpec::new("m1")
        };
        assert_eq!(
            super::compact_output_tokens(&model),
            expected,
            "window {window:?}, output {output:?}"
        );
    }
}

#[tokio::test]
async fn compaction_is_refused_while_a_turn_runs() {
    let f = Fixture::new(vec![calls(&[("call_1", "read_terminal", json!({}))])]).await;
    let (started, sink) = f.send(None, "look").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let err = f
        .manager
        .compact(&f.vault, f.env.as_ref(), &conv, &f.context())
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    f.manager.stop(&f.vault, f.env.as_ref(), &conv);
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
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
        system.contains("rather than with cat, sed or heredocs"),
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
            "fetch_url",
            "read_skill"
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

#[tokio::test]
async fn deleting_a_conversation_stops_its_turn() {
    let f = Fixture::new(vec![calls(&[("call_1", "read_terminal", json!({}))])]).await;
    let (started, sink) = f.send(None, "look").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    f.manager
        .delete_conversation(&f.vault, f.env.as_ref(), &conv)
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
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
    assert!(!system.contains("run_command runs"), "{system}");

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

// ───────────────────────── phase 2: MCP, AI-22, AI-26, AI-24 ─────────────────────────

impl Fixture {
    /// Gives the provider's model a known context window (AI-20).
    fn set_context_window(&self, window: u64) {
        let mut v = lock(&self.vault);
        let mut provider = v
            .get(&self.provider_id)
            .and_then(Item::as_ai_provider)
            .cloned()
            .unwrap();
        provider.models[0].context_window = Some(window);
        v.put(Some(&self.provider_id), Item::AiProvider(provider))
            .unwrap();
    }

    async fn edit(
        &self,
        conversation_id: &str,
        entry_id: &str,
        text: &str,
    ) -> AppResult<(AiSendStarted, Arc<Sink>)> {
        let sink = Arc::new(Sink::default());
        let (started, task) = self
            .manager
            .edit_resend(
                &self.vault,
                self.env(),
                conversation_id.to_owned(),
                entry_id,
                text.to_owned(),
                self.context(),
                sink.clone(),
            )
            .await?;
        tokio::spawn(task);
        Ok((started, sink))
    }

    fn context_start(&self, conversation_id: &str) -> Option<String> {
        self.detail(conversation_id).conversation.context_start
    }
}

fn entry_id(view: &AiEntryView) -> &str {
    match view {
        AiEntryView::User { entry_id, .. }
        | AiEntryView::Assistant { entry_id, .. }
        | AiEntryView::Tool { entry_id, .. }
        | AiEntryView::Summary { entry_id, .. } => entry_id,
    }
}

fn summaries(sink: &Sink) -> Vec<AiEntryView> {
    sink.entries()
        .into_iter()
        .filter(|e| matches!(e, AiEntryView::Summary { .. }))
        .collect()
}

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
            "fetch_url",
            "read_skill",
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
        crate::commands::mcp::tool_info(&lock(&f.vault), &f.mcp, &conv, name).expect("tool info")
    };
    let echo = info("mcp__files__echo");
    assert_eq!(
        (echo.server_id.as_str(), echo.server_name.as_str()),
        (files.as_str(), "files")
    );
    assert!(!echo.always_ask && !echo.tool.always_allow);
    assert_eq!(echo.tool.tool, "echo");
    assert_eq!(echo.tool.annotations.read_only_hint, Some(true));
    crate::commands::mcp::set_always_allow(&mut lock(&f.vault), &files, Some("echo"), true)
        .unwrap();
    assert!(info("mcp__files__echo").tool.always_allow);
    assert!(!info("mcp__files__fail").tool.always_allow);
    crate::commands::mcp::set_always_allow(&mut lock(&f.vault), &web, None, true).unwrap();
    assert!(info("mcp__web__lookup").tool.always_allow);
    {
        let mut v = lock(&f.vault);
        let mut server = v.get(&web).and_then(Item::as_mcp_server).cloned().unwrap();
        server.always_ask = true;
        v.put(Some(&web), Item::McpServer(server)).unwrap();
    }
    assert!(info("mcp__web__lookup").always_ask);
    assert!(
        crate::commands::mcp::tool_info(&lock(&f.vault), &f.mcp, &conv, "mcp__nope__x").is_none()
    );

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
async fn a_new_message_that_would_fill_the_context_is_sent_after_a_compaction() {
    let f = Fixture::new(vec![
        answer("First answer."),
        answer("SUMMARY: the user asked a first question."),
        answer("Second answer."),
    ])
    .await;
    // 90% of 1,000 tokens: the first answer used 105, and the long message adds 900 more.
    f.set_context_window(1_000);
    let (started, sink) = f.send(None, "first question").await;
    let conv = started.conversation.id;
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    assert_eq!(f.requests().await.len(), 1);

    let long = "disk ".repeat(720);
    let (started, sink) = f.send(Some(&conv), &long).await;
    // The summary was stored and sent on the turn's channel before the message was stored.
    let summary = summaries(&sink);
    assert_eq!(summary.len(), 1);
    let summary_id = entry_id(&summary[0]).to_owned();
    assert!(entry_id(&started.user_entry) > summary_id.as_str());
    assert_eq!(
        started.conversation.context_start.as_deref(),
        Some(summary_id.as_str())
    );
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;

    let requests = f.requests().await;
    assert_eq!(requests.len(), 3);
    let compact = &requests[1];
    assert_eq!(tool_names(compact), tool_names(&requests[0]));
    assert_eq!(
        messages(compact).last().unwrap()["content"],
        COMPACT_INSTRUCTION
    );
    assert!(compact.to_string().contains("first question"));
    assert!(!compact.to_string().contains("disk disk"));
    // The request carries the summary and the new message, not what came before.
    let next = requests[2].to_string();
    assert!(next.contains("SUMMARY: the user asked a first question."));
    assert!(next.contains("disk disk"));
    assert!(!next.contains("First answer."));
    let entries = f.entries(&conv);
    assert!(matches!(&entries[2].body, EntryBody::Summary { .. }));
    assert!(matches!(&entries[3].body, EntryBody::User { text } if *text == long));
}

#[tokio::test]
async fn a_full_context_between_tool_calls_is_compacted_once() {
    let long_summary = format!("SUMMARY-2 {}", "y".repeat(4_000));
    let f = Fixture::new(vec![
        calls(&[("c1", "read_terminal", json!({}))]),
        answer(&long_summary),
        answer("Done."),
    ])
    .await;
    f.set_context_window(1_000);
    let (started, sink) = f.send(None, "look at the screen").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    f.manager
        .0
        .seen
        .record(&conv, "ops@db:22", "/etc/hosts", b"x");
    // The screen alone is about 1,000 tokens.
    f.result(&conv, "c1", AiToolStatus::Ok, &"z".repeat(4_000), None)
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    // AI-22 forgets what the model saw of files, as Compact does (AI-38…40).
    assert!(!f.manager.0.seen.knows(&conv));

    let requests = f.requests().await;
    assert_eq!(requests.len(), 3);
    // The request the turn was about to send, asking where the task stands.
    let compact = &requests[1];
    assert_eq!(tool_names(compact), tool_names(&requests[0]));
    let roles: Vec<&Value> = messages(compact).iter().map(|m| &m["role"]).collect();
    assert_eq!(roles, ["system", "user", "assistant", "tool", "user"]);
    assert_eq!(
        messages(compact).last().unwrap()["content"],
        COMPACT_MID_TURN_INSTRUCTION
    );
    // The summary is over 90% too, but it is never compacted again right away.
    let next = requests[2].to_string();
    assert!(next.contains("SUMMARY-2"));
    assert!(!next.contains("look at the screen"));
    assert!(!tool_names(&requests[2]).is_empty());
    let summary = summaries(&sink);
    assert_eq!(summary.len(), 1);
    assert_eq!(
        f.context_start(&conv).as_deref(),
        Some(entry_id(&summary[0]))
    );
}

#[tokio::test]
async fn editing_a_message_deletes_it_and_what_followed_and_repairs_the_context_start() {
    let f = Fixture::new(vec![
        answer("Answer to gamma-edited."),
        answer("Answer to beta-edited."),
        answer("Answer to alpha-edited."),
    ])
    .await;
    let assistant = |text: &str| {
        AiEntry::assistant(
            2,
            AssistantEntry {
                provider_id: f.provider_id.clone(),
                model_id: "m1".into(),
                text: text.into(),
                finish: Finish::Stop,
                ..AssistantEntry::default()
            },
        )
    };
    // alpha, its answer, summary 1, beta, its answer, summary 2, gamma, its answer.
    let (conv, ids) = {
        let mut v = lock(&f.vault);
        let conv = v
            .put(
                None,
                Item::AiConversation(AiConversation {
                    title: "edit".into(),
                    ..AiConversation::default()
                }),
            )
            .unwrap();
        let entries = [
            AiEntry::user(1, "alpha"),
            assistant("answer alpha"),
            AiEntry::summary(3, "SUMMARY ONE"),
            AiEntry::user(4, "beta"),
            assistant("answer beta"),
            AiEntry::summary(6, "SUMMARY TWO"),
            AiEntry::user(7, "gamma"),
            assistant("answer gamma"),
        ];
        let ids: Vec<String> = entries
            .iter()
            .map(|e| append(&mut v, &conv, e).unwrap())
            .collect();
        let mut c = v
            .get(&conv)
            .and_then(Item::as_ai_conversation)
            .cloned()
            .unwrap();
        c.context_start = Some(ids[5].clone());
        v.put(Some(&conv), Item::AiConversation(c)).unwrap();
        (conv, ids)
    };

    // Only the user's own messages can be edited.
    let err = f.edit(&conv, &ids[1], "x").await.err().unwrap();
    assert_eq!(err.field.as_deref(), Some("entry_id"));
    let err = f
        .edit(&conv, "0190a0a0-0000-7000-8000-000000000000", "x")
        .await;
    assert_eq!(err.err().unwrap().code, ErrorCode::NotFound);
    let err = f.edit(&conv, &ids[6], "  ").await.err().unwrap();
    assert_eq!(err.field.as_deref(), Some("text"));
    assert_eq!(f.entries(&conv).len(), 8);

    // gamma comes after the context start (summary 2): the start stays. What the model saw of
    // files (AI-38…40) may be in the deleted entries, so it is forgotten.
    f.manager
        .0
        .seen
        .record(&conv, "ops@db:22", "/etc/hosts", b"x");
    let (started, sink) = f.edit(&conv, &ids[6], "gamma-edited").await.unwrap();
    assert!(!f.manager.0.seen.knows(&conv));
    assert_eq!(
        started.conversation.context_start.as_deref(),
        Some(ids[5].as_str())
    );
    assert!(entry_id(&started.user_entry) > ids[7].as_str());
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    let texts: Vec<String> = f
        .entries(&conv)
        .iter()
        .map(|e| match &e.body {
            EntryBody::User { text } | EntryBody::Summary { text } => text.clone(),
            EntryBody::Assistant(a) => a.text.clone(),
            EntryBody::Tool { content, .. } => content.clone(),
        })
        .collect();
    assert_eq!(
        texts,
        [
            "alpha",
            "answer alpha",
            "SUMMARY ONE",
            "beta",
            "answer beta",
            "SUMMARY TWO",
            "gamma-edited",
            "Answer to gamma-edited."
        ]
    );
    let request = f.requests().await[0].to_string();
    assert!(request.contains("SUMMARY TWO") && request.contains("gamma-edited"));
    assert!(!request.contains("answer gamma") && !request.contains("beta"));

    // beta comes before it: summary 2 is deleted, so the start moves to summary 1.
    let (started, sink) = f.edit(&conv, &ids[3], "beta-edited").await.unwrap();
    assert_eq!(
        started.conversation.context_start.as_deref(),
        Some(ids[2].as_str())
    );
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    assert_eq!(f.entries(&conv).len(), 5);
    assert_eq!(f.context_start(&conv).as_deref(), Some(ids[2].as_str()));

    // alpha comes before every summary: the start is cleared.
    let (started, sink) = f.edit(&conv, &ids[0], "alpha-edited").await.unwrap();
    assert_eq!(started.conversation.context_start, None);
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 2);
    assert!(matches!(&entries[0].body, EntryBody::User { text } if text == "alpha-edited"));
    let request = f.requests().await[2].to_string();
    assert!(!request.contains("SUMMARY ONE"));
}

#[tokio::test]
async fn editing_while_a_turn_runs_stops_it_first() {
    let f = Fixture::new(vec![
        calls(&[("c1", "read_terminal", json!({}))]),
        answer("Edited answer."),
    ])
    .await;
    let (started, first) = f.send(None, "original").await;
    let conv = started.conversation.id;
    let user = entry_id(&started.user_entry).to_owned();
    first.done_with(AiFinish::ToolCalls).await;
    let (_, second) = f.edit(&conv, &user, "edited").await.unwrap();
    assert_eq!(first.ended().await, AiTurnEndReason::Stopped);
    assert_eq!(second.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    // The open call and its cancelled result went with the edited message.
    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 2);
    assert!(matches!(&entries[0].body, EntryBody::User { text } if text == "edited"));
    assert!(
        !second
            .entries()
            .iter()
            .any(|e| matches!(e, AiEntryView::Tool { .. }))
    );
}

#[test]
fn titles_skip_the_attachment_blocks_a_message_starts_with() {
    // AI-23: the first line of the first message, cut to 60 characters.
    assert_eq!(title_of("  check the disk\nplease"), "check the disk");
    assert_eq!(title_of(&"长".repeat(80)).chars().count(), 60);

    // AI-10: the title comes from the typed text, not from the block (spec §13.3).
    let selection = "<terminal_selection host=\"prod-api\" lines=\"2\">\n$ make\nerror: boom\n\
                     </terminal_selection>\n\n  \nWhy does make fail?\nmore";
    assert_eq!(title_of(selection), "Why does make fail?");
    // Escaped attribute values, a truncated selection, and closing tags escaped in the body.
    let escaped = "<terminal_selection host=\"web &quot;a&quot; &lt;b&gt;\" lines=\"3\" truncated=\"true\">\n\
                   echo \"<\\/terminal_selection>\"\n<\\\\/terminal_selection> and <\\/TERMINAL_SELECTION>\n\
                   </terminal_selection>\n\nexplain this";
    assert_eq!(title_of(escaped), "explain this");
    // A closing tag that other text follows on its line does not end the block.
    let inner = "<terminal_selection host=\"x\" lines=\"3\">\na\n</terminal_selection> tail\nb\n\
                 </terminal_selection>\nwhat now";
    assert_eq!(title_of(inner), "what now");
    let diagnostics = "<connection_diagnostics host=\"db-1\">\nConnection refused (os error 111)\n\
                       </connection_diagnostics>\n\nWhy can't I connect?";
    assert_eq!(title_of(diagnostics), "Why can't I connect?");
    // Both kinds, one after the other.
    let both = "<connection_diagnostics host=\"db-1\">\nrefused\n</connection_diagnostics>\n\n\
                <terminal_selection host=\"db-1\" lines=\"1\">\n$ ssh db-1\n</terminal_selection>\n\nhelp";
    assert_eq!(title_of(both), "help");

    // Nothing typed: the block's name.
    assert_eq!(
        title_of("<terminal_selection host=\"x\" lines=\"1\">\nls\n</terminal_selection>"),
        "Terminal selection"
    );
    assert_eq!(
        title_of("<connection_diagnostics host=\"x\">\nboom\n</connection_diagnostics>\n\n \n"),
        "Connection diagnostics"
    );

    // AI-35: pasted text and files, several of each, after the diagnostics and the selection.
    let many = "<connection_diagnostics host=\"db-1\">\nrefused\n</connection_diagnostics>\n\n\
                <terminal_selection host=\"db-1\" lines=\"1\">\n$ ssh db-1\n</terminal_selection>\n\n\
                <pasted_text lines=\"2\">\na\n<\\/pasted_text>\n</pasted_text>\n\n\
                <file name=\"nginx.conf\" lines=\"1\">\nserver {}\n</file>\n\n\
                <pasted_text lines=\"1\">\nb\n</pasted_text>\n\n\
                <file name=\"a.log\" lines=\"1\">\n</file> inside\n</file>\n\n\
                What is wrong here?";
    assert_eq!(title_of(many), "What is wrong here?");
    assert_eq!(
        title_of("<pasted_text lines=\"3\">\na\nb\nc\n</pasted_text>"),
        "Pasted text"
    );
    // A file's name, with its attribute escapes read back.
    assert_eq!(
        title_of(
            "<file name=\"a &quot;b&quot; &lt;c&gt; &amp; d.txt\" lines=\"1\">\nx\n</file>\n\n"
        ),
        "a \"b\" <c> & d.txt"
    );
    assert_eq!(
        title_of(&format!(
            "<file name=\"{}.txt\" lines=\"1\">\nx\n</file>",
            "长".repeat(70)
        ))
        .chars()
        .count(),
        60
    );

    // AI-05, AI-09: Hatoba's notes come first and never title the conversation.
    let notes = "<host_change from=\"staging-web\" to=\"prod-db\">\nThe conversation moved.\n\
                 </host_change>\n\n<model_change from=\"A (a)\" to=\"B (b)\">\nOther model.\n\
                 </model_change>\n\n";
    assert_eq!(
        title_of(&format!(
            "{notes}<file name=\"a.log\" lines=\"1\">\nx\n</file>\n\nWhat now?"
        )),
        "What now?"
    );
    assert_eq!(
        title_of(&format!(
            "{notes}<pasted_text lines=\"1\">\nx\n</pasted_text>"
        )),
        "Pasted text"
    );
    assert_eq!(title_of(notes), "");
    // A note whose attributes are not the kind's own, a note after an attachment, or a second
    // note of a kind, is typed text.
    let odd = "<host_change to=\"b\" from=\"a\">\nx\n</host_change>\n\nq";
    assert_eq!(title_of(odd), "<host_change to=\"b\" from=\"a\">");
    let late = "<pasted_text lines=\"1\">\nx\n</pasted_text>\n\n\
                <host_change from=\"a\" to=\"b\">\nx\n</host_change>\n\nq";
    assert_eq!(title_of(late), "<host_change from=\"a\" to=\"b\">");
    let twice = "<model_change from=\"a\" to=\"b\">\nx\n</model_change>\n\n\
                 <model_change from=\"b\" to=\"c\">\nx\n</model_change>\n\nq";
    assert_eq!(title_of(twice), "<model_change from=\"b\" to=\"c\">");

    // Anything else is typed text.
    for text in [
        "see <terminal_selection host=\"x\" lines=\"1\">\nx\n</terminal_selection>",
        "<terminal_selection host=\"x\" lines=\"1\">\nno end",
        "<terminal_selection host=\"x\">\nno lines\n</terminal_selection>\n\nq",
        "<terminal_selection host=\"x\" lines=\"one\">\nx\n</terminal_selection>\n\nq",
        "<terminal_selection host=\"x\" lines=\"1\" truncated=\"yes\">\nx\n</terminal_selection>\n\nq",
        "<connection_diagnostics host=\"x\" lines=\"1\">\nx\n</connection_diagnostics>\n\nq",
        "<other_block host=\"x\">\nx\n</other_block>\n\nq",
        "<pasted_text>\nx\n</pasted_text>\n\nq",
        "<pasted_text lines=\"x\">\nx\n</pasted_text>\n\nq",
        "<file lines=\"1\" name=\"a.txt\">\nx\n</file>\n\nq",
        "<file name=\"a.txt\">\nx\n</file>\n\nq",
        "<files name=\"a.txt\" lines=\"1\">\nx\n</files>\n\nq",
        "<terminal_selection host=\"x\" lines=\"1\">x\n</terminal_selection>\n\nq",
    ] {
        assert_eq!(
            title_of(text),
            text.lines().next().unwrap().trim(),
            "{text:?}"
        );
    }
}

#[tokio::test]
async fn a_message_with_a_selection_keeps_the_block_and_is_titled_after_the_typed_text() {
    let f = Fixture::new(vec![answer("It failed to compile.")]).await;
    let text = "<terminal_selection host=\"prod-api\" lines=\"1\">\nerror: boom\n\
                </terminal_selection>\n\nWhy?";
    let (started, sink) = f.send(None, text).await;
    assert_eq!(started.conversation.title, "Why?");
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    // The block is meant for the model: the request carries the message as stored.
    let requests = f.requests().await;
    let user = messages(&requests[0])
        .iter()
        .find(|m| m["role"] == "user")
        .unwrap();
    assert_eq!(user["content"], text);
}

#[tokio::test]
async fn a_message_with_attachments_as_large_as_the_panel_allows_is_stored_whole() {
    // AI-35: the panel caps a message's attachments at 512 KB; the entry is split into parts of at
    // most 40 KB (§13.7), and nothing on the way cuts it.
    let f = Fixture::new(vec![answer("Read it all.")]).await;
    let file =
        "2026-10-09 09:41:12 nginx[812]: upstream \"timed out\" <\\/file> 日本語\n".repeat(3_500);
    let paste = "p".repeat(262_000);
    let text = format!(
        "<pasted_text lines=\"1\">\n{paste}\n</pasted_text>\n\n\
         <file name=\"error.log\" lines=\"3500\">\n{file}\n</file>\n\nWhat failed?"
    );
    assert!(text.len() > 500 * 1024, "{} bytes", text.len());
    let (started, sink) = f.send(None, &text).await;
    assert_eq!(started.conversation.title, "What failed?");
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    let detail = f.detail(&started.conversation.id);
    let stored = detail
        .entries
        .iter()
        .find_map(|e| match e {
            AiEntryView::User { text, .. } => Some(text.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(stored, text);
    // The model receives the message as stored.
    let requests = f.requests().await;
    let user = messages(&requests[0])
        .iter()
        .find(|m| m["role"] == "user")
        .unwrap();
    assert_eq!(user["content"], text.as_str());
}

#[test]
fn search_finds_titles_and_message_text_once_per_conversation() {
    let vault: SharedVault = {
        let mut vault = Vault::open_in_memory().unwrap();
        vault
            .create_with_params(PW, KdfParams::for_tests())
            .unwrap();
        Arc::new(Mutex::new(vault))
    };
    let conversation = |title: &str, entries: &[AiEntry]| -> (String, Vec<String>) {
        let mut v = lock(&vault);
        let id = v
            .put(
                None,
                Item::AiConversation(AiConversation {
                    title: title.into(),
                    ..AiConversation::default()
                }),
            )
            .unwrap();
        let ids = entries
            .iter()
            .map(|e| append(&mut v, &id, e).unwrap())
            .collect();
        (id, ids)
    };
    let answer = |text: &str| {
        AiEntry::assistant(
            2,
            AssistantEntry {
                text: text.into(),
                ..AssistantEntry::default()
            },
        )
    };
    let long = format!(
        "{}needle in the middle{}",
        "a ".repeat(100),
        " b".repeat(100)
    );
    let (disk, disk_ids) = conversation(
        "Disk usage on prod",
        &[
            AiEntry::user(1, "how full is /var?"),
            answer("It is at 42% now."),
            AiEntry::tool(3, "call_1", ToolStatus::Ok, "tool-only-needle"),
        ],
    );
    let (nginx, nginx_ids) = conversation(
        "nginx",
        &[
            AiEntry::user(1, "The NGINX config\nhas a typo"),
            AiEntry::user(2, "nginx again"),
            AiEntry::summary(3, &long),
            AiEntry::user(4, "Ärger mit Umlauten"),
        ],
    );

    let search = |q: &str| super::search(&vault, q).unwrap();
    assert!(search("   ").is_empty());
    // Only the title matches: no entry.
    let hits = search("DISK");
    assert_eq!(hits.len(), 1);
    assert_eq!(
        (
            hits[0].conversation_id.as_str(),
            hits[0].entry_id.as_deref()
        ),
        (disk.as_str(), None)
    );
    assert_eq!(hits[0].snippet, "Disk usage on prod");
    // An answer's text.
    let hits = search("42%");
    assert_eq!(hits[0].entry_id.as_deref(), Some(disk_ids[1].as_str()));
    assert_eq!(hits[0].snippet, "It is at 42% now.");
    // Tool results are not searched.
    assert!(search("tool-only-needle").is_empty());
    // One hit per conversation: the first matching entry, before the title.
    let hits = search("nginx");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].conversation_id, nginx);
    assert_eq!(hits[0].entry_id.as_deref(), Some(nginx_ids[0].as_str()));
    assert_eq!(hits[0].snippet, "The NGINX config has a typo");
    // A snippet of about 120 characters around the match, cut with an ellipsis.
    let hits = search("NEEDLE IN");
    assert_eq!(hits[0].entry_id.as_deref(), Some(nginx_ids[2].as_str()));
    let snippet = &hits[0].snippet;
    assert!(
        snippet.starts_with('…') && snippet.ends_with('…'),
        "{snippet}"
    );
    assert!(snippet.contains("needle in the middle"));
    assert!(snippet.chars().count() <= 122, "{snippet}");
    // Case folding beyond ASCII.
    assert_eq!(search("ärger").len(), 1);
    // One hit for each conversation that matches.
    assert_eq!(search("a").len(), 2);

    lock(&vault).lock();
    assert_eq!(
        super::search(&vault, "x").unwrap_err().code,
        ErrorCode::Locked
    );
}

#[test]
fn skills_saved_in_settings_are_listed_and_read_by_read_skill() {
    use crate::dto::{SkillFileView, SkillInput};
    use hatoba_ai::tools::ReadSkillArgs;

    let f = Fixture::with_base_url(None, "http://127.0.0.1:9/v1".into());
    let mut v = lock(&f.vault);
    let skill = |name: &str, enabled: bool| SkillInput {
        id: None,
        name: name.into(),
        description: format!("About {name}"),
        enabled,
        body: format!("Body of {name}"),
        files: vec![SkillFileView {
            path: "references/tls.md".into(),
            content: format!("TLS of {name}"),
        }],
    };
    crate::commands::skills::save_skill(&mut v, &skill("nginx", true)).unwrap();
    crate::commands::skills::save_skill(&mut v, &skill("apache", false)).unwrap();

    // AI-28: only enabled skills are listed.
    let (system, tools) = super::prompt(&v, &f.context(), true, None);
    assert!(system.contains("nginx: About nginx"), "{system}");
    assert!(!system.contains("apache"), "{system}");
    assert!(tools.iter().any(|t| t.name == "read_skill"));

    let read = |v: &Vault, name: &str, path: Option<&str>| match super::read_skill(
        v,
        &ReadSkillArgs {
            name: name.into(),
            path: path.map(str::to_owned),
        },
        VERSION,
    ) {
        super::Job::Ready(status, content) => (status, content),
        _ => panic!("read_skill reads the vault"),
    };
    assert_eq!(
        read(&v, "nginx", None),
        (ToolStatus::Ok, "Body of nginx".into())
    );
    assert_eq!(
        read(&v, "nginx", Some("./references\\tls.md")),
        (ToolStatus::Ok, "TLS of nginx".into())
    );
    assert_eq!(read(&v, "apache", None).0, ToolStatus::Error);

    // AI-34: the built-in skill is listed with them, sorted by name, and read with the version.
    // A user skill from an older build that took its name is shadowed.
    v.put(
        None,
        Item::Skill(Skill {
            name: "hatoba".into(),
            description: "Legacy notes".into(),
            enabled: true,
            ..Skill::default()
        }),
    )
    .unwrap();
    let builtin = hatoba_ai::skills::builtin_skill(VERSION);
    let (system, _) = super::prompt(&v, &f.context(), true, None);
    assert!(
        system.contains(&format!(
            "- hatoba: {}\n- nginx: About nginx\n",
            builtin.description
        )),
        "{system}"
    );
    assert!(!system.contains("Legacy notes"), "{system}");
    let (status, body) = read(&v, "hatoba", None);
    assert_eq!(
        (status, body.as_str()),
        (ToolStatus::Ok, builtin.body.as_str())
    );
    assert!(body.contains(VERSION));
    let file = &builtin.files[0];
    assert_eq!(
        read(&v, "hatoba", Some(&file.path)),
        (ToolStatus::Ok, file.content.clone())
    );
    let (status, missing) = read(&v, "hatoba", Some("nope.md"));
    assert_eq!(status, ToolStatus::Error);
    assert!(missing.contains("references/"), "{missing}");

    // Switched off in Settings → AI: neither listed nor read; the old skill stays shadowed.
    let mut settings = v.settings();
    settings.ai.builtin_skill_enabled = false;
    v.put(Some(SETTINGS_ID), Item::Settings(settings)).unwrap();
    let (system, tools) = super::prompt(&v, &f.context(), true, None);
    assert!(!system.contains("- hatoba:"), "{system}");
    assert!(tools.iter().any(|t| t.name == "read_skill"));
    let (status, refused) = read(&v, "hatoba", None);
    assert_eq!(status, ToolStatus::Error);
    assert!(refused.contains("Enabled skills: nginx."), "{refused}");

    // With no enabled skill at all, read_skill is not offered.
    let nginx = v
        .skills()
        .into_iter()
        .find(|(_, s)| s.name == "nginx")
        .unwrap()
        .0;
    crate::commands::skills::set_skill_enabled(&mut v, &nginx, false).unwrap();
    let (_, tools) = super::prompt(&v, &f.context(), true, None);
    assert!(!tools.iter().any(|t| t.name == "read_skill"));
}

/// `ai_send` in a task of its own, for a send that waits on a compaction.
fn spawn_send(
    f: &Fixture,
    conversation_id: &str,
    text: &str,
) -> (tokio::task::JoinHandle<AppResult<AiSendStarted>>, Arc<Sink>) {
    spawn_send_with(f, conversation_id, text, f.context())
}

fn spawn_send_with(
    f: &Fixture,
    conversation_id: &str,
    text: &str,
    context: AiTurnContext,
) -> (tokio::task::JoinHandle<AppResult<AiSendStarted>>, Arc<Sink>) {
    let sink = Arc::new(Sink::default());
    let (manager, vault, env) = (f.manager.clone(), f.vault.clone(), f.env());
    let input = AiSendInput {
        conversation_id: Some(conversation_id.to_owned()),
        text: text.to_owned(),
        context,
    };
    let events = sink.clone();
    let send = tokio::spawn(async move {
        let (started, task) = manager.send(&vault, env, input, events).await?;
        tokio::spawn(task);
        Ok(started)
    });
    (send, sink)
}

impl Fixture {
    /// A conversation with one exchange, whose context the next long message fills (AI-22).
    async fn nearly_full(&self) -> String {
        self.set_context_window(1_000);
        let (started, sink) = self.send(None, "first question").await;
        let conv = started.conversation.id;
        assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
        self.idle(&conv).await;
        conv
    }

    async fn wait_for_requests(&self, n: usize) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while self.requests().await.len() < n {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for request {n}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

#[tokio::test]
async fn a_message_stopped_while_the_context_is_compacted_is_not_stored() {
    let f = Fixture::new(vec![
        answer("First answer."),
        answer("SUMMARY").set_delay(Duration::from_secs(30)),
    ])
    .await;
    let conv = f.nearly_full().await;

    let (send, sink) = spawn_send(&f, &conv, &"disk ".repeat(720));
    // The Compact request is on its way when the user stops.
    f.wait_for_requests(2).await;
    f.manager.stop(&f.vault, f.env.as_ref(), &conv);
    let err = send.await.unwrap().unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled);
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert!(sink.entries().is_empty());
    // Neither the message nor a summary was stored, and nothing runs.
    assert_eq!(f.entries(&conv).len(), 2);
    assert!(!f.manager.is_running(&conv));
    assert_eq!(f.context_start(&conv), None);
}

#[tokio::test]
async fn a_move_stopped_while_the_context_is_compacted_is_not_stored() {
    use hatoba_ai::tools::host_change_block;

    let f = Fixture::new(vec![
        answer("First answer."),
        answer("SUMMARY").set_delay(Duration::from_secs(30)),
        answer("Short answer."),
    ])
    .await;
    f.set_context_window(1_000);
    let (conv, _) = f
        .exchange(None, "first question", f.quick("10.0.0.9"))
        .await;

    // AI-09: the move to another target goes with the message, which the stop keeps from being
    // stored, and so does the thinking level (AI-05).
    let elsewhere = AiTurnContext {
        effort: Some(AiEffort::High),
        ..f.quick("10.0.0.10")
    };
    let (send, sink) = spawn_send_with(&f, &conv, &"disk ".repeat(720), elsewhere);
    f.wait_for_requests(2).await;
    f.manager.stop(&f.vault, f.env.as_ref(), &conv);
    assert_eq!(send.await.unwrap().unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    let first = "root@10.0.0.9:22";
    assert_eq!(f.recorded_place(&conv), (None, Some(first.to_owned())));
    assert_eq!(f.detail(&conv).conversation.effort, None);

    // So the next message from that target still notes the move.
    f.exchange(Some(&conv), "short", f.quick("10.0.0.10")).await;
    assert_eq!(
        f.user_texts(&conv),
        [
            "first question".to_owned(),
            format!("{}short", host_change_block(first, "root@10.0.0.10:22"))
        ]
    );
    assert_eq!(
        f.recorded_place(&conv),
        (None, Some("root@10.0.0.10:22".to_owned()))
    );
}

#[tokio::test]
async fn a_message_replaced_while_the_context_is_compacted_is_not_stored() {
    let f = Fixture::new(vec![
        answer("First answer."),
        answer("SUMMARY").set_delay(Duration::from_secs(30)),
        answer("Short answer."),
    ])
    .await;
    let conv = f.nearly_full().await;

    let (send, first) = spawn_send(&f, &conv, &"disk ".repeat(720));
    f.wait_for_requests(2).await;
    // A newer message stops the turn that is compacting; only the newer one is stored.
    let (again, second) = f.send(Some(&conv), "short").await;
    assert_eq!(again.conversation.id, conv);
    assert_eq!(send.await.unwrap().unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(first.ended().await, AiTurnEndReason::Stopped);
    assert_eq!(second.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 4);
    assert!(matches!(&entries[2].body, EntryBody::User { text } if text == "short"));
    assert!(matches!(&entries[3].body, EntryBody::Assistant(a) if a.text == "Short answer."));
}

#[tokio::test]
async fn a_retry_compacts_a_context_that_would_fill_the_window_and_only_once() {
    let f = Fixture::new(vec![
        ResponseTemplate::new(400)
            .set_body_json(json!({"error": {"message": "prompt is too long"}})),
        answer("SUMMARY: the user asked about the disk."),
        ResponseTemplate::new(503).set_body_json(json!({"error": {"message": "overloaded"}})),
        answer("Recovered."),
    ])
    .await;
    // Sent while the model's context window was unknown, so nothing was compacted.
    let long = "disk ".repeat(740);
    let (started, sink) = f.send(None, &long).await;
    let conv = started.conversation.id;
    assert_eq!(sink.ended().await, AiTurnEndReason::Error);
    f.idle(&conv).await;

    // Now the window is known (or a smaller model was picked): the message passes 90% of it.
    f.set_context_window(1_000);
    let retry = f.retry(&conv, f.context()).unwrap();
    assert_eq!(retry.ended().await, AiTurnEndReason::Error);
    f.idle(&conv).await;
    let summary = summaries(&retry);
    assert_eq!(summary.len(), 1);
    assert_eq!(
        f.context_start(&conv).as_deref(),
        Some(entry_id(&summary[0]))
    );
    let requests = f.requests().await;
    assert_eq!(requests.len(), 3);
    assert_eq!(
        messages(&requests[1]).last().unwrap()["content"],
        COMPACT_INSTRUCTION
    );
    // The retried message is part of what is summarized, and the request starts at the summary.
    assert!(requests[1].to_string().contains("disk disk"));
    let next = requests[2].to_string();
    assert!(next.contains("SUMMARY: the user asked about the disk."));
    assert!(!next.contains("disk disk"));

    // The context ends with the summary: the next retry does not compact again.
    let again = f.retry(&conv, f.context()).unwrap();
    assert_eq!(again.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    assert!(summaries(&again).is_empty());
    assert_eq!(f.requests().await.len(), 4);
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
    let result = f.result(&conv, "c1", AiToolStatus::Error, "The tool failed", None);
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
    assert!(
        crate::commands::mcp::tool_info(&lock(&f.vault), &f.mcp, &conv, "mcp__files__pid")
            .is_none()
    );
    let c1 = f.run(&conv, "c1", None, None).await.unwrap();
    assert_eq!(view_content(&c1).1, AiToolStatus::Error);
    assert!(view_content(&c1).2.contains("was deleted"), "{c1:?}");
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.mcp.shutdown().await;
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
    assert!(super::guard(&manager.0.ops).is_empty());
}

// ───────────────────────── the prompt's context, notes, instructions (§13.1, §13.3) ─────────────────────────

impl Fixture {
    /// Stores a host with AI notes (AI-37) and returns its id.
    fn add_host(&self, name: &str, ai_notes: &str) -> String {
        lock(&self.vault)
            .put(
                None,
                Item::Host(hatoba_core::model::Host {
                    name: name.into(),
                    address: format!("{name}.example.org"),
                    username: "ops".into(),
                    ai_notes: ai_notes.into(),
                    ..hatoba_core::model::Host::default()
                }),
            )
            .unwrap()
    }

    /// Gives the provider another model.
    fn add_model(&self, id: &str, name: &str) {
        let mut v = lock(&self.vault);
        let mut provider = v
            .get(&self.provider_id)
            .and_then(Item::as_ai_provider)
            .cloned()
            .unwrap();
        provider.models.push(AiModel {
            id: id.into(),
            name: name.into(),
            ..AiModel::default()
        });
        v.put(Some(&self.provider_id), Item::AiProvider(provider))
            .unwrap();
    }

    /// Sets `Settings.ai.custom_instructions` (AI-36).
    fn set_instructions(&self, text: &str) {
        let mut v = lock(&self.vault);
        let mut settings = v.settings();
        settings.ai.custom_instructions = text.into();
        v.put(Some(SETTINGS_ID), Item::Settings(settings)).unwrap();
    }

    /// The text of every stored user entry, oldest first.
    fn user_texts(&self, conversation_id: &str) -> Vec<String> {
        self.entries(conversation_id)
            .into_iter()
            .filter_map(|e| match e.body {
                EntryBody::User { text } => Some(text),
                _ => None,
            })
            .collect()
    }

    /// Sends and waits until the turn has ended and let go of the conversation.
    async fn exchange(
        &self,
        conversation_id: Option<&str>,
        text: &str,
        context: AiTurnContext,
    ) -> (String, AiTurnEndReason) {
        let (started, sink) = self.send_with(conversation_id, text, context).await;
        let id = started.conversation.id;
        let reason = sink.ended().await;
        self.idle(&id).await;
        (id, reason)
    }

    async fn edit_with(
        &self,
        conversation_id: &str,
        entry_id: &str,
        text: &str,
        context: AiTurnContext,
    ) -> AiTurnEndReason {
        let sink = Arc::new(Sink::default());
        let (_, task) = self
            .manager
            .edit_resend(
                &self.vault,
                self.env(),
                conversation_id.to_owned(),
                entry_id,
                text.to_owned(),
                context,
                sink.clone(),
            )
            .await
            .unwrap();
        tokio::spawn(task);
        let reason = sink.ended().await;
        self.idle(conversation_id).await;
        reason
    }
}

/// The text of every user message a request carries.
fn user_messages(body: &Value) -> Vec<String> {
    messages(body)
        .iter()
        .filter(|m| m["role"] == "user")
        .map(|m| m["content"].as_str().unwrap().to_owned())
        .collect()
}

fn system_of(body: &Value) -> String {
    messages(body)[0]["content"].as_str().unwrap().to_owned()
}

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
        "- Follow the user's instructions and the host notes below unless",
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
        "first".to_owned(),
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
            "first".to_owned(),
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

impl Fixture {
    /// A tab connected to `root@{address}:22` without a saved host (HOST-12).
    fn quick(&self, address: &str) -> AiTurnContext {
        AiTurnContext {
            target: Some(QuickTarget {
                address: address.into(),
                port: 22,
                username: "root".into(),
            }),
            ..self.context()
        }
    }

    /// The host and the quick-connect target the conversation item records (AI-09), the target
    /// by its name in notes.
    fn recorded_place(&self, conversation_id: &str) -> (Option<String>, Option<String>) {
        let item = super::find_conversation(&lock(&self.vault), conversation_id).unwrap();
        (
            item.host_id,
            item.quick_target.as_ref().map(super::target_name),
        )
    }

    /// Stores a host at `root@{address}:22`, as Save as Host… makes one (HOST-12).
    fn add_saved_target(&self, name: &str, address: &str) -> String {
        lock(&self.vault)
            .put(
                None,
                Item::Host(hatoba_core::model::Host {
                    name: name.into(),
                    address: address.into(),
                    port: 22,
                    username: "root".into(),
                    ..hatoba_core::model::Host::default()
                }),
            )
            .unwrap()
    }
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
        "one".to_owned(),
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
            "one".to_owned(),
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
    assert_eq!(f.user_texts(&conv), ["one", "two", "three"]);

    // A conversation on a target from before targets were recorded came from no known place.
    {
        let mut v = lock(&f.vault);
        let mut item = super::find_conversation(&v, &conv).unwrap();
        item.quick_target = None;
        v.put(Some(&conv), Item::AiConversation(item)).unwrap();
    }
    let prod = f.add_host("prod-db", "");
    let on_prod = AiTurnContext {
        host_id: Some(prod),
        ..f.context()
    };
    f.exchange(Some(&conv), "four", on_prod).await;
    assert_eq!(f.user_texts(&conv), ["one", "two", "three", "four"]);
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
    assert_eq!(f.user_texts(&conv), ["one", "two", "three"]);
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
    assert_eq!(f.user_texts(&conv), ["one", "two, same target"]);
    // So does one from the host the target was saved as (Save as Host…, HOST-12).
    f.exchange(Some(&conv), "three", on(&prod)).await;
    let srv = f.add_saved_target("srv", "srv.example");
    let three = f.user_ids(&conv)[2].clone();
    f.edit_with(&conv, &three, "three, saved", on(&srv)).await;
    assert_eq!(
        f.user_texts(&conv),
        ["one", "two, same target", "three, saved"]
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
            "one".to_owned(),
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
        ["first".to_owned(), format!("{moved}second, edited")]
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

    // The first message has nothing before it, so it gets no note.
    let first = lock(&f.vault).ai_entries(&conv).unwrap()[0]
        .entry_id
        .clone();
    f.edit_with(&conv, &first, "first, edited", on(&prod)).await;
    assert_eq!(f.user_texts(&conv), ["first, edited"]);
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
        ["one".to_owned(), format!("{moved}two, edited")]
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
    assert_eq!(f.user_texts(&conv), ["one", "two, back"]);
    assert_eq!(f.recorded_place(&conv), (None, Some(first.to_owned())));

    // The first message has nothing before it.
    let one = f.user_ids(&conv)[0].clone();
    f.edit_with(&conv, &one, "one, edited", on_prod).await;
    assert_eq!(f.user_texts(&conv), ["one, edited"]);
    assert_eq!(f.recorded_place(&conv), (Some(prod), None));
}

impl Fixture {
    /// The ids of the stored user entries, oldest first.
    fn user_ids(&self, conversation_id: &str) -> Vec<String> {
        let v = lock(&self.vault);
        let entries = Entries::load(&v, conversation_id).unwrap();
        entries
            .ids
            .into_iter()
            .zip(entries.list)
            .filter(|(_, e)| matches!(e.body, EntryBody::User { .. }))
            .map(|(id, _)| id)
            .collect()
    }

    /// Whether each stored reply's request offered the terminal tools (AI-09), oldest first.
    fn reply_terminals(&self, conversation_id: &str) -> Vec<Option<bool>> {
        self.entries(conversation_id)
            .into_iter()
            .filter_map(|e| match e.body {
                EntryBody::Assistant(a) => Some(a.terminal),
                _ => None,
            })
            .collect()
    }

    /// Stores a message and its reply as another device (or an earlier version) wrote them.
    fn append_exchange(&self, conversation_id: &str, text: &str, terminal: Option<bool>) {
        let mut v = lock(&self.vault);
        append(&mut v, conversation_id, &AiEntry::user(1, text)).unwrap();
        let reply = AssistantEntry {
            provider_id: self.provider_id.clone(),
            model_id: "m1".into(),
            text: "Elsewhere.".into(),
            terminal,
            ..AssistantEntry::default()
        };
        append(&mut v, conversation_id, &AiEntry::assistant(2, reply)).unwrap();
    }
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
        "can you run df?".to_owned(),
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
            "first".to_owned(),
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
            "first".to_owned(),
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
            "first".to_owned(),
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
        ["first".to_owned(), format!("{attached}second, edited")]
    );
    // Edited from home, nothing changed since the first reply.
    let second = f.user_ids(&conv)[1].clone();
    f.edit_with(&conv, &second, "second, from home", home).await;
    assert_eq!(f.user_texts(&conv), ["first", "second, from home"]);

    // The first message has nothing before it.
    let first = f.user_ids(&conv)[0].clone();
    f.edit_with(&conv, &first, "first, edited", connected).await;
    assert_eq!(f.user_texts(&conv), ["first, edited"]);
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
            "one".to_owned(),
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
        ["one".to_owned(), detached, "two, edited".to_owned()]
    );
}

// ---- waiting for results ----

/// A turn that no manager runs: the test wakes it itself, as a stored result would.
fn bare_turn(f: &Fixture) -> Turn {
    Turn {
        cancel: CancellationToken::new(),
        sink: Arc::new(Sink::default()),
        context: f.context(),
        server_id: None,
        results: Notify::new(),
        ended: AtomicBool::new(false),
    }
}

/// Polls `wait` once: one check of the stored entries, then the wait for a result.
async fn poll_once<F: Future + Unpin>(wait: &mut F) -> Poll<F::Output> {
    std::future::poll_fn(|cx| Poll::Ready(Pin::new(&mut *wait).poll(cx))).await
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

/// Stores part `part` of `entry` split in two, with the id `entry_id`, as sync delivers it.
fn arrive(f: &Fixture, conversation_id: &str, entry_id: &str, entry: &AiEntry, part: u32) {
    let json = serde_json::to_string(entry).unwrap();
    let (first, second) = json.split_at(json.len() / 2);
    lock(&f.vault)
        .put(
            None,
            Item::AiMessage(AiMessage {
                conversation_id: conversation_id.into(),
                entry_id: entry_id.into(),
                part,
                part_count: 2,
                data: if part == 0 { first } else { second }.into(),
                ..AiMessage::default()
            }),
        )
        .unwrap();
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
