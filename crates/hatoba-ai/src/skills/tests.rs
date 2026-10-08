//! Skills: `SKILL.md` parsing and rendering, validation, folder and `.zip` import (including
//! hostile input), and `.zip` export.

use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::archive::MAX_ZIP_BYTES;
use super::import::MAX_PATH_DEPTH;
use super::*;

const GOOD: &str = "---\nname: nginx-ops\ndescription: Operate nginx. Use when editing nginx configs.\n---\n# Nginx\n\nBody.\n";

fn skill_md(front: &str, body: &str) -> String {
    format!("---\n{front}---\n{body}")
}

fn issue_path(issue: &SkillIssue) -> Option<&str> {
    match issue {
        SkillIssue::FileTooLarge { path, .. } | SkillIssue::UnsafePath(path) => Some(path),
        _ => None,
    }
}

fn package(import: &SkillImport) -> &SkillPackage {
    import.package.as_ref().expect("a package")
}

fn file_paths(pkg: &SkillPackage) -> Vec<&str> {
    pkg.files.iter().map(|f| f.path.as_str()).collect()
}

fn sample_package() -> SkillPackage {
    let mut frontmatter = Map::new();
    frontmatter.insert("allowed-tools".into(), json!(["Bash(ls:*)", "Read"]));
    frontmatter.insert("metadata".into(), json!({"author": "kc", "version": 2}));
    SkillPackage {
        name: "nginx-ops".into(),
        description: "Operate nginx: use when the user edits a config.".into(),
        frontmatter,
        body: "# Nginx\n\nSee references/tuning.md.\n".into(),
        files: vec![
            SkillFileData {
                path: "references/tuning.md".into(),
                content: "worker_processes auto;\n".into(),
            },
            SkillFileData {
                path: "scripts/reload.sh".into(),
                content: "#!/bin/sh\nnginx -s reload\n".into(),
            },
        ],
    }
}

// ---------------------------------------------------------------------------------------------
// Names, parsing and rendering
// ---------------------------------------------------------------------------------------------

#[test]
fn names_are_one_to_sixty_four_of_lowercase_digits_and_hyphens() {
    for ok in ["a", "nginx-ops", "a1-b2", "-", "0", &"a".repeat(64)] {
        assert!(valid_name(ok), "{ok}");
    }
    for bad in [
        "",
        "Nginx",
        "nginx_ops",
        "nginx ops",
        "nginx.ops",
        "ngínx",
        "../x",
        "a/b",
        &"a".repeat(65),
    ] {
        assert!(!valid_name(bad), "{bad}");
    }
}

#[test]
fn parse_splits_the_frontmatter_from_the_body() {
    let (fields, body) = parse_skill_md(GOOD).unwrap();
    assert_eq!(fields["name"], "nginx-ops");
    assert_eq!(
        fields["description"],
        "Operate nginx. Use when editing nginx configs."
    );
    assert_eq!(body, "# Nginx\n\nBody.\n");
}

#[test]
fn parse_keeps_the_body_exactly_including_blank_lines_and_dashes() {
    let text = skill_md(
        "name: a\ndescription: b\n",
        "\n\n# Title\n\n---\n\ntext without newline",
    );
    let (_, body) = parse_skill_md(&text).unwrap();
    assert_eq!(body, "\n\n# Title\n\n---\n\ntext without newline");
}

#[test]
fn parse_understands_crlf_and_a_byte_order_mark() {
    let text = "\u{feff}---\r\nname: a\r\ndescription: Does things.\r\n---\r\nBody\r\nmore\r\n";
    let (fields, body) = parse_skill_md(text).unwrap();
    assert_eq!(fields["name"], "a");
    assert_eq!(fields["description"], "Does things.");
    assert_eq!(body, "Body\r\nmore\r\n");
}

#[test]
fn parse_converts_yaml_fields_to_json() {
    let front = "name: a\ndescription: b\nallowed-tools:\n  - Bash(git:*)\n  - Read\n\
                 license: MIT\nmetadata:\n  version: 1.5\n  tags: [x, y]\n  on: true\n";
    let (fields, _) = parse_skill_md(&skill_md(front, "")).unwrap();
    assert_eq!(fields["allowed-tools"], json!(["Bash(git:*)", "Read"]));
    assert_eq!(fields["license"], "MIT");
    assert_eq!(
        fields["metadata"],
        json!({"version": 1.5, "tags": ["x", "y"], "on": true})
    );
}

#[test]
fn parse_reads_folded_and_literal_descriptions() {
    let front = "name: a\ndescription: >\n  First line\n  second line\nnotes: |\n  keep\n  lines\n";
    let (fields, _) = parse_skill_md(&skill_md(front, "")).unwrap();
    assert_eq!(fields["description"], "First line second line\n");
    assert_eq!(fields["notes"], "keep\nlines\n");
}

#[test]
fn parse_rejects_a_missing_unclosed_or_broken_frontmatter() {
    let cases = [
        "",
        "# No frontmatter\n",
        "\n---\nname: a\n---\n",
        "---\nname: a\ndescription: b\n",
        "---\n- just\n- a list\n---\n",
        "---\njust text\n---\n",
        "---\nname: [unclosed\n---\n",
        "---\nname: a\n  bad: indent\n---\n",
    ];
    for text in cases {
        assert!(
            matches!(parse_skill_md(text), Err(SkillIssue::InvalidFrontmatter(_))),
            "{text:?}"
        );
    }
}

#[test]
fn parse_names_the_line_of_a_yaml_error() {
    let text = skill_md("name: a\ndescription: Use when: x\n", "body\n");
    let Err(SkillIssue::InvalidFrontmatter(message)) = parse_skill_md(&text) else {
        panic!("expected an invalid frontmatter");
    };
    assert!(message.ends_with("(line 3)"), "{message}");
}

#[test]
fn parse_treats_an_empty_frontmatter_as_no_fields() {
    for front in ["", "\n", "# only a comment\n"] {
        let (fields, body) = parse_skill_md(&skill_md(front, "text\n")).unwrap();
        assert!(fields.is_empty(), "{front:?}");
        assert_eq!(body, "text\n");
    }
}

#[test]
fn parse_survives_yaml_aliases_that_multiply() {
    let mut front = String::from("name: a\ndescription: b\nl0: &l0 [x, x, x, x, x, x, x, x, x]\n");
    for i in 1..12 {
        let refs = vec![format!("*l{}", i - 1); 9].join(", ");
        front.push_str(&format!("l{i}: &l{i} [{refs}]\n"));
    }
    // Either result is fine; what matters is that it ends quickly and does not exhaust memory.
    let _ = parse_skill_md(&skill_md(&front, ""));
}

#[test]
fn render_writes_name_and_description_before_the_other_fields() {
    let text = render_skill_md(&sample_package());
    assert!(
        text.starts_with("---\nname: nginx-ops\ndescription: "),
        "{text}"
    );
    assert!(
        text.ends_with("---\n# Nginx\n\nSee references/tuning.md.\n"),
        "{text}"
    );
    let name = text.find("name:").unwrap();
    let description = text.find("description:").unwrap();
    let tools = text.find("allowed-tools:").unwrap();
    assert!(name < description && description < tools);
}

#[test]
fn render_then_parse_gives_back_the_fields_and_the_body() {
    let descriptions = [
        "Operate nginx: use when the user edits a config.",
        "says \"deploy\" and 'ship' # not a comment",
        "line one\nline two",
        "trailing colon:",
        "yes",
        "123",
        "- looks like a list",
        "@at and `tick` and {braces} and [brackets]",
        "ünïcödé — 日本語 ✓",
        "key: value",
    ];
    for description in descriptions {
        let mut pkg = sample_package();
        pkg.description = description.to_owned();
        pkg.body = "\n  indented first line\r\nCRLF\r\n".to_owned();
        let (mut fields, body) = parse_skill_md(&render_skill_md(&pkg)).unwrap();
        assert_eq!(
            fields.remove("name"),
            Some(json!("nginx-ops")),
            "{description:?}"
        );
        assert_eq!(
            fields.remove("description"),
            Some(json!(description)),
            "{description:?}"
        );
        assert_eq!(fields, pkg.frontmatter, "{description:?}");
        assert_eq!(body, pkg.body, "{description:?}");
    }
}

#[test]
fn render_ignores_name_and_description_inside_the_frontmatter_map() {
    let mut pkg = sample_package();
    pkg.frontmatter.insert("name".into(), json!("stale"));
    pkg.frontmatter.insert("description".into(), json!("stale"));
    let text = render_skill_md(&pkg);
    assert_eq!(text.matches("name:").count(), 1, "{text}");
    assert!(!text.contains("stale"), "{text}");
}

// ---------------------------------------------------------------------------------------------
// validate
// ---------------------------------------------------------------------------------------------

#[test]
fn a_sound_package_has_no_issue() {
    assert_eq!(validate(&sample_package()), vec![]);
}

#[test]
fn validate_checks_the_name() {
    for name in ["", "Bad", "has space", &"a".repeat(65)] {
        let mut pkg = sample_package();
        pkg.name = name.to_owned();
        assert_eq!(
            validate(&pkg),
            vec![SkillIssue::InvalidName(name.to_owned())]
        );
    }
}

#[test]
fn validate_checks_the_description() {
    let mut pkg = sample_package();
    pkg.description = " \n ".to_owned();
    assert_eq!(validate(&pkg), vec![SkillIssue::MissingDescription]);
    pkg.description = "é".repeat(MAX_DESCRIPTION_CHARS);
    assert_eq!(validate(&pkg), vec![]);
    pkg.description = "é".repeat(MAX_DESCRIPTION_CHARS + 1);
    assert_eq!(
        validate(&pkg),
        vec![SkillIssue::DescriptionTooLong(MAX_DESCRIPTION_CHARS + 1)]
    );
}

#[test]
fn validate_checks_sizes_to_the_byte() {
    let mut pkg = sample_package();
    pkg.body = "a".repeat(MAX_FILE_BYTES);
    pkg.files[0].content = "b".repeat(MAX_FILE_BYTES);
    assert_eq!(validate(&pkg), vec![]);
    pkg.body.push('a');
    pkg.files[0].content.push('b');
    assert_eq!(
        validate(&pkg),
        vec![
            SkillIssue::FileTooLarge {
                path: "SKILL.md".into(),
                size: MAX_FILE_BYTES as u64 + 1
            },
            SkillIssue::FileTooLarge {
                path: "references/tuning.md".into(),
                size: MAX_FILE_BYTES as u64 + 1
            },
        ]
    );
}

#[test]
fn validate_checks_the_file_paths() {
    for path in [
        "SKILL.md",
        "../x.md",
        "/etc/passwd",
        "C:/x.md",
        "a\\b.md",
        "./a.md",
        "a//b.md",
        "a/../b.md",
        "",
        "a/\0b",
        "references/tuning.md",
    ] {
        let mut pkg = sample_package();
        pkg.files.push(SkillFileData {
            path: path.to_owned(),
            content: String::new(),
        });
        // The last path duplicates the first file; every other one is unsafe.
        assert_eq!(
            validate(&pkg),
            vec![SkillIssue::UnsafePath(path.to_owned())],
            "{path:?}"
        );
    }
}

#[test]
fn validate_counts_files_and_total_size() {
    let mut pkg = sample_package();
    pkg.files = (0..MAX_FILES - 1)
        .map(|i| SkillFileData {
            path: format!("f/{i}.md"),
            content: String::new(),
        })
        .collect();
    assert_eq!(validate(&pkg), vec![]);
    pkg.files.push(SkillFileData {
        path: "f/last.md".into(),
        content: String::new(),
    });
    assert_eq!(
        validate(&pkg),
        vec![SkillIssue::TooManyFiles(MAX_FILES + 1)]
    );

    let mut pkg = sample_package();
    pkg.files = (0..170)
        .map(|i| SkillFileData {
            path: format!("f/{i}.md"),
            content: "x".repeat(MAX_FILE_BYTES),
        })
        .collect();
    let total = 170 * MAX_FILE_BYTES as u64 + pkg.body.len() as u64;
    assert_eq!(validate(&pkg), vec![SkillIssue::TooLarge(total)]);
}

// ---------------------------------------------------------------------------------------------
// Folder import
// ---------------------------------------------------------------------------------------------

fn write_file(root: &Path, rel: &str, data: &[u8]) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}

fn folder(files: &[(&str, &[u8])]) -> TempDir {
    let dir = TempDir::new().unwrap();
    for (rel, data) in files {
        write_file(dir.path(), rel, data);
    }
    dir
}

fn import_folder(files: &[(&str, &[u8])]) -> SkillImport {
    let dir = folder(files);
    read_folder(dir.path()).unwrap()
}

#[test]
fn a_valid_folder_imports_with_its_text_files() {
    let import = import_folder(&[
        ("SKILL.md", GOOD.as_bytes()),
        ("references/nginx.md", b"# Nginx notes\n"),
        ("references/deep/more.md", b"more\n"),
        ("scripts/reload.sh", b"#!/bin/sh\nnginx -s reload\n"),
        ("LICENSE", b"MIT\n"),
    ]);
    assert_eq!(import.issues, vec![]);
    assert_eq!(import.skipped, vec![]);
    assert!(import.is_importable());
    let pkg = package(&import);
    assert_eq!(pkg.name, "nginx-ops");
    assert_eq!(
        pkg.description,
        "Operate nginx. Use when editing nginx configs."
    );
    assert_eq!(pkg.body, "# Nginx\n\nBody.\n");
    assert!(pkg.frontmatter.is_empty());
    assert_eq!(
        file_paths(pkg),
        [
            "LICENSE",
            "references/deep/more.md",
            "references/nginx.md",
            "scripts/reload.sh"
        ]
    );
    assert_eq!(pkg.files[2].content, "# Nginx notes\n");
    assert_eq!(validate(pkg), vec![]);
}

#[test]
fn a_folder_with_a_single_skill_subfolder_imports_that_skill() {
    let import = import_folder(&[
        ("nginx-ops/SKILL.md", GOOD.as_bytes()),
        ("nginx-ops/references/a.md", b"a\n"),
        ("README.md", b"outside the skill\n"),
        ("other/notes.md", b"outside the skill\n"),
    ]);
    assert_eq!(import.issues, vec![]);
    assert_eq!(file_paths(package(&import)), ["references/a.md"]);
}

#[test]
fn the_skill_at_the_root_wins_over_nested_ones() {
    let import = import_folder(&[
        ("SKILL.md", GOOD.as_bytes()),
        ("sub/SKILL.md", GOOD.as_bytes()),
    ]);
    assert_eq!(import.issues, vec![]);
    assert_eq!(file_paths(package(&import)), ["sub/SKILL.md"]);
}

#[test]
fn a_folder_without_one_clear_skill_md_is_rejected() {
    for files in [
        vec![("README.md", &b"x"[..])],
        vec![("a/b/SKILL.md", GOOD.as_bytes())],
        vec![
            ("a/SKILL.md", GOOD.as_bytes()),
            ("b/SKILL.md", GOOD.as_bytes()),
        ],
        vec![],
    ] {
        let import = import_folder(&files);
        assert_eq!(import.issues, vec![SkillIssue::MissingSkillMd]);
        assert!(import.package.is_none());
        assert!(!import.is_importable());
    }
}

#[test]
fn a_missing_or_non_folder_path_is_an_error() {
    let dir = folder(&[("file.txt", b"x")]);
    assert!(matches!(
        read_folder(&dir.path().join("nope")),
        Err(SkillError::Io(_))
    ));
    assert!(matches!(
        read_folder(&dir.path().join("file.txt")),
        Err(SkillError::Io(_))
    ));
}

#[test]
fn the_name_and_description_rules_are_listed_together() {
    let long = "d".repeat(MAX_DESCRIPTION_CHARS + 1);
    let cases: Vec<(String, Vec<SkillIssue>)> = vec![
        (
            "name: Bad_Name\ndescription: ok\n".into(),
            vec![SkillIssue::InvalidName("Bad_Name".into())],
        ),
        (
            "description: ok\n".into(),
            vec![SkillIssue::InvalidName(String::new())],
        ),
        (
            "name: 123abc!\ndescription: ok\n".into(),
            vec![SkillIssue::InvalidName("123abc!".into())],
        ),
        ("name: a\n".into(), vec![SkillIssue::MissingDescription]),
        (
            "name: a\ndescription:\n".into(),
            vec![SkillIssue::MissingDescription],
        ),
        (
            "name: a\ndescription: '  '\n".into(),
            vec![SkillIssue::MissingDescription],
        ),
        (
            format!("name: a\ndescription: {long}\n"),
            vec![SkillIssue::DescriptionTooLong(MAX_DESCRIPTION_CHARS + 1)],
        ),
        (
            "name: Bad\n".into(),
            vec![
                SkillIssue::InvalidName("Bad".into()),
                SkillIssue::MissingDescription,
            ],
        ),
        (
            "name: a\ndescription: [x]\n".into(),
            vec![SkillIssue::InvalidFrontmatter(
                "`description` must be text".into(),
            )],
        ),
    ];
    for (front, issues) in cases {
        let import = import_folder(&[("SKILL.md", skill_md(&front, "body\n").as_bytes())]);
        assert_eq!(import.issues, issues, "{front}");
        // The package is kept so that it can be shown (and its name fixed).
        assert!(import.package.is_some(), "{front}");
        assert!(!import.is_importable(), "{front}");
    }
}

#[test]
fn a_name_that_is_not_text_is_invalid() {
    let import = import_folder(&[(
        "SKILL.md",
        skill_md("name: 123\ndescription: ok\n", "").as_bytes(),
    )]);
    let [SkillIssue::InvalidName(shown)] = import.issues.as_slice() else {
        panic!("{:?}", import.issues);
    };
    assert!(shown.contains("123"), "{shown}");
}

#[test]
fn the_description_loses_surrounding_whitespace() {
    let front = "name: a\ndescription: >\n  Folded text\n  continues here.\n";
    let import = import_folder(&[("SKILL.md", skill_md(front, "body\n").as_bytes())]);
    assert_eq!(import.issues, vec![]);
    assert_eq!(package(&import).description, "Folded text continues here.");
}

#[test]
fn an_unusable_skill_md_gives_no_package() {
    let too_big = format!("{GOOD}{}", "x".repeat(MAX_FILE_BYTES));
    let cases: Vec<(&[u8], SkillIssue)> = vec![
        (
            b"# nothing\n",
            SkillIssue::InvalidFrontmatter("SKILL.md must start with a `---` line".into()),
        ),
        (
            b"\xff\xfe-\0-\0-\0",
            SkillIssue::InvalidFrontmatter("SKILL.md is not UTF-8 text".into()),
        ),
        (
            b"---\nname: a\x00\n---\n",
            SkillIssue::InvalidFrontmatter("SKILL.md is not UTF-8 text".into()),
        ),
        (
            too_big.as_bytes(),
            SkillIssue::FileTooLarge {
                path: "SKILL.md".into(),
                size: too_big.len() as u64,
            },
        ),
    ];
    for (data, issue) in cases {
        let import = import_folder(&[("SKILL.md", data), ("references/a.md", b"fine\n")]);
        assert_eq!(import.issues, vec![issue]);
        assert!(import.package.is_none());
    }
}

#[test]
fn skill_md_may_be_exactly_the_limit() {
    let mut text = String::from(GOOD);
    text.push_str(&"x".repeat(MAX_FILE_BYTES - GOOD.len()));
    assert_eq!(text.len(), MAX_FILE_BYTES);
    let import = import_folder(&[("SKILL.md", text.as_bytes())]);
    assert_eq!(import.issues, vec![]);
}

#[test]
fn skill_md_in_crlf_with_a_byte_order_mark_imports() {
    let text = "\u{feff}---\r\nname: crlf-skill\r\ndescription: Windows line endings.\r\nlicense: MIT\r\n---\r\n# Title\r\n\r\nBody\r\n";
    let import = import_folder(&[("SKILL.md", text.as_bytes())]);
    assert_eq!(import.issues, vec![]);
    let pkg = package(&import);
    assert_eq!(pkg.name, "crlf-skill");
    assert_eq!(pkg.description, "Windows line endings.");
    assert_eq!(pkg.frontmatter["license"], "MIT");
    assert_eq!(pkg.body, "# Title\r\n\r\nBody\r\n");
}

#[test]
fn other_frontmatter_fields_are_kept_for_export() {
    let front = "name: a\ndescription: b\nallowed-tools: Bash(git:*), Read\nmetadata:\n  author: kc\n  tags: [x, y]\n";
    let import = import_folder(&[("SKILL.md", skill_md(front, "body\n").as_bytes())]);
    assert_eq!(import.issues, vec![]);
    let pkg = package(&import);
    assert_eq!(pkg.frontmatter.len(), 2);
    assert_eq!(pkg.frontmatter["allowed-tools"], "Bash(git:*), Read");
    assert_eq!(
        pkg.frontmatter["metadata"],
        json!({"author": "kc", "tags": ["x", "y"]})
    );
}

#[test]
fn files_that_are_not_text_are_skipped_and_listed() {
    let big_binary = vec![0xffu8; 100 * 1024];
    let mut latin1 = b"caf".to_vec();
    latin1.push(0xe9);
    let import = import_folder(&[
        ("SKILL.md", GOOD.as_bytes()),
        ("assets/logo.png", b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"),
        ("assets/big.bin", &big_binary),
        ("assets/with-nul.txt", b"looks like text\0but is not"),
        ("references/latin1.md", &latin1),
        ("references/utf16.md", b"\xff\xfeh\0i\0"),
        ("references/ok.md", "fine é\n".as_bytes()),
    ]);
    assert_eq!(import.issues, vec![]);
    assert_eq!(file_paths(package(&import)), ["references/ok.md"]);
    let skipped: Vec<_> = import.skipped.iter().map(|s| s.path.as_str()).collect();
    assert_eq!(
        skipped,
        [
            "assets/big.bin",
            "assets/logo.png",
            "assets/with-nul.txt",
            "references/latin1.md",
            "references/utf16.md"
        ]
    );
    assert!(
        import
            .skipped
            .iter()
            .all(|s| s.reason == SkipReason::NotText)
    );
}

#[test]
fn text_files_over_the_limit_are_rejected_to_the_byte() {
    let at_limit = "a".repeat(MAX_FILE_BYTES);
    let over = "a".repeat(MAX_FILE_BYTES + 1);
    // Two bytes of "é" straddle the end of what is read, so the cut file is not valid UTF-8.
    let straddling = format!("{}é", "a".repeat(MAX_FILE_BYTES));
    let multibyte = format!("{}é", "a".repeat(MAX_FILE_BYTES - 1));
    let huge = "a".repeat(2 * MAX_FILE_BYTES);
    let import = import_folder(&[
        ("SKILL.md", GOOD.as_bytes()),
        ("references/at-limit.md", at_limit.as_bytes()),
        ("references/over.md", over.as_bytes()),
        ("references/straddling.md", straddling.as_bytes()),
        ("references/multibyte.md", multibyte.as_bytes()),
        ("references/huge.md", huge.as_bytes()),
    ]);
    assert_eq!(file_paths(package(&import)), ["references/at-limit.md"]);
    assert_eq!(
        import.issues,
        vec![
            SkillIssue::FileTooLarge {
                path: "references/huge.md".into(),
                size: huge.len() as u64
            },
            SkillIssue::FileTooLarge {
                path: "references/multibyte.md".into(),
                size: multibyte.len() as u64
            },
            SkillIssue::FileTooLarge {
                path: "references/over.md".into(),
                size: over.len() as u64
            },
            SkillIssue::FileTooLarge {
                path: "references/straddling.md".into(),
                size: straddling.len() as u64
            },
        ]
    );
}

#[test]
fn a_leading_byte_order_mark_is_dropped_from_files() {
    let import = import_folder(&[
        ("SKILL.md", GOOD.as_bytes()),
        ("references/bom.md", "\u{feff}# Title\n".as_bytes()),
    ]);
    assert_eq!(package(&import).files[0].content, "# Title\n");
}

#[test]
fn os_and_version_control_junk_is_ignored() {
    let import = import_folder(&[
        ("SKILL.md", GOOD.as_bytes()),
        (".DS_Store", b"\0\0\0\x01Bud1"),
        ("references/.DS_Store", b"x"),
        ("Thumbs.db", b"x"),
        ("references/Thumbs.db", b"x"),
        ("__MACOSX/._SKILL.md", b"x"),
        (".git/HEAD", b"ref: refs/heads/main\n"),
        (".git/hooks/pre-commit.sample", b"#!/bin/sh\n"),
        ("references/a.md", b"a\n"),
    ]);
    assert_eq!(import.issues, vec![]);
    assert_eq!(import.skipped, vec![]);
    assert_eq!(file_paths(package(&import)), ["references/a.md"]);
}

#[test]
fn too_many_files_are_rejected_without_reading_them() {
    let dir = folder(&[("SKILL.md", GOOD.as_bytes())]);
    for i in 0..MAX_FILES {
        write_file(dir.path(), &format!("references/{i:03}.md"), b"x\n");
    }
    let import = read_folder(dir.path()).unwrap();
    assert_eq!(import.issues, vec![SkillIssue::TooManyFiles(MAX_FILES + 1)]);
    assert_eq!(package(&import).name, "nginx-ops");
    assert!(package(&import).files.is_empty());

    fs::remove_file(dir.path().join("references/000.md")).unwrap();
    let import = read_folder(dir.path()).unwrap();
    assert_eq!(import.issues, vec![]);
    assert_eq!(package(&import).files.len(), MAX_FILES - 1);
}

#[test]
fn a_huge_tree_stops_the_walk() {
    let dir = folder(&[("SKILL.md", GOOD.as_bytes())]);
    for i in 0..1_100 {
        fs::create_dir(dir.path().join(format!("d{i:04}"))).unwrap();
    }
    let import = read_folder(dir.path()).unwrap();
    let [SkillIssue::TooManyFiles(seen)] = import.issues.as_slice() else {
        panic!("{:?}", import.issues);
    };
    assert!(*seen > 1_000, "{seen}");
    assert!(package(&import).files.is_empty());
}

#[test]
fn files_together_over_the_total_limit_are_rejected() {
    let dir = folder(&[("SKILL.md", GOOD.as_bytes())]);
    let data = "x".repeat(MAX_FILE_BYTES);
    for i in 0..170 {
        write_file(
            dir.path(),
            &format!("references/{i:03}.md"),
            data.as_bytes(),
        );
    }
    let import = read_folder(dir.path()).unwrap();
    let total = 170 * MAX_FILE_BYTES as u64 + GOOD.len() as u64;
    assert_eq!(import.issues, vec![SkillIssue::TooLarge(total)]);
    assert!(package(&import).files.is_empty());
}

#[test]
fn folders_nested_too_deep_are_reported() {
    let ok = format!("{}ok.md", "d/".repeat(MAX_PATH_DEPTH - 1));
    let deep = format!("{}deep.md", "d/".repeat(MAX_PATH_DEPTH));
    let import = import_folder(&[
        ("SKILL.md", GOOD.as_bytes()),
        (
            &ok, b"ok
",
        ),
        (
            &deep, b"deep
",
        ),
    ]);
    assert_eq!(file_paths(package(&import)), [ok.as_str()]);
    assert_eq!(import.issues.len(), 1, "{:?}", import.issues);
    assert!(matches!(&import.issues[0], SkillIssue::UnsafePath(p) if deep.starts_with(p.as_str())));
}

fn make_symlink(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    let result = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let result = if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    };
    #[cfg(not(any(unix, windows)))]
    let result: std::io::Result<()> = Err(std::io::ErrorKind::Unsupported.into());
    result.is_ok()
}

#[test]
fn symbolic_links_are_not_followed() {
    let outside = folder(&[("secret.txt", b"top secret\n"), ("dir/more.txt", b"more\n")]);
    let dir = folder(&[("SKILL.md", GOOD.as_bytes()), ("references/a.md", b"a\n")]);
    let file_link = make_symlink(
        &outside.path().join("secret.txt"),
        &dir.path().join("references/link.md"),
    );
    let dir_link = make_symlink(outside.path(), &dir.path().join("linked-dir"));
    if !file_link && !dir_link {
        eprintln!("symbolic links cannot be created here; skipping");
        return;
    }
    let import = read_folder(dir.path()).unwrap();
    assert_eq!(import.issues, vec![]);
    assert_eq!(import.skipped, vec![]);
    assert_eq!(file_paths(package(&import)), ["references/a.md"]);
}

#[test]
fn a_symbolic_link_as_skill_md_is_not_a_skill() {
    let outside = folder(&[("SKILL.md", GOOD.as_bytes())]);
    let dir = folder(&[("references/a.md", b"a\n")]);
    if !make_symlink(
        &outside.path().join("SKILL.md"),
        &dir.path().join("SKILL.md"),
    ) {
        eprintln!("symbolic links cannot be created here; skipping");
        return;
    }
    let import = read_folder(dir.path()).unwrap();
    assert_eq!(import.issues, vec![SkillIssue::MissingSkillMd]);
}

// ---------------------------------------------------------------------------------------------
// Zip import
// ---------------------------------------------------------------------------------------------

fn deflated() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)
}

fn stored() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(CompressionMethod::Stored)
}

fn zip_bytes_with(options: SimpleFileOptions, entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in entries {
        writer.start_file(*name, options).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    zip_bytes_with(deflated(), entries)
}

fn zip_at(dir: &TempDir, bytes: &[u8]) -> PathBuf {
    let path = dir.path().join("skill.zip");
    fs::write(&path, bytes).unwrap();
    path
}

fn read_zip_bytes(bytes: &[u8]) -> Result<SkillImport, SkillError> {
    let dir = TempDir::new().unwrap();
    read_zip(&zip_at(&dir, bytes))
}

fn import_zip(entries: &[(&str, &[u8])]) -> SkillImport {
    read_zip_bytes(&zip_bytes(entries)).unwrap()
}

/// Positions of `signature` in `bytes`.
fn find_all(bytes: &[u8], signature: &[u8]) -> Vec<usize> {
    (0..bytes.len().saturating_sub(signature.len() - 1))
        .filter(|&i| bytes[i..].starts_with(signature))
        .collect()
}

/// Rewrites a field of every central directory header (`offset` from its start) and, when given,
/// of the local header (`local_offset`).
fn patch_zip(bytes: &mut [u8], offset: usize, local_offset: Option<usize>, value: &[u8]) {
    for at in find_all(bytes, b"PK\x01\x02") {
        bytes[at + offset..at + offset + value.len()].copy_from_slice(value);
    }
    if let Some(local) = local_offset {
        for at in find_all(bytes, b"PK\x03\x04") {
            bytes[at + local..at + local + value.len()].copy_from_slice(value);
        }
    }
}

#[test]
fn a_zip_with_skill_md_at_the_root_imports() {
    let import = import_zip(&[
        ("SKILL.md", GOOD.as_bytes()),
        ("references/nginx.md", b"# Nginx\n"),
        ("assets/", b""),
        ("assets/logo.png", b"\x89PNG\0"),
    ]);
    assert_eq!(import.issues, vec![]);
    let pkg = package(&import);
    assert_eq!(pkg.name, "nginx-ops");
    assert_eq!(file_paths(pkg), ["references/nginx.md"]);
    assert_eq!(
        import.skipped,
        vec![SkippedFile {
            path: "assets/logo.png".into(),
            reason: SkipReason::NotText
        }]
    );
}

#[test]
fn a_zip_wrapped_in_one_folder_imports_without_the_folder() {
    let import = import_zip(&[
        ("nginx-ops/", b""),
        ("nginx-ops/SKILL.md", GOOD.as_bytes()),
        ("nginx-ops/references/nginx.md", b"# Nginx\n"),
        ("__MACOSX/nginx-ops/._SKILL.md", b"junk"),
        ("__MACOSX/._nginx-ops", b"junk"),
        ("nginx-ops/.DS_Store", b"junk"),
        ("README.txt", b"outside the folder\n"),
    ]);
    assert_eq!(import.issues, vec![]);
    assert_eq!(import.skipped, vec![]);
    assert_eq!(file_paths(package(&import)), ["references/nginx.md"]);
}

#[test]
fn a_zip_without_one_clear_skill_md_is_rejected() {
    for entries in [
        vec![("README.md", &b"x"[..])],
        vec![("a/b/SKILL.md", GOOD.as_bytes())],
        vec![
            ("a/SKILL.md", GOOD.as_bytes()),
            ("b/SKILL.md", GOOD.as_bytes()),
        ],
        vec![("skill.md", GOOD.as_bytes())],
        vec![("SKILL.md/", b"")],
    ] {
        let import = import_zip(&entries);
        assert_eq!(import.issues, vec![SkillIssue::MissingSkillMd]);
        assert!(import.package.is_none());
    }
}

#[test]
fn every_issue_of_a_zip_is_listed() {
    let big = "a".repeat(MAX_FILE_BYTES + 1);
    let import = import_zip(&[
        ("SKILL.md", "---\nname: Bad Name\n---\nbody\n".as_bytes()),
        ("references/big.md", big.as_bytes()),
        ("../outside.md", b"x"),
    ]);
    assert_eq!(
        import.issues,
        vec![
            SkillIssue::UnsafePath("../outside.md".into()),
            SkillIssue::InvalidName("Bad Name".into()),
            SkillIssue::MissingDescription,
            SkillIssue::FileTooLarge {
                path: "references/big.md".into(),
                size: big.len() as u64
            },
        ]
    );
}

#[test]
fn unsafe_zip_paths_are_rejected() {
    let hostile = [
        "../evil.md",
        "references/../../evil.md",
        "..\\evil.md",
        "references\\..\\..\\evil.md",
        "/etc/passwd",
        "\\\\server\\share\\evil.md",
        "C:\\Windows\\evil.md",
        "C:/evil.md",
        "c:evil.md",
        "references/evil\0.md",
        "references/evil\n.md",
    ];
    for name in hostile {
        let import = import_zip(&[("SKILL.md", GOOD.as_bytes()), (name, b"x")]);
        assert_eq!(import.issues.len(), 1, "{name:?}: {:?}", import.issues);
        assert!(
            matches!(&import.issues[0], SkillIssue::UnsafePath(_)),
            "{name:?}"
        );
        assert!(!import.is_importable());
        assert!(
            package(&import)
                .files
                .iter()
                .all(|f| !f.path.contains("evil")),
            "{name:?}"
        );
    }
}

#[test]
fn a_hostile_entry_outside_the_skill_folder_still_rejects_the_zip() {
    let import = import_zip(&[
        ("skill/SKILL.md", GOOD.as_bytes()),
        (
            "../../../home/user/.ssh/authorized_keys",
            b"ssh-ed25519 AAAA",
        ),
    ]);
    assert_eq!(import.issues.len(), 1);
    assert!(matches!(&import.issues[0], SkillIssue::UnsafePath(_)));
}

#[test]
fn paths_that_collide_after_normalizing_are_rejected() {
    let import = import_zip(&[
        ("SKILL.md", GOOD.as_bytes()),
        ("references/a.md", b"one"),
        ("references//a.md", b"two"),
        ("./references/a.md", b"three"),
        ("references\\a.md", b"four"),
    ]);
    assert_eq!(import.issues.len(), 3);
    assert!(
        import
            .issues
            .iter()
            .all(|i| issue_path(i) == Some("references/a.md"))
    );
    assert_eq!(package(&import).files[0].content, "one");
}

#[test]
fn skill_md_cannot_hide_in_the_files() {
    let import = import_zip(&[("SKILL.md", GOOD.as_bytes()), ("./SKILL.md", b"other")]);
    assert_eq!(
        import.issues,
        vec![SkillIssue::UnsafePath("SKILL.md".into())]
    );
}

#[test]
fn symbolic_link_entries_and_directories_are_ignored() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file("SKILL.md", deflated()).unwrap();
    writer.write_all(GOOD.as_bytes()).unwrap();
    writer
        .add_symlink("references/passwd.md", "/etc/passwd", deflated())
        .unwrap();
    writer
        .add_symlink("references/up.md", "../../outside", deflated())
        .unwrap();
    writer.add_directory("references/dir/", deflated()).unwrap();
    writer.start_file("references/a.md", deflated()).unwrap();
    writer.write_all(b"a\n").unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    let import = read_zip_bytes(&bytes).unwrap();
    assert_eq!(import.issues, vec![]);
    assert_eq!(import.skipped, vec![]);
    assert_eq!(file_paths(package(&import)), ["references/a.md"]);
}

#[test]
fn a_zip_bomb_that_declares_its_size_is_rejected_without_reading_it() {
    let bomb = vec![b'a'; 64 * 1024 * 1024];
    let bytes = zip_bytes(&[("SKILL.md", GOOD.as_bytes()), ("references/bomb.md", &bomb)]);
    assert!(bytes.len() < 256 * 1024, "{}", bytes.len());
    let import = read_zip_bytes(&bytes).unwrap();
    assert_eq!(
        import.issues,
        vec![SkillIssue::TooLarge(bomb.len() as u64 + GOOD.len() as u64)]
    );
    assert!(package(&import).files.is_empty());
    assert!(import.skipped.is_empty());
}

#[test]
fn a_zip_bomb_that_lies_about_its_size_is_cut_at_the_file_limit() {
    let text_bomb = vec![b'a'; 8 * 1024 * 1024];
    let mut bytes = zip_bytes(&[
        ("SKILL.md", GOOD.as_bytes()),
        ("references/bomb.md", &text_bomb),
    ]);
    // Central directory, uncompressed size: claim 10 bytes for every entry.
    patch_zip(&mut bytes, 24, None, &10u32.to_le_bytes());
    let import = read_zip_bytes(&bytes).unwrap();
    // The bytes that were read decide; SKILL.md only claims to be smaller than it is.
    assert_eq!(
        import.issues,
        vec![SkillIssue::FileTooLarge {
            path: "references/bomb.md".into(),
            size: MAX_FILE_BYTES as u64 + 1
        }]
    );
    assert!(package(&import).files.is_empty());
}

#[test]
fn a_zip_bomb_of_zeros_that_lies_about_its_size_is_skipped_as_not_text() {
    let zeros = vec![0u8; 8 * 1024 * 1024];
    let mut bytes = zip_bytes(&[("SKILL.md", GOOD.as_bytes()), ("assets/zeros.bin", &zeros)]);
    patch_zip(&mut bytes, 24, None, &10u32.to_le_bytes());
    let import = read_zip_bytes(&bytes).unwrap();
    assert_eq!(import.issues, vec![]);
    assert_eq!(import.skipped.len(), 1);
}

#[test]
fn a_zip_with_many_small_entries_is_rejected() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file("SKILL.md", stored()).unwrap();
    writer.write_all(GOOD.as_bytes()).unwrap();
    for i in 0..MAX_FILES {
        writer
            .start_file(format!("references/{i:03}.md"), stored())
            .unwrap();
        writer.write_all(b"x\n").unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    let import = read_zip_bytes(&bytes).unwrap();
    assert_eq!(import.issues, vec![SkillIssue::TooManyFiles(MAX_FILES + 1)]);
    assert_eq!(package(&import).name, "nginx-ops");
    assert!(package(&import).files.is_empty());

    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file("SKILL.md", stored()).unwrap();
    writer.write_all(GOOD.as_bytes()).unwrap();
    for i in 0..2_000 {
        writer
            .start_file(format!("references/{i:04}.md"), stored())
            .unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    let import = read_zip_bytes(&bytes).unwrap();
    assert_eq!(import.issues, vec![SkillIssue::TooManyFiles(2_001)]);
    assert!(import.package.is_none());
}

#[test]
fn junk_entries_do_not_count_as_files() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file("SKILL.md", stored()).unwrap();
    writer.write_all(GOOD.as_bytes()).unwrap();
    for i in 0..400 {
        writer
            .start_file(format!("__MACOSX/._{i}"), stored())
            .unwrap();
        writer.add_directory(format!("dir{i}/"), stored()).unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    let import = read_zip_bytes(&bytes).unwrap();
    assert_eq!(import.issues, vec![]);
}

#[test]
fn an_entry_with_an_unsupported_method_is_skipped() {
    let mut bytes = zip_bytes_with(
        stored(),
        &[
            ("SKILL.md", GOOD.as_bytes()),
            ("references/odd.md", b"text that cannot be unpacked"),
        ],
    );
    // Entries are in order: SKILL.md first. Give only the second a method nobody supports.
    let central = find_all(&bytes, b"PK\x01\x02");
    let local = find_all(&bytes, b"PK\x03\x04");
    bytes[central[1] + 10..central[1] + 12].copy_from_slice(&17u16.to_le_bytes());
    bytes[local[1] + 8..local[1] + 10].copy_from_slice(&17u16.to_le_bytes());
    let import = read_zip_bytes(&bytes).unwrap();
    assert_eq!(import.issues, vec![]);
    assert_eq!(
        import.skipped,
        vec![SkippedFile {
            path: "references/odd.md".into(),
            reason: SkipReason::NotText
        }]
    );
}

#[test]
fn a_skill_md_with_an_unsupported_method_is_an_error() {
    let mut bytes = zip_bytes_with(stored(), &[("SKILL.md", GOOD.as_bytes())]);
    patch_zip(&mut bytes, 10, Some(8), &17u16.to_le_bytes());
    assert!(matches!(read_zip_bytes(&bytes), Err(SkillError::Zip(_))));
}

#[test]
fn damaged_archives_are_errors() {
    let good = zip_bytes_with(stored(), &[("SKILL.md", GOOD.as_bytes())]);
    // Not a zip, empty, cut off.
    assert!(matches!(
        read_zip_bytes(b"not a zip at all"),
        Err(SkillError::Zip(_))
    ));
    assert!(matches!(read_zip_bytes(b""), Err(SkillError::Zip(_))));
    assert!(matches!(
        read_zip_bytes(&good[..good.len() / 2]),
        Err(SkillError::Zip(_))
    ));
    // A flipped bit in the stored text fails its checksum.
    let mut corrupt = good.clone();
    let at = find_all(&corrupt, b"nginx-ops")[0];
    corrupt[at] ^= 0x01;
    assert!(matches!(read_zip_bytes(&corrupt), Err(SkillError::Zip(_))));
}

#[test]
fn duplicate_entry_names_in_a_zip_never_import_two_files_under_one_path() {
    let mut bytes = zip_bytes(&[
        ("SKILL.md", GOOD.as_bytes()),
        ("references/a.md", b"one"),
        ("references/b.md", b"two"),
    ]);
    for at in find_all(&bytes, b"references/b.md") {
        bytes[at + 11..at + 15].copy_from_slice(b"a.md");
    }
    match read_zip_bytes(&bytes) {
        Ok(import) => {
            let pkg = package(&import);
            assert!(pkg.files.len() <= 1, "{:?}", file_paths(pkg));
            assert!(pkg.files.len() == 1 || !import.issues.is_empty());
        }
        Err(err) => assert!(matches!(err, SkillError::Zip(_)), "{err:?}"),
    }
}

#[test]
fn a_missing_zip_is_an_io_error() {
    let dir = TempDir::new().unwrap();
    assert!(matches!(
        read_zip(&dir.path().join("nope.zip")),
        Err(SkillError::Io(_))
    ));
}

#[test]
fn an_archive_file_over_the_cap_is_refused() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("big.zip");
    let file = fs::File::create(&path).unwrap();
    file.set_len(MAX_ZIP_BYTES + 1).unwrap();
    assert!(matches!(read_zip(&path), Err(SkillError::Zip(m)) if m.contains("16 MB")));
}

// ---------------------------------------------------------------------------------------------
// Export and round trip
// ---------------------------------------------------------------------------------------------

fn zip_entry_names(path: &Path) -> Vec<String> {
    let archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    archive.file_names().map(str::to_owned).collect()
}

#[test]
fn write_zip_wraps_the_skill_in_a_folder_named_after_it() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("out.zip");
    write_zip(&sample_package(), &path).unwrap();
    assert_eq!(
        zip_entry_names(&path),
        [
            "nginx-ops/SKILL.md",
            "nginx-ops/references/tuning.md",
            "nginx-ops/scripts/reload.sh"
        ]
    );
}

#[test]
fn write_zip_is_deterministic() {
    let dir = TempDir::new().unwrap();
    let (a, b) = (dir.path().join("a.zip"), dir.path().join("b.zip"));
    let mut pkg = sample_package();
    write_zip(&pkg, &a).unwrap();
    pkg.files.reverse();
    write_zip(&pkg, &b).unwrap();
    assert_eq!(fs::read(a).unwrap(), fs::read(b).unwrap());
}

#[test]
fn a_skill_survives_zip_export_and_import() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("out.zip");
    let pkg = sample_package();
    write_zip(&pkg, &path).unwrap();
    let import = read_zip(&path).unwrap();
    assert_eq!(import.issues, vec![]);
    assert_eq!(import.skipped, vec![]);
    assert_eq!(package(&import), &pkg);
}

#[test]
fn a_folder_survives_import_export_and_import() {
    let front = "name: kept\ndescription: >-\n  Keeps\n  everything: even this.\n\
                 allowed-tools: [Bash, Read]\nmetadata:\n  author: kc\n  n: 3\n  nested: {a: [1, 2]}\n";
    let body = "\n# Title\r\n\r\ntext with trailing spaces   \n";
    let import = import_folder(&[
        ("SKILL.md", skill_md(front, body).as_bytes()),
        ("references/z.md", "z é\n".as_bytes()),
        ("references/a.md", b"a\r\n"),
        ("assets/logo.png", b"\x89PNG\0"),
    ]);
    assert_eq!(import.issues, vec![]);
    let first = package(&import).clone();
    assert_eq!(first.description, "Keeps everything: even this.");
    assert_eq!(first.body, body);

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("kept.zip");
    write_zip(&first, &path).unwrap();
    let again = read_zip(&path).unwrap();
    assert_eq!(again.issues, vec![]);
    assert_eq!(package(&again), &first);
    assert_eq!(render_skill_md(package(&again)), render_skill_md(&first));
}

#[test]
fn write_zip_refuses_an_unsafe_skill() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("out.zip");
    let mut pkg = sample_package();
    pkg.name = "../evil".into();
    assert!(matches!(write_zip(&pkg, &path), Err(SkillError::Zip(_))));
    let mut pkg = sample_package();
    pkg.files[0].path = "../evil.md".into();
    assert!(matches!(write_zip(&pkg, &path), Err(SkillError::Zip(_))));
    assert!(!path.exists());
}

#[test]
fn write_zip_exports_what_the_user_already_has_even_over_the_soft_limits() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("out.zip");
    let mut pkg = sample_package();
    pkg.description = "d".repeat(MAX_DESCRIPTION_CHARS + 10);
    pkg.files[0].content = "x".repeat(MAX_FILE_BYTES + 10);
    write_zip(&pkg, &path).unwrap();
    assert_eq!(zip_entry_names(&path).len(), 3);
}

#[test]
fn write_zip_to_a_missing_folder_is_an_io_error() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("missing").join("out.zip");
    assert!(matches!(
        write_zip(&sample_package(), &path),
        Err(SkillError::Io(_))
    ));
}

#[test]
fn imported_values_are_plain_json() {
    let (fields, _) = parse_skill_md(&skill_md(
        "name: a\ndescription: b\nx: .nan\ny: 1e3\nz: ~\n",
        "",
    ))
    .unwrap();
    assert_eq!(fields["x"], Value::Null);
    assert_eq!(fields["y"], json!(1000.0));
    assert_eq!(fields["z"], Value::Null);
}
