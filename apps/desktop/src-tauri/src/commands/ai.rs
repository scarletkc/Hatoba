//! AI assistant commands (spec §13): providers, search providers and the synced settings of
//! Settings → AI, conversations with history search (AI-24), and the turn protocol of §13.1 with
//! edit and resend (AI-26) and side questions (AI-42), which run in [`crate::ai`]. Skills and MCP
//! servers have their own modules ([`super::skills`], [`super::mcp`]).
//! API keys go one way to Rust: views say only whether a key is saved (AI-01).

use std::sync::Arc;

use hatoba_ai::AiError;
use hatoba_ai::models::{TestFailure, TestOutcome};
use hatoba_ai::provider::{ProviderConfig, validate_base_url};
use hatoba_ai::web::SearchConfig;
use hatoba_core::model::{
    AiAuthHeader as CoreAuthHeader, AiConversation, AiModel as CoreModel, AiModelRef as CoreRef,
    AiProtocol as CoreProtocol, AiProvider, AiSettings, Item, MAX_CUSTOM_INSTRUCTIONS_BYTES,
    MAX_ITEM_PLAINTEXT_BYTES, SETTINGS_ID, SearchKind as CoreSearchKind, SearchProvider,
};
use hatoba_core::{SnapshotKey, Vault};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

use crate::ai::{
    AiEnv, EventSink, MAX_TITLE_CHARS, auth_header, conversation_view, conversation_view_at,
    core_effort, effort_view, find_conversation, listed_effort, protocol, search_kind,
};
use crate::dto::{
    AiAuthHeader, AiAutoVerdict, AiConversationDetail, AiConversationView, AiEntryView,
    AiFileChange, AiFilePreview, AiFileRevert, AiHostFile, AiHostListing, AiJudgeSampleOutcome,
    AiModel, AiModelRef, AiProtocol, AiProviderInput, AiProviderView, AiSearchHit, AiSendInput,
    AiSendStarted, AiSettingsPatch, AiSettingsView, AiSideAnswer, AiSideEvent, AiSideQuestionInput,
    AiSkillCommand, AiTestFailure, AiTestResult, AiToolResultInput, AiToolRun, AiTurnContext,
    AiTurnEvent, Clearable, DroppedFile, SearchKind, SearchProviderInput, SearchProviderView,
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
        AiProtocol::SystemOne => CoreProtocol::SystemOne,
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
            CoreProtocol::SystemOne => AiProtocol::SystemOne,
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

fn model_ref_view(m: &CoreRef) -> AiModelRef {
    AiModelRef {
        provider_id: m.provider_id.clone(),
        model_id: m.model_id.clone(),
    }
}

fn ai_settings_view(s: &AiSettings) -> AiSettingsView {
    AiSettingsView {
        default_model: s.default_model.as_ref().map(model_ref_view),
        default_effort: s.default_effort.map(effort_view),
        search_provider_id: s.search_provider_id.clone(),
        native_search: s.native_search,
        builtin_skill_enabled: s.builtin_skill_enabled,
        custom_instructions: s.custom_instructions.clone(),
        send_terminal_screen: s.send_terminal_screen,
        judge_model: s.judge_model.as_ref().map(model_ref_view),
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
fn checked_base_url(base_url: &str) -> AppResult<String> {
    let base_url = base_url.trim();
    if base_url.is_empty() {
        return Err(AppError::invalid("base_url", "the base URL is required"));
    }
    validate_base_url(base_url).map_err(|e| AppError::invalid("base_url", e.to_string()))?;
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

/// Deletes a provider, and the default model and the judge of Auto mode (AI-47) that pointed at
/// it.
pub(crate) fn delete_provider(v: &mut Vault, id: &str) -> AppResult<()> {
    find_provider(v, id)?;
    v.delete(id)?;
    let mut settings = v.settings();
    let mut changed = false;
    for chosen in [&mut settings.ai.default_model, &mut settings.ai.judge_model] {
        if chosen.as_ref().is_some_and(|m| m.provider_id == id) {
            *chosen = None;
            changed = true;
        }
    }
    if changed {
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
    let base_url = checked_base_url(&input.base_url)?;
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
        SearchKind::Searxng => Some(checked_base_url(
            input.base_url.as_deref().unwrap_or_default(),
        )?),
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

/// The model reference the settings field `field` holds after `patch`, from `current`: one the
/// user changed must name an existing provider that lists the model, and with `chats` a provider
/// that chats, not a System One one (AI-47); an unchanged one is kept as it is, even when it
/// dangles.
fn patched_model(
    v: &Vault,
    field: &str,
    current: Option<&CoreRef>,
    patch: &Clearable<AiModelRef>,
    chats: bool,
) -> AppResult<Option<CoreRef>> {
    let m = match patch {
        Clearable::Keep => return Ok(current.cloned()),
        Clearable::Clear => return Ok(None),
        Clearable::Set(m) => m,
    };
    let unchanged =
        current.is_some_and(|c| c.provider_id == m.provider_id && c.model_id == m.model_id);
    if !unchanged {
        let provider = v
            .get(&m.provider_id)
            .and_then(Item::as_ai_provider)
            .ok_or_else(|| AppError::invalid(field, "provider not found"))?;
        if !provider.models.iter().any(|x| x.id == m.model_id) {
            return Err(AppError::invalid(field, "the provider has no such model"));
        }
        if chats && provider.protocol == CoreProtocol::SystemOne {
            return Err(AppError::invalid(field, "a System One model cannot chat"));
        }
    }
    Ok(Some(CoreRef {
        provider_id: m.provider_id.clone(),
        model_id: m.model_id.clone(),
    }))
}

/// Writes the fields `patch` sets over the stored `Settings.ai`, as `settings_save` does for the
/// terminal, and returns whether that changed them. A reference the user changed must point at an
/// existing provider and model or search provider; an unchanged one is kept as it is, even when it
/// dangles (e.g. the item it names has not synced to this device yet). The default model must be
/// one that chats; the judge of Auto mode (AI-47) may be a model of any provider, a System One one
/// included.
pub(crate) fn save_ai_settings(v: &mut Vault, patch: &AiSettingsPatch) -> AppResult<bool> {
    let mut settings = v.settings();
    let current = &settings.ai;
    let default_model = patched_model(
        v,
        "default_model",
        current.default_model.as_ref(),
        &patch.default_model,
        true,
    )?;
    let judge_model = patched_model(
        v,
        "judge_model",
        current.judge_model.as_ref(),
        &patch.judge_model,
        false,
    )?;
    let search_provider_id = match &patch.search_provider_id {
        Clearable::Keep => current.search_provider_id.clone(),
        Clearable::Clear => None,
        Clearable::Set(id) => {
            if current.search_provider_id.as_ref() != Some(id) {
                find_search_provider(v, id).map_err(|_| {
                    AppError::invalid("search_provider_id", "search provider not found")
                })?;
            }
            Some(id.clone())
        }
    };
    if patch
        .custom_instructions
        .as_ref()
        .is_some_and(|text| text.len() > MAX_CUSTOM_INSTRUCTIONS_BYTES)
    {
        return Err(AppError::invalid(
            "custom_instructions",
            "custom instructions are limited to 32 KB",
        ));
    }
    let ai = AiSettings {
        default_model,
        default_effort: match patch.default_effort {
            Clearable::Keep => current.default_effort,
            Clearable::Clear => None,
            Clearable::Set(effort) => Some(core_effort(effort)),
        },
        search_provider_id,
        native_search: patch.native_search.unwrap_or(current.native_search),
        builtin_skill_enabled: patch
            .builtin_skill_enabled
            .unwrap_or(current.builtin_skill_enabled),
        custom_instructions: patch
            .custom_instructions
            .clone()
            .unwrap_or_else(|| current.custom_instructions.clone()),
        send_terminal_screen: patch
            .send_terminal_screen
            .unwrap_or(current.send_terminal_screen),
        judge_model,
    };
    if ai == settings.ai {
        return Ok(false);
    }
    settings.ai = ai;
    let item = Item::Settings(settings);
    // With sync on, the Worker and D1 would refuse a larger settings item on every sync (§6.2);
    // only instructions whose JSON is far longer than their text (many quotes) get there.
    if v.sync_config()?.is_some() && item.plaintext_len()? > MAX_ITEM_PLAINTEXT_BYTES {
        return Err(AppError::invalid(
            "custom_instructions",
            "the custom instructions are too large to sync",
        ));
    }
    v.put(Some(SETTINGS_ID), item)?;
    Ok(true)
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
    patch: AiSettingsPatch,
) -> AppResult<()> {
    if state.with_unlocked(|v| save_ai_settings(v, &patch))? {
        sync::local_change(&app);
    }
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
    if !(1..=MAX_TITLE_CHARS).contains(&title.chars().count()) {
        return Err(AppError::invalid(
            "title",
            format!("the title must be 1 to {MAX_TITLE_CHARS} characters"),
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

/// AI-43: copies the conversation up to the entry `entry_id`, or all of it without one, into a
/// new conversation titled `title`, and returns the new conversation.
#[tauri::command]
#[specta::specta]
pub async fn ai_conversation_fork(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    entry_id: Option<String>,
    title: String,
) -> AppResult<AiConversationView> {
    let (ai, vault) = (state.ai.clone(), state.vault.clone());
    blocking(move || ai.fork_conversation(&vault, &AppEnv(app), &id, entry_id.as_deref(), &title))
        .await
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
    state
        .ai
        .tool_result(
            &state.vault,
            &AppEnv(app),
            &conversation_id,
            &tool_call_id,
            &result,
        )
        .await
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

/// The changes of the conversation's `edit_file` and `write_file` calls that this device keeps
/// and can revert, or has reverted (AI-41).
#[tauri::command]
#[specta::specta]
pub async fn ai_file_changes(
    state: State<'_, AppState>,
    conversation_id: String,
) -> AppResult<Vec<AiFileChange>> {
    state.ai.file_changes(&state.vault, &conversation_id)
}

/// What an `edit_file` or `write_file` call changed (AI-41), from its snapshot, for its block and
/// the revert's confirmation: the file before the call and as the call wrote it. `None` once the
/// change was reverted or dropped.
#[tauri::command]
#[specta::specta]
pub async fn ai_file_change_diff(
    state: State<'_, AppState>,
    conversation_id: String,
    response_id: String,
    tool_call_id: String,
) -> AppResult<Option<AiFilePreview>> {
    state.ai.file_change_diff(
        &state.vault,
        SnapshotKey {
            conversation_id: &conversation_id,
            response_id: &response_id,
            tool_call_id: &tool_call_id,
        },
    )
}

/// The server a revert (AI-41) on the tab `session_id` runs on, as `AiFileChange.server` names it;
/// null while the tab is not connected. Edit and resend asks before it offers to revert changes.
#[tauri::command]
#[specta::specta]
pub fn ai_session_server(app: AppHandle, session_id: Option<String>) -> Option<String> {
    crate::ai::session_server(&AppEnv(app), session_id.as_deref())
}

/// Reverts the change an `edit_file` or `write_file` call made (AI-41), on the conversation's tab
/// `session_id`. A file the call created is deleted, so the panel asks first. `restore`: Restore
/// anyway, over what the diff of the last revert showed, which the panel also asks about first.
#[tauri::command]
#[specta::specta]
pub async fn ai_file_revert(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    response_id: String,
    tool_call_id: String,
    session_id: Option<String>,
    restore: bool,
) -> AppResult<AiFileRevert> {
    state
        .ai
        .file_revert(
            &state.vault,
            &AppEnv(app),
            SnapshotKey {
                conversation_id: &conversation_id,
                response_id: &response_id,
                tool_call_id: &tool_call_id,
            },
            session_id.as_deref(),
            restore,
        )
        .await
}

/// Runs a tool that runs in Rust (`run_command` and the file tools on `session_id`,
/// `web_search`, `fetch_url`, `read_skill`, MCP tools) and stores its result. It also says whether
/// keeping the snapshot of a file change dropped older ones (AI-41). `judged`: Auto decided the
/// call (it runs on the judge's verdict, or its approval card showed one), so the result keeps the
/// verdict (AI-47).
#[tauri::command]
#[specta::specta]
pub async fn ai_tool_run(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    tool_call_id: String,
    session_id: Option<String>,
    edited_arguments: Option<String>,
    judged: bool,
) -> AppResult<AiToolRun> {
    state
        .ai
        .tool_run(
            &state.vault,
            &AppEnv(app),
            &conversation_id,
            &tool_call_id,
            session_id.as_deref(),
            edited_arguments.as_deref(),
            judged,
        )
        .await
}

/// AI-47: asks the judge of Auto mode about an open call that manual approval would ask about,
/// and returns its verdict: run the call without the approval card, or ask. `screen_tail` is the
/// screen's last lines, which a `send_input` call shows the judge. Never fails because the judge
/// does: without a judge, or when it fails, the verdict asks and says why.
#[tauri::command]
#[specta::specta]
pub async fn ai_judge(
    state: State<'_, AppState>,
    conversation_id: String,
    tool_call_id: String,
    screen_tail: Option<String>,
) -> AppResult<AiAutoVerdict> {
    state
        .ai
        .judge(
            &state.vault,
            &conversation_id,
            &tool_call_id,
            screen_tail.as_deref(),
        )
        .await
}

/// AI-47: **Test** beside the judge in Settings → AI. Judges sample calls with `judge`, which
/// need not be saved yet, or with the saved judge when it is `None`, and returns each verdict
/// with the decision expected. Fails only when the judge does not name a listed model.
#[tauri::command]
#[specta::specta]
pub async fn ai_judge_test(
    state: State<'_, AppState>,
    judge: Option<AiModelRef>,
) -> AppResult<Vec<AiJudgeSampleOutcome>> {
    state.ai.judge_test(&state.vault, judge).await
}

/// Stops the turn: aborts the request and running tools, cancels calls without a result.
#[tauri::command]
#[specta::specta]
pub async fn ai_stop(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
) -> AppResult<()> {
    state
        .ai
        .stop(&state.vault, &AppEnv(app), &conversation_id)
        .await;
    Ok(())
}

/// AI-21: summarizes the context with the given model and moves `context_start` to the summary.
/// `instructions`: what `/compact <instructions>` asks of the summary (AI-50).
#[tauri::command]
#[specta::specta]
pub async fn ai_compact(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    context: AiTurnContext,
    instructions: Option<String>,
) -> AppResult<AiEntryView> {
    state
        .ai
        .compact(
            &state.vault,
            &AppEnv(app),
            &conversation_id,
            &context,
            instructions.as_deref(),
        )
        .await
}

/// AI-50: the skills the panel's `/` menu offers as commands.
#[tauri::command]
#[specta::specta]
pub async fn ai_skill_commands(state: State<'_, AppState>) -> AppResult<Vec<AiSkillCommand>> {
    state.with_unlocked(|v| Ok(crate::ai::skill_commands(v)))
}

/// AI-50: the `SKILL.md` of the enabled skill `name`, which a `/<skill>` command attaches.
#[tauri::command]
#[specta::specta]
pub async fn ai_skill_text(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> AppResult<String> {
    let version = AppEnv(app).app_version();
    state.with_unlocked(|v| crate::ai::skill_text(v, &name, &version))
}

/// AI-51: the entries of the directory `path` on the host of the tab's session `session_id`, for
/// the `@` menu: to complete a path, and to attach a folder.
#[tauri::command]
#[specta::specta]
pub async fn ai_host_list(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    path: String,
) -> AppResult<AiHostListing> {
    state.with_unlocked(|_| Ok(()))?;
    let session = AppEnv(app).session(&session_id);
    Ok(crate::ai::mentions::list(session, &path).await)
}

/// AI-51: the text file `path` on the host of the tab's session `session_id`, which an `@`
/// mention attaches.
#[tauri::command]
#[specta::specta]
pub async fn ai_host_read(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    path: String,
) -> AppResult<AiHostFile> {
    state.with_unlocked(|_| Ok(()))?;
    let session = AppEnv(app).session(&session_id);
    Ok(crate::ai::mentions::read(session, &path).await)
}

/// AI-42: asks a side question about the conversation without stopping its turn. The answer
/// streams on `channel` and comes back whole; nothing is stored.
#[tauri::command]
#[specta::specta]
pub async fn ai_side_question(
    app: AppHandle,
    state: State<'_, AppState>,
    input: AiSideQuestionInput,
    channel: Channel<AiSideEvent>,
) -> AppResult<AiSideAnswer> {
    // A reloaded WebView drops its channels; the answer then has nowhere to go.
    let send = move |event: AiSideEvent| {
        let _ = channel.send(event);
    };
    state
        .ai
        .side_question(&state.vault, &AppEnv(app), input, &send)
        .await
}

/// AI-42: stops the conversation's side question `question_id`, if it is on its way; its turn
/// goes on, and so does a newer side question.
#[tauri::command]
#[specta::specta]
pub async fn ai_side_stop(
    state: State<'_, AppState>,
    conversation_id: String,
    question_id: u32,
) -> AppResult<()> {
    state.ai.side_stop(&conversation_id, question_id);
    Ok(())
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

    /// A change of `Settings.ai` to save.
    fn patch(change: impl FnOnce(&mut AiSettingsPatch)) -> AiSettingsPatch {
        let mut patch = AiSettingsPatch::default();
        change(&mut patch);
        patch
    }

    fn model(provider_id: &str, model_id: &str) -> AiModelRef {
        AiModelRef {
            provider_id: provider_id.into(),
            model_id: model_id.into(),
        }
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
        let chosen = patch(|p| {
            p.default_model = Clearable::Set(model(&provider.id, "m1"));
            p.search_provider_id = Clearable::Set(search.id.clone());
        });
        assert!(save_ai_settings(&mut v, &chosen).unwrap());
        assert_eq!(
            v.settings().ai.default_model,
            Some(CoreRef {
                provider_id: provider.id.clone(),
                model_id: "m1".into()
            })
        );

        // AI-34: the built-in skill's switch is stored with them.
        assert!(ai_settings_view(&v.settings().ai).builtin_skill_enabled);
        let off = patch(|p| p.builtin_skill_enabled = Some(false));
        save_ai_settings(&mut v, &off).unwrap();
        assert!(!v.settings().ai.builtin_skill_enabled);
        assert!(v.settings().ai.default_model.is_some());

        // AI-46: so is the switch of the screen note.
        assert!(ai_settings_view(&v.settings().ai).send_terminal_screen);
        let quiet = patch(|p| p.send_terminal_screen = Some(false));
        save_ai_settings(&mut v, &quiet).unwrap();
        assert!(!ai_settings_view(&v.settings().ai).send_terminal_screen);
        assert!(!v.settings().ai.builtin_skill_enabled);

        // AI-14: and so is native search, on until turned off.
        assert!(ai_settings_view(&v.settings().ai).native_search);
        let no_search = patch(|p| p.native_search = Some(false));
        save_ai_settings(&mut v, &no_search).unwrap();
        assert!(!ai_settings_view(&v.settings().ai).native_search);
        assert_eq!(
            v.settings().ai.search_provider_id.as_ref(),
            Some(&search.id)
        );

        // AI-05: the default thinking level is stored with them, and Default is `None`.
        let level = patch(|p| p.default_effort = Clearable::Set(AiEffort::Xhigh));
        save_ai_settings(&mut v, &level).unwrap();
        assert_eq!(v.settings().ai.default_effort, Some(CoreEffort::Xhigh));
        assert_eq!(
            ai_settings_view(&v.settings().ai).default_effort,
            Some(AiEffort::Xhigh)
        );
        let default = patch(|p| p.default_effort = Clearable::Clear);
        save_ai_settings(&mut v, &default).unwrap();
        assert_eq!(v.settings().ai.default_effort, None);

        // AI-36: custom instructions are stored with them.
        let instructions = |text: &str| patch(|p| p.custom_instructions = Some(text.into()));
        let before = ai_settings_view(&v.settings().ai);
        save_ai_settings(
            &mut v,
            &instructions("Answer in English.\n<host> means the tab's host."),
        )
        .unwrap();
        assert_eq!(
            ai_settings_view(&v.settings().ai),
            AiSettingsView {
                custom_instructions: "Answer in English.\n<host> means the tab's host.".into(),
                ..before
            }
        );
        // Up to 32 KB of UTF-8.
        let text = format!("{}xy", "日".repeat(MAX_CUSTOM_INSTRUCTIONS_BYTES / 3));
        assert_eq!(text.len(), MAX_CUSTOM_INSTRUCTIONS_BYTES);
        save_ai_settings(&mut v, &instructions(&text)).unwrap();
        // Within the limit, but its JSON does not fit a synced item: refused only with sync on.
        let quotes = "\"".repeat(MAX_CUSTOM_INSTRUCTIONS_BYTES);
        save_ai_settings(&mut v, &instructions(&quotes)).unwrap();
        save_ai_settings(&mut v, &instructions(&text)).unwrap();
        let config = hatoba_core::sync::SyncConfig::Worker {
            url: "https://sync.example.workers.dev".into(),
            deployment: None,
        };
        v.set_sync_config(Some(&config)).unwrap();
        for refused in [format!("{text}z"), quotes] {
            let err = save_ai_settings(&mut v, &instructions(&refused)).unwrap_err();
            assert_eq!(err.field.as_deref(), Some("custom_instructions"));
            assert_eq!(v.settings().ai.custom_instructions, text);
        }
        v.set_sync_config(None).unwrap();

        delete_search_provider(&mut v, &search.id).unwrap();
        assert_eq!(v.settings().ai.search_provider_id, None);
        assert!(v.settings().ai.default_model.is_some());
        delete_provider(&mut v, &provider.id).unwrap();
        assert_eq!(v.settings().ai.default_model, None);
    }

    #[test]
    fn ai_settings_reject_new_references_to_nothing_but_keep_dangling_ones() {
        let mut v = vault();
        let missing = patch(|p| {
            p.default_model = Clearable::Set(model("0190a0a0-0000-7000-8000-000000000000", "m1"));
        });
        let err = save_ai_settings(&mut v, &missing).unwrap_err();
        assert_eq!(err.field.as_deref(), Some("default_model"));
        let input = provider_input(None, None);
        let provider = save_provider(&mut v, &input, input.base_url.clone()).unwrap();
        let wrong_model = patch(|p| p.default_model = Clearable::Set(model(&provider.id, "m2")));
        assert!(save_ai_settings(&mut v, &wrong_model).is_err());
        let nope = patch(|p| p.search_provider_id = Clearable::Set("nope".into()));
        let err = save_ai_settings(&mut v, &nope).unwrap_err();
        assert_eq!(err.field.as_deref(), Some("search_provider_id"));
        assert_eq!(v.settings().ai, AiSettings::default());

        // A reference that synced in before its item is kept when another field changes, and
        // can be sent back unchanged.
        let mut settings = v.settings();
        settings.ai.default_model = Some(CoreRef {
            provider_id: "elsewhere".into(),
            model_id: "m9".into(),
        });
        v.put(Some(SETTINGS_ID), Item::Settings(settings)).unwrap();
        let level = patch(|p| p.default_effort = Clearable::Set(AiEffort::High));
        assert!(save_ai_settings(&mut v, &level).unwrap());
        let dangling = patch(|p| p.default_model = Clearable::Set(model("elsewhere", "m9")));
        assert!(!save_ai_settings(&mut v, &dangling).unwrap());
        assert_eq!(
            v.settings().ai.default_model,
            Some(CoreRef {
                provider_id: "elsewhere".into(),
                model_id: "m9".into(),
            })
        );
    }

    #[test]
    fn a_save_from_a_stale_pane_keeps_the_fields_it_leaves_out() {
        let mut v = vault();
        let input = provider_input(None, None);
        let provider = save_provider(&mut v, &input, input.base_url.clone()).unwrap();
        let chosen = patch(|p| p.default_model = Clearable::Set(model(&provider.id, "m1")));
        save_ai_settings(&mut v, &chosen).unwrap();
        // Settings → AI reads the settings, then a sync round pulls another device's changes.
        let shown = ai_settings_view(&v.settings().ai);
        let mut pulled = v.settings();
        pulled.ai.custom_instructions = "Answer in Japanese.".into();
        pulled.ai.builtin_skill_enabled = false;
        pulled.ai.send_terminal_screen = false;
        // AI-47: a judge chosen on another device, which the pane's patch leaves out.
        pulled.ai.judge_model = Some(CoreRef {
            provider_id: "elsewhere".into(),
            model_id: "jev-latest".into(),
        });
        v.put(Some(SETTINGS_ID), Item::Settings(pulled)).unwrap();

        // The user picks a thinking level in that pane.
        assert_eq!(shown.default_effort, None);
        let level = patch(|p| p.default_effort = Clearable::Set(AiEffort::Low));
        assert!(save_ai_settings(&mut v, &level).unwrap());

        let ai = v.settings().ai;
        assert_eq!(ai.default_effort, Some(CoreEffort::Low));
        assert_eq!(ai.custom_instructions, "Answer in Japanese.");
        assert!(!ai.builtin_skill_enabled && !ai.send_terminal_screen);
        assert_eq!(ai.default_model.map(|m| m.model_id), Some("m1".into()));
        assert_eq!(
            ai.judge_model.map(|m| m.model_id),
            Some("jev-latest".into())
        );
    }

    #[test]
    fn a_patch_that_changes_nothing_writes_nothing() {
        let mut v = vault();
        let before = v.settings();
        let shown = ai_settings_view(&before.ai);
        let same = patch(|p| {
            p.default_effort = shown
                .default_effort
                .map_or(Clearable::Clear, Clearable::Set);
            p.search_provider_id = Clearable::Clear;
            p.builtin_skill_enabled = Some(shown.builtin_skill_enabled);
            p.custom_instructions = Some(shown.custom_instructions.clone());
            p.send_terminal_screen = Some(shown.send_terminal_screen);
        });
        assert!(!save_ai_settings(&mut v, &same).unwrap());
        assert!(!save_ai_settings(&mut v, &AiSettingsPatch::default()).unwrap());
        assert_eq!(v.settings(), before);
    }

    #[test]
    fn a_patch_tells_a_field_left_out_from_one_cleared() {
        let read =
            |json: serde_json::Value| -> AiSettingsPatch { serde_json::from_value(json).unwrap() };
        let empty = read(serde_json::json!({}));
        assert_eq!(empty.default_model, Clearable::Keep);
        assert_eq!(empty.default_effort, Clearable::Keep);
        assert_eq!(empty.search_provider_id, Clearable::Keep);
        assert!(empty.builtin_skill_enabled.is_none() && empty.custom_instructions.is_none());
        assert!(empty.native_search.is_none());
        assert_eq!(empty.judge_model, Clearable::Keep);
        let cleared = read(serde_json::json!({
            "default_model": null,
            "default_effort": null,
            "search_provider_id": null,
            "judge_model": null,
        }));
        assert_eq!(cleared.default_model, Clearable::Clear);
        assert_eq!(cleared.default_effort, Clearable::Clear);
        assert_eq!(cleared.search_provider_id, Clearable::Clear);
        assert_eq!(cleared.judge_model, Clearable::Clear);
        let set = read(serde_json::json!({
            "default_model": {"provider_id": "p", "model_id": "m"},
            "default_effort": "high",
            "builtin_skill_enabled": false,
            "judge_model": {"provider_id": "j", "model_id": "jev-latest"},
        }));
        assert_eq!(set.default_model, Clearable::Set(model("p", "m")));
        assert_eq!(set.default_effort, Clearable::Set(AiEffort::High));
        assert_eq!(set.builtin_skill_enabled, Some(false));
        assert_eq!(set.search_provider_id, Clearable::Keep);
        assert_eq!(set.judge_model, Clearable::Set(model("j", "jev-latest")));
    }

    #[test]
    fn the_judge_must_name_a_listed_model_and_goes_with_its_provider() {
        let mut v = vault();
        let input = provider_input(None, Some(KEY));
        let chat = save_provider(&mut v, &input, input.base_url.clone()).unwrap();
        let mut jev = provider_input(None, Some(KEY));
        jev.name = "Jev".into();
        jev.protocol = AiProtocol::SystemOne;
        jev.base_url = "https://api.typesafe.ai/v1".into();
        jev.models = vec![AiModel {
            id: "jev-latest".into(),
            name: String::new(),
            context_window: None,
            max_output_tokens: None,
            efforts: None,
            adaptive_thinking: None,
        }];
        let jev = save_provider(&mut v, &jev, jev.base_url.clone()).unwrap();

        // AI-47: a judge the user picks must exist, and its provider must list it.
        for (provider_id, model_id) in [("nope", "jev-latest"), (jev.id.as_str(), "jev-9")] {
            let missing = patch(|p| p.judge_model = Clearable::Set(model(provider_id, model_id)));
            let err = save_ai_settings(&mut v, &missing).unwrap_err();
            assert_eq!(err.field.as_deref(), Some("judge_model"));
        }
        assert_eq!(v.settings().ai.judge_model, None);
        // A System One model cannot chat, so it is no default model.
        let default = patch(|p| p.default_model = Clearable::Set(model(&jev.id, "jev-latest")));
        let err = save_ai_settings(&mut v, &default).unwrap_err();
        assert_eq!(err.field.as_deref(), Some("default_model"));
        assert_eq!(v.settings().ai.default_model, None);
        // A System One model, or any provider's.
        for (provider_id, model_id) in [(jev.id.as_str(), "jev-latest"), (chat.id.as_str(), "m1")] {
            let chosen = patch(|p| p.judge_model = Clearable::Set(model(provider_id, model_id)));
            assert!(save_ai_settings(&mut v, &chosen).unwrap());
            assert_eq!(
                ai_settings_view(&v.settings().ai).judge_model,
                Some(model(provider_id, model_id))
            );
        }
        // A patch without it keeps it; `null` clears it.
        let level = patch(|p| p.default_effort = Clearable::Set(AiEffort::Low));
        assert!(save_ai_settings(&mut v, &level).unwrap());
        assert!(v.settings().ai.judge_model.is_some());
        assert!(save_ai_settings(&mut v, &patch(|p| p.judge_model = Clearable::Clear)).unwrap());
        assert_eq!(v.settings().ai.judge_model, None);

        // Deleting a provider clears the judge that named it, and only that.
        let both = patch(|p| {
            p.default_model = Clearable::Set(model(&chat.id, "m1"));
            p.judge_model = Clearable::Set(model(&jev.id, "jev-latest"));
        });
        assert!(save_ai_settings(&mut v, &both).unwrap());
        delete_provider(&mut v, &jev.id).unwrap();
        let ai = v.settings().ai;
        assert_eq!(ai.judge_model, None);
        assert_eq!(
            ai.default_model.map(|m| m.provider_id),
            Some(chat.id.clone())
        );
        let same = patch(|p| p.judge_model = Clearable::Set(model(&chat.id, "m1")));
        assert!(save_ai_settings(&mut v, &same).unwrap());
        delete_provider(&mut v, &chat.id).unwrap();
        let ai = v.settings().ai;
        assert_eq!((ai.default_model, ai.judge_model), (None, None));
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
