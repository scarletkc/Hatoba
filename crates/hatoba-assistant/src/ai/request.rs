//! Building a request (§13.1): the conversation's next request, its system prompt and built-in
//! tools, and the saved provider, model and thinking level as a request takes them (AI-03,
//! AI-05).

use std::collections::HashMap;

use hatoba_ai::chat::ToolDef;
use hatoba_ai::entry::{AiEntry, EntryBody};
use hatoba_ai::models::known_model;
use hatoba_ai::provider::{AuthHeader, Effort, ModelSpec, Protocol, ProviderConfig};
use hatoba_ai::skills::{BUILTIN_NAME, builtin_description};
use hatoba_ai::tools::{
    self, LocalPrompt, PromptContext, PromptModel, PromptTools, ToolPlace, ToolSet,
};
use hatoba_core::Vault;
use hatoba_core::model::{
    AiAuthHeader, AiEffort as CoreEffort, AiModel as CoreModel, AiProtocol as CoreProtocol,
    AiProvider, Item, MAX_CUSTOM_INSTRUCTIONS_BYTES, MAX_HOST_AI_NOTES_BYTES,
};
use hatoba_core::sync::SharedVault;
use zeroize::Zeroizing;

use super::entries::{Entries, cancel_open_calls, find_conversation};
use super::job::web_search_for;
use super::notes::mcp_deferred;
use super::places::{local_name, quick_label, saved_host};
use super::skills::enabled_skills;
use super::{AiEnv, Halt, Request, Turn, chatgpt, lock, memory};
use crate::dto::{AiEffort, AiLocalTab, AiTurnContext, AiTurnEvent, LocalShellKind};
use crate::mcp::Offer;

/// Reads what the next request needs, after storing a cancelled result for every call without
/// one (§13.1).
pub(super) fn prepare(
    vault: &SharedVault,
    env: &dyn AiEnv,
    conversation_id: &str,
    turn: &Turn,
    offer: &Offer,
) -> Result<Request, Halt> {
    let mut stored = false;
    let result = prepare_locked(&mut lock(vault), conversation_id, turn, offer, &mut stored);
    if stored {
        env.changed();
    }
    result
}

fn prepare_locked(
    v: &mut Vault,
    conversation_id: &str,
    turn: &Turn,
    offer: &Offer,
    stored: &mut bool,
) -> Result<Request, Halt> {
    if turn.cancel.is_cancelled() || !v.is_unlocked() {
        return Err(Halt::Stopped);
    }
    let conversation = find_conversation(v, conversation_id)
        .map_err(|_| Halt::Failed("The conversation no longer exists.".into()))?;
    let mut entries = Entries::load(v, conversation_id)?;
    let cancelled = cancel_open_calls(v, conversation_id, &entries, HashMap::new())?;
    if !cancelled.is_empty() {
        *stored = true;
        for entry in cancelled {
            turn.emit(AiTurnEvent::Entry { entry });
        }
        entries = Entries::load(v, conversation_id)?;
    }
    let from = entries.context_from(conversation.context_start.as_deref());
    let deferred = mcp_deferred(&entries.list);
    let context = entries.list.split_off(from);
    if nothing_to_send(&context) {
        return Err(Halt::Idle);
    }
    next_request(
        v,
        &turn.context,
        turn.server_id.as_deref(),
        offer,
        context,
        deferred,
    )
    .map_err(Halt::Failed)
}

/// The conversation's next request (§13.1) for its context `entries`: the system prompt that
/// offers tools, the built-in tools, then the MCP tools of `offer`, or `search_tools` and
/// `call_mcp_tool` when the conversation's MCP tools are `deferred` (AI-45). Every request of a
/// turn, every Compact request (AI-21) and every side question's (AI-42) is built here, so the last
/// two have their turn's prefix. The error is a sentence for the panel.
pub(super) fn next_request(
    v: &Vault,
    context: &AiTurnContext,
    server_id: Option<&str>,
    offer: &Offer,
    entries: Vec<AiEntry>,
    deferred: bool,
) -> Result<Request, String> {
    let (provider, model) = resolve_model(v, &context.provider_id, &context.model_id)?;
    let (system, mut tools) = prompt(v, context, None, server_id);
    // AI-30: after the built-in tools, those of the MCP servers, with or without a tab (AI-09),
    // or, once the conversation's MCP tools are deferred, the two tools that stand for them
    // whatever servers are on (AI-45).
    if deferred {
        tools.extend(tools::deferred_mcp_tools());
    } else {
        tools.extend(offer.tool_defs(provider.protocol));
    }
    Ok(Request {
        provider_id: context.provider_id.clone(),
        provider,
        model,
        effort: context.effort.map(request_effort),
        system,
        tools,
        entries,
    })
}

/// Whether a request built from these entries would carry nothing, or end with an assistant
/// message (newer models reject that as a prefill). The adapter skips blank assistant entries
/// and places every result right after its call, so those are looked past.
pub(super) fn nothing_to_send(entries: &[AiEntry]) -> bool {
    for entry in entries.iter().rev() {
        match &entry.body {
            EntryBody::Tool { .. } => {}
            EntryBody::Assistant(a) if a.text.trim().is_empty() && a.tool_calls.is_empty() => {}
            EntryBody::Assistant(a) => return a.tool_calls.is_empty(),
            EntryBody::User { .. } | EntryBody::Summary { .. } => return false,
        }
    }
    true
}

/// The system prompt and built-in tools of a request (§13.1): the terminal tools only with a
/// connected tab (AI-09), `web_search` when it can run (AI-14), `fetch_url` always, and
/// `read_skill` when an enabled skill exists, the built-in one included (AI-34). `fallback` is
/// `None` but for the request without tools that a Compact request (AI-21) or a side question
/// (AI-42) falls back to, whose `PromptTools` says what it asks for. `server_id` is the tab's SSH
/// server's identification string.
/// The prompt names the model and its provider, and carries the user's custom instructions
/// (AI-36) and the host's AI notes (AI-37), each cut to its limit.
pub(super) fn prompt(
    v: &Vault,
    context: &AiTurnContext,
    fallback: Option<PromptTools>,
    server_id: Option<&str>,
) -> (String, Vec<ToolDef>) {
    let host = context.host_id.as_deref().and_then(|id| saved_host(v, id));
    let target = context.target.as_ref().map(quick_label);
    // §7.5: a local tab's computer, named like a host, and its shell.
    let local = local_tab(context);
    let local_named = local.map(|l| local_name(&l.device));
    // A WSL distribution's user is its own, which Windows does not know.
    let local_user = local
        .filter(|l| l.kind != Some(LocalShellKind::Wsl))
        .map(|_| hatoba_local::system::default_user());
    let os = hatoba_local::system::os_label();
    let local_prompt = local.and_then(|l| {
        Some(LocalPrompt {
            device: &l.device,
            os: &os,
            shell: l.shell.as_deref()?,
            place: tool_place(context),
            wsl: l.kind == Some(LocalShellKind::Wsl),
        })
    });
    let provider = v.get(&context.provider_id).and_then(Item::as_ai_provider);
    let model_name = provider
        .and_then(|p| p.models.iter().find(|m| m.id == context.model_id))
        .map_or("", |m| m.name.as_str());
    let settings = v.settings();
    let mut skills: Vec<(String, String)> = enabled_skills(v)
        .into_iter()
        .map(|(_, s)| (s.name, s.description))
        .collect();
    if settings.ai.builtin_skill_enabled {
        skills.push((BUILTIN_NAME.to_owned(), builtin_description()));
    }
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let offered = match (fallback, context.tab) {
        (Some(plain), _) => plain,
        (None, false) => PromptTools::NoTerminal,
        (None, true) => PromptTools::Terminal,
    };
    let system = tools::system_prompt(&PromptContext {
        model: PromptModel {
            id: &context.model_id,
            name: model_name,
            provider: provider.map_or("", |p| p.name.as_str()),
        },
        host_name: host
            .as_ref()
            .map(|h| h.name)
            .or(target.as_deref())
            .or(local_named.as_deref()),
        host_user: host
            .as_ref()
            .map(|h| h.username)
            .or(context.target.as_ref().map(|t| t.username.as_str()))
            .or(local_user.as_deref()),
        server_id,
        local: local_prompt,
        telnet: context.telnet,
        date: &date,
        instructions: bytes_prefix(
            &settings.ai.custom_instructions,
            MAX_CUSTOM_INSTRUCTIONS_BYTES,
        ),
        host_notes: host.map_or("", |h| bytes_prefix(h.ai_notes, MAX_HOST_AI_NOTES_BYTES)),
        skills: &skills,
        memory: settings.ai.memory,
        tools: offered,
    });
    let defs = if fallback.is_none() {
        builtin_tools(v, context)
    } else {
        Vec::new()
    };
    (system, defs)
}

/// The built-in tools of a request (§13.4): the terminal tools only with a connected tab (AI-09),
/// `web_search` when a search provider is chosen or the model's provider has a search of its own
/// (AI-14), `fetch_url` always, `read_skill` when an enabled skill exists, the built-in one
/// included (AI-34), and `memory` while memory is on (AI-53).
pub(super) fn builtin_tools(v: &Vault, context: &AiTurnContext) -> Vec<ToolDef> {
    tools::builtin_tools(&ToolSet {
        terminal: context.tab,
        web_search: web_search_for(v, &context.provider_id, &context.model_id).is_some(),
        read_skill: !enabled_skills(v).is_empty() || v.settings().ai.builtin_skill_enabled,
        memory: memory::on(v),
        place: tool_place(context),
    })
}

/// A local tab's computer and shell (§7.5), while the context names neither a host nor a target.
fn local_tab(context: &AiTurnContext) -> Option<&AiLocalTab> {
    context
        .local
        .as_ref()
        .filter(|_| context.host_id.is_none() && context.target.is_none())
}

/// Where the terminal tools act, which their descriptions say: a host, or this computer with
/// `sh` (macOS, Linux, WSL) or Windows itself (§7.5).
fn tool_place(context: &AiTurnContext) -> ToolPlace {
    if context.telnet {
        return ToolPlace::Telnet;
    }
    match local_tab(context).and_then(|l| l.kind) {
        None => ToolPlace::Ssh,
        Some(LocalShellKind::Unix | LocalShellKind::Wsl) => ToolPlace::Local,
        Some(LocalShellKind::Powershell | LocalShellKind::Cmd | LocalShellKind::GitBash) => {
            ToolPlace::LocalWindows
        }
    }
}

/// At most the first `max` bytes of `s`, cut at a character. Saving refuses longer text; text
/// synced from elsewhere (an older build counted characters) is cut.
pub(super) fn bytes_prefix(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// §13.1: the identification string of the tab's SSH server, read once when a turn starts, so
/// every request of the turn has the same system prompt. Only with a connected tab.
pub(super) fn turn_server_id(env: &dyn AiEnv, context: &AiTurnContext) -> Option<String> {
    if !context.tab {
        return None;
    }
    context
        .session_id
        .as_deref()
        .and_then(|id| env.server_id(id))
}

/// The provider and model a request uses (AI-05). The error is a sentence for the panel.
pub(super) fn resolve_model(
    v: &Vault,
    provider_id: &str,
    model_id: &str,
) -> Result<(ProviderConfig, ModelSpec), String> {
    let provider = v
        .get(provider_id)
        .and_then(Item::as_ai_provider)
        .ok_or_else(|| "The model's provider no longer exists. Choose another model.".to_owned())?;
    let model = provider
        .models
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| {
            format!("The model \"{model_id}\" is not in its provider's model list. Choose another model.")
        })?;
    Ok((
        provider_config(provider_id, provider),
        model_spec(provider, model),
    ))
}

/// Saved model `model` of `provider` as a request takes it: what the row leaves unknown comes
/// from models.dev for the provider's protocol (AI-03), so a model saved before models.dev knew
/// it, or typed in, gets the same as one fetched now.
pub fn model_spec(provider: &AiProvider, model: &CoreModel) -> ModelSpec {
    let mut spec = ModelSpec {
        id: model.id.clone(),
        context_window: model.context_window,
        max_output_tokens: model.max_output_tokens,
        efforts: model
            .efforts
            .as_ref()
            .map(|levels| levels.iter().copied().map(model_effort).collect()),
        adaptive_thinking: model.adaptive_thinking,
    };
    spec.fill_known(request_protocol(provider));
    spec
}

/// The protocol of saved `provider`'s requests: a ChatGPT provider (AI-52) speaks Responses
/// whatever its stored protocol says.
fn request_protocol(provider: &AiProvider) -> Protocol {
    if provider.chatgpt.is_some() {
        Protocol::Responses
    } else {
        protocol(provider.protocol)
    }
}

/// AI-03: when the form saves `provider` with another protocol than `before` had, the levels and
/// adaptive thinking a row holds as models.dev gave them for the old protocol are dropped, so
/// models.dev fills what the new one takes. The form shows a row's unknown fields as models.dev
/// has them and saves what it shows, so without this a gateway's Claude model moved to Chat
/// Completions would keep sending Anthropic's levels there.
pub fn drop_levels_of_old_protocol(before: &AiProvider, provider: &mut AiProvider) {
    let old = request_protocol(before);
    if old == request_protocol(provider) {
        return;
    }
    for model in &mut provider.models {
        let Some(known) = known_model(&model.id, old) else {
            continue;
        };
        let levels = model
            .efforts
            .as_ref()
            .map(|levels| levels.iter().copied().map(model_effort).collect::<Vec<_>>());
        if known.efforts.is_some() && levels == known.efforts {
            model.efforts = None;
        }
        if known.adaptive_thinking.is_some() && model.adaptive_thinking == known.adaptive_thinking {
            model.adaptive_thinking = None;
        }
    }
}

/// What a request needs to reach saved provider `id`. A ChatGPT provider (AI-52) is a Responses
/// provider at OpenAI's API, whose requests carry an access token of this device's session.
pub fn provider_config(id: &str, p: &AiProvider) -> ProviderConfig {
    if p.chatgpt.is_some() {
        return ProviderConfig {
            protocol: Protocol::Responses,
            base_url: chatgpt::BASE_URL.to_owned(),
            api_key: Zeroizing::default(),
            auth_header: AuthHeader::Authorization,
            chatgpt: Some(hatoba_ai::chatgpt::ChatgptAuth {
                sessions: chatgpt::sessions().clone(),
                id: id.to_owned(),
            }),
        };
    }
    ProviderConfig {
        protocol: protocol(p.protocol),
        base_url: p.base_url.clone(),
        api_key: p.api_key.clone(),
        auth_header: auth_header(p.auth_header),
        chatgpt: None,
    }
}

pub fn protocol(p: CoreProtocol) -> Protocol {
    match p {
        CoreProtocol::ChatCompletions => Protocol::ChatCompletions,
        CoreProtocol::Anthropic => Protocol::Anthropic,
        CoreProtocol::Responses => Protocol::Responses,
        CoreProtocol::SystemOne => Protocol::SystemOne,
    }
}

pub fn auth_header(h: AiAuthHeader) -> AuthHeader {
    match h {
        AiAuthHeader::XApiKey => AuthHeader::XApiKey,
        AiAuthHeader::Authorization => AuthHeader::Authorization,
    }
}

/// A thinking level from the panel, as stored (AI-05).
pub fn core_effort(e: AiEffort) -> CoreEffort {
    match e {
        AiEffort::Low => CoreEffort::Low,
        AiEffort::Medium => CoreEffort::Medium,
        AiEffort::High => CoreEffort::High,
        AiEffort::Xhigh => CoreEffort::Xhigh,
        AiEffort::Max => CoreEffort::Max,
    }
}

/// A stored thinking level, for the panel.
pub fn effort_view(e: CoreEffort) -> AiEffort {
    match e {
        CoreEffort::Low => AiEffort::Low,
        CoreEffort::Medium => AiEffort::Medium,
        CoreEffort::High => AiEffort::High,
        CoreEffort::Xhigh => AiEffort::Xhigh,
        CoreEffort::Max => AiEffort::Max,
    }
}

/// A thinking level from the panel, as a request takes it.
fn request_effort(e: AiEffort) -> Effort {
    model_effort(core_effort(e))
}

/// A level a stored model offers, as a request takes it.
fn model_effort(e: CoreEffort) -> Effort {
    match e {
        CoreEffort::Low => Effort::Low,
        CoreEffort::Medium => Effort::Medium,
        CoreEffort::High => Effort::High,
        CoreEffort::Xhigh => Effort::Xhigh,
        CoreEffort::Max => Effort::Max,
    }
}

/// A level from a provider's model list (AI-03), for the panel.
pub fn listed_effort(e: Effort) -> AiEffort {
    match e {
        Effort::Low => AiEffort::Low,
        Effort::Medium => AiEffort::Medium,
        Effort::High => AiEffort::High,
        Effort::Xhigh => AiEffort::Xhigh,
        Effort::Max => AiEffort::Max,
    }
}
