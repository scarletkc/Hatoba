//! Turn tests (spec §12, AI bullet): the turn engine against a mock Chat Completions provider,
//! without Tauri. Approval, edit, rejection, stop, lock and the cancelled results of calls left
//! without one, and side questions (AI-42).

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::{Duration, Instant};

use hatoba_ai::entry::{
    AiEntry, AssistantEntry, EntryBody, EntryPlace, Finish, ToolCall, ToolStatus,
};
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
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};
use zeroize::Zeroizing;

use super::{
    AiEnv, AiManager, COMMAND_STOP_WAIT, COMPACT_INSTRUCTION, COMPACT_MID_TURN_INSTRUCTION,
    DISCONNECTED, EDIT_NOTE, Entries, EventSink, SIDE_AWAITING_APPROVAL, SIDE_PENDING,
    SIDE_QUESTION_INSTRUCTION,
};
use super::{
    Halt, OpGuard, OpKind, Streamed, Turn, append, leading_blocks, lock, open_call, side_parts,
    title_of, wait_for_results,
};
use crate::dto::{
    AiConversationDetail, AiEffort, AiEntryView, AiFileChangeState, AiFileRevert, AiFinish,
    AiPermissionMode, AiRevertFailure, AiScreen, AiSendInput, AiSendStarted, AiSideAnswer,
    AiSideEvent, AiSideQuestionInput, AiToolResultInput, AiToolStatus, AiTurnContext,
    AiTurnEndReason, AiTurnEvent, QuickTarget,
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

/// `text` after the note of manual approval, the mode of [`Fixture::context`], as a conversation's
/// first message and a summary start with it (AI-16).
fn manual(text: &str) -> String {
    use hatoba_ai::tools::{PermissionMode, permission_mode_block};
    format!("{}{text}", permission_mode_block(PermissionMode::Manual))
}

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
        AiTurnEvent::Opened { .. } => "opened",
        AiTurnEvent::RequestStarted => "request_started",
        AiTurnEvent::Text { .. } => "text",
        AiTurnEvent::Reasoning { .. } => "reasoning",
        AiTurnEvent::ToolCall { .. } => "tool_call",
        AiTurnEvent::Usage { .. } => "usage",
        AiTurnEvent::Entry { .. } => "entry",
        AiTurnEvent::Done { .. } => "done",
        AiTurnEvent::ToolRan { .. } => "tool_ran",
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

    fn session(&self, _session_id: &str) -> Option<crate::ai::conn::Conn> {
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

/// The delta of a Chat Completions stream that carries `text`.
fn text_delta(text: &str) -> Value {
    json!({"choices": [{"index": 0, "delta": {"content": text}}]})
}

/// A server whose first response sends the headers and `events`, then nothing (the stream stays
/// open), and whose later responses answer "Done.". Returns its base URL and the body of every
/// request it received.
async fn stalling_server(events: &[Value]) -> (String, Arc<Mutex<Vec<Value>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let received = bodies.clone();
    let first: String = events.iter().map(|e| format!("data: {e}\n\n")).collect();
    tokio::spawn(async move {
        let mut stalled = Vec::new();
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let body = request_body(&mut socket).await;
            received
                .lock()
                .unwrap()
                .push(serde_json::from_slice::<Value>(&body).unwrap());
            if stalled.is_empty() {
                let mut head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                                transfer-encoding: chunked\r\n\r\n"
                    .to_owned();
                if !first.is_empty() {
                    head.push_str(&format!("{:x}\r\n{first}\r\n", first.len()));
                }
                socket.write_all(head.as_bytes()).await.unwrap();
                stalled.push(socket);
            } else {
                let sse = [
                    text_delta("Done."),
                    json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
                ]
                .iter()
                .map(|e| format!("data: {e}\n\n"))
                .collect::<String>()
                    + "data: [DONE]\n\n";
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\
                     connection: close\r\n\r\n{sse}",
                    sse.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        }
    });
    (format!("http://{addr}/v1"), bodies)
}

/// Reads one HTTP request from `socket` and returns its body.
async fn request_body(socket: &mut tokio::net::TcpStream) -> Vec<u8> {
    let mut data = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = socket.read(&mut buf).await.unwrap();
        assert!(n > 0, "the request ended early");
        data.extend_from_slice(&buf[..n]);
        let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&data[..end]).to_ascii_lowercase();
        let length: usize = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .map_or(0, |value| value.trim().parse().unwrap());
        if data.len() >= end + 4 + length {
            return data[end + 4..end + 4 + length].to_vec();
        }
    }
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
        // Native Search (AI-14) off, so requests offer only the tools under test; the web_search
        // tests turn it on.
        let mut settings = vault.settings();
        settings.ai.native_search = false;
        vault
            .put(Some(SETTINGS_ID), Item::Settings(settings))
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
            local: None,
            tab: true,
            session_id: None,
            disabled_mcp_servers: Vec::new(),
            permission_mode: AiPermissionMode::Manual,
            screen: None,
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

    async fn result(
        &self,
        conversation_id: &str,
        tool_call_id: &str,
        status: AiToolStatus,
        content: &str,
        edited_arguments: Option<&str>,
    ) -> AppResult<AiEntryView> {
        self.manager
            .tool_result(
                &self.vault,
                self.env.as_ref(),
                conversation_id,
                tool_call_id,
                &AiToolResultInput {
                    status,
                    content: content.into(),
                    edited_arguments: edited_arguments.map(str::to_owned),
                    judged: false,
                },
            )
            .await
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
                false,
            )
            .await
            .map(|run| run.entry)
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
            ..
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
        matches!(&started.user_entry, AiEntryView::User { text, .. } if *text == manual("  check the disk\nplease"))
    );

    sink.done_with(AiFinish::ToolCalls).await;
    assert!(f.manager.is_running(&conv));
    assert!(f.detail(&conv).running);
    // Nothing goes out until the call has a result.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(f.requests().await.len(), 1);

    let stored = f
        .result(&conv, "call_1", AiToolStatus::Ok, "$ df -h\n42%", None)
        .await
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
            "grep",
            "glob",
            "fetch_url",
            "read_skill",
            "memory"
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
        .await
        .unwrap_err();
    assert_eq!(again.code, ErrorCode::NotFound);
    assert_eq!(
        f.result(&conv, "nope", AiToolStatus::Ok, "", None)
            .await
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
        .await
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
        .await
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

/// Waits until the turn of `sink` has streamed `text`.
async fn streamed(sink: &Sink, text: &str) {
    sink.until("text", |e| {
        e.contains(&AiTurnEvent::Text { delta: text.into() })
    })
    .await;
}

#[tokio::test]
async fn stopping_mid_stream_keeps_the_text_that_streamed() {
    let (base_url, _) = stalling_server(&[
        json!({"choices": [{"index": 0, "delta": {"reasoning_content": "Thinking"}}]}),
        text_delta("Hel"),
    ])
    .await;
    let f = Fixture::with_base_url(None, base_url);
    let (started, sink) = f.send(None, "hello").await;
    let conv = started.conversation.id;
    streamed(&sink, "Hel").await;

    let at = Instant::now();
    f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert!(at.elapsed() < Duration::from_secs(5));
    assert!(!f.manager.is_running(&conv));
    // The task notices the cancellation and sends nothing more.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);

    // What the user saw is stored, marked, without the reasoning or a `raw` to replay.
    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 2);
    assert!(matches!(&entries[0].body, EntryBody::User { .. }));
    assert_eq!(
        entries[1].body,
        EntryBody::Assistant(AssistantEntry {
            provider_id: f.provider_id.clone(),
            model_id: "m1".into(),
            text: "Hel".into(),
            terminal: Some(true),
            interrupted: true,
            ..AssistantEntry::default()
        })
    );
    // It reaches the panel before the end of the turn.
    let events = sink.events();
    assert!(matches!(
        &events[events.len() - 2],
        AiTurnEvent::Entry { entry: AiEntryView::Assistant { text, .. } } if text == "Hel"
    ));
    assert_eq!(f.detail(&conv).entries.len(), 2);
}

#[test]
fn a_stop_keeps_the_text_and_the_complete_calls_that_streamed() {
    let streamed = |text: &str, tool_calls: Vec<ToolCall>| Streamed {
        provider_id: "p".into(),
        model_id: "m".into(),
        text: text.into(),
        tool_calls,
    };
    let call = ToolCall {
        id: "c1".into(),
        name: "run_command".into(),
        arguments: r#"{"command":"df -h"}"#.into(),
    };
    let entry = streamed("Checking.", vec![call.clone()])
        .into_entry(false)
        .unwrap();
    assert_eq!(
        entry,
        AssistantEntry {
            provider_id: "p".into(),
            model_id: "m".into(),
            text: "Checking.".into(),
            tool_calls: vec![call.clone()],
            finish: Finish::ToolCalls,
            terminal: Some(false),
            interrupted: true,
            ..AssistantEntry::default()
        }
    );
    // A call alone was shown too; its result is the cancelled one the stop stores.
    let entry = streamed("", vec![call]).into_entry(true).unwrap();
    assert!(entry.text.is_empty() && entry.finish == Finish::ToolCalls);
    assert_eq!(
        streamed("Hi", vec![]).into_entry(true).unwrap().finish,
        Finish::Stop
    );
    // Nothing but white space: nothing to keep.
    assert_eq!(streamed(" \n", vec![]).into_entry(true), None);
}

/// The turn's task runs on another thread and acts the moment a stop cancels it: its request ends,
/// it drops what streamed and ends the turn. The test makes that happen after the stop's cancel
/// and before the stop stores anything, the order that lost the text or its `entry` event. The
/// stop still stores the text, and sends it before the one `turn_ended`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::await_holding_lock,
    reason = "the lock holds the stop while the task, which never takes it, runs on another thread"
)]
async fn a_stop_keeps_the_text_when_the_task_acts_first() {
    let f = Fixture::with_base_url(None, "http://127.0.0.1:9/v1".into());
    let conv = {
        let mut v = lock(&f.vault);
        let conv = v
            .put(
                None,
                Item::AiConversation(AiConversation {
                    title: "race".into(),
                    ..AiConversation::default()
                }),
            )
            .unwrap();
        append(&mut v, &conv, &AiEntry::user(1, "df?")).unwrap();
        conv
    };
    let sink = Arc::new(Sink::default());
    let turn = f.manager.register(&conv, f.context(), None, sink.clone());
    *turn.streamed.lock().unwrap() = Some(Streamed {
        provider_id: f.provider_id.clone(),
        model_id: "m1".into(),
        text: "The disk is 4".into(),
        tool_calls: Vec::new(),
    });
    // What the turn loop and `task` do when the request returns cancelled.
    let task = {
        let turn = turn.clone();
        tokio::spawn(async move {
            turn.cancel.cancelled().await;
            turn.streamed.lock().unwrap().take();
            turn.finish(AiTurnEndReason::Stopped);
        })
    };

    // Right after its cancel the stop forgets the conversation's file bases; holding their lock
    // keeps it there until the task has acted.
    let file_bases = f.manager.0.file_bases.lock().unwrap();
    let stop = {
        let (manager, vault, env) = (f.manager.clone(), f.vault.clone(), f.env.clone());
        let conv = conv.clone();
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            runtime.block_on(manager.stop(&vault, env.as_ref(), &conv));
        })
    };
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("the stop cancels the turn before it forgets the file bases")
        .unwrap();
    assert!(sink.events().is_empty());
    drop(file_bases);
    stop.await.unwrap();

    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 2);
    assert!(
        matches!(&entries[1].body, EntryBody::Assistant(a) if a.text == "The disk is 4" && a.interrupted)
    );
    assert_eq!(kinds(&sink.events()), ["entry", "turn_ended"]);
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
}

#[tokio::test]
async fn stopping_before_any_text_stores_nothing() {
    let (base_url, _) = stalling_server(&[json!(
        {"choices": [{"index": 0, "delta": {"reasoning_content": "Thinking"}}]}
    )])
    .await;
    let f = Fixture::with_base_url(None, base_url);
    let (started, sink) = f.send(None, "hello").await;
    let conv = started.conversation.id;
    sink.until("reasoning", |e| {
        e.iter().any(|e| matches!(e, AiTurnEvent::Reasoning { .. }))
    })
    .await;

    f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    f.idle(&conv).await;
    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 1);
    assert!(matches!(&entries[0].body, EntryBody::User { .. }));
    assert!(sink.entries().is_empty());
}

#[tokio::test]
async fn the_message_after_a_stopped_reply_says_it_was_interrupted() {
    use hatoba_ai::tools::interrupted_block;

    let (base_url, bodies) = stalling_server(&[text_delta("The disk is 4")]).await;
    let f = Fixture::with_base_url(None, base_url);
    let (started, sink) = f.send(None, "df?").await;
    let conv = started.conversation.id;
    streamed(&sink, "The disk is 4").await;
    f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    f.idle(&conv).await;

    let context = f.context();
    assert_eq!(
        f.exchange(Some(&conv), "never mind", context.clone())
            .await
            .1,
        AiTurnEndReason::Completed
    );
    // Noted once: the reply after it was not stopped.
    f.exchange(Some(&conv), "thanks", context).await;
    let noted = format!("{}never mind", interrupted_block());
    assert_eq!(
        f.user_texts(&conv),
        [manual("df?"), noted.clone(), "thanks".to_owned()]
    );

    // The model gets the part it wrote, rebuilt, then the note with the message.
    let bodies = bodies.lock().unwrap().clone();
    assert_eq!(bodies.len(), 3);
    let sent = messages(&bodies[1]);
    assert_eq!(sent.len(), 4);
    assert_eq!(
        sent[2],
        json!({"role": "assistant", "content": "The disk is 4"})
    );
    assert_eq!(sent[3], json!({"role": "user", "content": noted}));
}

#[tokio::test]
async fn sending_while_a_reply_streams_keeps_the_part_that_streamed() {
    use hatoba_ai::tools::interrupted_block;

    let (base_url, _) = stalling_server(&[text_delta("Hel")]).await;
    let f = Fixture::with_base_url(None, base_url);
    let (started, first) = f.send(None, "one").await;
    let conv = started.conversation.id;
    streamed(&first, "Hel").await;

    let (_, second) = f.send(Some(&conv), "two").await;
    assert_eq!(first.ended().await, AiTurnEndReason::Stopped);
    assert_eq!(second.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;

    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 4);
    assert!(
        matches!(&entries[1].body, EntryBody::Assistant(a) if a.text == "Hel" && a.interrupted)
    );
    assert!(
        matches!(&entries[2].body, EntryBody::User { text, .. } if *text == format!("{}two", interrupted_block()))
    );
    assert!(
        matches!(&entries[3].body, EntryBody::Assistant(a) if a.text == "Done." && !a.interrupted)
    );
    // The panel listens to the new turn's channel by then, so the stopped reply goes there too.
    assert!(matches!(
        second.entries().first(),
        Some(AiEntryView::Assistant { text, .. }) if text == "Hel"
    ));
}

#[tokio::test]
async fn a_stop_noted_on_a_message_without_a_reply_is_not_noted_again() {
    use hatoba_ai::tools::interrupted_block;

    let f = Fixture::new(vec![
        ResponseTemplate::new(500).set_body_json(json!({"error": {"message": "overloaded"}})),
        answer("Two."),
        answer("Three."),
    ])
    .await;
    let conv = {
        let mut v = lock(&f.vault);
        let conv = v
            .put(
                None,
                Item::AiConversation(AiConversation {
                    title: "stopped".into(),
                    ..AiConversation::default()
                }),
            )
            .unwrap();
        append(&mut v, &conv, &AiEntry::user(1, "one")).unwrap();
        let stopped = AssistantEntry {
            provider_id: f.provider_id.clone(),
            model_id: "m1".into(),
            text: "Partly".into(),
            terminal: Some(true),
            interrupted: true,
            ..AssistantEntry::default()
        };
        append(&mut v, &conv, &AiEntry::assistant(2, stopped)).unwrap();
        conv
    };
    let (_, failed) = f.exchange(Some(&conv), "two", f.context()).await;
    assert_eq!(failed, AiTurnEndReason::Error);
    // A retry stores no message, so nothing is noted.
    assert_eq!(
        f.retry(&conv, f.context()).unwrap().ended().await,
        AiTurnEndReason::Completed
    );
    f.idle(&conv).await;
    f.exchange(Some(&conv), "three", f.context()).await;
    // "one" named no mode, as in a conversation from before every first message did, so "two"
    // names it.
    assert_eq!(
        f.user_texts(&conv),
        [
            "one".to_owned(),
            format!("{}{}", interrupted_block(), manual("two")),
            "three".to_owned()
        ]
    );
}

/// The panel runs the calls of a response that only read at once (§13.1); their results are
/// stored in the order of the calls, whichever finishes first, so the conversation reads as the
/// model made the calls.
#[tokio::test]
async fn results_are_stored_in_the_order_of_the_calls() {
    let f = Fixture::new(vec![
        calls(&[
            ("call_1", "read_terminal", json!({})),
            ("call_2", "read_skill", json!({"name": "hatoba"})),
            ("call_3", "read_terminal", json!({"lines": 5})),
        ]),
        answer("All read."),
    ])
    .await;
    let (started, sink) = f.send(None, "look around").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;

    let first = async {
        // The later calls finish first, and wait.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let kinds: Vec<&str> = f
            .entries(&conv)
            .iter()
            .map(|e| match e.body {
                EntryBody::Tool { .. } => "tool",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["other", "other"], "nothing stored before call_1");
        f.result(&conv, "call_1", AiToolStatus::Ok, "first", None)
            .await
    };
    let (third, second, first) = tokio::join!(
        f.result(&conv, "call_3", AiToolStatus::Ok, "third", None),
        f.run(&conv, "call_2", None, None),
        first,
    );
    for stored in [first, second, third] {
        stored.unwrap();
    }
    let entries = f.entries(&conv);
    let stored: Vec<&str> = entries[2..5].iter().map(|e| tool_entry(e).0).collect();
    assert_eq!(stored, ["call_1", "call_2", "call_3"]);
    let shown: Vec<String> = sink
        .entries()
        .into_iter()
        .filter_map(|e| match e {
            AiEntryView::Tool { tool_call_id, .. } => Some(tool_call_id),
            _ => None,
        })
        .collect();
    assert_eq!(shown, ["call_1", "call_2", "call_3"]);
    // read_skill's run said it ended before its result waited for call_1's, so the panel could
    // count its channel as free meanwhile.
    let events = sink.events();
    let ran = events
        .iter()
        .position(
            |e| matches!(e, AiTurnEvent::ToolRan { tool_call_id } if tool_call_id == "call_2"),
        )
        .expect("call_2 ran");
    let first = events
        .iter()
        .position(|e| matches!(e, AiTurnEvent::Entry { entry: AiEntryView::Tool { tool_call_id, .. } } if tool_call_id == "call_1"))
        .expect("call_1 stored");
    assert!(ran < first);
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
}

/// A result that waits for an earlier call's stops waiting on a stop, which stores what the tools
/// that ended returned and gives every other call without a result a cancelled one (AI-18).
#[tokio::test]
async fn stopping_ends_the_wait_of_results_for_earlier_calls() {
    let f = Fixture::new(vec![calls(&[
        ("call_1", "run_command", json!({"command": "uptime"})),
        ("call_2", "read_skill", json!({"name": "hatoba"})),
        ("call_3", "read_terminal", json!({})),
    ])])
    .await;
    let (started, sink) = f.send(None, "look around").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;

    let stop = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
    };
    let (run, result, ()) = tokio::join!(
        f.run(&conv, "call_2", None, None),
        f.result(&conv, "call_3", AiToolStatus::Ok, "late", None),
        stop,
    );
    // read_skill had ended, so the stop stored what it returned.
    let run = run.unwrap();
    assert_eq!(view_content(&run).1, AiToolStatus::Ok);
    assert_eq!(result.unwrap_err().code, ErrorCode::InvalidInput);
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    let entries = f.entries(&conv);
    let stored: Vec<(&str, ToolStatus)> = entries[2..]
        .iter()
        .map(|e| {
            let (id, status, _) = tool_entry(e);
            (id, status)
        })
        .collect();
    assert_eq!(
        stored,
        [
            ("call_1", ToolStatus::Cancelled),
            ("call_2", ToolStatus::Ok),
            ("call_3", ToolStatus::Cancelled),
        ]
    );
    assert_eq!(tool_entry(&entries[3]).2, view_content(&run).2);
    assert_eq!(sent_results(&sink), ["call_1", "call_2", "call_3"]);
    f.idle(&conv).await;
}

/// A response with a batch of a read and a `run_command`, whose calls the panel runs at once.
async fn a_read_and_a_command() -> (Fixture, String, Arc<Sink>) {
    let f = Fixture::new(vec![
        calls(&[
            ("c1", "read_file", json!({"path": "/etc/hosts"})),
            ("c2", "run_command", json!({"command": "ls /etc"})),
        ]),
        answer("Fine."),
    ])
    .await;
    let (started, sink) = f.send(None, "look around").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    (f, conv, sink)
}

/// The command has run (without a session, it reports that) and its result waits for the read's:
/// it no longer counts as a running command.
async fn until_the_command_waits(f: &Fixture, conv: &str) {
    tokio::time::sleep(Duration::from_millis(200)).await;
    let entries = Entries::load(&lock(&f.vault), conv).unwrap();
    assert!(open_call(&entries, "c2").is_ok());
    assert!(f.manager.running_commands(conv).is_empty());
}

/// A stop that does not wait for commands, such as a message sent while the turn runs, stores
/// what a command that has ended returned while its result waits for an earlier call's, rather
/// than a result that says it may still run (AI-18).
#[tokio::test]
async fn a_message_keeps_the_result_of_a_command_that_ended_and_waits() {
    let (f, conv, sink) = a_read_and_a_command().await;
    // The read still runs.
    let read = f.manager.start_call(&conv, "c1", OpKind::Other).unwrap();
    let send = async {
        until_the_command_waits(&f, &conv).await;
        f.send(Some(&conv), "never mind").await
    };
    let (command, (_, next)) = tokio::join!(f.run(&conv, "c2", None, None), send);
    drop(read);
    assert_eq!(
        view_content(&command.unwrap()),
        ("c2", AiToolStatus::Error, DISCONNECTED)
    );
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert_eq!(next.ended().await, AiTurnEndReason::Completed);
    assert_eq!(
        tool_results(&f.entries(&conv)),
        [
            ("c1", ToolStatus::Cancelled, ""),
            ("c2", ToolStatus::Error, DISCONNECTED),
        ]
    );
    assert_eq!(sent_results(&sink), ["c1", "c2"]);
    f.idle(&conv).await;
}

/// The lock, like a message, stores what a command that has ended returned (AI-18).
#[tokio::test]
async fn the_lock_keeps_the_result_of_a_command_that_ended_and_waits() {
    let (f, conv, sink) = a_read_and_a_command().await;
    let read = f.manager.start_call(&conv, "c1", OpKind::Other).unwrap();
    let lock_now = async {
        until_the_command_waits(&f, &conv).await;
        let mut v = lock(&f.vault);
        f.manager.stop_all(&mut v);
        v.lock();
    };
    let (command, ()) = tokio::join!(f.run(&conv, "c2", None, None), lock_now);
    drop(read);
    assert_eq!(command.unwrap_err().code, ErrorCode::Locked);
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert_eq!(sent_results(&sink), ["c1", "c2"]);
    lock(&f.vault).unlock(PW).unwrap();
    assert_eq!(
        tool_results(&f.entries(&conv)),
        [
            ("c1", ToolStatus::Cancelled, ""),
            ("c2", ToolStatus::Error, DISCONNECTED),
        ]
    );
}

/// A stop that waits for a `run_command` of a batch (AI-18) does not wait on the calls before
/// it: the command's result, which waits for an earlier read's, is stored as the stop cancels
/// the turn, within [`COMMAND_STOP_WAIT`], and the read gets a cancelled result.
#[tokio::test]
async fn a_stopped_command_of_a_batch_stores_its_result_without_the_earlier_calls() {
    let f = Fixture::new(vec![calls(&[
        ("c1", "read_file", json!({"path": "/etc/hosts"})),
        ("c2", "run_command", json!({"command": "ls /etc"})),
    ])])
    .await;
    let (started, sink) = f.send(None, "look around").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    // The read still runs.
    let read = f.manager.start_call(&conv, "c1", OpKind::Other).unwrap();

    let stop = async {
        // The command has run (without a session, it reports that) and waits for the read.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(open_call(&Entries::load(&lock(&f.vault), &conv).unwrap(), "c2").is_ok());
        let started = Instant::now();
        f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
        started.elapsed()
    };
    let (command, took) = tokio::join!(f.run(&conv, "c2", None, None), stop);
    drop(read);
    assert!(took < COMMAND_STOP_WAIT, "the stop took {took:?}");
    assert_eq!(
        view_content(&command.unwrap()),
        ("c2", AiToolStatus::Error, DISCONNECTED)
    );
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert_eq!(
        tool_results(&f.entries(&conv)),
        [
            ("c2", ToolStatus::Error, DISCONNECTED),
            ("c1", ToolStatus::Cancelled, ""),
        ]
    );
    assert_eq!(sent_results(&sink), ["c2", "c1"]);
    f.idle(&conv).await;
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
        .await
        .unwrap();

    f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
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
            .await
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
            .await
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
    // The built-in skill and memory are on by default (AI-34, AI-53), so read_skill and memory
    // are offered too.
    assert_eq!(
        tool_names(&requests[0]),
        ["fetch_url", "read_skill", "memory"]
    );
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
    assert!(matches!(&entries[0].body, EntryBody::User { text, .. } if *text == manual("one")));
    assert!(matches!(&entries[1].body, EntryBody::User { text, .. } if text == "two"));
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

#[tokio::test]
async fn a_reply_without_an_answer_ends_the_turn_as_empty_and_retry_leaves_it_out() {
    let finish =
        |reason: &str| json!({"choices": [{"index": 0, "delta": {}, "finish_reason": reason}]});
    let f = Fixture::new(vec![
        // Reasoning only.
        sse(&[
            json!({"choices": [{"index": 0, "delta": {"role": "assistant", "reasoning_content": "Thinking it over."}}]}),
            finish("stop"),
        ]),
        // Blank text.
        sse(&[text_delta(" \n "), finish("stop")]),
        // Cut off before any text: the output limit is what the panel names.
        sse(&[
            json!({"choices": [{"index": 0, "delta": {"reasoning_content": "Still thinking."}}]}),
            finish("length"),
        ]),
        answer("Recovered."),
    ])
    .await;
    let (started, sink) = f.send(None, "hi").await;
    let conv = started.conversation.id;
    assert_eq!(sink.ended().await, AiTurnEndReason::Empty);
    assert!(sink.errors().is_empty());
    f.idle(&conv).await;
    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 2);
    assert!(matches!(&entries[1].body, EntryBody::Assistant(a)
        if a.text.is_empty() && a.reasoning.as_deref() == Some("Thinking it over.")));

    // The empty reply is no answer, so there is something to retry.
    let retry = f.retry(&conv, f.context()).unwrap();
    assert_eq!(retry.ended().await, AiTurnEndReason::Empty);
    f.idle(&conv).await;
    let retry = f.retry(&conv, f.context()).unwrap();
    assert_eq!(retry.ended().await, AiTurnEndReason::Length);
    f.idle(&conv).await;
    let retry = f.retry(&conv, f.context()).unwrap();
    assert_eq!(retry.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    let entries = f.entries(&conv);
    assert_eq!(entries.len(), 5);
    assert!(matches!(&entries[4].body, EntryBody::Assistant(a) if a.text == "Recovered."));

    // Every request leaves the empty replies out.
    let requests = f.requests().await;
    assert_eq!(requests.len(), 4);
    for body in &requests {
        let roles: Vec<&str> = messages(body)
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["system", "user"]);
    }
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
        .await
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
    // What the model saw of files (AI-38…40) is in the entries the summary replaces, and no
    // approval card waits on a read once the context is summarized.
    remember_a_file(&f, &conv, "call_x");

    let summary = f
        .manager
        .compact(&f.vault, f.env.as_ref(), &conv, &f.context(), None)
        .await
        .unwrap();
    assert!(!f.manager.0.seen.knows(&conv));
    assert!(!holds_a_file_base(&f, &conv));
    let AiEntryView::Summary { entry_id, text, .. } = &summary else {
        panic!("expected a summary, got {summary:?}");
    };
    assert_eq!(text, &manual("SUMMARY: the user checked the disk."));
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

/// AI-53: a conversation's first message and each summary start with the global memories and
/// those of its host, and the `memory` tool saves one without changing the system prompt or the
/// tools, so the provider's cache keeps serving them. The saved memory shows from the summary on.
#[tokio::test]
async fn memories_start_the_first_message_and_each_summary() {
    use super::memory::{Place, Scope};
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
    super::memory::save(
        &mut lock(&f.vault),
        &[],
        &Scope::Global,
        "Replies",
        "Answer in Chinese.",
    )
    .unwrap()
    .unwrap();
    let before = super::memory::note(&lock(&f.vault), Some(&place));
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
    let after = super::memory::note(&lock(&f.vault), Some(&place));
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
    let hits = super::search(&f.vault, "Chinese").unwrap();
    assert!(hits.is_empty(), "{hits:?}");
}

/// AI-53: with memory off, conversations start without the note and the tool is not offered.
#[tokio::test]
async fn memory_off_sends_no_note_and_offers_no_tool() {
    let f = Fixture::new(vec![answer("Hi.")]).await;
    {
        let mut v = lock(&f.vault);
        super::memory::save(
            &mut v,
            &[],
            &super::memory::Scope::Global,
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
    use super::memory::{Place, Scope, note, place_note, save};
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
    use super::memory::{Place, Scope, place_note, save};
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
        super::with_place_memories(&v, &earlier, &notes, Some(&web)),
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
            super::with_place_memories(&v, &earlier, &notes, Some(&web)),
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
            super::with_place_memories(&v, &earlier, &notes, Some(&web)),
            notes
        );
    }
    let noted = format!("{permission}{note}{screen}");
    assert_eq!(
        super::with_place_memories(&v, &[summary("db-1")], &noted, Some(&web)),
        noted
    );
    // A message that showed them before the compaction leaves them to the summary.
    assert_eq!(
        super::with_place_memories(&v, &[summary("web-1")], &noted, Some(&web)),
        notes
    );
    assert_eq!(
        super::with_place_memories(&v, &[summary("db-1")], &notes, None),
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
        .await
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;

    f.manager
        .compact(&f.vault, f.env.as_ref(), &conv, &context, None)
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
    anthropic_stream(&events)
}

/// An Anthropic response that calls `name` with no input, as the call `id`.
fn anthropic_call(id: &str, name: &str) -> ResponseTemplate {
    let events = [
        json!({"type": "message_start", "message": {"id": "msg_1", "type": "message",
               "role": "assistant", "model": "m1", "content": [], "stop_reason": null,
               "usage": {"input_tokens": 20, "cache_read_input_tokens": 80, "output_tokens": 1}}}),
        json!({"type": "content_block_start", "index": 0,
               "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}}),
        json!({"type": "content_block_delta", "index": 0,
               "delta": {"type": "input_json_delta", "partial_json": "{}"}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"},
               "usage": {"output_tokens": 5}}),
        json!({"type": "message_stop"}),
    ];
    anthropic_stream(&events)
}

fn anthropic_stream(events: &[Value]) -> ResponseTemplate {
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
        .compact(&f.vault, f.env.as_ref(), &conv, &f.context(), None)
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

/// AI-22 on Anthropic: the compaction between tool calls asks for the output of a Compact
/// request too, 9% of the window here, not the model's whole limit as the turn's requests do.
#[tokio::test]
async fn an_anthropic_compaction_between_tool_calls_asks_for_less_output() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(Script(Mutex::new(
            vec![
                anthropic_call("c1", "read_terminal"),
                anthropic_answer("SUMMARY: the screen."),
                anthropic_answer("Done."),
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
        provider.models[0].context_window = Some(1_000);
        provider.models[0].max_output_tokens = Some(64_000);
        v.put(Some(&f.provider_id), Item::AiProvider(provider))
            .unwrap();
    }
    let (started, sink) = f.send(None, "look at the screen").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    // The screen alone is about 1,000 tokens.
    f.result(&conv, "c1", AiToolStatus::Ok, &"z".repeat(4_000), None)
        .await
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    assert_eq!(summaries(&sink).len(), 1);

    let requests = f.requests().await;
    let max_tokens: Vec<&Value> = requests.iter().map(|r| &r["max_tokens"]).collect();
    assert_eq!(max_tokens, [&json!(64_000), &json!(90), &json!(64_000)]);
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
        .compact(&f.vault, f.env.as_ref(), &conv, &f.context(), None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Ai);
    assert_eq!(err.detail, "the model returned no summary");
    assert_eq!(f.requests().await.len(), 3);
    assert_eq!(f.entries(&conv).len(), 2);

    let summary = f
        .manager
        .compact(&f.vault, f.env.as_ref(), &conv, &f.context(), None)
        .await
        .unwrap();
    let AiEntryView::Summary { text, .. } = &summary else {
        panic!("expected a summary, got {summary:?}");
    };
    assert_eq!(text, &manual("SUMMARY from the request without tools."));
    let requests = f.requests().await;
    assert_eq!(requests.len(), 5);
    for (with_tools, plain) in [(&requests[1], &requests[2]), (&requests[3], &requests[4])] {
        assert!(!tool_names(with_tools).is_empty());
        assert!(plain.get("tools").is_none());
        assert!(
            system_of(plain).contains("only asks for a summary of the conversation"),
            "{}",
            system_of(plain)
        );
        assert_eq!(messages(plain)[1..], messages(with_tools)[1..]);
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
        .compact(&f.vault, f.env.as_ref(), &conv, &f.context(), None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
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
        system.contains("over the same work with find, grep, cat, sed or heredocs in the shell"),
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

/// §13.4, "File tools": a stop does not cut off a write that has begun, so the cancelled result of
/// an edit or write that was running tells the model the file may have changed.
#[tokio::test]
async fn a_stop_while_a_file_tool_runs_says_the_file_may_have_changed() {
    let f = Fixture::new(vec![calls(&[
        (
            "w1",
            "write_file",
            json!({"path": "/tmp/x", "content": "y"}),
        ),
        ("r1", "read_file", json!({"path": "/tmp/x"})),
        ("w2", "edit_file", json!({"path": "/tmp/x"})),
    ])])
    .await;
    let (started, sink) = f.send(None, "write it").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    // w1 and r1 run; w2 has not started.
    let _write = f.manager.start_call(&conv, "w1", OpKind::Other).unwrap();
    let _read = f.manager.start_call(&conv, "r1", OpKind::Other).unwrap();
    f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);

    let results: Vec<(String, ToolStatus, String)> = f
        .entries(&conv)
        .into_iter()
        .filter_map(|e| match e.body {
            EntryBody::Tool {
                tool_call_id,
                status,
                content,
                ..
            } => Some((tool_call_id, status, content)),
            _ => None,
        })
        .collect();
    assert_eq!(
        results,
        [
            (
                "w1".into(),
                ToolStatus::Cancelled,
                super::files::WRITE_STOPPED.into()
            ),
            ("r1".into(), ToolStatus::Cancelled, String::new()),
            ("w2".into(), ToolStatus::Cancelled, String::new()),
        ]
    );
}

/// What `tool_run` does for a `run_command` call that a stop cancels: the command takes `takes`
/// to end, then the call stores the result that says so, unless the call has one by then, and
/// lets go of its op.
async fn command_ends(f: &Fixture, conv: &str, op: OpGuard<'_>, takes: Duration) {
    op.token().cancelled().await;
    tokio::time::sleep(takes).await;
    let mut v = lock(&f.vault);
    if open_call(&Entries::load(&v, conv).unwrap(), "c1").is_ok() {
        f.manager
            .store_result(
                &mut v,
                conv,
                "c1",
                ToolStatus::Cancelled,
                "ended".into(),
                false,
            )
            .unwrap();
    }
    drop(v);
    drop(op);
}

/// The calls of the results a turn's channel sent, in order.
fn sent_results(sink: &Sink) -> Vec<String> {
    sink.entries()
        .iter()
        .filter(|e| matches!(e, AiEntryView::Tool { .. }))
        .map(|e| view_content(e).0.to_owned())
        .collect()
}

fn tool_results(entries: &[AiEntry]) -> Vec<(&str, ToolStatus, &str)> {
    entries
        .iter()
        .filter(|e| matches!(e.body, EntryBody::Tool { .. }))
        .map(tool_entry)
        .collect()
}

/// AI-18: a stop waits for a running command to end, so the call keeps the result that says how
/// it ended, and the stop sends it with the cancelled results of the other calls.
#[tokio::test]
async fn a_stop_waits_for_a_running_command_and_keeps_its_result() {
    let f = Fixture::new(vec![calls(&[
        ("c1", "run_command", json!({"command": "sleep 300"})),
        ("c2", "run_command", json!({"command": "ls"})),
    ])])
    .await;
    let (started, sink) = f.send(None, "run it").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let op = f.manager.start_call(&conv, "c1", OpKind::Command).unwrap();

    // Nothing new starts in the turn while the stop waits.
    let meanwhile = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(f.manager.is_running(&conv));
        assert!(f.manager.start_op(&conv).token().is_cancelled());
    };
    tokio::join!(
        f.manager.stop(&f.vault, f.env.as_ref(), &conv),
        command_ends(&f, &conv, op, Duration::from_millis(100)),
        meanwhile,
    );
    assert!(!f.manager.is_running(&conv));
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert_eq!(
        tool_results(&f.entries(&conv)),
        [
            ("c1", ToolStatus::Cancelled, "ended"),
            ("c2", ToolStatus::Cancelled, ""),
        ]
    );
    assert_eq!(sent_results(&sink), ["c1", "c2"]);
}

/// AI-18: while a stop waits for a command, the conversation counts as running, and a message
/// sent meanwhile settles the stop at once: the command's result says it may still run, and comes
/// before the message, which the new turn's channel sends it with.
#[tokio::test]
async fn a_message_sent_while_a_stop_waits_settles_it() {
    let f = Fixture::new(vec![
        calls(&[("c1", "run_command", json!({"command": "sleep 300"}))]),
        answer("Stopped it."),
    ])
    .await;
    let (started, sink) = f.send(None, "run it").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let op = f.manager.start_call(&conv, "c1", OpKind::Command).unwrap();

    let send = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(f.manager.is_running(&conv));
        f.send(Some(&conv), "never mind").await
    };
    let ((), (_, next), ()) = tokio::join!(
        f.manager.stop(&f.vault, f.env.as_ref(), &conv),
        send,
        command_ends(&f, &conv, op, Duration::from_millis(300)),
    );
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert_eq!(next.ended().await, AiTurnEndReason::Completed);
    let entries = f.entries(&conv);
    assert_eq!(
        tool_results(&entries),
        [(
            "c1",
            ToolStatus::Cancelled,
            hatoba_ai::tools::COMMAND_STOP_UNCONFIRMED
        )]
    );
    let result = entries
        .iter()
        .position(|e| matches!(e.body, EntryBody::Tool { .. }))
        .unwrap();
    assert!(
        matches!(&entries[result + 1].body, EntryBody::User { text, .. } if text.ends_with("never mind"))
    );
    assert_eq!(sent_results(&next), ["c1"]);
}

/// AI-18: the lock settles a stop that waits for a command, so the command's call never stays
/// without a result behind the lock screen.
#[tokio::test]
async fn the_lock_settles_a_stop_that_waits() {
    let f = Fixture::new(vec![calls(&[
        ("c1", "run_command", json!({"command": "sleep 300"})),
        ("c2", "run_command", json!({"command": "ls"})),
    ])])
    .await;
    let (started, sink) = f.send(None, "run it").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let op = f.manager.start_call(&conv, "c1", OpKind::Command).unwrap();
    let lock_now = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        f.manager.stop_all(&mut lock(&f.vault));
    };
    tokio::join!(
        f.manager.stop(&f.vault, f.env.as_ref(), &conv),
        lock_now,
        command_ends(&f, &conv, op, Duration::from_millis(300)),
    );
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert!(!f.manager.is_running(&conv));
    assert_eq!(
        tool_results(&f.entries(&conv)),
        [
            (
                "c1",
                ToolStatus::Cancelled,
                hatoba_ai::tools::COMMAND_STOP_UNCONFIRMED
            ),
            ("c2", ToolStatus::Cancelled, ""),
        ]
    );
    assert_eq!(sent_results(&sink), ["c1", "c2"]);
}

/// AI-18: a stop that something else settled while it waited leaves alone a later stop of the
/// same conversation when its own wait ends.
#[tokio::test]
async fn a_settled_stop_leaves_a_later_stop_alone() {
    let f = Fixture::new(vec![calls(&[(
        "c1",
        "run_command",
        json!({"command": "sleep 300"}),
    )])])
    .await;
    let (started, sink) = f.send(None, "run it").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let first_command = f.manager.start_call(&conv, "c1", OpKind::Command).unwrap();
    let later = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        // A message settles the first stop at once, and its turn runs a command of its own.
        f.manager.stop_locked(&mut lock(&f.vault), &conv);
        let next = Arc::new(Sink::default());
        let _turn = f.manager.register(&conv, f.context(), None, next.clone());
        let second_command = f.manager.start_call(&conv, "c2", OpKind::Command).unwrap();
        let ends = async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            // The first stop's command ends, which ends its wait.
            drop(first_command);
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(f.manager.is_running(&conv), "the second stop still waits");
            assert!(next.events().is_empty());
            drop(second_command);
        };
        tokio::join!(f.manager.stop(&f.vault, f.env.as_ref(), &conv), ends);
        assert_eq!(next.ended().await, AiTurnEndReason::Stopped);
    };
    tokio::join!(f.manager.stop(&f.vault, f.env.as_ref(), &conv), later);
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert!(!f.manager.is_running(&conv));
}

/// AI-18: locking the vault stops the turn without waiting for a running command, so its result
/// says the command may still run.
#[tokio::test]
async fn a_lock_while_a_command_runs_says_it_may_still_run() {
    let f = Fixture::new(vec![calls(&[(
        "c1",
        "run_command",
        json!({"command": "sleep 300"}),
    )])])
    .await;
    let (started, sink) = f.send(None, "run it").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let op = f.manager.start_call(&conv, "c1", OpKind::Command).unwrap();
    f.manager.stop_all(&mut lock(&f.vault));
    assert!(op.token().is_cancelled());
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    assert_eq!(
        tool_results(&f.entries(&conv)),
        [(
            "c1",
            ToolStatus::Cancelled,
            hatoba_ai::tools::COMMAND_STOP_UNCONFIRMED
        )]
    );
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
            "grep",
            "glob",
            "fetch_url",
            "read_skill",
            "memory"
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

/// An approval card of `call` read a file, and the model has seen it (AI-38…40).
fn remember_a_file(f: &Fixture, conv: &str, call: &str) {
    f.manager
        .0
        .seen
        .record(conv, "response", "ops@db:22", "/etc/hosts", b"x");
    super::guard(&f.manager.0.file_bases).insert(
        (conv.to_owned(), call.to_owned()),
        super::files::Base {
            path: "/etc/hosts".into(),
            server: "ops@db:22".into(),
            file: None,
        },
    );
}

/// The manager still holds file content an approval card of the conversation read.
fn holds_a_file_base(f: &Fixture, conv: &str) -> bool {
    super::guard(&f.manager.0.file_bases)
        .keys()
        .any(|(id, _)| id == conv)
}

#[tokio::test]
async fn deleting_a_conversation_stops_its_turn() {
    let f = Fixture::new(vec![calls(&[("call_1", "read_terminal", json!({}))])]).await;
    let (started, sink) = f.send(None, "look").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    remember_a_file(&f, &conv, "call_1");
    f.manager
        .delete_conversation(&f.vault, f.env.as_ref(), &conv)
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    // What the conversation read of files (AI-38…40) goes with it.
    assert!(!holds_a_file_base(&f, &conv));
    assert!(!f.manager.0.seen.knows(&conv));
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
            "memory",
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
    assert!(!system.contains("run_command"), "{system}");

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

impl Fixture {
    /// Turns Native Search (AI-14) on or off.
    fn native_search(&self, on: bool) {
        let mut v = lock(&self.vault);
        let mut settings = v.settings();
        settings.ai.native_search = on;
        v.put(Some(SETTINGS_ID), Item::Settings(settings)).unwrap();
    }
}

/// AI-14: with no search provider chosen, `web_search` is offered, and the model that made the
/// call searches with its own provider's search.
#[tokio::test]
async fn without_a_search_provider_the_model_searches_with_its_own_provider() {
    let f = Fixture::new(vec![
        calls(&[
            ("c1", "web_search", json!({"query": "zfs"})),
            ("c2", "web_search", json!({"query": "btrfs"})),
        ]),
        answer("Done."),
    ])
    .await;
    f.native_search(true);
    let (started, sink) = f.send(None, "look it up").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    // A Chat Completions server Hatoba does not know is tried as OpenAI is.
    assert!(tool_names(&f.requests().await[0]).contains(&"web_search".to_owned()));

    let native = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_partial_json(json!({
            "model": "m1",
            "tools": [{"type": "web_search_20250305", "name": "web_search"}],
            "messages": [{"role": "user", "content": "Search the web for: zfs"}],
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [
                {"type": "server_tool_use", "id": "s1", "name": "web_search", "input": {"query": "zfs"}},
                {"type": "web_search_tool_result", "tool_use_id": "s1", "content": [
                    {"type": "web_search_result", "url": "https://openzfs.org", "title": "OpenZFS", "encrypted_content": "e"}
                ]},
                {"type": "text", "text": "ZFS is a file system."}
            ],
            "stop_reason": "end_turn"
        })))
        .mount(&native)
        .await;
    let (protocol, base_url) = {
        let mut v = lock(&f.vault);
        let mut provider = v
            .get(&f.provider_id)
            .and_then(Item::as_ai_provider)
            .unwrap()
            .clone();
        let before = (provider.protocol, provider.base_url.clone());
        provider.protocol = AiProtocol::Anthropic;
        provider.base_url = native.uri();
        v.put(Some(&f.provider_id), Item::AiProvider(provider))
            .unwrap();
        before
    };
    let c1 = f.run(&conv, "c1", None, None).await.unwrap();
    assert_eq!(
        view_content(&c1),
        (
            "c1",
            AiToolStatus::Ok,
            "1. OpenZFS\n   https://openzfs.org\n\nWhat the results say:\nZFS is a file system.\n"
        )
    );

    // Turned off, nothing searches.
    f.native_search(false);
    {
        let mut v = lock(&f.vault);
        let offered = super::builtin_tools(&v, &f.context());
        assert!(offered.iter().all(|t| t.name != "web_search"));
        let mut provider = v
            .get(&f.provider_id)
            .and_then(Item::as_ai_provider)
            .unwrap()
            .clone();
        (provider.protocol, provider.base_url) = (protocol, base_url);
        v.put(Some(&f.provider_id), Item::AiProvider(provider))
            .unwrap();
    }
    let c2 = f.run(&conv, "c2", None, None).await.unwrap();
    assert_eq!(
        view_content(&c2),
        (
            "c2",
            AiToolStatus::Error,
            "No search provider is chosen in Settings → AI, so web_search cannot run."
        )
    );
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
}

/// AI-14: a provider Hatoba does not know that turns the native search away gets the same result
/// for the next call without a request, and `web_search` stays offered: a conversation's tools
/// never change under the model.
#[tokio::test]
async fn a_provider_that_turns_native_search_away_is_not_asked_again() {
    let f = Fixture::new(vec![
        calls(&[
            ("c1", "web_search", json!({"query": "zfs"})),
            ("c2", "web_search", json!({"query": "btrfs"})),
        ]),
        answer("Done."),
    ])
    .await;
    let server = f.server.as_ref().unwrap();
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(json!({"error": {"message": "Not Found"}})),
        )
        .mount(server)
        .await;
    f.native_search(true);
    let (started, sink) = f.send(None, "look it up").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;

    let c1 = f.run(&conv, "c1", None, None).await.unwrap();
    let (_, status, text) = view_content(&c1);
    assert_eq!(status, AiToolStatus::Error);
    assert!(
        text.starts_with("The search failed: HTTP 404: Not Found."),
        "{text}"
    );
    assert!(text.contains("Settings → AI"), "{text}");
    let c2 = f.run(&conv, "c2", None, None).await.unwrap();
    assert_eq!(view_content(&c2), ("c2", AiToolStatus::Error, text));
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);

    let received = server.received_requests().await.unwrap();
    let to = |suffix: &str| {
        received
            .iter()
            .filter(|r| r.url.path().ends_with(suffix))
            .map(|r| r.body_json::<Value>().unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(to("/responses").len(), 1);
    let chats = to("/chat/completions");
    assert_eq!(chats.len(), 2);
    for chat in &chats {
        assert!(tool_names(chat).contains(&"web_search".to_owned()));
    }
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
            "grep",
            "glob",
            "fetch_url",
            "read_skill",
            "memory",
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
    assert!(matches!(&entries[3].body, EntryBody::User { text, .. } if *text == long));
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
    remember_a_file(&f, &conv, "c1");
    // The screen alone is about 1,000 tokens.
    f.result(&conv, "c1", AiToolStatus::Ok, &"z".repeat(4_000), None)
        .await
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    f.idle(&conv).await;
    // AI-22 forgets what the model saw of files, as Compact does (AI-38…40), and the next request
    // drops what approval cards read, since no call waits for one.
    assert!(!f.manager.0.seen.knows(&conv));
    assert!(!holds_a_file_base(&f, &conv));

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
        .record(&conv, "response", "ops@db:22", "/etc/hosts", b"x");
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
            EntryBody::User { text, .. } | EntryBody::Summary { text } => text.clone(),
            EntryBody::Assistant(a) => a.text.clone(),
            EntryBody::Tool { content, .. } => content.clone(),
        })
        .collect();
    // The stored summaries name no mode, as before every summary did, so the edit names it.
    assert_eq!(
        texts,
        [
            "alpha",
            "answer alpha",
            "SUMMARY ONE",
            "beta",
            "answer beta",
            "SUMMARY TWO",
            manual("gamma-edited").as_str(),
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
    assert!(
        matches!(&entries[0].body, EntryBody::User { text, .. } if *text == manual("alpha-edited"))
    );
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
    assert!(matches!(&entries[0].body, EntryBody::User { text, .. } if *text == manual("edited")));
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
    let notes = "<interrupted>\nStopped.\n</interrupted>\n\n\
                 <host_change from=\"staging-web\" to=\"prod-db\">\nThe conversation moved.\n\
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

    // A selection block typed after an attached selection is typed text, as the panel reads it.
    let typed_block = "<terminal_selection host=\"x\" lines=\"1\">\nls\n</terminal_selection>\n\n\
                       <terminal_selection host=\"example\" lines=\"1\">\nWhy?\n</terminal_selection>";
    assert_eq!(
        title_of(typed_block),
        "<terminal_selection host=\"example\" lines=\"1\">"
    );
}

#[test]
fn leading_blocks_read_the_shared_cases_as_the_panel_does() {
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        text: String,
        blocks: usize,
        typed: String,
    }
    let cases: Vec<Case> = serde_json::from_str(include_str!(
        "../../../src/features/ai/blocks.fixtures.json"
    ))
    .unwrap();
    for case in cases {
        let (blocks, typed) = leading_blocks(&case.text);
        assert_eq!(
            (blocks.len(), typed),
            (case.blocks, case.typed.as_str()),
            "{}",
            case.name
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
    assert_eq!(user["content"], manual(text));
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
    assert_eq!(stored, manual(&text));
    // The model receives the message as stored.
    let requests = f.requests().await;
    let user = messages(&requests[0])
        .iter()
        .find(|m| m["role"] == "user")
        .unwrap();
    assert_eq!(user["content"], stored);
}

#[test]
fn search_finds_titles_and_message_text_once_per_conversation() {
    use hatoba_ai::tools::{PermissionMode, permission_mode_block, terminal_state_block};

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
            AiEntry::user(
                1,
                format!(
                    "{}{}how full is /var?",
                    permission_mode_block(PermissionMode::Bypass),
                    terminal_state_block("prod", false, "$ df\nscreen-needle")
                ),
            ),
            answer("It is at 42% now."),
            AiEntry::tool(3, "call_1", ToolStatus::Ok, "tool-only-needle"),
        ],
    );
    let (nginx, nginx_ids) = conversation(
        "nginx",
        &[
            AiEntry::user(1, "The NGINX config\nhas a typo"),
            AiEntry::user(2, "nginx again"),
            AiEntry::summary(
                3,
                format!("{}{long}", permission_mode_block(PermissionMode::Bypass)),
            ),
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
    // Tool results are not searched, and neither is the screen a message showed (AI-46) or the
    // permission mode note of a message or a summary (§13.5).
    assert!(search("tool-only-needle").is_empty());
    assert!(search("screen-needle").is_empty());
    assert!(search("bypass").is_empty());
    assert_eq!(search("/var")[0].snippet, "how full is /var?");
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
    let (system, tools) = super::prompt(&v, &f.context(), None, None);
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
    let (system, _) = super::prompt(&v, &f.context(), None, None);
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
    let (system, tools) = super::prompt(&v, &f.context(), None, None);
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
    let (_, tools) = super::prompt(&v, &f.context(), None, None);
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
    f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
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
    f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
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
            manual("first question"),
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
    assert!(matches!(&entries[2].body, EntryBody::User { text, .. } if text == "short"));
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
        .await
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
    let result = f
        .result(&conv, "c1", AiToolStatus::Error, "The tool failed", None)
        .await;
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
        .await
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

// ────────────────── the prompt's context, notes, instructions (§13.1, §13.3) ──────────────────

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
                EntryBody::User { text, .. } => Some(text),
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
        assert_eq!(super::noted_permission(&noted), Some(mode));
        // AI-47: a summary carries the mode, whichever it is.
        let summarized = vec![AiEntry::summary(2, format!("{note}SUMMARY"))];
        assert_eq!(super::noted_permission(&summarized), Some(mode));
    }
    // None before any note, and after a summary without one, as in a conversation from before
    // every first message named its mode: its next message names it.
    assert_eq!(super::noted_permission(&[]), None);
    let old = vec![AiEntry::user(0, "first"), AiEntry::summary(1, "SUMMARY")];
    assert_eq!(super::noted_permission(&old), None);
}

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

// ---- waiting for results ----

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

// ---- side questions (AI-42) ----

/// The input of the side question `question_id` about `conversation_id`, with the fixture's
/// context.
fn side_input(
    f: &Fixture,
    conversation_id: &str,
    question_id: u32,
    text: &str,
    approval_call_id: Option<&str>,
) -> AiSideQuestionInput {
    AiSideQuestionInput {
        conversation_id: conversation_id.into(),
        question_id,
        text: text.into(),
        context: f.context(),
        approval_call_id: approval_call_id.map(str::to_owned),
    }
}

impl Fixture {
    /// Asks a side question and returns its outcome and the events it streamed.
    async fn side(
        &self,
        conversation_id: &str,
        text: &str,
        approval_call_id: Option<&str>,
    ) -> (AppResult<AiSideAnswer>, Vec<AiSideEvent>) {
        let events = Mutex::new(Vec::new());
        let send = |event: AiSideEvent| events.lock().unwrap().push(event);
        let input = side_input(self, conversation_id, 0, text, approval_call_id);
        let result = self
            .manager
            .side_question(&self.vault, self.env.as_ref(), input, &send)
            .await;
        (result, events.into_inner().unwrap())
    }

    /// Asks the side question `question_id` in a task of its own.
    fn spawn_side(
        &self,
        conversation_id: &str,
        question_id: u32,
        text: &str,
    ) -> tokio::task::JoinHandle<AppResult<AiSideAnswer>> {
        let (manager, vault, env) = (self.manager.clone(), self.vault.clone(), self.env.clone());
        let input = side_input(self, conversation_id, question_id, text, None);
        tokio::spawn(async move {
            manager
                .side_question(&vault, env.as_ref(), input, &|_: AiSideEvent| {})
                .await
        })
    }
}

fn streamed_text(events: &[AiSideEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            AiSideEvent::Text { delta } => Some(delta.as_str()),
            _ => None,
        })
        .collect()
}

/// The last user message of a side question's request: the question after its instruction.
fn side_message(question: &str) -> String {
    format!("{SIDE_QUESTION_INSTRUCTION}\n\n<side_question>\n{question}\n</side_question>")
}

/// AI-42 while an approval card waits: the request is the turn's next request, where each call
/// without a result gets one for this request only (the call whose card waits says so), and the
/// question comes last. Nothing is stored, the turn keeps waiting, and its next request carries
/// neither the question nor those results.
#[tokio::test]
async fn a_side_question_leaves_a_waiting_turn_alone() {
    let f = Fixture::new(vec![
        calls(&[
            ("c1", "run_command", json!({"command": "rm -rf /tmp/build"})),
            ("c2", "read_terminal", json!({})),
        ]),
        answer("It deletes /tmp/build and everything in it."),
        answer("Done."),
    ])
    .await;
    let (started, sink) = f.send(None, "clean up the build").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;

    let (answer, events) = f.side(&conv, "Is that safe?", Some("c1")).await;
    let answer = answer.unwrap();
    assert_eq!(answer.text, "It deletes /tmp/build and everything in it.");
    assert_eq!(answer.finish, AiFinish::Stop);
    assert_eq!(streamed_text(&events), answer.text);
    // Nothing stored, and the turn still waits for its calls.
    assert_eq!(f.entries(&conv).len(), 2);
    assert!(f.manager.is_running(&conv));
    assert!(!kinds(&sink.events()).contains(&"turn_ended"));

    let requests = f.requests().await;
    let (turn, side) = (&requests[0], &requests[1]);
    assert_eq!(tool_names(side), tool_names(turn));
    assert_eq!(system_of(side), system_of(turn));
    let (before, after) = messages(side).split_at(messages(turn).len());
    assert_eq!(before, &messages(turn)[..]);
    assert_eq!(after.len(), 4);
    assert_eq!(after[0]["tool_calls"].as_array().unwrap().len(), 2);
    assert_eq!(tool_message(side, "c1")["content"], SIDE_AWAITING_APPROVAL);
    assert_eq!(tool_message(side, "c2")["content"], SIDE_PENDING);
    assert_eq!(
        after[3],
        json!({"role": "user", "content": side_message("Is that safe?")})
    );

    f.result(&conv, "c1", AiToolStatus::Ok, "removed", None)
        .await
        .unwrap();
    f.result(&conv, "c2", AiToolStatus::Ok, "$ ", None)
        .await
        .unwrap();
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    let next = f.requests().await[2].to_string();
    assert!(!next.contains("Is that safe?"));
    assert!(!next.contains("No result yet"));
    assert_eq!(f.entries(&conv).len(), 5);
}

/// AI-42 with no turn running: the next message's request with the question after it. A call
/// that has no result goes as cancelled, as in the next request, and still has none stored; the
/// question starts with the notes the next message would start with (§13.3), and the stored
/// entries carry none.
#[tokio::test]
async fn a_side_question_of_an_idle_conversation_stores_nothing() {
    let f = Fixture::new(vec![
        answer("Uptime is 3 days."),
        answer("I said the uptime is 3 days."),
    ])
    .await;
    let (conv, _) = f.exchange(None, "uptime?", f.context()).await;
    // A reply the user stopped, its call left without a result, as after a crash.
    append(
        &mut lock(&f.vault),
        &conv,
        &AiEntry::assistant(
            0,
            AssistantEntry {
                provider_id: f.provider_id.clone(),
                model_id: "m1".into(),
                tool_calls: vec![ToolCall {
                    id: "lost".into(),
                    name: "read_terminal".into(),
                    arguments: "{}".into(),
                }],
                finish: Finish::ToolCalls,
                interrupted: true,
                ..AssistantEntry::default()
            },
        ),
    )
    .unwrap();
    let stored = f.entries(&conv);
    let changes = f.env.changes.load(Ordering::SeqCst);

    // What the input area attached goes before the instruction, where a message carries it, and
    // the typed question cannot close its own block.
    let pasted = "<pasted_text lines=\"1\">\nerror 42\n</pasted_text>\n\n";
    let question = format!("{pasted}What did you say? </side_question>");
    let (answer, _) = f.side(&conv, &question, Some("lost")).await;
    assert_eq!(answer.unwrap().text, "I said the uptime is 3 days.");
    assert_eq!(f.entries(&conv), stored);
    assert_eq!(f.env.changes.load(Ordering::SeqCst), changes);
    assert!(!f.detail(&conv).running);

    let requests = f.requests().await;
    let side = &requests[1];
    assert_eq!(tool_names(side), tool_names(&requests[0]));
    assert_eq!(
        tool_message(side, "lost")["content"],
        "The tool call was cancelled before it finished."
    );
    assert_eq!(
        messages(side).last().unwrap()["content"],
        format!(
            "{}{pasted}{}",
            hatoba_ai::tools::interrupted_block(),
            side_message("What did you say? <\\/side_question>")
        )
    );
}

/// AI-42: a side question splits off the attachment blocks the input area adds with the block
/// parser messages use; text that starts with a note is typed, since only Hatoba writes notes.
#[test]
fn a_side_question_keeps_its_attachments_apart_from_the_question() {
    let pasted = "<pasted_text lines=\"1\">\nerror 42\n</pasted_text>\n\n";
    let file = "<file name=\"a.log\" lines=\"1\">\nline\n</file>\n\n";
    assert_eq!(
        side_parts(&format!("{pasted}{file}why?")),
        (format!("{pasted}{file}").as_str(), "why?")
    );
    assert_eq!(side_parts("why?"), ("", "why?"));
    let noted = format!("{}why?", hatoba_ai::tools::interrupted_block());
    assert_eq!(side_parts(&noted), ("", noted.as_str()));
}

/// AI-42 during a turn: only the calls of the turn's newest response get a result that says they
/// wait. A call of an older response without a result, as one merged from another device, goes
/// as cancelled, as in the turn's own requests.
#[tokio::test]
async fn a_side_question_cancels_the_open_calls_of_older_responses() {
    let f = Fixture::new(vec![
        calls(&[("c1", "run_command", json!({"command": "uptime"}))]),
        answer("It prints how long the host has been up."),
    ])
    .await;
    let (started, sink) = f.send(None, "uptime?").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let response = entry_id(sink.entries().last().unwrap()).to_owned();
    // Another device's reply, which sorts before the turn's response and whose call has no result.
    let stale = AiEntry::assistant(
        0,
        AssistantEntry {
            provider_id: f.provider_id.clone(),
            model_id: "m1".into(),
            tool_calls: vec![ToolCall {
                id: "lost".into(),
                name: "read_terminal".into(),
                arguments: "{}".into(),
            }],
            finish: Finish::ToolCalls,
            ..AssistantEntry::default()
        },
    );
    let stale_id = format!("{}-7000-8000-000000000000", &response[..13]);
    assert!(stale_id < response);
    arrive(&f, &conv, &stale_id, &stale, 0);
    arrive(&f, &conv, &stale_id, &stale, 1);

    let (answer, _) = f.side(&conv, "What does it do?", Some("c1")).await;
    assert_eq!(
        answer.unwrap().text,
        "It prints how long the host has been up."
    );
    let side = &f.requests().await[1];
    assert_eq!(tool_message(side, "c1")["content"], SIDE_AWAITING_APPROVAL);
    assert_eq!(
        tool_message(side, "lost")["content"],
        "The tool call was cancelled before it finished."
    );
    assert!(f.manager.is_running(&conv));
}

/// AI-42: an answer that calls tools is dropped (nothing runs or is stored), the panel is told to
/// discard what streamed, and the question goes once more without tools, with the system prompt
/// that says why. When that answer calls tools too, the question fails.
#[tokio::test]
async fn a_side_answer_that_calls_tools_is_asked_again_without_tools() {
    let f = Fixture::new(vec![
        answer("First answer."),
        calls(&[("x1", "run_command", json!({"command": "uptime"}))]),
        answer("Plain answer."),
        calls(&[("x2", "run_command", json!({"command": "uptime"}))]),
        calls(&[("x3", "run_command", json!({"command": "uptime"}))]),
    ])
    .await;
    let (conv, _) = f.exchange(None, "first question", f.context()).await;

    let (answer, events) = f.side(&conv, "and the load?", None).await;
    assert_eq!(answer.unwrap().text, "Plain answer.");
    let restarted = events
        .iter()
        .position(|e| *e == AiSideEvent::Restarted)
        .unwrap();
    assert_eq!(streamed_text(&events[..restarted]), "Checking.");
    assert_eq!(streamed_text(&events[restarted..]), "Plain answer.");

    let requests = f.requests().await;
    let (with_tools, plain) = (&requests[1], &requests[2]);
    assert!(!tool_names(with_tools).is_empty());
    assert!(plain.get("tools").is_none());
    assert!(
        system_of(plain)
            .contains("only asks for the answer to a side question of the user, so it offers no"),
        "{}",
        system_of(plain)
    );
    assert_eq!(messages(plain)[1..], messages(with_tools)[1..]);
    assert_eq!(
        messages(plain).last().unwrap()["content"],
        side_message("and the load?")
    );

    let (answer, _) = f.side(&conv, "and the load?", None).await;
    let err = answer.unwrap_err();
    assert_eq!(err.code, ErrorCode::Ai);
    assert_eq!(f.requests().await.len(), 5);
    // The calls never ran and were never stored.
    assert_eq!(f.entries(&conv).len(), 2);
}

/// AI-42: Stop on a side question stops only it, and only the question it names; a newer side
/// question replaces it, stopping the turn leaves it running, and the lock stops it. It is
/// refused while the conversation is compacting.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_side_question_stops_on_its_own() {
    let slow = || answer("slow").set_delay(Duration::from_secs(30));
    let f = Fixture::new(vec![
        calls(&[("c1", "read_terminal", json!({}))]),
        slow(),
        slow(),
        slow(),
        slow(),
    ])
    .await;
    let (started, sink) = f.send(None, "look").await;
    let conv = started.conversation.id;
    sink.done_with(AiFinish::ToolCalls).await;
    let stopped = |result: AppResult<AiSideAnswer>| {
        assert_eq!(result.unwrap_err().code, ErrorCode::Cancelled);
    };
    let within = Duration::from_secs(10);
    let still_running = async |side: &tokio::task::JoinHandle<_>| {
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!side.is_finished());
    };

    // Stop names the question it stops.
    let side = f.spawn_side(&conv, 1, "what is it doing?");
    f.wait_for_requests(2).await;
    f.manager.side_stop(&conv, 2);
    still_running(&side).await;
    f.manager.side_stop(&conv, 1);
    stopped(tokio::time::timeout(within, side).await.unwrap().unwrap());
    assert!(f.manager.is_running(&conv));

    // A newer side question replaces the one on its way, and a stop meant for that one, which
    // arrives late, leaves the newer one alone.
    let first = f.spawn_side(&conv, 2, "first");
    f.wait_for_requests(3).await;
    let second = f.spawn_side(&conv, 3, "second");
    stopped(tokio::time::timeout(within, first).await.unwrap().unwrap());
    f.wait_for_requests(4).await;
    f.manager.side_stop(&conv, 2);
    still_running(&second).await;

    // Stopping the turn leaves it running; the lock stops it.
    f.manager.stop(&f.vault, f.env.as_ref(), &conv).await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Stopped);
    still_running(&second).await;
    f.manager.stop_all(&mut lock(&f.vault));
    stopped(tokio::time::timeout(within, second).await.unwrap().unwrap());
    assert!(f.manager.0.ops.lock().unwrap().is_empty());

    // Compacting.
    let compaction = f.manager.start_compaction(&conv);
    let (answer, _) = f.side(&conv, "now?", None).await;
    assert_eq!(answer.unwrap_err().code, ErrorCode::InvalidInput);
    drop(compaction);
    assert!(!f.manager.is_compacting(&conv));
}

/// AI-42: a Compact counts as compacting from its start until the summary is stored, so a side
/// question asked meanwhile is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_side_question_is_refused_while_the_conversation_compacts() {
    let f = Fixture::new(vec![
        answer("First answer."),
        answer("SUMMARY").set_delay(Duration::from_millis(500)),
        answer("After."),
    ])
    .await;
    let (conv, _) = f.exchange(None, "first question", f.context()).await;
    let compact = {
        let (manager, vault, env) = (f.manager.clone(), f.vault.clone(), f.env.clone());
        let (conv, context) = (conv.clone(), f.context());
        tokio::spawn(async move {
            manager
                .compact(&vault, env.as_ref(), &conv, &context, None)
                .await
        })
    };
    f.wait_for_requests(2).await;
    let (answer, _) = f.side(&conv, "now?", None).await;
    assert_eq!(answer.unwrap_err().code, ErrorCode::InvalidInput);
    assert!(matches!(
        compact.await.unwrap().unwrap(),
        AiEntryView::Summary { .. }
    ));
    assert!(!f.manager.is_compacting(&conv));
    let (answer, _) = f.side(&conv, "now?", None).await;
    assert_eq!(answer.unwrap().text, "After.");
}

/// AI-42: the automatic compactions count too (AI-22), the one before a message that would fill
/// the context and the one between tool calls, so a side question asked during either is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_side_question_is_refused_while_the_context_is_compacted_automatically() {
    let f = Fixture::new(vec![
        answer("First answer."),
        answer("SUMMARY").set_delay(Duration::from_millis(500)),
        calls(&[("c1", "read_terminal", json!({}))]),
        answer("SUMMARY-2").set_delay(Duration::from_millis(500)),
        answer("Done."),
    ])
    .await;
    let conv = f.nearly_full().await;
    let refused = async || {
        let (answer, _) = f.side(&conv, "now?", None).await;
        assert_eq!(answer.unwrap_err().code, ErrorCode::InvalidInput);
    };

    // Before the message.
    let (send, sink) = spawn_send(&f, &conv, &"disk ".repeat(720));
    f.wait_for_requests(2).await;
    refused().await;
    send.await.unwrap().unwrap();
    sink.done_with(AiFinish::ToolCalls).await;
    assert!(!f.manager.is_compacting(&conv));

    // Between tool calls: the screen alone is about 1,000 tokens.
    f.result(&conv, "c1", AiToolStatus::Ok, &"z".repeat(4_000), None)
        .await
        .unwrap();
    f.wait_for_requests(4).await;
    refused().await;
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    assert_eq!(summaries(&sink).len(), 2);
    assert!(!f.manager.is_compacting(&conv));
}

/// AI-42 on Anthropic: a side question asks for as much output as the conversation's next
/// request, unlike a Compact request (AI-21), so that its answer is not cut short when thinking is
/// on or the model's context window is small.
#[tokio::test]
async fn an_anthropic_side_question_asks_for_as_much_output_as_the_next_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(Script(Mutex::new(
            vec![
                anthropic_answer("First answer."),
                anthropic_answer("Side answer."),
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
        // A Compact request would ask for 9% of the window: 2,880 tokens.
        provider.models[0].context_window = Some(32_000);
        provider.models[0].max_output_tokens = Some(8_000);
        v.put(Some(&f.provider_id), Item::AiProvider(provider))
            .unwrap();
    }
    let (conv, _) = f.exchange(None, "first question", f.context()).await;
    let (answer, _) = f.side(&conv, "and why?", None).await;
    assert_eq!(answer.unwrap().text, "Side answer.");

    let requests = f.requests().await;
    let (turn, side) = (&requests[0], &requests[1]);
    assert_eq!(side["max_tokens"], 8_000);
    assert_eq!(without_messages(side), without_messages(turn));
    let (before, after) = messages(side).split_at(messages(turn).len());
    assert_eq!(before, &messages(turn)[..]);
    assert_eq!(
        after,
        [
            json!({"role": "assistant", "content": [{"type": "text", "text": "First answer."}]}),
            json!({"role": "user", "content": [{"type": "text", "text": side_message("and why?")}]}),
        ]
    );
}

/// The change `call` of the response `response_id` makes, as the file tools build it.
fn a_change(path: &str, tool: &str) -> super::files::Change {
    let created = tool == "write_file";
    super::files::Change {
        meta: super::files::ChangeMeta {
            server: "ops@db:22".into(),
            path: path.into(),
            tool: tool.into(),
            created,
            mode: None,
        },
        content: super::files::ChangeContent {
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
) -> super::KeepSnapshot<'a> {
    super::KeepSnapshot::new(&f.vault, revert_key(conv, response_id, call))
}

/// A change `call` of the response `response_id` made, kept as its snapshot (AI-41): the file tools
/// keep one before they write, which needs a live SSH session, and mark it written after.
fn keep_a_change(f: &Fixture, conv: &str, response_id: &str, call: &str, path: &str, tool: &str) {
    use super::files::Keep as _;
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

/// The user entry's text as the request after it sent it.
fn last_user_message(body: &Value) -> String {
    messages(body)
        .iter()
        .rev()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_str())
        .unwrap()
        .to_owned()
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
    assert_eq!(super::title_of(&format!("{notes}typed")), "typed");
    assert_eq!(super::noted_reverts(&format!("{notes}typed")), ["e1", "w1"]);

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
    use super::files::Keep as _;
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
    let meta = |path: &str| super::files::ChangeMeta {
        server: "ops@db:22".into(),
        path: path.into(),
        tool: "write_file".into(),
        created: true,
        mode: None,
    };
    {
        let v = lock(&f.vault);
        assert!(super::later_writes(
            &v,
            &revert_key(&conv, &response, "w3"),
            &meta("/srv/b")
        ));
        assert!(!super::later_writes(
            &v,
            &revert_key(&conv, &response, "e1"),
            &meta("/srv/b")
        ));
        assert!(!super::later_writes(
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
    assert!(!super::later_writes(
        &lock(&f.vault),
        &revert_key(&conv, &response, "w3"),
        &meta("/srv/b")
    ));

    // The lock before the write begins stops the call; nothing is kept.
    lock(&f.vault).lock();
    assert!(matches!(
        keeper(&f, &conv, &response, "w2").keep(&a_change("/srv/a", "write_file")),
        Err(super::files::Stop::Cancelled)
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

    let reverting = super::Reverting::new(&f.manager.0, &conv);
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

    let entries = |calls: &[&str]| super::Entries {
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
    assert!(super::shares_id(&entries(&["dup", "dup", "c3"]), "dup"));
    assert!(!super::shares_id(&entries(&["dup", "dup", "c3"]), "c3"));

    // A revert note is about the call of the newest response before it with that id, also when
    // an older response used the same id.
    let response = |calls: &[&str]| entries(calls).list.remove(0);
    let list = [
        response(&["call_1"]),
        AiEntry::user(2, "next"),
        response(&["call_1", "call_2"]),
        AiEntry::user(4, "noted"),
    ];
    assert_eq!(super::response_with_call(&list[..3], "call_1"), Some(2));
    assert_eq!(super::response_with_call(&list[..2], "call_1"), Some(0));
    assert_eq!(super::response_with_call(&list[..3], "call_2"), Some(2));
    assert_eq!(super::response_with_call(&list[..1], "call_2"), None);
}

// ---- forking (AI-43) ----

impl Fixture {
    fn fork(
        &self,
        conversation_id: &str,
        entry_id: Option<&str>,
        title: &str,
    ) -> AppResult<crate::dto::AiConversationView> {
        self.manager.fork_conversation(
            &self.vault,
            self.env.as_ref(),
            conversation_id,
            entry_id,
            title,
        )
    }

    /// The stored entries of a conversation as `(entry_id, JSON)`.
    fn stored(&self, conversation_id: &str) -> Vec<(String, String)> {
        lock(&self.vault)
            .ai_entries(conversation_id)
            .unwrap()
            .into_iter()
            .map(|e| (e.entry_id, e.json.to_string()))
            .collect()
    }

    /// Stores a conversation with these entries and returns its id and theirs.
    fn conversation_of(&self, item: AiConversation, entries: &[AiEntry]) -> (String, Vec<String>) {
        let mut v = lock(&self.vault);
        let conv = v.put(None, Item::AiConversation(item)).unwrap();
        let ids = entries
            .iter()
            .map(|e| append(&mut v, &conv, e).unwrap())
            .collect();
        (conv, ids)
    }

    /// A stored response with these calls of `read_terminal`, whose `raw` holds a field only it
    /// keeps.
    fn response(&self, text: &str, calls: &[&str]) -> AiEntry {
        AiEntry::assistant(
            20,
            AssistantEntry {
                provider_id: self.provider_id.clone(),
                model_id: "m1".into(),
                text: text.into(),
                tool_calls: calls
                    .iter()
                    .map(|id| ToolCall {
                        id: (*id).into(),
                        name: "read_terminal".into(),
                        arguments: "{}".into(),
                    })
                    .collect(),
                finish: if calls.is_empty() {
                    Finish::Stop
                } else {
                    Finish::ToolCalls
                },
                raw: json!({"role": "assistant", "content": text, "provider_note": "RAW-ONLY"}),
                ..AssistantEntry::default()
            },
        )
    }
}

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
        let mut c = super::find_conversation(&v, &conv).unwrap();
        c.context_start = Some(ids[5].clone());
        v.put(Some(&conv), Item::AiConversation(c)).unwrap();
    }
    let original = super::find_conversation(&lock(&f.vault), &conv).unwrap();
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
        super::find_conversation(&lock(&f.vault), &conv).unwrap(),
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
        let item = super::find_conversation(&lock(&f.vault), &fork.id).unwrap();
        let place = (
            item.host_id,
            item.quick_target.as_ref().map(super::target_name),
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
    let item = super::find_conversation(&lock(&f.vault), &fork).unwrap();
    assert_eq!(item.origin.as_deref(), Some("old-name"));
    // The next message in a tab on a host notes the move from it, and the move ends it.
    f.exchange(Some(&fork), "on twin", on(&twin)).await;
    assert_eq!(
        f.user_texts(&fork)[2],
        format!("{}on twin", host_change_block("old-name", "twin"))
    );
    let item = super::find_conversation(&lock(&f.vault), &fork).unwrap();
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
    use super::{Place, parse_target, place_named, target_name};

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

mod deferred;
mod judge;

#[test]
fn what_commands_and_mentions_attach_is_titled_and_shown_to_the_judge() {
    use super::typed_text;
    let blocks = "<skill name=\"nginx\">\nRun nginx -t.\n</skill>\n\n\
                  <host_file host=\"web\" path=\"/etc/nginx/nginx.conf\" lines=\"2\">\na\nb\n\
                  </host_file>\n\n\
                  <host_directory host=\"web\" path=\"/etc/nginx\" entries=\"3\" truncated=\"true\">\
                  \nconf.d/\nnginx.conf\nsites/\n</host_directory>\n\n\
                  <mcp_resource server=\"docs\" uri=\"repo://x/a.md\" lines=\"1\">\n# A\n\
                  </mcp_resource>\n\n\
                  <mcp_prompt server=\"github\" name=\"review_pr\">\nReview PR 42\n</mcp_prompt>\n\n";
    assert_eq!(
        title_of(&format!("{blocks}/nginx check @/etc/nginx/nginx.conf")),
        "/nginx check @/etc/nginx/nginx.conf"
    );
    // Nothing typed: the first block's name, as the panel's mock titles it.
    assert_eq!(title_of(blocks), "/nginx");
    assert_eq!(
        title_of(
            "<host_file host=\"web\" path=\"/etc/nginx/nginx.conf\" lines=\"1\">\nx\n</host_file>"
        ),
        "nginx.conf"
    );
    assert_eq!(
        title_of("<host_directory host=\"web\" path=\"/etc\" entries=\"0\">\n\n</host_directory>"),
        "/etc"
    );
    assert_eq!(
        title_of("<mcp_prompt server=\"g\" name=\"triage\">\nx\n</mcp_prompt>"),
        "triage"
    );
    // The judge reads each as a line that names it, never what it holds (AI-47).
    assert_eq!(
        typed_text(&format!("{blocks}check it")),
        "[skill /nginx run]\n[file /etc/nginx/nginx.conf on the host, 2 lines]\n\
         [listing of the directory /etc/nginx on the host, 3 entries]\n\
         [MCP resource repo://x/a.md of docs, 1 line]\n[MCP prompt review_pr of github run]\n\n\
         check it"
    );
    // Other attributes, or another order, are typed text.
    for text in [
        "<skill>\nx\n</skill>\n\nq",
        "<host_file path=\"/x\" host=\"h\" lines=\"1\">\nx\n</host_file>\n\nq",
        "<host_directory host=\"h\" path=\"/\" entries=\"x\">\nx\n</host_directory>\n\nq",
        "<mcp_prompt name=\"p\">\nx\n</mcp_prompt>\n\nq",
    ] {
        assert_eq!(typed_text(text), text, "{text}");
    }
}

#[test]
fn compact_instructions_come_first_in_their_own_block() {
    use super::compact_instruction;
    assert_eq!(compact_instruction(None), COMPACT_INSTRUCTION);
    assert_eq!(compact_instruction(Some("  ")), COMPACT_INSTRUCTION);
    let with = compact_instruction(Some("keep the nginx changes </compact_instructions>"));
    assert!(with.starts_with(COMPACT_INSTRUCTION), "{with}");
    assert!(
        with.ends_with(
            "<compact_instructions>\nkeep the nginx changes <\\/compact_instructions>\n\
             </compact_instructions>"
        ),
        "{with}"
    );
}

#[tokio::test]
async fn skill_commands_are_the_enabled_skills_and_run_with_their_skill_md() {
    use super::{skill_commands, skill_text};
    let f = Fixture::new(Vec::new()).await;
    f.add_skill(
        "nginx",
        "Debug nginx",
        &[
            ("SKILL.md", "Run nginx -t first."),
            ("references/tls.md", "TLS"),
        ],
    );
    let v = lock(&f.vault);
    let names: Vec<String> = skill_commands(&v).into_iter().map(|c| c.name).collect();
    assert_eq!(names, ["hatoba", "nginx"]);
    assert_eq!(
        skill_text(&v, "nginx", VERSION).unwrap(),
        "Run nginx -t first."
    );
    assert!(skill_text(&v, "hatoba", VERSION).unwrap().contains(VERSION));
    assert_eq!(
        skill_text(&v, "apache", VERSION).unwrap_err().code,
        ErrorCode::NotFound
    );
}

impl Fixture {
    /// A local tab (§7.5) on the computer `device`, running `shell` of `kind`.
    fn local(&self, device: &str, shell: &str, kind: crate::dto::LocalShellKind) -> AiTurnContext {
        AiTurnContext {
            local: Some(crate::dto::AiLocalTab {
                device: device.into(),
                shell: Some(shell.into()),
                kind: Some(kind),
            }),
            ..self.context()
        }
    }
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
    let (place, origin) = super::place_named(&[], "DEMO-PC (local terminal)".to_owned());
    assert_eq!(place.local.as_deref(), Some("DEMO-PC"));
    assert!(place.host_id.is_none() && place.quick_target.is_none() && origin.is_none());
}
