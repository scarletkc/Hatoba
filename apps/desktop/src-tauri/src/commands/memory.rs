//! AI memory commands (AI-53, spec §13.12): Settings → AI lists, edits and deletes the memories
//! the assistant keeps, under the rules its `memory` tool follows ([`crate::ai::memory`]).

use hatoba_core::model::{AiMemory, Item};
use tauri::{AppHandle, State};

use crate::ai::memory;
use crate::dto::{AiMemoryInput, AiMemoryView};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::sync;

fn view(id: String, m: AiMemory) -> AiMemoryView {
    AiMemoryView {
        id,
        name: m.name,
        content: m.content,
        host_id: m.host_id,
        local: m.local,
        updated_at: m.updated_at,
    }
}

/// Every memory, oldest first, of every scope.
#[tauri::command]
#[specta::specta]
pub async fn ai_memories_list(state: State<'_, AppState>) -> AppResult<Vec<AiMemoryView>> {
    state.with_unlocked(|v| {
        Ok(v.ai_memories()
            .into_iter()
            .map(|(id, m)| view(id, m))
            .collect())
    })
}

/// Saves a memory in the scope the input names, a saved host's that exists (or the deleted host
/// the edited memory already belongs to) or a computer's: `id` replaces that one, which moves it
/// there when it was elsewhere, together with the others of its name there when the edit keeps
/// both ([`memory::edited`]); none adds one.
#[tauri::command]
#[specta::specta]
pub async fn ai_memory_save(
    app: AppHandle,
    state: State<'_, AppState>,
    input: AiMemoryInput,
) -> AppResult<AiMemoryView> {
    let saved = state.with_unlocked(|v| {
        let kept = |host_id: &str| {
            input
                .id
                .as_deref()
                .and_then(|id| v.get(id))
                .and_then(Item::as_ai_memory)
                .is_some_and(|m| m.host_id.as_deref() == Some(host_id))
        };
        if let Some(host_id) = &input.host_id
            && v.get(host_id).and_then(Item::as_host).is_none()
            && !kept(host_id)
        {
            return Err(AppError::not_found("host"));
        }
        let local = input.local.as_deref().filter(|d| !d.trim().is_empty());
        let scope = memory::Scope::new(input.host_id.as_deref(), local);
        let ids = match &input.id {
            Some(id) if v.get(id).and_then(Item::as_ai_memory).is_none() => {
                return Err(AppError::not_found("memory"));
            }
            Some(id) => memory::edited(v, id, &scope, &input.name),
            None => Vec::new(),
        };
        memory::save(v, &ids, &scope, &input.name, &input.content)?
            .map(|(id, m)| view(id, m))
            .map_err(|refusal| AppError::invalid("memory", refusal.message()))
    })?;
    sync::local_change(&app);
    Ok(saved)
}

/// Deletes a memory.
#[tauri::command]
#[specta::specta]
pub async fn ai_memory_delete(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    state.with_unlocked(|v| {
        if v.get(&id).and_then(Item::as_ai_memory).is_none() {
            return Err(AppError::not_found("memory"));
        }
        Ok(v.delete(&id)?)
    })?;
    sync::local_change(&app);
    Ok(())
}

/// Deletes every memory.
#[tauri::command]
#[specta::specta]
pub async fn ai_memories_delete_all(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let deleted = state.with_unlocked(|v| Ok(v.ai_memories_delete_all()?))?;
    tracing::info!(deleted, "AI memories deleted");
    sync::local_change(&app);
    Ok(())
}
