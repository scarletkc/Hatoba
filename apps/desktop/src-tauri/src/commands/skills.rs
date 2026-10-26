//! AI skill commands (spec §13.8, AI-27): Settings → AI creates, edits, imports, exports, enables
//! and deletes skills, which `hatoba_assistant::skills` keeps in the vault. File IO runs off the
//! async runtime.
//!
//! Skill text reaches the model as instructions, so it is never logged (SEC-04).

use std::path::Path;

use hatoba_ai::skills::{self, SkillError};
use hatoba_assistant::skills::{
    builtin_view, import_skill, package_of, preview, read_source, save_skill, set_skill_enabled,
    skill_detail, skill_view,
};
use tauri::{AppHandle, State};

use crate::dto::{BuiltinSkillView, SkillDetail, SkillImportPreview, SkillInput, SkillView};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, blocking};
use crate::sync;

#[tauri::command]
#[specta::specta]
pub async fn skills_list(state: State<'_, AppState>) -> AppResult<Vec<SkillView>> {
    state.with_unlocked(|v| {
        let mut list: Vec<SkillView> = v
            .skills()
            .iter()
            .map(|(id, s)| skill_view(v, id, s))
            .collect();
        list.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        Ok(list)
    })
}

#[tauri::command]
#[specta::specta]
pub async fn skill_get(state: State<'_, AppState>, id: String) -> AppResult<SkillDetail> {
    state.with_unlocked(|v| skill_detail(v, &id))
}

/// The built-in `hatoba` skill (AI-34), with this app's version, for the viewer in Settings.
#[tauri::command]
#[specta::specta]
pub async fn skill_builtin_get(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<BuiltinSkillView> {
    let version = app.package_info().version.to_string();
    state.with_unlocked(|v| Ok(builtin_view(v, &version)))
}

/// Validates like an import (AI-27); rejects with `invalid_input` naming the field.
#[tauri::command]
#[specta::specta]
pub async fn skill_save(
    app: AppHandle,
    state: State<'_, AppState>,
    input: SkillInput,
) -> AppResult<SkillView> {
    let view = state.with_unlocked(|v| save_skill(v, &input))?;
    tracing::info!(skill_id = %view.id, files = view.files.len(), "skill saved");
    sync::local_change(&app);
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub async fn skill_delete(app: AppHandle, state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.with_unlocked(|v| Ok(v.skill_delete(&id)?))?;
    tracing::info!(skill_id = %id, "skill deleted");
    sync::local_change(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn skill_set_enabled(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> AppResult<()> {
    state.with_unlocked(|v| set_skill_enabled(v, &id, enabled))?;
    sync::local_change(&app);
    Ok(())
}

/// Reads a folder or `.zip` the user picked; nothing is saved.
#[tauri::command]
#[specta::specta]
pub async fn skill_import_preview(
    state: State<'_, AppState>,
    path: String,
) -> AppResult<SkillImportPreview> {
    state.with_unlocked(|_| Ok(()))?;
    let import = blocking(move || read_source(Path::new(&path))).await?;
    state.with_unlocked(|v| Ok(preview(v, &import)))
}

/// Imports the skill at `path`, reading it again. `replace_id` replaces that skill; `rename`
/// saves it under a new name.
#[tauri::command]
#[specta::specta]
pub async fn skill_import(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    replace_id: Option<String>,
    rename: Option<String>,
) -> AppResult<SkillView> {
    state.with_unlocked(|_| Ok(()))?;
    let import = blocking(move || read_source(Path::new(&path))).await?;
    let view = state
        .with_unlocked(|v| import_skill(v, import, replace_id.as_deref(), rename.as_deref()))?;
    tracing::info!(skill_id = %view.id, files = view.files.len(), "skill imported");
    sync::local_change(&app);
    Ok(view)
}

/// Writes the skill as a `.zip` to a path the user picked.
#[tauri::command]
#[specta::specta]
pub async fn skill_export(state: State<'_, AppState>, id: String, path: String) -> AppResult<()> {
    let pkg = state.with_unlocked(|v| package_of(v, &id))?;
    blocking(move || {
        skills::write_zip(&pkg, Path::new(&path)).map_err(|e| match e {
            SkillError::Io(_) => AppError::io("the .zip file could not be written"),
            SkillError::Zip(detail) => AppError::invalid("id", detail),
        })
    })
    .await?;
    tracing::info!(skill_id = %id, "skill exported");
    Ok(())
}
