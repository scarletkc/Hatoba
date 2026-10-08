//! AI skill commands (spec §13.8, AI-27): Settings → AI creates, edits, imports, exports, enables
//! and deletes skills. A skill is a `skill` item and one `skill_file` item per file, with the body
//! of `SKILL.md` in the file whose path is `SKILL.md` (§5.1). Edits and imports are checked
//! against the same rules (`hatoba_ai::skills::validate`); file IO runs off the async runtime.
//!
//! Skill text reaches the model as instructions, so it is never logged (SEC-04).

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::path::Path;

use hatoba_ai::skills::{
    self, MAX_DESCRIPTION_CHARS, MAX_FILES, SkillError, SkillFileData, SkillImport,
    SkillIssue as Issue, SkillPackage,
};
use hatoba_core::Vault;
use hatoba_core::model::{Item, Skill, SkillFile};
use serde_json::{Map, Value};
use tauri::{AppHandle, State};

use crate::dto::{
    SkillDetail, SkillFileView, SkillImportPreview, SkillInput, SkillIssue, SkillView,
};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, blocking};
use crate::sync;

/// The file that holds a skill's instructions (its body, without frontmatter).
const SKILL_MD: &str = "SKILL.md";

/// What a name must be (AI-27), for messages.
const NAME_RULE: &str = "Use 1 to 64 lowercase letters, digits and hyphens.";

// ───────────────────────── issues ─────────────────────────

fn issue_view(issue: &Issue) -> SkillIssue {
    match issue {
        Issue::MissingSkillMd => SkillIssue::MissingSkillMd,
        Issue::InvalidFrontmatter(detail) => SkillIssue::InvalidFrontmatter {
            detail: detail.clone(),
        },
        Issue::InvalidName(name) => SkillIssue::InvalidName { name: name.clone() },
        Issue::MissingDescription => SkillIssue::MissingDescription,
        Issue::DescriptionTooLong(chars) => SkillIssue::DescriptionTooLong {
            chars: *chars as u64,
        },
        Issue::FileTooLarge { path, size } => SkillIssue::FileTooLarge {
            path: path.clone(),
            size: *size,
        },
        Issue::UnsafePath(path) => SkillIssue::UnsafePath { path: path.clone() },
        Issue::TooManyFiles(count) => SkillIssue::TooManyFiles {
            count: *count as u64,
        },
        Issue::TooLarge(bytes) => SkillIssue::TooLarge { bytes: *bytes },
    }
}

/// The form field an issue belongs to.
fn issue_field(issue: &Issue) -> &'static str {
    match issue {
        Issue::InvalidName(_) => "name",
        Issue::MissingDescription | Issue::DescriptionTooLong(_) => "description",
        Issue::FileTooLarge { path, .. } if path == SKILL_MD => "body",
        Issue::MissingSkillMd | Issue::InvalidFrontmatter(_) => "body",
        Issue::FileTooLarge { .. }
        | Issue::UnsafePath(_)
        | Issue::TooManyFiles(_)
        | Issue::TooLarge(_) => "files",
    }
}

/// A short sentence for the user, naming the file or the field (§9).
fn issue_text(issue: &Issue) -> String {
    match issue {
        Issue::MissingSkillMd => "No SKILL.md was found.".to_owned(),
        Issue::InvalidFrontmatter(detail) => {
            format!("The frontmatter of SKILL.md is not valid: {detail}.")
        }
        Issue::InvalidName(_) => format!("The name is not valid. {NAME_RULE}"),
        Issue::MissingDescription => "A description is required.".to_owned(),
        Issue::DescriptionTooLong(chars) => {
            format!("The description has {chars} characters; the limit is {MAX_DESCRIPTION_CHARS}.")
        }
        Issue::FileTooLarge { path, .. } if path == SKILL_MD => {
            "The instructions are larger than 32 KB.".to_owned()
        }
        Issue::FileTooLarge { path, .. } => format!("{path} is larger than 32 KB."),
        Issue::UnsafePath(path) => format!("\"{path}\" is not a valid file path."),
        Issue::TooManyFiles(_) => format!("A skill can have at most {MAX_FILES} files."),
        Issue::TooLarge(_) => "The files together are larger than 5 MB.".to_owned(),
    }
}

fn source_error(e: SkillError) -> AppError {
    match e {
        SkillError::Io(_) => AppError::invalid("path", "The folder or file could not be read."),
        SkillError::Zip(detail) => AppError::invalid(
            "path",
            format!("The .zip file could not be read: {detail}."),
        ),
    }
}

// ───────────────────────── stored skills ─────────────────────────

fn find_skill(v: &Vault, id: &str) -> AppResult<Skill> {
    v.get(id)
        .and_then(Item::as_skill)
        .cloned()
        .ok_or_else(|| AppError::not_found("skill"))
}

/// The frontmatter fields a skill item keeps: all but `name` and `description`.
fn other_fields(mut fields: Map<String, Value>) -> Map<String, Value> {
    fields.remove("name");
    fields.remove("description");
    fields
}

/// A skill's files by path, as `(item id, file)`. Of two items with one path (two devices
/// wrote it), the newest wins; the others are returned to delete.
fn files_by_path(
    v: &Vault,
    skill_id: &str,
) -> (BTreeMap<String, (String, SkillFile)>, Vec<String>) {
    let mut files: BTreeMap<String, (String, SkillFile)> = BTreeMap::new();
    let mut extra = Vec::new();
    for (id, file) in v.skill_files(skill_id) {
        match files.entry(file.path.clone()) {
            Entry::Vacant(slot) => {
                slot.insert((id, file));
            }
            Entry::Occupied(mut slot) if file.updated_at > slot.get().1.updated_at => {
                let (older, _) = slot.insert((id, file));
                extra.push(older);
            }
            Entry::Occupied(_) => extra.push(id),
        }
    }
    (files, extra)
}

pub(crate) fn skill_view(v: &Vault, id: &str, skill: &Skill) -> SkillView {
    let (files, _) = files_by_path(v, id);
    SkillView {
        id: id.to_owned(),
        name: skill.name.clone(),
        description: skill.description.clone(),
        enabled: skill.enabled,
        files: files.into_keys().filter(|p| p != SKILL_MD).collect(),
        updated_at: skill.updated_at,
    }
}

/// A stored skill as a package, for export.
fn package_of(v: &Vault, id: &str) -> AppResult<SkillPackage> {
    let skill = find_skill(v, id)?;
    let (mut files, _) = files_by_path(v, id);
    let body = files
        .remove(SKILL_MD)
        .map(|(_, f)| f.content)
        .unwrap_or_default();
    Ok(SkillPackage {
        name: skill.name,
        description: skill.description,
        frontmatter: skill.frontmatter,
        body,
        files: files
            .into_values()
            .map(|(_, f)| SkillFileData {
                path: f.path,
                content: f.content,
            })
            .collect(),
    })
}

pub(crate) fn skill_detail(v: &Vault, id: &str) -> AppResult<SkillDetail> {
    let skill = find_skill(v, id)?;
    let package = package_of(v, id)?;
    Ok(SkillDetail {
        skill: skill_view(v, id, &skill),
        body: package.body,
        files: package
            .files
            .into_iter()
            .map(|f| SkillFileView {
                path: f.path,
                content: f.content,
            })
            .collect(),
        frontmatter_keys: other_fields(skill.frontmatter)
            .into_iter()
            .map(|(k, _)| k)
            .collect(),
    })
}

/// The id of another skill named `name`.
fn named(v: &Vault, name: &str, except: Option<&str>) -> Option<String> {
    v.skills()
        .into_iter()
        .find(|(id, s)| s.name == name && Some(id.as_str()) != except)
        .map(|(id, _)| id)
}

fn taken(field: &str, name: &str) -> AppError {
    AppError::invalid(field, format!("A skill named \"{name}\" already exists."))
}

/// Stores a package as a skill and its files: under `id` (its fields and files replaced) or as a
/// new skill. A file left out is deleted; an unchanged one is not written again.
fn store_skill(
    v: &mut Vault,
    id: Option<&str>,
    pkg: &SkillPackage,
    enabled: bool,
) -> AppResult<String> {
    let skill = Skill {
        name: pkg.name.clone(),
        description: pkg.description.clone(),
        frontmatter: other_fields(pkg.frontmatter.clone()),
        enabled,
        updated_at: 0,
    };
    let skill_id = v.put(id, Item::Skill(skill))?;
    let (existing, duplicates) = files_by_path(v, &skill_id);
    let wanted: Vec<(&str, &str)> = std::iter::once((SKILL_MD, pkg.body.as_str()))
        .chain(
            pkg.files
                .iter()
                .map(|f| (f.path.as_str(), f.content.as_str())),
        )
        .collect();
    for (path, content) in &wanted {
        let file_id = match existing.get(*path) {
            Some((_, file)) if file.content == *content => continue,
            Some((file_id, _)) => Some(file_id.as_str()),
            None => None,
        };
        v.put(
            file_id,
            Item::SkillFile(SkillFile {
                skill_id: skill_id.clone(),
                path: (*path).to_owned(),
                content: (*content).to_owned(),
                updated_at: 0,
            }),
        )?;
    }
    for (path, (file_id, _)) in &existing {
        if !wanted.iter().any(|(p, _)| p == path) {
            v.delete(file_id)?;
        }
    }
    for file_id in duplicates {
        v.delete(&file_id)?;
    }
    Ok(skill_id)
}

/// `skill_save`: checks the skill like an import (AI-27) and stores it. The frontmatter fields of
/// a saved skill are kept.
pub(crate) fn save_skill(v: &mut Vault, input: &SkillInput) -> AppResult<SkillView> {
    let existing = match &input.id {
        Some(id) => Some(find_skill(v, id)?),
        None => None,
    };
    let pkg = SkillPackage {
        name: input.name.trim().to_owned(),
        description: input.description.trim().to_owned(),
        frontmatter: existing.map(|s| s.frontmatter).unwrap_or_default(),
        body: input.body.clone(),
        files: input
            .files
            .iter()
            .map(|f| SkillFileData {
                path: f.path.clone(),
                content: f.content.clone(),
            })
            .collect(),
    };
    if let Some(issue) = skills::validate(&pkg).first() {
        return Err(AppError::invalid(issue_field(issue), issue_text(issue)));
    }
    if named(v, &pkg.name, input.id.as_deref()).is_some() {
        return Err(taken("name", &pkg.name));
    }
    let id = store_skill(v, input.id.as_deref(), &pkg, input.enabled)?;
    Ok(skill_view(v, &id, &find_skill(v, &id)?))
}

pub(crate) fn set_skill_enabled(v: &mut Vault, id: &str, enabled: bool) -> AppResult<()> {
    let mut skill = find_skill(v, id)?;
    if skill.enabled != enabled {
        skill.enabled = enabled;
        v.put(Some(id), Item::Skill(skill))?;
    }
    Ok(())
}

// ───────────────────────── import ─────────────────────────

/// Reads a folder, a `.zip`, or the folder of a picked `SKILL.md`.
fn read_source(path: &Path) -> AppResult<SkillImport> {
    let meta = std::fs::metadata(path)
        .map_err(|_| AppError::invalid("path", "The folder or file could not be opened."))?;
    let result = if meta.is_dir() {
        skills::read_folder(path)
    } else if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
    {
        skills::read_zip(path)
    } else if path
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case(SKILL_MD))
        && let Some(folder) = path.parent()
    {
        skills::read_folder(folder)
    } else {
        return Err(AppError::invalid(
            "path",
            "Choose a folder or a .zip file that holds a SKILL.md.",
        ));
    };
    result.map_err(source_error)
}

pub(crate) fn preview(v: &Vault, import: &SkillImport) -> SkillImportPreview {
    let package = import.package.as_ref();
    SkillImportPreview {
        name: package.map(|p| p.name.clone()),
        description: package.map(|p| p.description.clone()),
        body: package.map(|p| p.body.clone()),
        files: package
            .map(|p| {
                p.files
                    .iter()
                    .map(|f| SkillFileView {
                        path: f.path.clone(),
                        content: f.content.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        frontmatter_keys: package
            .map(|p| {
                other_fields(p.frontmatter.clone())
                    .into_iter()
                    .map(|(k, _)| k)
                    .collect()
            })
            .unwrap_or_default(),
        skipped: import.skipped.iter().map(|s| s.path.clone()).collect(),
        issues: import.issues.iter().map(issue_view).collect(),
        existing_id: package.and_then(|p| named(v, &p.name, None)),
    }
}

/// `skill_import`: saves what the preview showed. `replace_id` keeps that skill's id and
/// enabled state and rewrites its fields and files; `rename` saves under a new, valid name;
/// otherwise a taken name is refused.
pub(crate) fn import_skill(
    v: &mut Vault,
    import: SkillImport,
    replace_id: Option<&str>,
    rename: Option<&str>,
) -> AppResult<SkillView> {
    if let Some(issue) = import.issues.first() {
        return Err(AppError::invalid("path", issue_text(issue)));
    }
    let Some(mut pkg) = import.package else {
        return Err(AppError::invalid("path", "No SKILL.md was found."));
    };
    let field = if let Some(name) = rename {
        let name = name.trim();
        if !skills::valid_name(name) {
            return Err(AppError::invalid(
                "rename",
                format!("The name is not valid. {NAME_RULE}"),
            ));
        }
        name.clone_into(&mut pkg.name);
        "rename"
    } else {
        "name"
    };
    let enabled = match replace_id {
        Some(id) => find_skill(v, id)?.enabled,
        None => true,
    };
    if named(v, &pkg.name, replace_id).is_some() {
        return Err(taken(field, &pkg.name));
    }
    let id = store_skill(v, replace_id, &pkg, enabled)?;
    Ok(skill_view(v, &id, &find_skill(v, &id)?))
}

// ───────────────────────── commands ─────────────────────────

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

/// Imports what the preview showed, reading the path again. `replace_id` replaces that skill;
/// `rename` saves it under a new name.
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

#[cfg(test)]
mod tests;
