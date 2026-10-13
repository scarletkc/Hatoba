//! [`McpConnection`]: one MCP session over `rmcp`, with the stdio child or the HTTP session.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use reqwest::header::{HeaderName, HeaderValue};
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CancelledNotificationParam, ClientCapabilities,
    ClientConfig, ClientRequest, CustomResult, ErrorData, GetPromptRequest, GetPromptRequestParams,
    Implementation, InitializeRequestParams, JsonRpcMessage, JsonRpcNotification,
    ListPromptsRequest, ListResourcesRequest, ListToolsRequest, PaginatedRequestParams,
    ProtocolVersion, ReadResourceRequest, ReadResourceRequestParams, ServerNotification,
    ServerResult, Tool,
};
use rmcp::service::{
    ClientInitializeError, PeerRequestOptions, RunningService, RxJsonRpcMessage, TxJsonRpcMessage,
};
use rmcp::transport::async_rw::AsyncRwTransport;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransportConfig, StreamableHttpError,
};
use rmcp::transport::{StreamableHttpClientTransport, Transport};
use rmcp::{ClientHandler, Peer, RoleClient, ServiceError};
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use super::stdio::{self, ChildGuard, StderrTail};
use super::{
    McpError, McpPrompt, McpPromptArgument, McpResource, McpTool, McpToolResult,
    McpTransportConfig, ToolAnnotations,
};
use crate::error::AiError;
use crate::net::{clean_message, network_error};

/// How long a stdio server gets to exit by itself after its stdin closes, and an HTTP session to
/// be deleted, before shutdown kills or drops it.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
/// How long a failed start waits for the child's exit status, to name it in the error.
const EXIT_STATUS_WAIT: Duration = Duration::from_millis(500);
/// How long sending `notifications/cancelled` may take before a cancelled call returns anyway.
const CANCEL_NOTICE_WAIT: Duration = Duration::from_secs(1);
/// Most `tools/list`, `prompts/list` or `resources/list` pages read; a server that pages forever
/// is refused.
const MAX_TOOL_PAGES: usize = 100;
/// Headers the Streamable HTTP transport or the HTTP stack sets itself.
const RESERVED_HEADERS: [&str; 9] = [
    "accept",
    "connection",
    "content-length",
    "content-type",
    "host",
    "last-event-id",
    "mcp-protocol-version",
    "mcp-session-id",
    "transfer-encoding",
];

/// The client side of the session: declares no capability.
struct Handler;

impl ClientHandler for Handler {
    fn get_info(&self) -> ClientConfig {
        // No sampling, elicitation or roots (§13.9). Asks for the newest protocol version that
        // still has the `initialize` handshake; a server that speaks only older ones answers
        // with one of those.
        InitializeRequestParams::new(
            ClientCapabilities::default(),
            Implementation::new("Hatoba", env!("CARGO_PKG_VERSION")),
        )
        .with_protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE)
    }
}

/// The `list_changed` notifications a session has had since it last listed each kind. The
/// prompts and resources a server offers start set, as none are listed yet, and a failed listing
/// of them sets its flag again, so that the next asks again.
#[derive(Default)]
struct ListChanges {
    tools: AtomicBool,
    prompts: AtomicBool,
    resources: AtomicBool,
}

/// A transport that records `tools/list_changed`, `prompts/list_changed` and
/// `resources/list_changed` as the message is read. The SDK runs notification handlers in tasks
/// of their own, so one could set the flag only after the answer that followed the notification
/// was returned, and the next request would miss the change (AI-30). Messages are read in order,
/// so here the flag is set before that answer is.
struct Watched<T> {
    inner: T,
    changes: Arc<ListChanges>,
}

impl<T: Transport<RoleClient>> Transport<RoleClient> for Watched<T> {
    type Error = T::Error;

    fn name() -> Cow<'static, str> {
        T::name()
    }

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.inner.send(item)
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        let message = self.inner.receive().await;
        if let Some(JsonRpcMessage::Notification(JsonRpcNotification { notification, .. })) =
            &message
        {
            let flag = match notification {
                ServerNotification::ToolListChangedNotification(_) => Some(&self.changes.tools),
                ServerNotification::PromptListChangedNotification(_) => Some(&self.changes.prompts),
                ServerNotification::ResourceListChangedNotification(_) => {
                    Some(&self.changes.resources)
                }
                _ => None,
            };
            if let Some(flag) = flag {
                flag.store(true, Ordering::SeqCst);
            }
        }
        message
    }

    fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.inner.close()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Stdio,
    /// `token`: the configuration sends an `Authorization` header (see [`http_error_message`]).
    Http {
        token: bool,
    },
}

struct Inner {
    kind: Kind,
    /// Sends requests; cloned out of the service so calls need no lock.
    peer: Peer<RoleClient>,
    /// Taken by `shutdown`.
    service: tokio::sync::Mutex<Option<RunningService<RoleClient, Handler>>>,
    /// The stdio child's process tree, killed by `shutdown` or when this is dropped.
    child: tokio::sync::Mutex<Option<ChildGuard>>,
    changes: Arc<ListChanges>,
    stderr: StderrTail,
    closed: AtomicBool,
    /// Per-page timeout of `list_tools`: the timeout given to `connect`.
    list_timeout: Duration,
}

/// A live session with one MCP server (§13.9).
///
/// Cloning is cheap and shares the session (an `Arc` inside), so the shell can keep one per
/// server and hand clones to running calls. Calls may run concurrently. Dropping the last clone
/// closes the session and kills a stdio server's process tree; [`Self::shutdown`] does the same
/// gracefully and waits for it.
#[derive(Clone)]
pub struct McpConnection {
    inner: Arc<Inner>,
}

impl fmt::Debug for McpConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpConnection")
            .field("transport", &self.inner.kind)
            .field("closed", &self.is_closed())
            .finish()
    }
}

impl McpConnection {
    /// Starts a stdio child or opens a Streamable HTTP session, then initializes it, all within
    /// `timeout` (which also becomes the per-page timeout of [`Self::list_tools`]).
    ///
    /// stdio: the command is looked up on `PATH` like a shell does (with `PATHEXT` on Windows, so
    /// `npx` finds `npx.cmd`, and with the `PATH` from `env` when it sets one). The child
    /// inherits the environment plus `env`, gets no console window on Windows, and its stderr
    /// goes to an in-memory ring of the last [`super::STDERR_LINES`] lines that is never logged.
    ///
    /// HTTP: `headers` go with every request; the URL must be `http` or `https` (the provider base
    /// URL rules, AI-02). `http` is the shared client from [`crate::provider::http_client`].
    ///
    /// On failure a started child is killed and its stderr is lost; use
    /// [`Self::connect_with_stderr`] to keep it.
    pub async fn connect(
        cfg: &McpTransportConfig,
        http: &reqwest::Client,
        timeout: Duration,
    ) -> Result<Self, McpError> {
        Self::connect_with_stderr(cfg, http, timeout, StderrTail::new()).await
    }

    /// [`Self::connect`] that writes a stdio server's stderr into `stderr`, which the caller
    /// keeps, so the last lines can be shown when the start fails (AI-32). Unused for HTTP.
    pub async fn connect_with_stderr(
        cfg: &McpTransportConfig,
        http: &reqwest::Client,
        timeout: Duration,
        stderr: StderrTail,
    ) -> Result<Self, McpError> {
        match cfg {
            McpTransportConfig::Stdio { command, args, env } => {
                connect_stdio(command, args, env, timeout, stderr).await
            }
            McpTransportConfig::Http { url, headers } => {
                connect_http(url, headers, http, timeout).await
            }
        }
    }

    fn new(
        kind: Kind,
        service: RunningService<RoleClient, Handler>,
        child: Option<ChildGuard>,
        changes: Arc<ListChanges>,
        stderr: StderrTail,
        list_timeout: Duration,
    ) -> Self {
        let conn = Self {
            inner: Arc::new(Inner {
                kind,
                peer: service.peer().clone(),
                service: tokio::sync::Mutex::new(Some(service)),
                child: tokio::sync::Mutex::new(child),
                changes,
                stderr,
                closed: AtomicBool::new(false),
                list_timeout,
            }),
        };
        // Whoever lists the prompts and resources first lists them all (AI-50, AI-51).
        if conn.offers_prompts() {
            conn.inner.changes.prompts.store(true, Ordering::SeqCst);
        }
        if conn.offers_resources() {
            conn.inner.changes.resources.store(true, Ordering::SeqCst);
        }
        conn
    }

    /// Every tool of the server, reading all `tools/list` pages. Clears [`Self::tools_changed`]
    /// (it stays set when the listing fails).
    pub async fn list_tools(&self) -> Result<Vec<McpTool>, McpError> {
        let was_changed = self.inner.changes.tools.swap(false, Ordering::SeqCst);
        let result = self.list_all_tools().await;
        if result.is_err() && was_changed {
            self.inner.changes.tools.store(true, Ordering::SeqCst);
        }
        result
    }

    async fn list_all_tools(&self) -> Result<Vec<McpTool>, McpError> {
        self.check_open()?;
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        for _ in 0..MAX_TOOL_PAGES {
            let params = PaginatedRequestParams::default().with_cursor(cursor.take());
            let request = ClientRequest::ListToolsRequest(ListToolsRequest::with_param(params));
            let next_cursor = match self.request(request, self.inner.list_timeout, None).await? {
                ServerResult::ListToolsResult(page) => {
                    tools.extend(page.tools.into_iter().map(convert_tool));
                    page.next_cursor
                }
                // A page the SDK could not parse, such as one with a tool that lacks its
                // `inputSchema`: the usable tools are kept.
                ServerResult::CustomResult(CustomResult(page)) => {
                    let Some(listed) = page.get("tools").and_then(Value::as_array) else {
                        return Err(McpError::Protocol(
                            "the answer to tools/list is not a tool list".into(),
                        ));
                    };
                    tools.extend(listed.iter().filter_map(tool_from_value));
                    page.get("nextCursor")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                }
                _ => {
                    return Err(McpError::Protocol(
                        "the answer to tools/list is not a tool list".into(),
                    ));
                }
            };
            match next_cursor {
                Some(next) if !next.is_empty() => {
                    if !seen.insert(next.clone()) {
                        return Err(McpError::Protocol(
                            "the server repeated a tools/list cursor".into(),
                        ));
                    }
                    cursor = Some(next);
                }
                _ => return Ok(tools),
            }
        }
        Err(McpError::Protocol(format!(
            "the tool list has more than {MAX_TOOL_PAGES} pages"
        )))
    }

    /// Calls `tool` (its name on the server) with `arguments`. Text content is kept; any other
    /// content becomes one line naming its type (AI-30). A tool that ran and failed is
    /// `Ok` with `is_error`; `Err` means the call did not complete.
    ///
    /// When `cancel` fires or `timeout` passes, the server is sent `notifications/cancelled`
    /// and the call returns [`McpError::Cancelled`] or [`McpError::Timeout`]; the session stays
    /// usable.
    pub async fn call_tool(
        &self,
        tool: &str,
        arguments: Map<String, Value>,
        cancel: &CancellationToken,
        timeout: Duration,
    ) -> Result<McpToolResult, McpError> {
        if cancel.is_cancelled() {
            return Err(McpError::Cancelled);
        }
        self.check_open()?;
        let params = CallToolRequestParams::new(tool.to_owned()).with_arguments(arguments);
        let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
        match self.request(request, timeout, Some(cancel)).await? {
            ServerResult::CallToolResult(result) => Ok(tool_result(
                &serde_json::to_value(&result).unwrap_or(Value::Null),
            )),
            // A result with content blocks this SDK does not know arrives unparsed.
            ServerResult::CustomResult(CustomResult(value)) => Ok(tool_result(&value)),
            ServerResult::EmptyResult(_) => Ok(McpToolResult {
                is_error: false,
                content: String::new(),
            }),
            ServerResult::InputRequiredResult(_) => Err(McpError::Protocol(
                "the server asked for user input during the call, which Hatoba does not provide"
                    .into(),
            )),
            ServerResult::CreateTaskResult(_) => Err(McpError::Protocol(
                "the server turned the call into a background task, which Hatoba does not support"
                    .into(),
            )),
            _ => Err(McpError::Protocol(
                "the answer to tools/call is not a tool result".into(),
            )),
        }
    }

    /// Whether the server offers prompts (its `prompts` capability).
    #[must_use]
    pub fn offers_prompts(&self) -> bool {
        self.inner
            .peer
            .peer_info()
            .is_some_and(|info| info.capabilities.prompts.is_some())
    }

    /// Whether the server offers resources (its `resources` capability).
    #[must_use]
    pub fn offers_resources(&self) -> bool {
        self.inner
            .peer
            .peer_info()
            .is_some_and(|info| info.capabilities.resources.is_some())
    }

    /// Every prompt of the server (AI-50), reading all `prompts/list` pages; none when the server
    /// does not offer prompts. Clears [`Self::prompts_changed`], or sets it when the listing
    /// fails, the first one included, so that the next asks again.
    pub async fn list_prompts(&self) -> Result<Vec<McpPrompt>, McpError> {
        self.inner.changes.prompts.store(false, Ordering::SeqCst);
        if !self.offers_prompts() {
            return Ok(Vec::new());
        }
        let result = self
            .list_pages("prompts/list", "prompts", |params| {
                ClientRequest::ListPromptsRequest(ListPromptsRequest::with_param(params))
            })
            .await
            .map(|items| items.iter().filter_map(prompt_from_value).collect());
        if result.is_err() {
            self.inner.changes.prompts.store(true, Ordering::SeqCst);
        }
        result
    }

    /// Every resource of the server (AI-51), reading all `resources/list` pages; none when the
    /// server does not offer resources. Resource templates are not listed. Clears
    /// [`Self::resources_changed`], or sets it when the listing fails, the first one included, so
    /// that the next asks again.
    pub async fn list_resources(&self) -> Result<Vec<McpResource>, McpError> {
        self.inner.changes.resources.store(false, Ordering::SeqCst);
        if !self.offers_resources() {
            return Ok(Vec::new());
        }
        let result = self
            .list_pages("resources/list", "resources", |params| {
                ClientRequest::ListResourcesRequest(ListResourcesRequest::with_param(params))
            })
            .await
            .map(|items| items.iter().filter_map(resource_from_value).collect());
        if result.is_err() {
            self.inner.changes.resources.store(true, Ordering::SeqCst);
        }
        result
    }

    /// The items under `key` of every page of a list request, read as JSON so that items this SDK
    /// cannot parse are not lost with their page.
    async fn list_pages(
        &self,
        method: &str,
        key: &str,
        request: impl Fn(PaginatedRequestParams) -> ClientRequest,
    ) -> Result<Vec<Value>, McpError> {
        self.check_open()?;
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        for _ in 0..MAX_TOOL_PAGES {
            let params = PaginatedRequestParams::default().with_cursor(cursor.take());
            let page = result_value(
                self.request(request(params), self.inner.list_timeout, None)
                    .await?,
            );
            let Some(listed) = page.get(key).and_then(Value::as_array) else {
                return Err(McpError::Protocol(format!(
                    "the answer to {method} is not a list"
                )));
            };
            items.extend(listed.iter().cloned());
            match page.get("nextCursor").and_then(Value::as_str) {
                Some(next) if !next.is_empty() => {
                    if !seen.insert(next.to_owned()) {
                        return Err(McpError::Protocol(format!(
                            "the server repeated a {method} cursor"
                        )));
                    }
                    cursor = Some(next.to_owned());
                }
                _ => return Ok(items),
            }
        }
        Err(McpError::Protocol(format!(
            "the answer to {method} has more than {MAX_TOOL_PAGES} pages"
        )))
    }

    /// Gets the prompt `name` (its name on the server) with `arguments` (AI-50), and returns its
    /// messages as text ([`prompt_text`]).
    pub async fn get_prompt(
        &self,
        name: &str,
        arguments: Map<String, Value>,
        timeout: Duration,
    ) -> Result<String, McpError> {
        self.check_open()?;
        let mut params = GetPromptRequestParams::new(name);
        if !arguments.is_empty() {
            params = params.with_arguments(arguments);
        }
        let request = ClientRequest::GetPromptRequest(GetPromptRequest::new(params));
        let answer = result_value(self.request(request, timeout, None).await?);
        prompt_text(&answer)
            .ok_or_else(|| McpError::Protocol("the answer to prompts/get has no messages".into()))
    }

    /// Reads the resource at `uri` (AI-51), and returns its contents as text
    /// ([`resource_text`]).
    pub async fn read_resource(&self, uri: &str, timeout: Duration) -> Result<String, McpError> {
        self.check_open()?;
        let request = ClientRequest::ReadResourceRequest(ReadResourceRequest::new(
            ReadResourceRequestParams::new(uri),
        ));
        let answer = result_value(self.request(request, timeout, None).await?);
        resource_text(&answer)
    }

    /// Sends one request with a timeout and, when given, a cancellation token.
    async fn request(
        &self,
        request: ClientRequest,
        timeout: Duration,
        cancel: Option<&CancellationToken>,
    ) -> Result<ServerResult, McpError> {
        let peer = &self.inner.peer;
        let handle = peer
            .send_request_with_option(request, PeerRequestOptions::with_timeout(timeout))
            .await
            .map_err(|e| self.service_error(e))?;
        let id = handle.id.clone();
        let response = match cancel {
            Some(cancel) => {
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => {
                        let notice = CancelledNotificationParam::new(
                            Some(id),
                            Some("cancelled by the user".into()),
                        );
                        let _ = tokio::time::timeout(
                            CANCEL_NOTICE_WAIT,
                            peer.notify_cancelled(notice),
                        )
                        .await;
                        return Err(McpError::Cancelled);
                    }
                    response = handle.await_response() => response,
                }
            }
            None => handle.await_response().await,
        };
        response.map_err(|e| self.service_error(e))
    }

    fn check_open(&self) -> Result<(), McpError> {
        if self.is_closed() {
            Err(McpError::Closed)
        } else {
            Ok(())
        }
    }

    fn service_error(&self, err: ServiceError) -> McpError {
        match err {
            ServiceError::McpError(data) => McpError::Server(server_message(&data)),
            ServiceError::Timeout { .. } => McpError::Timeout,
            ServiceError::Cancelled { .. } => McpError::Cancelled,
            ServiceError::TransportClosed => McpError::Closed,
            // A stdio write fails only when the child's stdin is gone.
            ServiceError::TransportSend(_) if self.inner.kind == Kind::Stdio => McpError::Closed,
            ServiceError::TransportSend(e) => McpError::Connect(describe(
                e.error.as_ref(),
                self.inner.kind == Kind::Http { token: true },
            )),
            ServiceError::UnexpectedResponse => {
                McpError::Protocol("the answer does not match the request".into())
            }
            _ => McpError::Protocol("the request failed".into()),
        }
    }

    /// Whether a `notifications/tools/list_changed` arrived since the last [`Self::list_tools`]
    /// (AI-30: refresh the list before the next request).
    #[must_use]
    pub fn tools_changed(&self) -> bool {
        self.inner.changes.tools.load(Ordering::SeqCst)
    }

    /// Whether the prompts are to be listed: the server offers them and none were listed yet, a
    /// `notifications/prompts/list_changed` arrived since the last [`Self::list_prompts`], or
    /// that listing failed.
    #[must_use]
    pub fn prompts_changed(&self) -> bool {
        self.inner.changes.prompts.load(Ordering::SeqCst)
    }

    /// Whether the resources are to be listed: the server offers them and none were listed yet, a
    /// `notifications/resources/list_changed` arrived since the last [`Self::list_resources`], or
    /// that listing failed.
    #[must_use]
    pub fn resources_changed(&self) -> bool {
        self.inner.changes.resources.load(Ordering::SeqCst)
    }

    /// The last lines of a stdio server's stderr, oldest first (empty for HTTP). They stay in
    /// memory only and must not be logged.
    #[must_use]
    pub fn stderr_tail(&self) -> Vec<String> {
        self.inner.stderr.lines()
    }

    /// Whether the session is over: [`Self::shutdown`] ran, the child exited or closed its
    /// output, or the HTTP transport stopped. Calls then return [`McpError::Closed`].
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst) || self.inner.peer.is_transport_closed()
    }

    /// Closes the session: for stdio, closes the child's stdin, gives it a moment to exit, then
    /// kills its whole process tree (Job Object or process group); for HTTP, deletes the
    /// session. Waits for that to finish (a few seconds at most). Running calls end with
    /// [`McpError::Closed`]. Calling it again does nothing.
    pub async fn shutdown(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        let service = self.inner.service.lock().await.take();
        if let Some(mut service) = service {
            let _ = service.close_with_timeout(SHUTDOWN_GRACE).await;
        }
        let child = self.inner.child.lock().await.take();
        if let Some(mut child) = child {
            child.stop(SHUTDOWN_GRACE).await;
        }
    }
}

async fn connect_stdio(
    command: &str,
    args: &[String],
    env: &[(String, Zeroizing<String>)],
    timeout: Duration,
    stderr: StderrTail,
) -> Result<McpConnection, McpError> {
    let program = stdio::find_command(command, env)?;
    let stdio::Spawned {
        mut child,
        stdin,
        stdout,
        stderr: child_stderr,
    } = stdio::spawn(&program, args, env)?;
    if let Some(pipe) = child_stderr {
        stdio::capture_stderr(pipe, stderr.clone());
    }
    let changes = Arc::new(ListChanges::default());
    let transport = Watched {
        inner: AsyncRwTransport::new_client(stdout, stdin),
        changes: changes.clone(),
    };
    let started = tokio::time::timeout(timeout, rmcp::serve_client(Handler, transport)).await;
    match started {
        Ok(Ok(service)) => Ok(McpConnection::new(
            Kind::Stdio,
            service,
            Some(child),
            changes,
            stderr,
            timeout,
        )),
        Ok(Err(err)) => {
            let status = child.exit_status_within(EXIT_STATUS_WAIT).await;
            child.kill_now();
            Err(stdio_init_error(&err, status))
        }
        Err(_) => {
            child.kill_now();
            Err(McpError::Timeout)
        }
    }
}

fn stdio_init_error(
    err: &ClientInitializeError,
    status: Option<std::process::ExitStatus>,
) -> McpError {
    if let Some(status) = status {
        return McpError::Spawn(format!(
            "the server {} before it finished starting",
            stdio::describe_exit(status)
        ));
    }
    match err {
        ClientInitializeError::ConnectionClosed(_)
        | ClientInitializeError::TransportError { .. } => {
            McpError::Spawn("the server closed its output before it finished starting".into())
        }
        other => init_error(other, false),
    }
}

/// The errors both transports share; transport failures are mapped by the caller. `token` as
/// in [`http_error_message`].
fn init_error(err: &ClientInitializeError, token: bool) -> McpError {
    match err {
        ClientInitializeError::JsonRpcError(data) => McpError::Server(server_message(data)),
        ClientInitializeError::Cancelled => McpError::Cancelled,
        ClientInitializeError::ConnectionClosed(_) => {
            McpError::Connect("the connection closed before the server answered".into())
        }
        ClientInitializeError::TransportError { error, .. } => {
            McpError::Connect(describe(error.error.as_ref(), token))
        }
        ClientInitializeError::NoCompatibleProtocolVersion { .. } => {
            McpError::Protocol("the server supports no protocol version Hatoba speaks".into())
        }
        _ => McpError::Protocol("the server did not answer initialize correctly".into()),
    }
}

async fn connect_http(
    url: &str,
    headers: &[(String, Zeroizing<String>)],
    http: &reqwest::Client,
    timeout: Duration,
) -> Result<McpConnection, McpError> {
    let url = server_url(url)?;
    let token = headers
        .iter()
        .any(|(name, _)| name.trim().eq_ignore_ascii_case("authorization"));
    let headers = header_map(headers)?;
    let config =
        StreamableHttpClientTransportConfig::with_uri(url.as_str()).custom_headers(headers);
    let transport = StreamableHttpClientTransport::with_client(http.clone(), config);
    let changes = Arc::new(ListChanges::default());
    let transport = Watched {
        inner: transport,
        changes: changes.clone(),
    };
    match tokio::time::timeout(timeout, rmcp::serve_client(Handler, transport)).await {
        Ok(Ok(service)) => Ok(McpConnection::new(
            Kind::Http { token },
            service,
            None,
            changes,
            StderrTail::new(),
            timeout,
        )),
        Ok(Err(err)) => Err(init_error(&err, token)),
        Err(_) => Err(McpError::Timeout),
    }
}

/// AI-02's base URL rules, with messages about the server URL.
fn server_url(url: &str) -> Result<url::Url, McpError> {
    crate::provider::validate_base_url(url).map_err(|e| match e {
        AiError::InvalidUrl(message) => {
            McpError::InvalidUrl(message.replace("base URL", "server URL"))
        }
        other => McpError::InvalidUrl(other.to_string()),
    })
}

/// The configured headers as sensitive header values. Messages name the header, never its value.
fn header_map(
    headers: &[(String, Zeroizing<String>)],
) -> Result<HashMap<HeaderName, HeaderValue>, McpError> {
    let mut map = HashMap::new();
    for (name, value) in headers {
        let name = name.trim();
        let header = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
            McpError::InvalidConfig(format!(
                "\"{}\" is not a valid header name",
                clean_message(name)
            ))
        })?;
        if RESERVED_HEADERS.contains(&header.as_str()) {
            return Err(McpError::InvalidConfig(format!(
                "the {name} header is set by the MCP transport and cannot be configured"
            )));
        }
        let mut value = HeaderValue::from_str(value.trim()).map_err(|_| {
            McpError::InvalidConfig(format!(
                "the value of the {name} header contains characters an HTTP header cannot carry"
            ))
        })?;
        value.set_sensitive(true);
        map.insert(header, value);
    }
    Ok(map)
}

/// A transport failure as one line without the URL (reqwest puts it in its messages). `token`
/// as in [`http_error_message`].
fn describe(err: &(dyn std::error::Error + Send + Sync + 'static), token: bool) -> String {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = current {
        if let Some(e) = e.downcast_ref::<StreamableHttpError<reqwest::Error>>() {
            return http_error_message(e, token);
        }
        if let Some(e) = e.downcast_ref::<reqwest::Error>() {
            return network_message(e);
        }
        current = e.source();
    }
    clean_message(&strip_urls(&err.to_string()))
}

fn network_message(err: &reqwest::Error) -> String {
    match network_error(err) {
        AiError::Network(message) => message,
        other => other.to_string(),
    }
}

/// `token`: the configuration sends an `Authorization` header, so a 401, with a challenge or
/// without one, means the server refused that token. Only the header's name is looked at.
fn http_error_message(err: &StreamableHttpError<reqwest::Error>, token: bool) -> String {
    const TOKEN_REFUSED: &str = "the token is invalid or has expired (HTTP 401)";
    match err {
        StreamableHttpError::Client(e) => network_message(e),
        StreamableHttpError::AuthRequired(_) if token => TOKEN_REFUSED.into(),
        StreamableHttpError::UnexpectedServerResponse(message)
            if token && message.starts_with("HTTP 401") =>
        {
            TOKEN_REFUSED.into()
        }
        StreamableHttpError::AuthRequired(_) => "the server requires authorization (HTTP 401): \
             give it the credentials it expects as a request header; Hatoba does not sign in \
             to MCP servers with OAuth"
            .into(),
        StreamableHttpError::InsufficientScope(_) => {
            "the server refused the credentials (HTTP 403)".into()
        }
        StreamableHttpError::UnexpectedServerResponse(message) => {
            clean_message(&strip_urls(message))
        }
        StreamableHttpError::UnexpectedContentType(_) => {
            "the server answered with neither JSON nor an event stream; is this an MCP endpoint?"
                .into()
        }
        StreamableHttpError::SessionExpired => "the server ended the session".into(),
        other => clean_message(&strip_urls(&other.to_string())),
    }
}

/// Removes `(…)` groups that follow "url" (reqwest's "for url (…)"), and any `http(s)://…` word.
fn strip_urls(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    let mut rest = message;
    while let Some(start) = rest.find("url (") {
        out.push_str(&rest[..start]);
        out.push_str("url");
        rest = &rest[start + 5..];
        match rest.find(')') {
            Some(end) => rest = &rest[end + 1..],
            None => rest = "",
        }
    }
    out.push_str(rest);
    out.split(' ')
        .map(|word| {
            if word.contains("http://") || word.contains("https://") {
                "<url>"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The server's JSON-RPC error message and code, on one line.
fn server_message(data: &ErrorData) -> String {
    let message = clean_message(&data.message);
    if message.is_empty() {
        format!("code {}", data.code.0)
    } else {
        format!("{message} (code {})", data.code.0)
    }
}

fn convert_tool(tool: Tool) -> McpTool {
    let annotations = tool.annotations.unwrap_or_default();
    McpTool {
        name: tool.name.into_owned(),
        description: tool.description.map(|d| d.into_owned()).unwrap_or_default(),
        input_schema: Value::Object((*tool.input_schema).clone()),
        annotations: ToolAnnotations {
            title: tool.title.or(annotations.title),
            read_only_hint: annotations.read_only_hint,
            destructive_hint: annotations.destructive_hint,
            idempotent_hint: annotations.idempotent_hint,
            open_world_hint: annotations.open_world_hint,
        },
    }
}

/// A tool from JSON the SDK could not parse: the name is required, a missing or malformed
/// `inputSchema` becomes `{"type": "object"}`, and malformed hints are left out.
fn tool_from_value(tool: &Value) -> Option<McpTool> {
    let name = tool.get("name")?.as_str()?.to_owned();
    let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_owned);
    let annotations = tool.get("annotations");
    let hint = |key: &str| {
        annotations
            .and_then(|a| a.get(key))
            .and_then(Value::as_bool)
    };
    Some(McpTool {
        name,
        description: text(tool.get("description")).unwrap_or_default(),
        input_schema: tool
            .get("inputSchema")
            .filter(|schema| schema.is_object())
            .cloned()
            .unwrap_or_else(|| serde_json::json!({"type": "object"})),
        annotations: ToolAnnotations {
            title: text(tool.get("title")).or_else(|| text(annotations?.get("title"))),
            read_only_hint: hint("readOnlyHint"),
            destructive_hint: hint("destructiveHint"),
            idempotent_hint: hint("idempotentHint"),
            open_world_hint: hint("openWorldHint"),
        },
    })
}

/// A result as JSON: what the SDK parsed, written back, or what it could not parse.
fn result_value(result: ServerResult) -> Value {
    match result {
        ServerResult::CustomResult(CustomResult(value)) => value,
        other => serde_json::to_value(&other).unwrap_or(Value::Null),
    }
}

/// A string field of a listed item, or `None` when it is missing or empty.
fn text_field(item: &Value, key: &str) -> Option<String> {
    item.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// A prompt from `prompts/list`; the name is required.
fn prompt_from_value(prompt: &Value) -> Option<McpPrompt> {
    let arguments = prompt
        .get("arguments")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    Some(McpPromptArgument {
                        name: text_field(a, "name")?,
                        description: text_field(a, "description").unwrap_or_default(),
                        required: a.get("required").and_then(Value::as_bool).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(McpPrompt {
        name: text_field(prompt, "name")?,
        title: text_field(prompt, "title"),
        description: text_field(prompt, "description").unwrap_or_default(),
        arguments,
    })
}

/// A resource from `resources/list`; the URI is required, and the name is the URI when missing.
fn resource_from_value(resource: &Value) -> Option<McpResource> {
    let uri = text_field(resource, "uri")?;
    Some(McpResource {
        name: text_field(resource, "name").unwrap_or_else(|| uri.clone()),
        title: text_field(resource, "title"),
        description: text_field(resource, "description").unwrap_or_default(),
        mime_type: text_field(resource, "mimeType"),
        uri,
    })
}

/// The messages of a `prompts/get` result as one text (AI-50): each message's content as a tool
/// result's would be ([`tool_result`]), a blank line apart. When any message is not the user's,
/// each starts with a line naming its role, such as `[assistant]`. `None` without messages.
#[must_use]
pub fn prompt_text(result: &Value) -> Option<String> {
    let messages = result.get("messages").and_then(Value::as_array)?;
    if messages.is_empty() {
        return None;
    }
    let role = |m: &Value| {
        m.get("role")
            .and_then(Value::as_str)
            .unwrap_or("user")
            .to_owned()
    };
    let roles = messages.iter().any(|m| role(m) != "user");
    let parts: Vec<String> = messages
        .iter()
        .map(|m| {
            let content = m.get("content").map(block_text).unwrap_or_default();
            if roles {
                let name: String = role(m)
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .take(20)
                    .collect();
                format!("[{name}]\n{content}")
            } else {
                content
            }
        })
        .collect();
    Some(parts.join("\n\n"))
}

/// The contents of a `resources/read` result as one text (AI-51): each text content, a line
/// apart, and a line naming every binary one, such as `[binary content omitted]`. A resource
/// with no text content at all is an error, since there is nothing to attach.
pub fn resource_text(result: &Value) -> Result<String, McpError> {
    let contents = result
        .get("contents")
        .and_then(Value::as_array)
        .filter(|contents| !contents.is_empty())
        .ok_or_else(|| McpError::Protocol("the answer to resources/read has no contents".into()))?;
    let mut text = false;
    let parts: Vec<&str> = contents
        .iter()
        .map(|c| match c.get("text").and_then(Value::as_str) {
            Some(t) => {
                text = true;
                t
            }
            None => "[binary content omitted]",
        })
        .collect();
    if !text {
        return Err(McpError::Protocol(
            "the resource has no text content, only binary content, which Hatoba does not attach"
                .into(),
        ));
    }
    Ok(parts.join("\n"))
}

/// Reduces a `CallToolResult` (as JSON, so content types unknown to the SDK work too) to text.
pub(super) fn tool_result(result: &Value) -> McpToolResult {
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut parts: Vec<String> = result
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| blocks.iter().map(block_text).collect())
        .unwrap_or_default();
    if parts.is_empty()
        && let Some(structured) = result.get("structuredContent").filter(|v| !v.is_null())
    {
        parts.push(structured.to_string());
    }
    McpToolResult {
        is_error,
        content: parts.join("\n"),
    }
}

fn block_text(block: &Value) -> String {
    let kind = block.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "text" => block
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        // An embedded text resource is text content too.
        "resource" => match block.pointer("/resource/text").and_then(Value::as_str) {
            Some(text) => text.to_owned(),
            None => "[resource content omitted]".to_owned(),
        },
        "" => "[content omitted]".to_owned(),
        other => {
            let name: String = other
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/' | '.'))
                .take(40)
                .collect();
            format!("[{name} content omitted]")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn results_keep_text_and_name_other_content() {
        let result = tool_result(&json!({
            "content": [
                {"type": "text", "text": "first"},
                {"type": "image", "data": "aGk=", "mimeType": "image/png"},
                {"type": "audio", "data": "aGk=", "mimeType": "audio/wav"},
                {"type": "resource_link", "uri": "file:///x", "name": "x"},
                {"type": "resource", "resource": {"uri": "file:///a", "text": "embedded"}},
                {"type": "resource", "resource": {"uri": "file:///b", "blob": "aGk="}},
                {"type": "hologram\n<script>"},
                {"text": "no type"},
                {"type": "text", "text": "last"}
            ],
            "isError": true
        }));
        assert!(result.is_error);
        assert_eq!(
            result.content,
            "first\n[image content omitted]\n[audio content omitted]\n\
             [resource_link content omitted]\nembedded\n[resource content omitted]\n\
             [hologramscript content omitted]\n[content omitted]\nlast"
        );

        let structured = tool_result(&json!({"content": [], "structuredContent": {"a": 1}}));
        assert_eq!(structured.content, r#"{"a":1}"#);
        assert!(!structured.is_error);
        assert_eq!(tool_result(&json!({})).content, "");
    }

    #[test]
    fn urls_are_stripped_from_messages() {
        assert_eq!(
            strip_urls("error sending request for url (https://h.example/mcp?key=s3cret): refused"),
            "error sending request for url: refused"
        );
        assert_eq!(
            strip_urls("HTTP 500: see https://h.example/x?token=abc for details"),
            "HTTP 500: see <url> for details"
        );
    }

    #[test]
    fn headers_are_sensitive_and_checked() {
        let headers = vec![
            (
                "Authorization".to_owned(),
                Zeroizing::new("Bearer s3cret".to_owned()),
            ),
            ("X-Team".to_owned(), Zeroizing::new(" t1 ".to_owned())),
        ];
        let map = header_map(&headers).unwrap();
        assert!(map.values().all(HeaderValue::is_sensitive));
        assert_eq!(map[&HeaderName::from_static("x-team")], "t1");
        assert!(!format!("{map:?}").contains("s3cret"));

        for (name, value) in [("Mcp-Session-Id", "x"), ("Accept", "x"), ("bad name", "x")] {
            let headers = vec![(name.to_owned(), Zeroizing::new(value.to_owned()))];
            assert!(matches!(
                header_map(&headers),
                Err(McpError::InvalidConfig(_))
            ));
        }
        let headers = vec![("X-Key".to_owned(), Zeroizing::new("se\ncret".to_owned()))];
        let err = header_map(&headers).unwrap_err().to_string();
        assert!(err.contains("X-Key") && !err.contains("cret"), "{err}");
    }

    #[test]
    fn tools_convert_with_annotations() {
        let tool: Tool = serde_json::from_value(json!({
            "name": "read_file",
            "title": "Read file",
            "description": "Reads a file",
            "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}},
            "annotations": {"title": "ignored", "readOnlyHint": true, "openWorldHint": false}
        }))
        .unwrap();
        let tool = convert_tool(tool);
        assert_eq!(tool.name, "read_file");
        assert_eq!(tool.description, "Reads a file");
        assert_eq!(tool.input_schema["properties"]["path"]["type"], "string");
        assert_eq!(
            tool.annotations,
            ToolAnnotations {
                title: Some("Read file".into()),
                read_only_hint: Some(true),
                destructive_hint: None,
                idempotent_hint: None,
                open_world_hint: Some(false),
            }
        );
        let bare: Tool = serde_json::from_value(json!({
            "name": "t", "inputSchema": {"type": "object"},
            "annotations": {"title": "From annotations"}
        }))
        .unwrap();
        let bare = convert_tool(bare);
        assert_eq!(bare.description, "");
        assert_eq!(bare.annotations.title.as_deref(), Some("From annotations"));
    }
}
