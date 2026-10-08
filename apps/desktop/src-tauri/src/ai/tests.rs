//! Turn tests (spec §12, AI bullet): the turn engine against a mock Chat Completions provider,
//! without Tauri. Approval, edit, rejection, stop, lock and the cancelled results of calls left
//! without one; the tool call limit is the frontend's (AI-18).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hatoba_ai::entry::{AiEntry, AssistantEntry, EntryBody, Finish, ToolCall, ToolStatus};
use hatoba_core::model::{AiConversation, AiModel, AiProtocol, AiProvider, Item, Skill, SkillFile};
use hatoba_core::sync::SharedVault;
use hatoba_core::{KdfParams, Vault};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};
use zeroize::Zeroizing;

use super::{AiEnv, AiManager, COMPACT_INSTRUCTION, DISCONNECTED, EDIT_NOTE, Entries, EventSink};
use super::{append, lock};
use crate::dto::{
    AiConversationDetail, AiEntryView, AiFinish, AiSendInput, AiSendStarted, AiToolResultInput,
    AiToolStatus, AiTurnContext, AiTurnEndReason, AiTurnEvent,
};
use crate::error::{AppResult, ErrorCode};

const PW: &str = "correct horse battery staple";
const KEY: &str = "sk-test-turns-NEVER-SHOWN";

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
        Self {
            server,
            vault: Arc::new(Mutex::new(vault)),
            manager: AiManager::default(),
            env: Arc::new(Env::default()),
            provider_id,
        }
    }

    fn context(&self) -> AiTurnContext {
        AiTurnContext {
            provider_id: self.provider_id.clone(),
            model_id: "m1".into(),
            host_id: None,
            tab: true,
            disabled_mcp_servers: Vec::new(),
        }
    }

    fn env(&self) -> Arc<dyn AiEnv> {
        self.env.clone()
    }

    fn send(&self, conversation_id: Option<&str>, text: &str) -> (AiSendStarted, Arc<Sink>) {
        let sink = Arc::new(Sink::default());
        let (started, task) = self
            .manager
            .send(
                &self.vault,
                self.env(),
                AiSendInput {
                    conversation_id: conversation_id.map(str::to_owned),
                    text: text.into(),
                    context: self.context(),
                },
                sink.clone(),
            )
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
    let (started, sink) = f.send(None, "  check the disk\nplease");
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
    // No search provider and no skill: neither web_search nor read_skill is offered.
    assert_eq!(
        tool_names(first),
        ["read_terminal", "run_command", "send_input", "fetch_url"]
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
    let (started, sink) = f.send(None, "clean up");
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
    let (started, sink) = f.send(None, "fix it");
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
    let (started, sink) = f.send(None, "hello");
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
    let (started, sink) = f.send(None, "check");
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
    let (started, sink) = f.send(None, "uptime?");
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

    // No tab now: the request offers no tools (AI-09).
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
    assert!(requests[0].get("tools").is_none());
    let system = messages(&requests[0])[0]["content"].as_str().unwrap();
    assert!(system.contains("No terminal is attached"), "{system}");
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
    let (started, first) = f.send(None, "one");
    let conv = started.conversation.id;
    let deadline = Instant::now() + Duration::from_secs(20);
    while f.requests().await.is_empty() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let (again, second) = f.send(Some(&conv), "two");
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
    let (started, sink) = f.send(None, "hi");
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
        .unwrap();
    tokio::spawn(task);
    assert_eq!(sink.ended().await, AiTurnEndReason::Error);
    assert_eq!(sink.errors().len(), 1);
    assert!(sink.errors()[0].1.contains("gone"));
    assert_eq!(f.entries(&started.conversation.id).len(), 1);
    assert!(f.requests().await.is_empty());

    let empty = f.manager.send(
        &f.vault,
        f.env(),
        AiSendInput {
            conversation_id: None,
            text: " \n ".into(),
            context: f.context(),
        },
        sink,
    );
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
    let (started, sink) = f.send(None, "first question");
    let conv = started.conversation.id;
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;

    let summary = f
        .manager
        .compact(&f.vault, f.env.as_ref(), &conv, &f.context())
        .await
        .unwrap();
    let AiEntryView::Summary { entry_id, text, .. } = &summary else {
        panic!("expected a summary, got {summary:?}");
    };
    assert_eq!(text, "SUMMARY: the user checked the disk.");
    let detail = f.detail(&conv);
    assert_eq!(detail.conversation.context_start.as_ref(), Some(entry_id));
    assert_eq!(detail.entries.len(), 3);

    let requests = f.requests().await;
    let compact = &requests[1];
    assert!(compact.get("tools").is_none());
    let last = messages(compact).last().unwrap();
    assert_eq!(last["role"], "user");
    assert_eq!(last["content"], COMPACT_INSTRUCTION);
    assert!(compact.to_string().contains("first question"));

    let (_, sink) = f.send(Some(&conv), "next question");
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    let requests = f.requests().await;
    let next = requests[2].to_string();
    assert!(!next.contains("first question"));
    assert!(next.contains("Summary of the earlier conversation"));
    assert!(next.contains("SUMMARY: the user checked the disk."));
    assert!(next.contains("next question"));
}

#[tokio::test]
async fn compaction_is_refused_while_a_turn_runs() {
    let f = Fixture::new(vec![calls(&[("call_1", "read_terminal", json!({}))])]).await;
    let (started, sink) = f.send(None, "look");
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
    {
        let mut v = lock(&f.vault);
        let skill = v
            .put(
                None,
                Item::Skill(Skill {
                    name: "nginx".into(),
                    description: "Debug nginx".into(),
                    enabled: true,
                    ..Skill::default()
                }),
            )
            .unwrap();
        for (path, content) in [
            ("SKILL.md", "Run nginx -t first."),
            ("references/tls.md", "TLS notes"),
        ] {
            v.put(
                None,
                Item::SkillFile(SkillFile {
                    skill_id: skill.clone(),
                    path: path.into(),
                    content: content.into(),
                    updated_at: 0,
                }),
            )
            .unwrap();
        }
    }
    let (started, sink) = f.send(None, "help");
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;

    let requests = f.requests().await;
    assert_eq!(
        tool_names(&requests[0]),
        [
            "read_terminal",
            "run_command",
            "send_input",
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
    assert!(
        view_content(&c2)
            .2
            .contains("MCP servers are not available yet")
    );
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
    assert!(view_content(&c6).2.contains("Enabled skills: nginx"));
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
    let (started, sink) = f.send(None, "look");
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
