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
        updated_at: m.updated_at,
    }
}

/// Every memory, oldest first.
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

/// Saves a memory: `id` replaces that one, together with the others of its name when the edit
/// keeps it ([`memory::edited`]); none adds one.
#[tauri::command]
#[specta::specta]
pub async fn ai_memory_save(
    app: AppHandle,
    state: State<'_, AppState>,
    input: AiMemoryInput,
) -> AppResult<AiMemoryView> {
    let saved = state.with_unlocked(|v| {
        let ids = match &input.id {
            Some(id) if v.get(id).and_then(Item::as_ai_memory).is_none() => {
                return Err(AppError::not_found("memory"));
            }
            Some(id) => memory::edited(v, id, &input.name),
            None => Vec::new(),
        };
        memory::save(v, &ids, &input.name, &input.content)?
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
