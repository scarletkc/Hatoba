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

use super::blocks::leading_blocks;
use super::calls::EDIT_NOTE;
use super::calls::open_call;
use super::compact::{COMPACT_INSTRUCTION, COMPACT_MID_TURN_INSTRUCTION};
use super::entries::Entries;
use super::entries::{append, title_of};
use super::job::DISCONNECTED;
use super::ops::COMMAND_STOP_WAIT;
use super::side::side_parts;
use super::side::{SIDE_AWAITING_APPROVAL, SIDE_PENDING, SIDE_QUESTION_INSTRUCTION};
use super::turn::wait_for_results;
use super::{AiEnv, AiManager, EventSink};
use super::{Halt, OpGuard, OpKind, Streamed, Turn, lock};
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
        AiTurnEvent::Thinking => "thinking",
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
            telnet: false,
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

/// Waits until the turn of `sink` has streamed `text`.
async fn streamed(sink: &Sink, text: &str) {
    sink.until("text", |e| {
        e.contains(&AiTurnEvent::Text { delta: text.into() })
    })
    .await;
}

fn without_messages(body: &Value) -> Value {
    let mut body = body.clone();
    body.as_object_mut().unwrap().remove("messages");
    body
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
            item.quick_target.as_ref().map(super::places::target_name),
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

/// Polls `wait` once: one check of the stored entries, then the wait for a result.
async fn poll_once<F: Future + Unpin>(wait: &mut F) -> Poll<F::Output> {
    std::future::poll_fn(|cx| Poll::Ready(Pin::new(&mut *wait).poll(cx))).await
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

/// The last user message of a side question's request: the question after its instruction.
fn side_message(question: &str) -> String {
    format!("{SIDE_QUESTION_INSTRUCTION}\n\n<side_question>\n{question}\n</side_question>")
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

mod blocks;
mod calls;
mod changes;
mod compact;
mod deferred;
mod edit;
mod fork;
mod history;
mod judge;
mod memory;
mod notes;
mod prompt;
mod side;
mod skills;
mod stop;
mod turn;
mod waiting;
