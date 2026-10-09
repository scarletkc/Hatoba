//! Skill command tests (AI-27, AI-28): import from a folder and a `.zip`, replace and rename,
//! export round trips, and saves that break a rule.

use std::path::PathBuf;

use hatoba_ai::skills::{SkillFileData, SkillPackage};
use hatoba_core::KdfParams;
use serde_json::json;

use super::*;
use crate::error::ErrorCode;

/// A directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "hatoba-skills-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, rel: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn vault() -> Vault {
    let mut vault = Vault::open_in_memory().unwrap();
    vault
        .create_with_params("correct horse battery staple", KdfParams::for_tests())
        .unwrap();
    vault
}

const BODY: &str = "# nginx\r\nRun `nginx -t` first.\r\n";

/// A skill folder with frontmatter beyond name and description, a reference and a binary file.
fn skill_folder(tag: &str) -> TempDir {
    let dir = TempDir::new(tag);
    dir.write(
        "SKILL.md",
        format!(
            "---\nname: nginx-debug\ndescription: Debug nginx\nlicense: MIT\nallowed-tools: [Bash]\n---\r\n{BODY}"
        )
        .as_bytes(),
    );
    dir.write("references/tls.md", b"TLS notes");
    dir.write("logo.png", &[0x89, b'P', b'N', b'G', 0, 1, 2]);
    dir
}

fn input(name: &str) -> SkillInput {
    SkillInput {
        id: None,
        name: name.into(),
        description: "Does things".into(),
        enabled: true,
        body: "Steps".into(),
        files: vec![SkillFileView {
            path: "references/a.md".into(),
            content: "A".into(),
        }],
    }
}

fn stored_paths(v: &Vault, id: &str) -> Vec<String> {
    let mut paths: Vec<String> = v.skill_files(id).into_iter().map(|(_, f)| f.path).collect();
    paths.sort();
    paths
}

/// `import_skill` with the token a preview of the same read showed.
fn import_previewed(
    v: &mut Vault,
    import: SkillImport,
    replace_id: Option<&str>,
    rename: Option<&str>,
) -> AppResult<SkillView> {
    let token = preview(v, &import).token;
    import_skill(v, import, &token, replace_id, rename)
}

#[test]
fn an_import_saves_only_what_its_preview_showed() {
    let mut v = vault();
    let dir = skill_folder("toctou");
    let shown = preview(&v, &read_source(&dir.0).unwrap());
    // The same files read the same, whenever they are read.
    assert_eq!(
        shown.token,
        preview(&v, &read_source(&dir.0).unwrap()).token
    );
    assert_eq!(shown.token.len(), 64);

    // A file changed on disk after the preview: nothing is saved.
    dir.write("references/tls.md", b"TLS notes, edited");
    let err = import_skill(
        &mut v,
        read_source(&dir.0).unwrap(),
        &shown.token,
        None,
        None,
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert_eq!(err.field.as_deref(), Some("path"));
    assert!(err.detail.contains("Preview it again"), "{}", err.detail);
    assert!(v.skills().is_empty());

    // A file added, or a skipped one changed into text, changes the token too.
    let again = preview(&v, &read_source(&dir.0).unwrap());
    assert_ne!(again.token, shown.token);
    dir.write("references/more.md", b"More");
    assert_ne!(
        preview(&v, &read_source(&dir.0).unwrap()).token,
        again.token
    );

    // Previewed again, it imports.
    let now = preview(&v, &read_source(&dir.0).unwrap());
    let view = import_skill(&mut v, read_source(&dir.0).unwrap(), &now.token, None, None).unwrap();
    assert_eq!(view.files, ["references/more.md", "references/tls.md"]);
}

#[test]
fn files_that_would_not_fit_a_sync_envelope_are_refused() {
    let mut v = vault();
    // Under 32 KB of raw bytes, but control characters take six bytes each once escaped.
    let escaped = "\u{1b}[0m".repeat(7_000);
    assert!(escaped.len() < 32 * 1024);
    let mut edit = input("ansi-notes");
    edit.files[0].content = escaped.clone();
    let err = save_skill(&mut v, &edit).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert_eq!(err.field.as_deref(), Some("files"));
    assert!(
        err.detail
            .starts_with("references/a.md is too large to sync"),
        "{}",
        err.detail
    );

    let mut edit = input("ansi-notes");
    edit.body = "\"\\".repeat(12_000);
    let err = save_skill(&mut v, &edit).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("body"));
    assert!(
        err.detail.starts_with("The instructions are too large"),
        "{}",
        err.detail
    );
    assert!(v.skills().is_empty(), "nothing was saved");

    // The same goes for an import, and for frontmatter that only an import brings.
    let dir = TempDir::new("escaped");
    dir.write(
        "SKILL.md",
        b"---\nname: ansi-notes\ndescription: Colors\n---\nBody\n",
    );
    dir.write("references/colors.md", escaped.as_bytes());
    let import = read_source(&dir.0).unwrap();
    assert!(preview(&v, &import).issues.is_empty());
    let err = import_previewed(&mut v, import, None, None).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("path"));
    assert!(
        err.detail.starts_with("references/colors.md is too large"),
        "{}",
        err.detail
    );

    let pkg = SkillPackage {
        name: "wide".into(),
        description: "Wide".into(),
        frontmatter: [("notes".to_owned(), json!("\u{1}".repeat(8_000)))]
            .into_iter()
            .collect(),
        body: "Body".into(),
        files: Vec::new(),
    };
    assert_eq!(oversized(None, &pkg).unwrap(), Some(Oversized::Frontmatter));
    // What fits is saved as before.
    let ok = save_skill(&mut v, &input("plain")).unwrap();
    assert_eq!(
        oversized(Some(&ok.id), &package_of(&v, &ok.id).unwrap()).unwrap(),
        None
    );
}

#[test]
fn a_folder_import_shows_every_file_then_saves_the_skill() {
    let mut v = vault();
    let dir = skill_folder("folder");
    let import = read_source(&dir.0).unwrap();
    let shown = preview(&v, &import);
    assert_eq!(shown.name.as_deref(), Some("nginx-debug"));
    assert_eq!(shown.description.as_deref(), Some("Debug nginx"));
    assert_eq!(shown.body.as_deref(), Some(BODY));
    assert_eq!(
        shown.files,
        [SkillFileView {
            path: "references/tls.md".into(),
            content: "TLS notes".into()
        }]
    );
    assert_eq!(shown.frontmatter_keys, ["allowed-tools", "license"]);
    assert_eq!(shown.skipped, ["logo.png"]);
    assert!(shown.issues.is_empty());
    assert_eq!(shown.existing_id, None);
    // A picked SKILL.md reads its folder.
    let picked = read_source(&dir.0.join("SKILL.md")).unwrap();
    assert_eq!(picked, import);

    let view = import_previewed(&mut v, import, None, None).unwrap();
    assert!(view.enabled);
    assert_eq!(view.files, ["references/tls.md"]);
    let skill = find_skill(&v, &view.id).unwrap();
    // Other frontmatter fields are kept for export; name and description live in the item.
    assert_eq!(skill.frontmatter.get("license"), Some(&json!("MIT")));
    assert!(
        !skill.frontmatter.contains_key("name") && !skill.frontmatter.contains_key("description")
    );
    assert_eq!(
        stored_paths(&v, &view.id),
        ["SKILL.md", "references/tls.md"]
    );
    let detail = skill_detail(&v, &view.id).unwrap();
    assert_eq!(detail.body, BODY);
    assert_eq!(detail.frontmatter_keys, ["allowed-tools", "license"]);

    // The name is taken now: the preview names the skill, and a plain import is refused.
    let again = read_source(&dir.0).unwrap();
    assert_eq!(
        preview(&v, &again).existing_id.as_deref(),
        Some(view.id.as_str())
    );
    let err = import_previewed(&mut v, again.clone(), None, None).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("name"));

    // Rename: a second skill, with a valid name that is free.
    let err = import_previewed(&mut v, again.clone(), None, Some("Bad Name")).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("rename"));
    let err = import_previewed(&mut v, again.clone(), None, Some("nginx-debug")).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("rename"));
    let copy = import_previewed(&mut v, again, None, Some(" nginx-copy ")).unwrap();
    assert_ne!(copy.id, view.id);
    assert_eq!(copy.name, "nginx-copy");
    assert_eq!(v.skills().len(), 2);

    // Replace: the same id and enabled state, the new fields and files; a dropped file goes.
    set_skill_enabled(&mut v, &view.id, false).unwrap();
    std::fs::remove_file(dir.0.join("references/tls.md")).unwrap();
    dir.write("references/new.md", b"New notes");
    dir.write(
        "SKILL.md",
        b"---\nname: nginx-debug\ndescription: Debug nginx better\n---\nNew body\n",
    );
    let replaced =
        import_previewed(&mut v, read_source(&dir.0).unwrap(), Some(&view.id), None).unwrap();
    assert_eq!(replaced.id, view.id);
    assert!(!replaced.enabled);
    assert_eq!(replaced.description, "Debug nginx better");
    assert_eq!(replaced.files, ["references/new.md"]);
    assert_eq!(
        stored_paths(&v, &view.id),
        ["SKILL.md", "references/new.md"]
    );
    assert!(find_skill(&v, &view.id).unwrap().frontmatter.is_empty());
    assert_eq!(skill_detail(&v, &view.id).unwrap().body, "New body\n");
    assert_eq!(v.skills().len(), 2);
    assert_eq!(
        import_previewed(
            &mut v,
            read_source(&dir.0).unwrap(),
            Some("0190a0a0-0000-7000-8000-000000000000"),
            None
        )
        .unwrap_err()
        .code,
        ErrorCode::NotFound
    );
}

#[test]
fn an_import_with_issues_is_refused_naming_the_problem() {
    let mut v = vault();
    let dir = TempDir::new("bad");
    dir.write("SKILL.md", b"---\nname: Bad Name\n---\nbody");
    dir.write("references/huge.md", &vec![b'x'; 33 * 1024]);
    let import = read_source(&dir.0).unwrap();
    let shown = preview(&v, &import);
    assert_eq!(shown.name.as_deref(), Some("Bad Name"));
    assert!(shown.issues.contains(&SkillIssue::InvalidName {
        name: "Bad Name".into()
    }));
    assert!(shown.issues.contains(&SkillIssue::MissingDescription));
    assert!(shown.issues.iter().any(|i| matches!(
        i,
        SkillIssue::FileTooLarge { path, .. } if path == "references/huge.md"
    )));
    let err = import_previewed(&mut v, import, None, None).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("path"));
    assert!(err.detail.ends_with('.'), "{}", err.detail);
    assert!(v.skills().is_empty());

    let empty = TempDir::new("empty");
    let none = read_source(&empty.0).unwrap();
    assert_eq!(preview(&v, &none).issues, [SkillIssue::MissingSkillMd]);
    assert_eq!(preview(&v, &none).name, None);

    // Neither a folder nor a .zip; nothing there.
    let other = empty.write("notes.txt", b"hello");
    assert_eq!(
        read_source(&other).unwrap_err().field.as_deref(),
        Some("path")
    );
    let missing = empty.0.join("missing");
    assert_eq!(
        read_source(&missing).unwrap_err().field.as_deref(),
        Some("path")
    );
    let broken = empty.write("broken.zip", b"not a zip");
    assert_eq!(
        read_source(&broken).unwrap_err().field.as_deref(),
        Some("path")
    );
}

#[test]
fn a_zip_imports_and_an_export_round_trips() {
    let mut v = vault();
    let dir = TempDir::new("zip");
    let mut frontmatter = serde_json::Map::new();
    frontmatter.insert("license".into(), json!("MIT"));
    frontmatter.insert("metadata".into(), json!({"version": 2}));
    let original = SkillPackage {
        name: "deploy-helper".into(),
        description: "Deploys things".into(),
        frontmatter,
        body: "Line one\r\nLine two\n".into(),
        files: vec![
            SkillFileData {
                path: "references/a.md".into(),
                content: "A".into(),
            },
            SkillFileData {
                path: "scripts/check.sh".into(),
                content: "#!/bin/sh\necho ok\n".into(),
            },
        ],
    };
    let zip = dir.0.join("in.zip");
    hatoba_ai::skills::write_zip(&original, &zip).unwrap();
    let view = import_previewed(&mut v, read_source(&zip).unwrap(), None, None).unwrap();
    assert_eq!(view.files, ["references/a.md", "scripts/check.sh"]);

    let exported = package_of(&v, &view.id).unwrap();
    assert_eq!(exported, original);
    let out = dir.0.join("out.zip");
    hatoba_ai::skills::write_zip(&exported, &out).unwrap();
    let back = hatoba_ai::skills::read_zip(&out).unwrap();
    assert!(back.issues.is_empty());
    assert_eq!(back.package.unwrap(), original);
}

#[test]
fn saves_follow_the_import_rules_and_name_the_field() {
    let mut v = vault();
    let field = |v: &mut Vault, input: &SkillInput| {
        save_skill(v, input).unwrap_err().field.unwrap_or_default()
    };
    assert_eq!(field(&mut v, &input("Bad Name")), "name");
    assert_eq!(field(&mut v, &input("")), "name");
    let mut i = input("ok");
    i.description = "  ".into();
    assert_eq!(field(&mut v, &i), "description");
    i.description = "d".repeat(1_025);
    assert_eq!(field(&mut v, &i), "description");
    let mut i = input("ok");
    i.body = "b".repeat(33 * 1024);
    assert_eq!(field(&mut v, &i), "body");
    for path in ["../x.md", "/abs.md", "SKILL.md", "a\\b.md"] {
        let mut i = input("ok");
        i.files[0].path = path.into();
        assert_eq!(field(&mut v, &i), "files", "{path}");
    }
    let mut i = input("ok");
    i.files.push(i.files[0].clone());
    assert_eq!(field(&mut v, &i), "files");
    let mut i = input("ok");
    i.files[0].content = "c".repeat(33 * 1024);
    let err = save_skill(&mut v, &i).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("files"));
    assert_eq!(err.detail, "references/a.md is larger than 32 KB.");
    assert!(v.skills().is_empty());

    // A valid skill; its name is then taken.
    let saved = save_skill(&mut v, &input(" ok ")).unwrap();
    assert_eq!(saved.name, "ok");
    assert_eq!(saved.files, ["references/a.md"]);
    assert_eq!(field(&mut v, &input("ok")), "name");

    // An edit: a changed file is rewritten in place, a dropped one deleted, a new one added.
    let file_id = |v: &Vault, path: &str| {
        v.skill_files(&saved.id)
            .into_iter()
            .find(|(_, f)| f.path == path)
            .map(|(id, _)| id)
    };
    let skill_md = file_id(&v, "SKILL.md").unwrap();
    let mut edit = input("ok");
    edit.id = Some(saved.id.clone());
    edit.body = "New steps".into();
    edit.files = vec![SkillFileView {
        path: "references/b.md".into(),
        content: "B".into(),
    }];
    let edited = save_skill(&mut v, &edit).unwrap();
    assert_eq!(edited.id, saved.id);
    assert_eq!(edited.files, ["references/b.md"]);
    assert_eq!(file_id(&v, "SKILL.md"), Some(skill_md));
    assert_eq!(file_id(&v, "references/a.md"), None);
    assert_eq!(skill_detail(&v, &saved.id).unwrap().body, "New steps");

    // Enable and delete.
    set_skill_enabled(&mut v, &saved.id, false).unwrap();
    assert!(!find_skill(&v, &saved.id).unwrap().enabled);
    v.skill_delete(&saved.id).unwrap();
    assert!(v.skill_files(&saved.id).is_empty());
    assert_eq!(
        skill_detail(&v, &saved.id).unwrap_err().code,
        ErrorCode::NotFound
    );
}

#[test]
fn an_edit_keeps_the_imported_frontmatter() {
    let mut v = vault();
    let dir = skill_folder("keep");
    let view = import_previewed(&mut v, read_source(&dir.0).unwrap(), None, None).unwrap();
    let mut edit = input("nginx-debug");
    edit.id = Some(view.id.clone());
    save_skill(&mut v, &edit).unwrap();
    let skill = find_skill(&v, &view.id).unwrap();
    assert_eq!(skill.frontmatter.get("license"), Some(&json!("MIT")));
    assert_eq!(skill.description, "Does things");
}

#[test]
fn the_built_in_skills_name_is_reserved() {
    let mut v = vault();
    // AI-34: saving, importing and renaming to `hatoba` are refused.
    let err = save_skill(&mut v, &input("hatoba")).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert_eq!(err.field.as_deref(), Some("name"));
    assert!(err.detail.contains("reserved"), "{}", err.detail);

    let dir = TempDir::new("reserved");
    dir.write(
        "SKILL.md",
        b"---\nname: hatoba\ndescription: Mine\n---\nBody\n",
    );
    let import = read_source(&dir.0).unwrap();
    let shown = preview(&v, &import);
    assert!(shown.reserved_name);
    assert!(shown.issues.is_empty());
    let err = import_previewed(&mut v, import.clone(), None, None).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("name"));
    let mine = import_previewed(&mut v, import.clone(), None, Some("hatoba-notes")).unwrap();
    assert_eq!(mine.name, "hatoba-notes");
    assert!(!preview(&v, &read_source(&skill_folder("free").0).unwrap()).reserved_name);
    let other = read_source(&skill_folder("other").0).unwrap();
    let err = import_previewed(&mut v, other, None, Some("hatoba")).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("rename"));

    // One from an older build stays listed and can be renamed, not saved under the name.
    let old = v
        .put(
            None,
            Item::Skill(Skill {
                name: "hatoba".into(),
                description: "Old".into(),
                enabled: true,
                ..Skill::default()
            }),
        )
        .unwrap();
    assert_eq!(
        preview(&v, &import).existing_id.as_deref(),
        Some(old.as_str())
    );
    let err = import_previewed(&mut v, import, Some(&old), None).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("name"));
    let edit = SkillInput {
        id: Some(old.clone()),
        ..input("hatoba")
    };
    assert_eq!(
        save_skill(&mut v, &edit).unwrap_err().field.as_deref(),
        Some("name")
    );
    let renamed = save_skill(
        &mut v,
        &SkillInput {
            name: "hatoba-old".into(),
            ..edit
        },
    )
    .unwrap();
    assert_eq!(renamed.id, old);
}

#[test]
fn the_built_in_skill_shows_with_its_switch_and_the_version() {
    let mut v = vault();
    let view = builtin_view(&v, "4.5.6");
    assert_eq!(view.name, "hatoba");
    assert!(view.enabled);
    assert!(!view.description.is_empty());
    assert!(view.body.contains("4.5.6"));
    assert!(!view.body.contains("{{HATOBA_VERSION}}"));
    assert!(view.files.iter().all(|f| f.path.starts_with("references/")));
    let mut settings = v.settings();
    settings.ai.builtin_skill_enabled = false;
    v.put(
        Some(hatoba_core::model::SETTINGS_ID),
        Item::Settings(settings),
    )
    .unwrap();
    assert!(!builtin_view(&v, "4.5.6").enabled);
}

#[test]
fn skill_text_never_reaches_debug_output() {
    let mut i = input("ok");
    i.body = "SECRET-BODY".into();
    i.files[0].content = "SECRET-FILE".into();
    let debug = format!("{i:?}");
    assert!(!debug.contains("SECRET"), "{debug}");
    assert!(debug.contains("references/a.md"));
}
