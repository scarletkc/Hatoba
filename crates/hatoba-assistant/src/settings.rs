//! Settings → AI (spec §13): what `ai_settings_save` writes to the synced `Settings.ai`.

use hatoba_core::Vault;
use hatoba_core::model::{
    AI_TEXT_SIZE_MAX, AI_TEXT_SIZE_MIN, AiModelRef as CoreRef, AiProtocol as CoreProtocol,
    AiSettings, Item, MAX_CUSTOM_INSTRUCTIONS_BYTES, MAX_ITEM_PLAINTEXT_BYTES, SETTINGS_ID,
    SearchProvider,
};

use crate::ai::core_effort;
use crate::dto::{AiModelRef, AiSettingsPatch, Clearable};
use crate::error::{AppError, AppResult};

pub fn find_search_provider(v: &Vault, id: &str) -> AppResult<SearchProvider> {
    v.get(id)
        .and_then(Item::as_search_provider)
        .cloned()
        .ok_or_else(|| AppError::not_found("search provider"))
}

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
pub fn save_ai_settings(v: &mut Vault, patch: &AiSettingsPatch) -> AppResult<bool> {
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
        memory: patch.memory.unwrap_or(current.memory),
        text_size: patch.text_size.map_or(current.text_size, |size| {
            size.clamp(AI_TEXT_SIZE_MIN, AI_TEXT_SIZE_MAX)
        }),
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
