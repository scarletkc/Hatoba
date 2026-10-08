//! The built-in `hatoba` skill (spec §13.8, AI-34): instructions about Hatoba itself, bundled in
//! the binary from `builtin/hatoba/`. `SKILL.md` holds [`VERSION_PLACEHOLDER`], which
//! [`builtin_skill`] replaces with the version of the running app, so the caller passes it in.

use serde_json::{Map, Value};

use super::{SKILL_MD, SkillFileData, SkillPackage, parse_skill_md};

/// The built-in skill's name. No user skill may take it.
pub const BUILTIN_NAME: &str = "hatoba";

/// Replaced with the app's version wherever the skill is served.
pub const VERSION_PLACEHOLDER: &str = "{{HATOBA_VERSION}}";

/// The skill's files as `(path, content)`: `SKILL.md` first, then the others sorted by path. A
/// test checks the list against the folder.
const FILES: &[(&str, &str)] = &[
    (SKILL_MD, include_str!("builtin/hatoba/SKILL.md")),
    (
        "references/ai-attachments.md",
        include_str!("builtin/hatoba/references/ai-attachments.md"),
    ),
    (
        "references/ai-models.md",
        include_str!("builtin/hatoba/references/ai-models.md"),
    ),
    (
        "references/ai-panel.md",
        include_str!("builtin/hatoba/references/ai-panel.md"),
    ),
    (
        "references/ai-settings.md",
        include_str!("builtin/hatoba/references/ai-settings.md"),
    ),
    (
        "references/hosts-and-connecting.md",
        include_str!("builtin/hatoba/references/hosts-and-connecting.md"),
    ),
    (
        "references/keys-vault-and-lock.md",
        include_str!("builtin/hatoba/references/keys-vault-and-lock.md"),
    ),
    (
        "references/port-forwarding.md",
        include_str!("builtin/hatoba/references/port-forwarding.md"),
    ),
    (
        "references/settings.md",
        include_str!("builtin/hatoba/references/settings.md"),
    ),
    (
        "references/sftp.md",
        include_str!("builtin/hatoba/references/sftp.md"),
    ),
    (
        "references/shortcuts.md",
        include_str!("builtin/hatoba/references/shortcuts.md"),
    ),
    (
        "references/sync-worker.md",
        include_str!("builtin/hatoba/references/sync-worker.md"),
    ),
    (
        "references/sync.md",
        include_str!("builtin/hatoba/references/sync.md"),
    ),
    (
        "references/terminal.md",
        include_str!("builtin/hatoba/references/terminal.md"),
    ),
    (
        "references/troubleshooting-ai.md",
        include_str!("builtin/hatoba/references/troubleshooting-ai.md"),
    ),
    (
        "references/troubleshooting-connections.md",
        include_str!("builtin/hatoba/references/troubleshooting-connections.md"),
    ),
    (
        "references/troubleshooting-sync.md",
        include_str!("builtin/hatoba/references/troubleshooting-sync.md"),
    ),
    (
        "references/troubleshooting.md",
        include_str!("builtin/hatoba/references/troubleshooting.md"),
    ),
];

/// The built-in skill, with `version` in place of every [`VERSION_PLACEHOLDER`]. Its name is
/// always [`BUILTIN_NAME`]; the tests check that `SKILL.md` parses and passes the import rules.
#[must_use]
pub fn builtin_skill(version: &str) -> SkillPackage {
    let fill = |text: &str| text.replace(VERSION_PLACEHOLDER, version);
    let skill_md = fill(FILES[0].1);
    let (mut frontmatter, body) =
        parse_skill_md(&skill_md).unwrap_or_else(|_| (Map::new(), skill_md.clone()));
    let description = take_description(&mut frontmatter);
    frontmatter.remove("name");
    SkillPackage {
        name: BUILTIN_NAME.to_owned(),
        description,
        frontmatter,
        body,
        files: FILES[1..]
            .iter()
            .map(|(path, content)| SkillFileData {
                path: (*path).to_owned(),
                content: fill(content),
            })
            .collect(),
    }
}

/// The built-in skill's description, as the system prompt lists it (AI-28), without reading the
/// rest of the skill.
#[must_use]
pub fn builtin_description() -> String {
    parse_skill_md(FILES[0].1)
        .map(|(mut frontmatter, _)| take_description(&mut frontmatter))
        .unwrap_or_default()
}

fn take_description(frontmatter: &mut Map<String, Value>) -> String {
    match frontmatter.remove("description") {
        Some(Value::String(description)) => description.trim().to_owned(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::skills::{MAX_DESCRIPTION_CHARS, valid_name, validate};
    use crate::tools::{MAX_RESULT_CHARS, truncate_result};

    /// Every file under `dir`, as paths relative to `root` with forward slashes.
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let relative = path.strip_prefix(root).unwrap();
                out.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }

    #[test]
    fn the_list_matches_the_folder() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/skills/builtin/hatoba");
        let mut on_disk = Vec::new();
        walk(&root, &root, &mut on_disk);
        on_disk.sort();
        let mut listed: Vec<String> = FILES.iter().map(|(p, _)| (*p).to_owned()).collect();
        assert_eq!(listed[0], SKILL_MD);
        let others = listed.split_off(1);
        let mut sorted = others.clone();
        sorted.sort();
        assert_eq!(others, sorted, "the other files are sorted by path");
        listed.extend(others);
        listed.sort();
        assert_eq!(listed, on_disk);
        // Each path names the file its content came from.
        for (path, content) in FILES {
            assert_eq!(
                *content,
                std::fs::read_to_string(root.join(path)).unwrap(),
                "{path}"
            );
        }
    }

    #[test]
    fn it_is_a_valid_skill_named_hatoba() {
        let (fields, _) = parse_skill_md(FILES[0].1).expect("SKILL.md parses");
        assert_eq!(fields["name"], BUILTIN_NAME);
        assert!(valid_name(BUILTIN_NAME));

        let pkg = builtin_skill("1.2.3");
        assert_eq!(pkg.name, BUILTIN_NAME);
        assert!(!pkg.description.is_empty());
        assert_eq!(builtin_description(), pkg.description);
        assert!(pkg.description.chars().count() <= MAX_DESCRIPTION_CHARS);
        assert_eq!(validate(&pkg), []);
        assert!(
            !pkg.frontmatter.contains_key("name") && !pkg.frontmatter.contains_key("description")
        );
        assert_eq!(pkg.files.len(), FILES.len() - 1);
    }

    #[test]
    fn the_version_is_filled_in_and_read_skill_never_shortens_a_file() {
        assert!(FILES[0].1.contains(VERSION_PLACEHOLDER));
        // Far longer than any real version.
        let version = "10.20.30-really.long.prerelease.identifier.42+build.0123456789abcdef";
        let pkg = builtin_skill(version);
        assert!(pkg.body.contains(version));
        let texts = std::iter::once(("SKILL.md", &pkg.body))
            .chain(pkg.files.iter().map(|f| (f.path.as_str(), &f.content)));
        for (path, text) in texts {
            assert!(!text.contains(VERSION_PLACEHOLDER), "{path}");
            assert!(text.chars().count() <= MAX_RESULT_CHARS, "{path}");
            assert_eq!(&truncate_result(text), text, "{path}");
        }
    }
}
