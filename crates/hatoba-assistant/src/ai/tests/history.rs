//! History search (AI-24).

use super::*;

#[test]
fn search_finds_titles_and_message_text_once_per_conversation() {
    use hatoba_ai::tools::{PermissionMode, permission_mode_block, terminal_state_block};

    let vault: SharedVault = {
        let mut vault = Vault::open_in_memory().unwrap();
        vault
            .create_with_params(PW, KdfParams::for_tests())
            .unwrap();
        Arc::new(Mutex::new(vault))
    };
    let conversation = |title: &str, entries: &[AiEntry]| -> (String, Vec<String>) {
        let mut v = lock(&vault);
        let id = v
            .put(
                None,
                Item::AiConversation(AiConversation {
                    title: title.into(),
                    ..AiConversation::default()
                }),
            )
            .unwrap();
        let ids = entries
            .iter()
            .map(|e| append(&mut v, &id, e).unwrap())
            .collect();
        (id, ids)
    };
    let answer = |text: &str| {
        AiEntry::assistant(
            2,
            AssistantEntry {
                text: text.into(),
                ..AssistantEntry::default()
            },
        )
    };
    let long = format!(
        "{}needle in the middle{}",
        "a ".repeat(100),
        " b".repeat(100)
    );
    let (disk, disk_ids) = conversation(
        "Disk usage on prod",
        &[
            AiEntry::user(
                1,
                format!(
                    "{}{}how full is /var?",
                    permission_mode_block(PermissionMode::Bypass),
                    terminal_state_block("prod", false, "$ df\nscreen-needle")
                ),
            ),
            answer("It is at 42% now."),
            AiEntry::tool(3, "call_1", ToolStatus::Ok, "tool-only-needle"),
        ],
    );
    let (nginx, nginx_ids) = conversation(
        "nginx",
        &[
            AiEntry::user(1, "The NGINX config\nhas a typo"),
            AiEntry::user(2, "nginx again"),
            AiEntry::summary(
                3,
                format!("{}{long}", permission_mode_block(PermissionMode::Bypass)),
            ),
            AiEntry::user(4, "Ärger mit Umlauten"),
        ],
    );

    let search = |q: &str| crate::ai::search(&vault, q).unwrap();
    assert!(search("   ").is_empty());
    // Only the title matches: no entry.
    let hits = search("DISK");
    assert_eq!(hits.len(), 1);
    assert_eq!(
        (
            hits[0].conversation_id.as_str(),
            hits[0].entry_id.as_deref()
        ),
        (disk.as_str(), None)
    );
    assert_eq!(hits[0].snippet, "Disk usage on prod");
    // An answer's text.
    let hits = search("42%");
    assert_eq!(hits[0].entry_id.as_deref(), Some(disk_ids[1].as_str()));
    assert_eq!(hits[0].snippet, "It is at 42% now.");
    // Tool results are not searched, and neither is the screen a message showed (AI-46) or the
    // permission mode note of a message or a summary (§13.5).
    assert!(search("tool-only-needle").is_empty());
    assert!(search("screen-needle").is_empty());
    assert!(search("bypass").is_empty());
    assert_eq!(search("/var")[0].snippet, "how full is /var?");
    // One hit per conversation: the first matching entry, before the title.
    let hits = search("nginx");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].conversation_id, nginx);
    assert_eq!(hits[0].entry_id.as_deref(), Some(nginx_ids[0].as_str()));
    assert_eq!(hits[0].snippet, "The NGINX config has a typo");
    // A snippet of about 120 characters around the match, cut with an ellipsis.
    let hits = search("NEEDLE IN");
    assert_eq!(hits[0].entry_id.as_deref(), Some(nginx_ids[2].as_str()));
    let snippet = &hits[0].snippet;
    assert!(
        snippet.starts_with('…') && snippet.ends_with('…'),
        "{snippet}"
    );
    assert!(snippet.contains("needle in the middle"));
    assert!(snippet.chars().count() <= 122, "{snippet}");
    // Case folding beyond ASCII.
    assert_eq!(search("ärger").len(), 1);
    // One hit for each conversation that matches.
    assert_eq!(search("a").len(), 2);

    lock(&vault).lock();
    assert_eq!(
        crate::ai::search(&vault, "x").unwrap_err().code,
        ErrorCode::Locked
    );
}
