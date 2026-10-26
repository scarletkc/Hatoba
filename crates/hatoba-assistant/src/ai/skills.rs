//! Skills (§13.8): the enabled ones, `read_skill` (AI-28), and the skills the panel's `/` menu
//! offers as commands (AI-50).

use hatoba_ai::entry::ToolStatus;
use hatoba_ai::skills::{BUILTIN_NAME, builtin_description, builtin_skill};
use hatoba_ai::tools::{self, ReadSkillArgs};
use hatoba_core::Vault;

use super::job::Job;
use crate::dto::AiSkillCommand;
use crate::error::{AppError, AppResult};

/// A skill's main file (§13.8).
const SKILL_MD: &str = "SKILL.md";

/// The enabled skills the user saved. One named like the built-in skill (from a build before
/// the name was reserved) is left out: the name means the built-in skill (AI-34).
pub(super) fn enabled_skills(v: &Vault) -> Vec<(String, hatoba_core::model::Skill)> {
    v.skills()
        .into_iter()
        .filter(|(_, s)| s.enabled && s.name != BUILTIN_NAME)
        .collect()
}

/// AI-28: the `SKILL.md` body of an enabled skill, or the file at `path` inside it. `hatoba` is
/// the built-in skill while it is switched on, with `version` filled in (AI-34).
pub(super) fn read_skill(v: &Vault, args: &ReadSkillArgs, version: &str) -> Job {
    let name = args.name.trim();
    let Some(files) = enabled_skill_files(v, name, version) else {
        let mut names: Vec<String> = skill_commands(v).into_iter().map(|c| c.name).collect();
        names.sort_unstable();
        let list = if names.is_empty() {
            "none".to_owned()
        } else {
            names.join(", ")
        };
        return Job::error(format!(
            "There is no enabled skill named \"{name}\". Enabled skills: {list}."
        ));
    };
    let path = args
        .path
        .as_deref()
        .map(skill_path)
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| SKILL_MD.to_owned());
    match files.iter().find(|(p, _)| *p == path) {
        Some((_, content)) => Job::Ready(ToolStatus::Ok, tools::truncate_result(content)),
        None => {
            let mut paths: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
            paths.sort_unstable();
            Job::error(format!(
                "The skill \"{name}\" has no file \"{path}\". Its files: {}.",
                if paths.is_empty() {
                    "none".to_owned()
                } else {
                    paths.join(", ")
                }
            ))
        }
    }
}

/// `(path, content)` of every file of the enabled skill `name`, `SKILL.md` included: the built-in
/// one while it is switched on, with `version` filled in (AI-34). `None` when no enabled skill
/// has that name.
fn enabled_skill_files(v: &Vault, name: &str, version: &str) -> Option<Vec<(String, String)>> {
    if name == BUILTIN_NAME {
        if !v.settings().ai.builtin_skill_enabled {
            return None;
        }
        let skill = builtin_skill(version);
        return Some(
            std::iter::once((SKILL_MD.to_owned(), skill.body))
                .chain(skill.files.into_iter().map(|f| (f.path, f.content)))
                .collect(),
        );
    }
    let enabled = enabled_skills(v);
    let (skill_id, _) = enabled.iter().find(|(_, s)| s.name == name)?;
    Some(
        v.skill_files(skill_id)
            .into_iter()
            .map(|(_, f)| (f.path, f.content))
            .collect(),
    )
}

/// AI-50: the skills the panel's `/` menu offers as commands: every enabled skill, and the
/// built-in one while it is switched on (AI-34), sorted by name, as the system prompt lists them
/// (AI-28).
pub fn skill_commands(v: &Vault) -> Vec<AiSkillCommand> {
    let mut commands: Vec<AiSkillCommand> = enabled_skills(v)
        .into_iter()
        .map(|(_, s)| AiSkillCommand {
            name: s.name,
            description: s.description,
        })
        .collect();
    if v.settings().ai.builtin_skill_enabled {
        commands.push(AiSkillCommand {
            name: BUILTIN_NAME.to_owned(),
            description: builtin_description(),
        });
    }
    commands.sort_by(|a, b| a.name.cmp(&b.name));
    commands
}

/// AI-50: what a `/<skill>` command attaches in its `skill` block: the skill's `SKILL.md` as
/// `read_skill` reads it, but whole, where `read_skill` cuts a result past 30,000 characters.
pub fn skill_text(v: &Vault, name: &str, version: &str) -> AppResult<String> {
    enabled_skill_files(v, name.trim(), version)
        .and_then(|files| files.into_iter().find(|(p, _)| p == SKILL_MD))
        .map(|(_, content)| content)
        .ok_or_else(|| AppError::not_found("skill"))
}

/// A path inside a skill as the model may write it: `./references\x.md` is `references/x.md`.
fn skill_path(path: &str) -> String {
    let path = path.trim().replace('\\', "/");
    let mut rest = path.as_str();
    while let Some(stripped) = rest.strip_prefix("./").or_else(|| rest.strip_prefix('/')) {
        rest = stripped;
    }
    rest.to_owned()
}
