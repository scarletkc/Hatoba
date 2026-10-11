//! AI assistant commands (spec §13): providers, search providers and the synced settings of
//! Settings → AI, conversations with history search (AI-24), and the turn protocol of §13.1 with
//! edit and resend (AI-26), which runs in [`crate::ai`]. Skills and MCP servers have their own
//! modules ([`super::skills`], [`super::mcp`]).
//! API keys go one way to Rust: views say only whether a key is saved (AI-01).

use std::sync::Arc;

use hatoba_ai::AiError;
use hatoba_ai::models::{TestFailure, TestOutcome};
use hatoba_ai::provider::{ProviderConfig, validate_base_url};
use hatoba_ai::web::SearchConfig;
use hatoba_core::Vault;
use hatoba_core::model::{
    AiAuthHeader as CoreAuthHeader, AiConversation, AiModel as CoreModel, AiModelRef as CoreRef,
    AiProtocol as CoreProtocol, AiProvider, AiSettings, Item, MAX_CUSTOM_INSTRUCTIONS_CHARS,
    SETTINGS_ID, SearchKind as CoreSearchKind, SearchProvider,
};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

use crate::ai::{
    AiEnv, EventSink, auth_header, conversation_view, conversation_view_at, core_effort,
    effort_view, find_conversation, listed_effort, protocol, search_kind,
};
use crate::dto::{
    AiAuthHeader, AiConversationDetail, AiConversationView, AiEntryView, AiFilePreview, AiModel,
    AiModelRef, AiProtocol, AiProviderInput, AiProviderView, AiSearchHit, AiSendInput,
    AiSendStarted, AiSettingsView, AiTestFailure, AiTestResult, AiToolResultInput, AiTurnContext,
    AiTurnEvent, DroppedFile, SearchKind, SearchProviderInput, SearchProviderView,
};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, blocking, state};
use crate::{dropped, sync};

/// The query `search_provider_test` sends.
const TEST_QUERY: &str = "Hatoba SSH client";

/// The app side of the AI runtime: the sync trigger and the terminal tabs' sessions.
struct AppEnv(AppHandle);

impl AiEnv for AppEnv {
    fn changed(&self) {
        sync::local_change(&self.0);
    }

    fn session(&self, session_id: &str) -> Option<hatoba_ssh::SshSession> {
        state(&self.0)
            .ssh
            .get(session_id)
            .ok()
            .map(|live| live.session.clone())
    }

    fn app_version(&self) -> String {
        self.0.package_info().version.to_string()
    }
}

// ───────────────────────── conversions ─────────────────────────

fn core_protocol(p: AiProtocol) -> CoreProtocol {
    match p {
        AiProtocol::ChatCompletions => CoreProtocol::ChatCompletions,
        AiProtocol::Anthropic => CoreProtocol::Anthropic,
    }
}

fn core_auth_header(h: AiAuthHeader) -> CoreAuthHeader {
    match h {
        AiAuthHeader::XApiKey => CoreAuthHeader::XApiKey,
        AiAuthHeader::Authorization => CoreAuthHeader::Authorization,
    }
}

fn core_search_kind(kind: SearchKind) -> CoreSearchKind {
    match kind {
        SearchKind::Brave => CoreSearchKind::Brave,
        SearchKind::Tavily => CoreSearchKind::Tavily,
        SearchKind::Searxng => CoreSearchKind::Searxng,
    }
}

pub(crate) fn provider_view(id: &str, p: &AiProvider) -> AiProviderView {
    AiProviderView {
        id: id.to_owned(),
        name: p.name.clone(),
        protocol: match p.protocol {
            CoreProtocol::ChatCompletions => AiProtocol::ChatCompletions,
            CoreProtocol::Anthropic => AiProtocol::Anthropic,
        },
        base_url: p.base_url.clone(),
        has_api_key: !p.api_key.is_empty(),
        auth_header: match p.auth_header {
            CoreAuthHeader::XApiKey => AiAuthHeader::XApiKey,
            CoreAuthHeader::Authorization => AiAuthHeader::Authorization,
        },
        models: p
            .models
            .iter()
            .map(|m| AiModel {
                id: m.id.clone(),
                name: m.name.clone(),
                context_window: m.context_window,
                max_output_tokens: m.max_output_tokens,
                efforts: m
                    .efforts
                    .as_ref()
                    .map(|levels| levels.iter().copied().map(effort_view).collect()),
                adaptive_thinking: m.adaptive_thinking,
            })
            .collect(),
        updated_at: p.updated_at,
    }
}

pub(crate) fn search_provider_view(id: &str, p: &SearchProvider) -> SearchProviderView {
    SearchProviderView {
        id: id.to_owned(),
        kind: match p.kind {
            CoreSearchKind::Brave => SearchKind::Brave,
            CoreSearchKind::Tavily => SearchKind::Tavily,
            CoreSearchKind::Searxng => SearchKind::Searxng,
        },
        base_url: p.base_url.clone(),
        has_api_key: !p.api_key.is_empty(),
        updated_at: p.updated_at,
    }
}

fn ai_settings_view(s: &AiSettings) -> AiSettingsView {
    AiSettingsView {
        default_model: s.default_model.as_ref().map(|m| AiModelRef {
            provider_id: m.provider_id.clone(),
            model_id: m.model_id.clone(),
        }),
        default_effort: s.default_effort.map(effort_view),
        search_provider_id: s.search_provider_id.clone(),
        builtin_skill_enabled: s.builtin_skill_enabled,
        custom_instructions: s.custom_instructions.clone(),
    }
}

fn test_result(outcome: TestOutcome) -> AiTestResult {
    AiTestResult {
        ok: outcome.ok,
        failure: outcome.failure.map(|f| match f {
            TestFailure::Auth => AiTestFailure::Auth,
            TestFailure::Network => AiTestFailure::Network,
            TestFailure::UnknownModel => AiTestFailure::UnknownModel,
            TestFailure::InvalidUrl => AiTestFailure::InvalidUrl,
            TestFailure::Other => AiTestFailure::Other,
        }),
        status: outcome.status,
        message: outcome.message,
    }
}

/// A failed search provider test, told apart like AI-04 does for model providers.
fn search_test_failure(e: &AiError) -> AiTestResult {
    let failure = match e {
        AiError::InvalidUrl(_) => AiTestFailure::InvalidUrl,
        AiError::Config(_)
        | AiError::Http {
            status: 401 | 403, ..
        } => AiTestFailure::Auth,
        AiError::Network(_) => AiTestFailure::Network,
        _ => AiTestFailure::Other,
    };
    let message = match e {
        AiError::Http { message, .. }
        | AiError::InvalidUrl(message)
        | AiError::Config(message)
        | AiError::Network(message)
        | AiError::Protocol(message) => message.clone(),
        other => other.to_string(),
    };
    AiTestResult {
        ok: false,
        failure: Some(failure),
        status: e.status(),
        message: Some(message),
    }
}

/// A key from a form: `None` keeps `saved` (HOST-08), `""` clears it.
fn api_key(input: Option<&str>, saved: Option<&Zeroizing<String>>) -> Zeroizing<String> {
    match (input, saved) {
        (Some(key), _) => Zeroizing::new(key.trim().to_owned()),
        (None, Some(saved)) => saved.clone(),
        (None, None) => Zeroizing::new(String::new()),
    }
}

/// AI-02: checks a base URL and returns it as entered, trimmed.
async fn checked_base_url(base_url: &str) -> AppResult<String> {
    let base_url = base_url.trim();
    if base_url.is_empty() {
        return Err(AppError::invalid("base_url", "the base URL is required"));
    }
    validate_base_url(base_url)
        .await
        .map_err(|e| AppError::invalid("base_url", e.to_string()))?;
    Ok(base_url.to_owned())
}

// ───────────────────────── providers (AI-01…04) ─────────────────────────

fn find_provider(v: &Vault, id: &str) -> AppResult<AiProvider> {
    v.get(id)
        .and_then(Item::as_ai_provider)
        .cloned()
        .ok_or_else(|| AppError::not_found("provider"))
}

/// Validates the model list: unique, non-empty ids; a blank name shows the id. Thinking levels
/// are stored lowest first, each once (AI-05).
fn models_from(models: &[AiModel]) -> AppResult<Vec<CoreModel>> {
    let mut out: Vec<CoreModel> = Vec::with_capacity(models.len());
    for m in models {
        let id = m.id.trim();
        if id.is_empty() {
            return Err(AppError::invalid("models", "a model has no id"));
        }
        if out.iter().any(|o| o.id == id) {
            return Err(AppError::invalid(
                "models",
                format!("the model {id} is listed twice"),
            ));
        }
        let name = m.name.trim();
        let efforts = m.efforts.as_ref().map(|levels| {
            let mut levels: Vec<_> = levels.iter().copied().map(core_effort).collect();
            levels.sort_unstable();
            levels.dedup();
            levels
        });
        out.push(CoreModel {
            id: id.to_owned(),
            name: if name.is_empty() { id } else { name }.to_owned(),
            context_window: m.context_window.filter(|n| *n > 0),
            max_output_tokens: m.max_output_tokens.filter(|n| *n > 0),
            efforts,
            adaptive_thinking: m.adaptive_thinking,
        });
    }
    Ok(out)
}

/// Stores a provider from the form; `base_url` is already checked.
pub(crate) fn save_provider(
    v: &mut Vault,
    input: &AiProviderInput,
    base_url: String,
) -> AppResult<AiProviderView> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::invalid("name", "name is required"));
    }
    if name.chars().count() > 100 {
        return Err(AppError::invalid("name", "name is too long"));
    }
    let existing = match &input.id {
        Some(id) => Some(find_provider(v, id)?),
        None => None,
    };
    let provider = AiProvider {
        name: name.to_owned(),
        protocol: core_protocol(input.protocol),
        base_url,
        api_key: api_key(
            input.api_key.as_deref(),
            existing.as_ref().map(|p| &p.api_key),
        ),
        auth_header: core_auth_header(input.auth_header),
        models: models_from(&input.models)?,
        updated_at: 0,
    };
    let id = v.put(input.id.as_deref(), Item::AiProvider(provider))?;
    Ok(provider_view(&id, &find_provider(v, &id)?))
}

/// The connection of a possibly unsaved provider. With an id, a `None` key is the saved key.
pub(crate) fn provider_config_for(v: &Vault, input: &AiProviderInput) -> AppResult<ProviderConfig> {
    let saved = match (&input.api_key, &input.id) {
        (None, Some(id)) => Some(find_provider(v, id)?.api_key),
        _ => None,
    };
    Ok(ProviderConfig {
        protocol: protocol(core_protocol(input.protocol)),
        base_url: input.base_url.trim().to_owned(),
        api_key: api_key(input.api_key.as_deref(), saved.as_ref()),
        auth_header: auth_header(core_auth_header(input.auth_header)),
    })
}

/// Deletes a provider and the default model that pointed at it.
pub(crate) fn delete_provider(v: &mut Vault, id: &str) -> AppResult<()> {
    find_provider(v, id)?;
    v.delete(id)?;
    let mut settings = v.settings();
    if settings
        .ai
        .default_model
        .as_ref()
        .is_some_and(|m| m.provider_id == id)
    {
        settings.ai.default_model = None;
        v.put(Some(SETTINGS_ID), Item::Settings(settings))?;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn ai_providers_list(state: State<'_, AppState>) -> AppResult<Vec<AiProviderView>> {
    state.with_unlocked(|v| {
        let mut list: Vec<AiProviderView> = v
            .ai_providers()
            .iter()
            .map(|(id, p)| provider_view(id, p))
            .collect();
        list.sort_by_cached_key(|p| p.name.to_lowercase());
        Ok(list)
    })
}

#[tauri::command]
#[specta::specta]
pub async fn ai_provider_save(
    app: AppHandle,
    state: State<'_, AppState>,
    input: AiProviderInput,
) -> AppResult<AiProviderView> {
    let base_url = checked_base_url(&input.base_url).await?;
    let view = state.with_unlocked(|v| save_provider(v, &input, base_url))?;
    sync::local_change(&app);
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub fn ai_provider_delete(app: AppHandle, state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.with_unlocked(|v| delete_provider(v, &id))?;
    sync::local_change(&app);
    Ok(())
}

/// AI-03: the model list of a saved or unsaved provider.
#[tauri::command]
#[specta::specta]
pub async fn ai_provider_models(
    state: State<'_, AppState>,
    input: AiProviderInput,
) -> AppResult<Vec<AiModel>> {
    // Registered with the read, so a lock either came first or aborts the request.
    let (config, op) =
        state.with_unlocked(|v| Ok((provider_config_for(v, &input)?, state.ai.background_op())))?;
    let models = hatoba_ai::models::list_models(&state.ai.http(), &config, op.token())
        .await
        .map_err(|e| {
            tracing::info!(
                kind = crate::ai::error_kind(&e),
                status = ?e.status(),
                "AI model list failed"
            );
            AppError::from(e)
        })?;
    Ok(models
        .into_iter()
        .map(|m| AiModel {
            id: m.id,
            name: m.name,
            context_window: m.context_window,
            max_output_tokens: m.max_output_tokens,
            efforts: m
                .efforts
                .map(|levels| levels.into_iter().map(listed_effort).collect()),
            adaptive_thinking: m.adaptive_thinking,
        })
        .collect())
}

/// AI-04: a minimal request to the first model, or the model list when there is none.
#[tauri::command]
#[specta::specta]
pub async fn ai_provider_test(
    state: State<'_, AppState>,
    input: AiProviderInput,
) -> AppResult<AiTestResult> {
    let (config, op) =
        state.with_unlocked(|v| Ok((provider_config_for(v, &input)?, state.ai.background_op())))?;
    let model = input.models.first().map(|m| m.id.trim().to_owned());
    let outcome =
        hatoba_ai::models::test_connection(&state.ai.http(), &config, model.as_deref(), op.token())
            .await;
    tracing::info!(ok = outcome.ok, failure = ?outcome.failure, status = ?outcome.status, "AI provider test");
    Ok(test_result(outcome))
}

// ───────────────────────── search providers (AI-14) ─────────────────────────

fn find_search_provider(v: &Vault, id: &str) -> AppResult<SearchProvider> {
    v.get(id)
        .and_then(Item::as_search_provider)
        .cloned()
        .ok_or_else(|| AppError::not_found("search provider"))
}

/// Stores a search provider from the form; `base_url` is the checked SearXNG URL.
pub(crate) fn save_search_provider(
    v: &mut Vault,
    input: &SearchProviderInput,
    base_url: Option<String>,
) -> AppResult<SearchProviderView> {
    let existing = match &input.id {
        Some(id) => Some(find_search_provider(v, id)?),
        None => None,
    };
    let kind = core_search_kind(input.kind);
    let api_key = if kind == CoreSearchKind::Searxng {
        Zeroizing::new(String::new())
    } else {
        // A key saved for another kind of service is not reused.
        let saved = existing
            .as_ref()
            .filter(|p| p.kind == kind)
            .map(|p| &p.api_key);
        let key = api_key(input.api_key.as_deref(), saved);
        if key.is_empty() {
            return Err(AppError::invalid("api_key", "an API key is required"));
        }
        key
    };
    let provider = SearchProvider {
        kind,
        base_url: if kind == CoreSearchKind::Searxng {
            base_url
        } else {
            None
        },
        api_key,
        updated_at: 0,
    };
    let id = v.put(input.id.as_deref(), Item::SearchProvider(provider))?;
    Ok(search_provider_view(&id, &find_search_provider(v, &id)?))
}

/// The configuration of a possibly unsaved search provider.
fn search_config_for(v: &Vault, input: &SearchProviderInput) -> AppResult<SearchConfig> {
    let kind = core_search_kind(input.kind);
    let saved = match (&input.api_key, &input.id) {
        (None, Some(id)) => Some(find_search_provider(v, id)?).filter(|p| p.kind == kind),
        _ => None,
    };
    Ok(SearchConfig {
        kind: search_kind(kind),
        base_url: input
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .map(str::to_owned),
        api_key: api_key(input.api_key.as_deref(), saved.as_ref().map(|p| &p.api_key)),
    })
}

/// Deletes a search provider and the choice that pointed at it.
pub(crate) fn delete_search_provider(v: &mut Vault, id: &str) -> AppResult<()> {
    find_search_provider(v, id)?;
    v.delete(id)?;
    let mut settings = v.settings();
    if settings.ai.search_provider_id.as_deref() == Some(id) {
        settings.ai.search_provider_id = None;
        v.put(Some(SETTINGS_ID), Item::Settings(settings))?;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn search_providers_list(state: State<'_, AppState>) -> AppResult<Vec<SearchProviderView>> {
    state.with_unlocked(|v| {
        let mut list: Vec<SearchProviderView> = v
            .search_providers()
            .iter()
            .map(|(id, p)| search_provider_view(id, p))
            .collect();
        list.sort_by_key(|p| p.updated_at);
        Ok(list)
    })
}

#[tauri::command]
#[specta::specta]
pub async fn search_provider_save(
    app: AppHandle,
    state: State<'_, AppState>,
    input: SearchProviderInput,
) -> AppResult<SearchProviderView> {
    let base_url = match input.kind {
        SearchKind::Searxng => {
            Some(checked_base_url(input.base_url.as_deref().unwrap_or_default()).await?)
        }
        SearchKind::Brave | SearchKind::Tavily => None,
    };
    let view = state.with_unlocked(|v| save_search_provider(v, &input, base_url))?;
    sync::local_change(&app);
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub fn search_provider_delete(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    state.with_unlocked(|v| delete_search_provider(v, &id))?;
    sync::local_change(&app);
    Ok(())
}

/// A real query through a saved or unsaved search provider.
#[tauri::command]
#[specta::specta]
pub async fn search_provider_test(
    state: State<'_, AppState>,
    input: SearchProviderInput,
) -> AppResult<AiTestResult> {
    let (config, op) =
        state.with_unlocked(|v| Ok((search_config_for(v, &input)?, state.ai.background_op())))?;
    let result =
        hatoba_ai::web::web_search(&state.ai.http(), &config, TEST_QUERY, op.token()).await;
    Ok(match result {
        Ok(_) => AiTestResult {
            ok: true,
            failure: None,
            status: None,
            message: None,
        },
        Err(e) => {
            tracing::info!(kind = crate::ai::error_kind(&e), status = ?e.status(), "search provider test failed");
            search_test_failure(&e)
        }
    })
}

// ───────────────────────── synced settings ─────────────────────────

/// Saves `Settings.ai`. A reference the user changed must point at an existing provider and model
/// or search provider; an unchanged one is kept as it is, even when it dangles (e.g. the item it
/// names has not synced to this device yet).
pub(crate) fn save_ai_settings(v: &mut Vault, input: &AiSettingsView) -> AppResult<()> {
    let mut settings = v.settings();
    let current = &settings.ai;
    let default_model = match &input.default_model {
        None => None,
        Some(m) => {
            let unchanged = current
                .default_model
                .as_ref()
                .is_some_and(|c| c.provider_id == m.provider_id && c.model_id == m.model_id);
            if !unchanged {
                let provider = v
                    .get(&m.provider_id)
                    .and_then(Item::as_ai_provider)
                    .ok_or_else(|| AppError::invalid("default_model", "provider not found"))?;
                if !provider.models.iter().any(|x| x.id == m.model_id) {
                    return Err(AppError::invalid(
                        "default_model",
                        "the provider has no such model",
                    ));
                }
            }
            Some(CoreRef {
                provider_id: m.provider_id.clone(),
                model_id: m.model_id.clone(),
            })
        }
    };
    let search_provider_id = match &input.search_provider_id {
        Some(id) if current.search_provider_id.as_ref() != Some(id) => {
            find_search_provider(v, id).map_err(|_| {
                AppError::invalid("search_provider_id", "search provider not found")
            })?;
            Some(id.clone())
        }
        other => other.clone(),
    };
    if input.custom_instructions.chars().count() > MAX_CUSTOM_INSTRUCTIONS_CHARS {
        return Err(AppError::invalid(
            "custom_instructions",
            "custom instructions are limited to 4,000 characters",
        ));
    }
    let ai = AiSettings {
        default_model,
        default_effort: input.default_effort.map(core_effort),
        search_provider_id,
        builtin_skill_enabled: input.builtin_skill_enabled,
        custom_instructions: input.custom_instructions.clone(),
    };
    if ai == settings.ai {
        return Ok(());
    }
    settings.ai = ai;
    v.put(Some(SETTINGS_ID), Item::Settings(settings))?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn ai_settings_get(state: State<'_, AppState>) -> AppResult<AiSettingsView> {
    state.with_unlocked(|v| Ok(ai_settings_view(&v.settings().ai)))
}

#[tauri::command]
#[specta::specta]
pub fn ai_settings_save(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: AiSettingsView,
) -> AppResult<()> {
    state.with_unlocked(|v| save_ai_settings(v, &settings))?;
    sync::local_change(&app);
    Ok(())
}

// ───────────────────────── conversations (AI-23) ─────────────────────────

fn update_conversation(
    v: &mut Vault,
    id: &str,
    change: impl FnOnce(&mut AiConversation),
) -> AppResult<AiConversationView> {
    let mut conversation = find_conversation(v, id)?;
    let before = conversation.clone();
    change(&mut conversation);
    if conversation != before {
        v.put(Some(id), Item::AiConversation(conversation))?;
    }
    Ok(conversation_view(v, id, &find_conversation(v, id)?))
}

pub(crate) fn rename_conversation(
    v: &mut Vault,
    id: &str,
    title: &str,
) -> AppResult<AiConversationView> {
    let title = title.trim();
    if !(1..=200).contains(&title.chars().count()) {
        return Err(AppError::invalid(
            "title",
            "the title must be 1 to 200 characters",
        ));
    }
    update_conversation(v, id, |c| title.clone_into(&mut c.title))
}

/// Every conversation, unsorted; the panel sorts by `last_activity`, pinned first.
#[tauri::command]
#[specta::specta]
pub async fn ai_conversations_list(
    state: State<'_, AppState>,
) -> AppResult<Vec<AiConversationView>> {
    state.with_unlocked(|v| {
        let last = v.ai_last_activities();
        Ok(v.ai_conversations()
            .iter()
            .map(|(id, c)| conversation_view_at(id, c, last.get(id).copied().unwrap_or(0)))
            .collect())
    })
}

#[tauri::command]
#[specta::specta]
pub async fn ai_conversation_get(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<AiConversationDetail> {
    state.ai.conversation_detail(&state.vault, &id)
}

#[tauri::command]
#[specta::specta]
pub async fn ai_conversation_rename(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    title: String,
) -> AppResult<AiConversationView> {
    let view = state.with_unlocked(|v| rename_conversation(v, &id, &title))?;
    sync::local_change(&app);
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub async fn ai_conversation_pin(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    pinned: bool,
) -> AppResult<AiConversationView> {
    let view = state.with_unlocked(|v| update_conversation(v, &id, |c| c.pinned = pinned))?;
    sync::local_change(&app);
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub async fn ai_conversation_delete(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    state
        .ai
        .delete_conversation(&state.vault, &AppEnv(app), &id)
}

// ───────────────────────── turns (§13.1) ─────────────────────────

/// Stores the user's message and starts a turn; its events stream on `channel` until
/// `turn_ended`.
#[tauri::command]
#[specta::specta]
pub async fn ai_send(
    app: AppHandle,
    state: State<'_, AppState>,
    input: AiSendInput,
    channel: Channel<AiTurnEvent>,
) -> AppResult<AiSendStarted> {
    let sink: Arc<dyn EventSink> = Arc::new(channel);
    let (started, task) = state
        .ai
        .send(&state.vault, Arc::new(AppEnv(app)), input, sink)
        .await?;
    tauri::async_runtime::spawn(task);
    Ok(started)
}

/// Retry after an error: the next request from the stored conversation, on a new channel.
#[tauri::command]
#[specta::specta]
pub async fn ai_retry(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    context: AiTurnContext,
    channel: Channel<AiTurnEvent>,
) -> AppResult<()> {
    let sink: Arc<dyn EventSink> = Arc::new(channel);
    let task = state.ai.retry(
        &state.vault,
        Arc::new(AppEnv(app)),
        conversation_id,
        context,
        sink,
    )?;
    tauri::async_runtime::spawn(task);
    Ok(())
}

/// Stores a result the frontend produced (`read_terminal`, `send_input`, a rejection).
#[tauri::command]
#[specta::specta]
pub async fn ai_tool_result(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    tool_call_id: String,
    result: AiToolResultInput,
) -> AppResult<AiEntryView> {
    state.ai.tool_result(
        &state.vault,
        &AppEnv(app),
        &conversation_id,
        &tool_call_id,
        &result,
    )
}

/// The change an open `edit_file` or `write_file` call would make to the file on `session_id`'s
/// host, for its approval card (AI-39, AI-40), with the arguments as the user edited them on the
/// card, if they did.
#[tauri::command]
#[specta::specta]
pub async fn ai_file_preview(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    tool_call_id: String,
    session_id: Option<String>,
    edited_arguments: Option<String>,
) -> AppResult<AiFilePreview> {
    state
        .ai
        .file_preview(
            &state.vault,
            &AppEnv(app),
            &conversation_id,
            &tool_call_id,
            session_id.as_deref(),
            edited_arguments.as_deref(),
        )
        .await
}

/// Runs a tool that runs in Rust (`run_command` and the file tools on `session_id`,
/// `web_search`, `fetch_url`, `read_skill`, MCP tools) and stores its result.
#[tauri::command]
#[specta::specta]
pub async fn ai_tool_run(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    tool_call_id: String,
    session_id: Option<String>,
    edited_arguments: Option<String>,
) -> AppResult<AiEntryView> {
    state
        .ai
        .tool_run(
            &state.vault,
            &AppEnv(app),
            &conversation_id,
            &tool_call_id,
            session_id.as_deref(),
            edited_arguments.as_deref(),
        )
        .await
}

/// Stops the turn: aborts the request and running tools, cancels calls without a result.
#[tauri::command]
#[specta::specta]
pub async fn ai_stop(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
) -> AppResult<()> {
    state.ai.stop(&state.vault, &AppEnv(app), &conversation_id);
    Ok(())
}

/// AI-21: summarizes the context with the given model and moves `context_start` to the summary.
#[tauri::command]
#[specta::specta]
pub async fn ai_compact(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    context: AiTurnContext,
) -> AppResult<AiEntryView> {
    state
        .ai
        .compact(&state.vault, &AppEnv(app), &conversation_id, &context)
        .await
}

/// AI-24: conversations whose title or message text contains `query`, ignoring case.
#[tauri::command]
#[specta::specta]
pub async fn ai_search(state: State<'_, AppState>, query: String) -> AppResult<Vec<AiSearchHit>> {
    let vault = state.vault.clone();
    blocking(move || crate::ai::search(&vault, &query)).await
}

/// AI-26: replaces the user's message `entry_id` with `text`, deletes every entry after it, and
/// starts a turn whose events stream on `channel`, like `ai_send`.
#[tauri::command]
#[specta::specta]
pub async fn ai_edit_resend(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    entry_id: String,
    text: String,
    context: AiTurnContext,
    channel: Channel<AiTurnEvent>,
) -> AppResult<AiSendStarted> {
    let sink: Arc<dyn EventSink> = Arc::new(channel);
    let (started, task) = state
        .ai
        .edit_resend(
            &state.vault,
            Arc::new(AppEnv(app)),
            conversation_id,
            &entry_id,
            text,
            context,
            sink,
        )
        .await?;
    tauri::async_runtime::spawn(task);
    Ok(started)
}

/// AI-35: text files dropped on the panel in the desktop app, whose webview hands the WebView
/// only their paths. Reads only paths of the window's last drop, each once
/// (`crate::dropped`); with any other path, nothing is read. One result per path, in order,
/// naming the file by its base name. The per-message total stays the panel's to check.
#[tauri::command]
#[specta::specta]
pub async fn ai_read_dropped_files(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> AppResult<Vec<DroppedFile>> {
    state.with_unlocked(|_| Ok(()))?;
    let paths = state.dropped.take(&paths).ok_or_else(|| {
        AppError::invalid("paths", "Only files dropped on the window can be read.")
    })?;
    let files: Vec<DroppedFile> =
        blocking(move || Ok(paths.iter().map(|p| dropped::read(p)).collect())).await?;
    tracing::info!(files = files.len(), "dropped files read");
    Ok(files)
}

#[cfg(test)]
mod tests {
    use hatoba_core::KdfParams;
    use hatoba_core::model::AiEffort as CoreEffort;

    use super::*;
    use crate::dto::AiEffort;

    const KEY: &str = "sk-test-NEVER-SHOWN-4242";

    fn vault() -> Vault {
        let mut vault = Vault::open_in_memory().unwrap();
        vault
            .create_with_params("correct horse battery staple", KdfParams::for_tests())
            .unwrap();
        vault
    }

    fn provider_input(id: Option<String>, api_key: Option<&str>) -> AiProviderInput {
        AiProviderInput {
            id,
            name: " Local ".into(),
            protocol: AiProtocol::Anthropic,
            base_url: "https://api.example.com".into(),
            api_key: api_key.map(str::to_owned),
            auth_header: AiAuthHeader::Authorization,
            models: vec![AiModel {
                id: " m1 ".into(),
                name: String::new(),
                context_window: Some(200_000),
                max_output_tokens: Some(0),
                efforts: Some(vec![AiEffort::Max, AiEffort::Low, AiEffort::Max]),
                adaptive_thinking: Some(true),
            }],
        }
    }

    #[test]
    fn provider_views_and_input_debug_never_show_the_key() {
        let mut v = vault();
        let input = provider_input(None, Some(KEY));
        assert!(!format!("{input:?}").contains(KEY));
        let view = save_provider(&mut v, &input, input.base_url.clone()).unwrap();
        assert!(view.has_api_key);
        assert_eq!(view.name, "Local");
        assert_eq!(view.models[0].id, "m1");
        assert_eq!(view.models[0].name, "m1");
        assert_eq!(view.models[0].max_output_tokens, None);
        // AI-05: levels are kept lowest first, each once, with the adaptive thinking flag.
        assert_eq!(
            view.models[0].efforts,
            Some(vec![AiEffort::Low, AiEffort::Max])
        );
        assert_eq!(view.models[0].adaptive_thinking, Some(true));
        assert!(!serde_json::to_string(&view).unwrap().contains(KEY));

        // `None` keeps the saved key, and tests with it.
        let keep = provider_input(Some(view.id.clone()), None);
        assert!(!format!("{keep:?}").contains(KEY));
        let config = provider_config_for(&v, &keep).unwrap();
        assert_eq!(config.api_key.as_str(), KEY);
        assert!(!format!("{config:?}").contains(KEY));
        let kept = save_provider(&mut v, &keep, keep.base_url.clone()).unwrap();
        assert!(kept.has_api_key);
        let listed = v.ai_providers();
        assert_eq!(listed[0].1.api_key.as_str(), KEY);

        // "" clears it.
        let clear = provider_input(Some(view.id.clone()), Some(""));
        assert!(
            !save_provider(&mut v, &clear, clear.base_url.clone())
                .unwrap()
                .has_api_key
        );
    }

    #[test]
    fn provider_models_must_have_unique_ids() {
        let mut v = vault();
        let mut input = provider_input(None, None);
        input.models.push(AiModel {
            id: "m1".into(),
            name: "again".into(),
            context_window: None,
            max_output_tokens: None,
            efforts: None,
            adaptive_thinking: None,
        });
        let err = save_provider(&mut v, &input, input.base_url.clone()).unwrap_err();
        assert_eq!(err.field.as_deref(), Some("models"));
        input.models = vec![AiModel {
            id: "  ".into(),
            name: String::new(),
            context_window: None,
            max_output_tokens: None,
            efforts: None,
            adaptive_thinking: None,
        }];
        let err = save_provider(&mut v, &input, input.base_url.clone()).unwrap_err();
        assert_eq!(err.field.as_deref(), Some("models"));
        input.models.clear();
        input.name = " ".into();
        let err = save_provider(&mut v, &input, input.base_url.clone()).unwrap_err();
        assert_eq!(err.field.as_deref(), Some("name"));
    }

    #[test]
    fn search_views_and_input_debug_never_show_the_key() {
        let mut v = vault();
        let input = SearchProviderInput {
            id: None,
            kind: SearchKind::Brave,
            base_url: Some("https://ignored.example".into()),
            api_key: Some(KEY.into()),
        };
        assert!(!format!("{input:?}").contains(KEY));
        let view = save_search_provider(&mut v, &input, None).unwrap();
        assert!(view.has_api_key && view.base_url.is_none());
        assert!(!serde_json::to_string(&view).unwrap().contains(KEY));

        let keep = SearchProviderInput {
            id: Some(view.id.clone()),
            kind: SearchKind::Brave,
            base_url: None,
            api_key: None,
        };
        assert_eq!(search_config_for(&v, &keep).unwrap().api_key.as_str(), KEY);
        // Switching to another service does not reuse the key.
        let switched = |kind| SearchProviderInput {
            id: Some(view.id.clone()),
            kind,
            base_url: None,
            api_key: None,
        };
        let err = save_search_provider(&mut v, &switched(SearchKind::Tavily), None).unwrap_err();
        assert_eq!(err.field.as_deref(), Some("api_key"));
        let view = save_search_provider(
            &mut v,
            &switched(SearchKind::Searxng),
            Some("http://127.0.0.1:8888".into()),
        )
        .unwrap();
        assert!(!view.has_api_key);
        assert_eq!(view.base_url.as_deref(), Some("http://127.0.0.1:8888"));
        let stored = v.search_providers();
        assert_eq!(stored[0].1.kind, CoreSearchKind::Searxng);
        assert!(stored[0].1.api_key.is_empty());
    }

    #[test]
    fn deleting_providers_clears_the_settings_that_named_them() {
        let mut v = vault();
        let input = provider_input(None, Some(KEY));
        let provider = save_provider(&mut v, &input, input.base_url.clone()).unwrap();
        let search = save_search_provider(
            &mut v,
            &SearchProviderInput {
                id: None,
                kind: SearchKind::Tavily,
                base_url: None,
                api_key: Some(KEY.into()),
            },
            None,
        )
        .unwrap();
        save_ai_settings(
            &mut v,
            &AiSettingsView {
                default_model: Some(AiModelRef {
                    provider_id: provider.id.clone(),
                    model_id: "m1".into(),
                }),
                default_effort: None,
                search_provider_id: Some(search.id.clone()),
                builtin_skill_enabled: true,
                custom_instructions: String::new(),
            },
        )
        .unwrap();
        assert_eq!(
            v.settings().ai.default_model,
            Some(CoreRef {
                provider_id: provider.id.clone(),
                model_id: "m1".into()
            })
        );

        // AI-34: the built-in skill's switch is stored with them.
        assert!(ai_settings_view(&v.settings().ai).builtin_skill_enabled);
        let off = AiSettingsView {
            builtin_skill_enabled: false,
            ..ai_settings_view(&v.settings().ai)
        };
        save_ai_settings(&mut v, &off).unwrap();
        assert!(!v.settings().ai.builtin_skill_enabled);
        assert!(v.settings().ai.default_model.is_some());

        // AI-05: the default thinking level is stored with them, and Default is `None`.
        let level = AiSettingsView {
            default_effort: Some(AiEffort::Xhigh),
            ..ai_settings_view(&v.settings().ai)
        };
        save_ai_settings(&mut v, &level).unwrap();
        assert_eq!(v.settings().ai.default_effort, Some(CoreEffort::Xhigh));
        assert_eq!(
            ai_settings_view(&v.settings().ai).default_effort,
            Some(AiEffort::Xhigh)
        );
        let default = AiSettingsView {
            default_effort: None,
            ..ai_settings_view(&v.settings().ai)
        };
        save_ai_settings(&mut v, &default).unwrap();
        assert_eq!(v.settings().ai.default_effort, None);

        // AI-36: custom instructions are stored with them, up to 4,000 characters.
        let instructions = AiSettingsView {
            custom_instructions: "Answer in English.\n<host> means the tab's host.".into(),
            ..ai_settings_view(&v.settings().ai)
        };
        save_ai_settings(&mut v, &instructions).unwrap();
        assert_eq!(
            v.settings().ai.custom_instructions,
            "Answer in English.\n<host> means the tab's host."
        );
        assert_eq!(ai_settings_view(&v.settings().ai), instructions);
        let longest = AiSettingsView {
            custom_instructions: "日".repeat(4_000),
            ..ai_settings_view(&v.settings().ai)
        };
        save_ai_settings(&mut v, &longest).unwrap();
        let too_long = AiSettingsView {
            custom_instructions: "日".repeat(4_001),
            ..ai_settings_view(&v.settings().ai)
        };
        let err = save_ai_settings(&mut v, &too_long).unwrap_err();
        assert_eq!(err.field.as_deref(), Some("custom_instructions"));
        assert_eq!(v.settings().ai.custom_instructions, "日".repeat(4_000));

        delete_search_provider(&mut v, &search.id).unwrap();
        assert_eq!(v.settings().ai.search_provider_id, None);
        assert!(v.settings().ai.default_model.is_some());
        delete_provider(&mut v, &provider.id).unwrap();
        assert_eq!(v.settings().ai.default_model, None);
    }

    #[test]
    fn ai_settings_reject_new_references_to_nothing_but_keep_dangling_ones() {
        let mut v = vault();
        let err = save_ai_settings(
            &mut v,
            &AiSettingsView {
                default_model: Some(AiModelRef {
                    provider_id: "0190a0a0-0000-7000-8000-000000000000".into(),
                    model_id: "m1".into(),
                }),
                default_effort: None,
                search_provider_id: None,
                builtin_skill_enabled: true,
                custom_instructions: String::new(),
            },
        )
        .unwrap_err();
        assert_eq!(err.field.as_deref(), Some("default_model"));
        let input = provider_input(None, None);
        let provider = save_provider(&mut v, &input, input.base_url.clone()).unwrap();
        let wrong_model = AiSettingsView {
            default_model: Some(AiModelRef {
                provider_id: provider.id.clone(),
                model_id: "m2".into(),
            }),
            default_effort: None,
            search_provider_id: None,
            builtin_skill_enabled: true,
            custom_instructions: String::new(),
        };
        assert!(save_ai_settings(&mut v, &wrong_model).is_err());
        let err = save_ai_settings(
            &mut v,
            &AiSettingsView {
                default_model: None,
                default_effort: None,
                search_provider_id: Some("nope".into()),
                builtin_skill_enabled: true,
                custom_instructions: String::new(),
            },
        )
        .unwrap_err();
        assert_eq!(err.field.as_deref(), Some("search_provider_id"));

        // A reference that synced in before its item is kept when another field changes.
        let mut settings = v.settings();
        settings.ai.default_model = Some(CoreRef {
            provider_id: "elsewhere".into(),
            model_id: "m9".into(),
        });
        v.put(Some(SETTINGS_ID), Item::Settings(settings)).unwrap();
        let dangling = AiSettingsView {
            default_model: Some(AiModelRef {
                provider_id: "elsewhere".into(),
                model_id: "m9".into(),
            }),
            default_effort: None,
            search_provider_id: None,
            builtin_skill_enabled: true,
            custom_instructions: String::new(),
        };
        save_ai_settings(&mut v, &dangling).unwrap();
    }

    #[test]
    fn conversations_rename_within_limits_and_pin() {
        let mut v = vault();
        let id = v
            .put(
                None,
                Item::AiConversation(AiConversation {
                    title: "first".into(),
                    ..AiConversation::default()
                }),
            )
            .unwrap();
        assert_eq!(
            rename_conversation(&mut v, &id, "  disk usage  ")
                .unwrap()
                .title,
            "disk usage"
        );
        assert_eq!(
            rename_conversation(&mut v, &id, "   ")
                .unwrap_err()
                .field
                .as_deref(),
            Some("title")
        );
        assert!(rename_conversation(&mut v, &id, &"x".repeat(201)).is_err());
        assert!(rename_conversation(&mut v, &id, &"字".repeat(200)).is_ok());
        assert!(
            update_conversation(&mut v, &id, |c| c.pinned = true)
                .unwrap()
                .pinned
        );
        assert_eq!(
            rename_conversation(&mut v, "0190a0a0-0000-7000-8000-000000000000", "t")
                .unwrap_err()
                .code,
            crate::error::ErrorCode::NotFound
        );
    }
}
