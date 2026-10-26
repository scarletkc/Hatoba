//! The tools that run in Rust (§13.4), with what they need read from the vault beforehand
//! ([`Job`]): `run_command` (AI-12), `web_search` (AI-14), and the MCP tool a `call_mcp_tool` call
//! reaches (AI-45).

use hatoba_ai::AiError;
use hatoba_ai::entry::ToolStatus;
use hatoba_ai::mcp::deferred;
use hatoba_ai::provider::{ModelSpec, ProviderConfig};
use hatoba_ai::tools::{
    self, CommandStop, FetchUrlArgs, MemoryArgs, ReadSkillArgs, RunCommandArgs, StoppedCommand,
    WebSearchArgs,
};
use hatoba_ai::web::{self, NativeSearch, SearchConfig, SearchKind as WebSearchKind};
use hatoba_core::Vault;
use hatoba_core::model::{Item, SearchKind as CoreSearchKind, SearchProvider};
use hatoba_core::sync::SharedVault;
use hatoba_ssh::remote_command;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use super::files::FileJob;
use super::request::resolve_model;
use super::search::SearchJob;
use super::skills::read_skill;
use super::{AiEnv, conn, files, lock, memory, parse, search};
use crate::mcp::{McpManager, OfferedTool};

/// The `run_command` result while the tab has no live session (AI-08).
pub(super) const DISCONNECTED: &str = "The terminal tab is disconnected, so the command did not run. Ask \
     the user to reconnect the tab.";

/// What runs `web_search` (AI-14).
pub(super) enum WebSearch {
    /// The search provider `Settings.ai` chose.
    Service(SearchConfig),
    /// The search of the model's own provider.
    Native(ProviderConfig, ModelSpec),
}

/// What runs `web_search` for model `model_id` of `provider_id` (AI-14): the search provider
/// `Settings.ai` chose, when it exists, or else, unless native search is off, the model's provider
/// with its own search. Every provider that chats has one; one Hatoba does not know is tried as
/// OpenAI is ([`NativeSearch::Other`]), so the tools a conversation is offered do not depend on
/// how a search went.
pub(super) fn web_search_for(v: &Vault, provider_id: &str, model_id: &str) -> Option<WebSearch> {
    let settings = v.settings().ai;
    let chosen = settings
        .search_provider_id
        .as_deref()
        .and_then(|id| v.get(id))
        .and_then(Item::as_search_provider);
    if let Some(chosen) = chosen {
        return Some(WebSearch::Service(search_config(chosen)));
    }
    if !settings.native_search {
        return None;
    }
    let (provider, model) = resolve_model(v, provider_id, model_id).ok()?;
    NativeSearch::of(&provider)?;
    Some(WebSearch::Native(provider, model))
}

/// Why a `web_search` call cannot run (AI-14).
fn no_web_search(v: &Vault) -> &'static str {
    if v.settings().ai.native_search {
        "No search provider is chosen in Settings → AI, and the model that made the call is no \
         longer in its provider's model list, so web_search cannot run."
    } else {
        "No search provider is chosen in Settings → AI, so web_search cannot run."
    }
}

pub fn search_config(p: &SearchProvider) -> SearchConfig {
    SearchConfig {
        kind: search_kind(p.kind),
        base_url: p.base_url.clone(),
        api_key: p.api_key.clone(),
    }
}

pub fn search_kind(kind: CoreSearchKind) -> WebSearchKind {
    match kind {
        CoreSearchKind::Brave => WebSearchKind::Brave,
        CoreSearchKind::Tavily => WebSearchKind::Tavily,
        CoreSearchKind::Searxng => WebSearchKind::Searxng,
    }
}

/// A tool call that runs in Rust, with what it needs read from the vault beforehand.
pub(super) enum Job {
    /// The result is known without running anything: a vault read, or a call that cannot run.
    Ready(ToolStatus, String),
    RunCommand(RunCommandArgs),
    /// `read_file`, `edit_file` or `write_file`, with what the approval card read for it.
    File(FileJob, Option<files::Base>),
    /// `grep` or `glob`.
    Search(SearchJob),
    /// `web_search` through the search provider `Settings.ai` chose.
    WebSearch(SearchConfig, String),
    /// `web_search` with the search of the model's own provider (AI-14), which
    /// [`AiManager::native_search`] runs, since it keeps the providers that turned it away.
    NativeSearch(Box<(ProviderConfig, ModelSpec)>, String),
    /// `fetch_url`: the URL, the offset, and whether the whole page is asked for.
    FetchUrl(String, u64, bool),
    /// An MCP tool the request offered, with its arguments (AI-30), called directly or through
    /// `call_mcp_tool` (AI-45).
    Mcp(Box<OfferedTool>, Map<String, Value>),
    /// `search_tools` (AI-45), which [`AiManager::search_tools`] runs, since it reads the
    /// conversation's MCP tools.
    SearchTools(deferred::SearchArgs),
    /// `memory` (AI-53), which writes to the vault: its arguments, and the host or computer of
    /// the conversation, whose host memories it changes.
    Memory(MemoryArgs, Option<memory::Place>),
}

impl Job {
    pub(super) fn error(message: impl Into<String>) -> Self {
        Self::Ready(ToolStatus::Error, message.into())
    }

    /// A call of an MCP tool: `tool` is what the offer that made the call names `name`. A server
    /// whose item is gone (deleted on another device, before a request stopped it here) runs
    /// nothing, even while its connection is still up.
    pub(super) fn mcp(v: &Vault, tool: Option<OfferedTool>, name: &str, arguments: &str) -> Self {
        let Some(tool) = tool else {
            return Self::error(format!(
                "There is no MCP tool named \"{name}\": no MCP server offers it now."
            ));
        };
        if !crate::mcp::server_exists(v, &tool.server_id) {
            return Self::error(crate::mcp::server_gone(&tool.server_name));
        }
        match parse::<Map<String, Value>>(arguments) {
            Ok(arguments) => Self::Mcp(Box::new(tool), arguments),
            Err(message) => Self::error(message),
        }
    }

    /// `version`: the app's, for the built-in skill (AI-34). `caller`: the provider and model of
    /// the response that made the call, whose own search `web_search` may use (AI-14).
    pub(super) fn new(
        v: &Vault,
        name: &str,
        arguments: &str,
        version: &str,
        caller: (&str, &str),
    ) -> Self {
        if let Some(parsed) = FileJob::parse(name, arguments) {
            return match parsed {
                Ok(job) => Self::File(job, None),
                Err(message) => Self::error(message),
            };
        }
        if let Some(parsed) = SearchJob::parse(name, arguments) {
            return match parsed {
                Ok(job) => Self::Search(job),
                Err(message) => Self::error(message),
            };
        }
        match name {
            tools::RUN_COMMAND => match parse::<RunCommandArgs>(arguments) {
                Ok(args) if args.command.trim().is_empty() => Self::error("The command is empty."),
                Ok(args) => Self::RunCommand(args),
                Err(message) => Self::error(message),
            },
            tools::WEB_SEARCH => match web_search_for(v, caller.0, caller.1) {
                None => Self::error(no_web_search(v)),
                Some(search) => match (parse::<WebSearchArgs>(arguments), search) {
                    (Ok(args), WebSearch::Service(config)) => Self::WebSearch(config, args.query),
                    (Ok(args), WebSearch::Native(provider, model)) => {
                        Self::NativeSearch(Box::new((provider, model)), args.query)
                    }
                    (Err(message), _) => Self::error(message),
                },
            },
            tools::FETCH_URL => match parse::<FetchUrlArgs>(arguments) {
                Ok(args) => Self::FetchUrl(
                    args.url,
                    args.offset.unwrap_or(0),
                    args.full_page.unwrap_or(false),
                ),
                Err(message) => Self::error(message),
            },
            tools::READ_SKILL => match parse::<ReadSkillArgs>(arguments) {
                Ok(args) => read_skill(v, &args, version),
                Err(message) => Self::error(message),
            },
            tools::SEARCH_TOOLS => match deferred::parse_search(arguments) {
                Ok(args) => Self::SearchTools(args),
                Err(message) => Self::error(message),
            },
            tools::MEMORY => match parse::<MemoryArgs>(arguments) {
                Ok(args) => Self::Memory(args, None),
                Err(message) => Self::error(message),
            },
            tools::READ_TERMINAL | tools::SEND_INPUT => Self::error(format!(
                "{name} runs in the terminal tab and cannot run here."
            )),
            _ => Self::error(format!("There is no tool named \"{name}\".")),
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "what the tools that run in Rust need, each from its own place"
    )]
    pub(super) async fn run(
        self,
        http: &reqwest::Client,
        mcp: &McpManager,
        vault: &SharedVault,
        env: &dyn AiEnv,
        session_id: Option<&str>,
        seen: files::SeenBy<'_>,
        cancel: &CancellationToken,
    ) -> (ToolStatus, String) {
        match self {
            Self::Ready(status, content) => (status, content),
            Self::Mcp(tool, arguments) => mcp.call(vault, &tool, arguments, cancel).await,
            // `tool_run` runs them: with the conversation's MCP tools, and with what the manager
            // keeps of native searches.
            Self::SearchTools(_) => (
                ToolStatus::Error,
                "search_tools cannot run here.".to_owned(),
            ),
            Self::NativeSearch(..) => (ToolStatus::Error, "web_search cannot run here.".to_owned()),
            Self::RunCommand(args) => run_command(env, session_id, &args, cancel).await,
            Self::Memory(args, place) => {
                let mut v = lock(vault);
                if v.is_unlocked() {
                    memory::run(&mut v, place.as_ref(), &args)
                } else {
                    (ToolStatus::Error, "The vault is locked.".to_owned())
                }
            }
            Self::File(job, base) => {
                files::run(live_session(env, session_id), job, base, seen, cancel).await
            }
            Self::Search(job) => search::run(live_session(env, session_id), job, cancel).await,
            Self::WebSearch(config, query) => {
                match web::web_search(http, &config, &query, cancel).await {
                    Ok(results) => (
                        ToolStatus::Ok,
                        tools::truncate_result(&web::format_search_results(&results)),
                    ),
                    Err(AiError::Cancelled) => (ToolStatus::Cancelled, String::new()),
                    Err(e) => (ToolStatus::Error, format!("The search failed: {e}")),
                }
            }
            // Pages through long content instead of being shortened (§13.4).
            Self::FetchUrl(url, offset, full_page) => {
                match web::fetch_url(&url, offset, full_page, cancel).await {
                    Ok(page) => (ToolStatus::Ok, web::format_fetch_result(&page)),
                    Err(AiError::Cancelled) => (ToolStatus::Cancelled, String::new()),
                    Err(e) => (
                        ToolStatus::Error,
                        format!("The page could not be fetched: {e}"),
                    ),
                }
            }
        }
    }
}

/// AI-45: the MCP tool a `call_mcp_tool` call with `arguments` calls in the conversation, with the
/// tool's arguments, resolved through the offer recorded last (the newest message's or the latest
/// search's, AI-30). The error is the call's result when it cannot run, so it neither asks nor
/// runs: arguments that cannot be read, a name the conversation has no tool of, a server that was
/// deleted, or arguments that do not match the tool's input schema. The approval card
/// (`mcp_proxy_info`) and the run (`ai_tool_run`) both resolve here.
pub fn proxy_call(
    v: &Vault,
    mcp: &McpManager,
    conversation_id: &str,
    arguments: &str,
) -> Result<(OfferedTool, Map<String, Value>), String> {
    let call = deferred::parse_call(arguments)?;
    let Some(tool) = mcp.offered(conversation_id, &call.tool) else {
        return Err(format!(
            "The call did not run: this conversation has no MCP tool named \"{}\" now. Call {} to \
             find the tools you can call.",
            call.tool,
            tools::SEARCH_TOOLS
        ));
    };
    if !crate::mcp::server_exists(v, &tool.server_id) {
        return Err(crate::mcp::server_gone(&tool.server_name));
    }
    let problems = deferred::check_arguments(&tool.tool.input_schema, &call.arguments);
    if !problems.is_empty() {
        return Err(tools::truncate_result(&deferred::mismatch_result(
            &tool.name,
            &problems,
            &tool.tool.input_schema,
        )));
    }
    Ok((tool, call.arguments))
}

/// What the tab's tools run over while it is open (AI-08).
pub(super) fn live_session(env: &dyn AiEnv, session_id: Option<&str>) -> Option<conn::Conn> {
    session_id
        .and_then(|id| env.session(id))
        .filter(|s| !s.is_closed())
}

/// The server a revert (AI-41) on the tab `session_id` runs on, as file changes name it; `None`
/// while the tab is not connected.
pub fn session_server(env: &dyn AiEnv, session_id: Option<&str>) -> Option<String> {
    live_session(env, session_id).map(|s| s.server())
}

/// AI-12: `command` on a new exec channel of the tab's connection, or apart from a local tab's
/// shell (§7.5), in the call's `workdir` and with its timeout. A stop or the timeout ends the
/// command and the processes it started, and the result says whether they ended (AI-18).
async fn run_command(
    env: &dyn AiEnv,
    session_id: Option<&str>,
    args: &RunCommandArgs,
    cancel: &CancellationToken,
) -> (ToolStatus, String) {
    let Some(session) = live_session(env, session_id) else {
        return (ToolStatus::Error, DISCONNECTED.to_owned());
    };
    let limit = args.timeout();
    let stop = async {
        tokio::select! {
            () = cancel.cancelled() => {}
            () = tokio::time::sleep(limit) => {}
        }
    };
    let workdir = args.workdir();
    let ran = match &session {
        conn::Conn::Ssh(ssh) => remote_command::run(ssh, &args.command, workdir, stop)
            .await
            .map_err(|e| e.message),
        conn::Conn::Local(local) => local.tools.run_command(&args.command, workdir, stop).await,
    };
    match ran {
        Ok(run) => {
            let stdout = String::from_utf8_lossy(&run.output.stdout);
            let stderr = String::from_utf8_lossy(&run.output.stderr);
            let Some(stopped) = run.stopped else {
                return (
                    ToolStatus::Ok,
                    tools::truncate_result(&tools::format_exec_result(
                        run.output.exit_status,
                        &stdout,
                        &stderr,
                    )),
                );
            };
            let state = match stopped {
                remote_command::Stopped::NotStarted => StoppedCommand::NotStarted,
                remote_command::Stopped::Ended => StoppedCommand::Ended,
                remote_command::Stopped::MayRun => StoppedCommand::MayRun,
            };
            let (status, stop) = if cancel.is_cancelled() {
                (ToolStatus::Cancelled, CommandStop::User)
            } else {
                (ToolStatus::Error, CommandStop::Timeout(limit.as_secs()))
            };
            let content = tools::stopped_command_result(stop, state, &stdout, &stderr);
            (status, tools::truncate_result(&content))
        }
        Err(_) if session.is_closed() => (ToolStatus::Error, DISCONNECTED.to_owned()),
        Err(message) => (
            ToolStatus::Error,
            format!("The command could not run: {message}"),
        ),
    }
}
