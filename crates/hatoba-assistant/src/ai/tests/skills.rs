//! Skills (§13.8): `read_skill` (AI-28) and the `/` commands (AI-50).

use super::*;

#[test]
fn skills_saved_in_settings_are_listed_and_read_by_read_skill() {
    use crate::dto::{SkillFileView, SkillInput};
    use hatoba_ai::tools::ReadSkillArgs;

    let f = Fixture::with_base_url(None, "http://127.0.0.1:9/v1".into());
    let mut v = lock(&f.vault);
    let skill = |name: &str, enabled: bool| SkillInput {
        id: None,
        name: name.into(),
        description: format!("About {name}"),
        enabled,
        body: format!("Body of {name}"),
        files: vec![SkillFileView {
            path: "references/tls.md".into(),
            content: format!("TLS of {name}"),
        }],
    };
    crate::skills::save_skill(&mut v, &skill("nginx", true)).unwrap();
    crate::skills::save_skill(&mut v, &skill("apache", false)).unwrap();

    // AI-28: only enabled skills are listed.
    let (system, tools) = crate::ai::request::prompt(&v, &f.context(), None, None);
    assert!(system.contains("nginx: About nginx"), "{system}");
    assert!(!system.contains("apache"), "{system}");
    assert!(tools.iter().any(|t| t.name == "read_skill"));

    let read = |v: &Vault, name: &str, path: Option<&str>| match crate::ai::skills::read_skill(
        v,
        &ReadSkillArgs {
            name: name.into(),
            path: path.map(str::to_owned),
        },
        VERSION,
    ) {
        crate::ai::job::Job::Ready(status, content) => (status, content),
        _ => panic!("read_skill reads the vault"),
    };
    assert_eq!(
        read(&v, "nginx", None),
        (ToolStatus::Ok, "Body of nginx".into())
    );
    assert_eq!(
        read(&v, "nginx", Some("./references\\tls.md")),
        (ToolStatus::Ok, "TLS of nginx".into())
    );
    assert_eq!(read(&v, "apache", None).0, ToolStatus::Error);

    // AI-34: the built-in skill is listed with them, sorted by name, and read with the version.
    // A user skill from an older build that took its name is shadowed.
    v.put(
        None,
        Item::Skill(Skill {
            name: "hatoba".into(),
            description: "Legacy notes".into(),
            enabled: true,
            ..Skill::default()
        }),
    )
    .unwrap();
    let builtin = hatoba_ai::skills::builtin_skill(VERSION);
    let (system, _) = crate::ai::request::prompt(&v, &f.context(), None, None);
    assert!(
        system.contains(&format!(
            "- hatoba: {}\n- nginx: About nginx\n",
            builtin.description
        )),
        "{system}"
    );
    assert!(!system.contains("Legacy notes"), "{system}");
    let (status, body) = read(&v, "hatoba", None);
    assert_eq!(
        (status, body.as_str()),
        (ToolStatus::Ok, builtin.body.as_str())
    );
    assert!(body.contains(VERSION));
    let file = &builtin.files[0];
    assert_eq!(
        read(&v, "hatoba", Some(&file.path)),
        (ToolStatus::Ok, file.content.clone())
    );
    let (status, missing) = read(&v, "hatoba", Some("nope.md"));
    assert_eq!(status, ToolStatus::Error);
    assert!(missing.contains("references/"), "{missing}");

    // Switched off in Settings → AI: neither listed nor read; the old skill stays shadowed.
    let mut settings = v.settings();
    settings.ai.builtin_skill_enabled = false;
    v.put(Some(SETTINGS_ID), Item::Settings(settings)).unwrap();
    let (system, tools) = crate::ai::request::prompt(&v, &f.context(), None, None);
    assert!(!system.contains("- hatoba:"), "{system}");
    assert!(tools.iter().any(|t| t.name == "read_skill"));
    let (status, refused) = read(&v, "hatoba", None);
    assert_eq!(status, ToolStatus::Error);
    assert!(refused.contains("Enabled skills: nginx."), "{refused}");

    // With no enabled skill at all, read_skill is not offered.
    let nginx = v
        .skills()
        .into_iter()
        .find(|(_, s)| s.name == "nginx")
        .unwrap()
        .0;
    crate::skills::set_skill_enabled(&mut v, &nginx, false).unwrap();
    let (_, tools) = crate::ai::request::prompt(&v, &f.context(), None, None);
    assert!(!tools.iter().any(|t| t.name == "read_skill"));
}

#[tokio::test]
async fn skill_commands_are_the_enabled_skills_and_run_with_their_skill_md() {
    use crate::ai::{skill_commands, skill_text};
    let f = Fixture::new(Vec::new()).await;
    f.add_skill(
        "nginx",
        "Debug nginx",
        &[
            ("SKILL.md", "Run nginx -t first."),
            ("references/tls.md", "TLS"),
        ],
    );
    let v = lock(&f.vault);
    let names: Vec<String> = skill_commands(&v).into_iter().map(|c| c.name).collect();
    assert_eq!(names, ["hatoba", "nginx"]);
    assert_eq!(
        skill_text(&v, "nginx", VERSION).unwrap(),
        "Run nginx -t first."
    );
    assert!(skill_text(&v, "hatoba", VERSION).unwrap().contains(VERSION));
    assert_eq!(
        skill_text(&v, "apache", VERSION).unwrap_err().code,
        ErrorCode::NotFound
    );
}
