//! History search (AI-24): the conversations whose title or message text contains a query.

use std::borrow::Cow;

use hatoba_ai::entry::EntryBody;
use hatoba_ai::tools;
use hatoba_core::sync::SharedVault;

use super::blocks::{MEMORY, PERMISSION_MODE, TERMINAL_STATE, note_block};
use super::entries::Entries;
use super::unlocked;
use crate::dto::AiSearchHit;
use crate::error::AppResult;

/// Characters of a search snippet (AI-24), and how many of them come before the match.
const SNIPPET_CHARS: usize = 120;
const SNIPPET_BEFORE: usize = 40;

/// `ai_search`: conversations whose title or message text contains `query`, ignoring case, newest
/// activity first. One hit per conversation: its first matching user, assistant or summary entry
/// (tool results are not searched), or its title. Decrypts every conversation, one at a time
/// under the vault guard, so run it off the async runtime.
pub fn search(vault: &SharedVault, query: &str) -> AppResult<Vec<AiSearchHit>> {
    let needle = fold(query.trim());
    if needle.is_empty() {
        return Ok(Vec::new());
    }
    let mut conversations: Vec<(String, String, i64)> = {
        let v = unlocked(vault)?;
        let last = v.ai_last_activities();
        v.ai_conversations()
            .into_iter()
            .map(|(id, c)| {
                let at = last.get(&id).copied().unwrap_or(0);
                (id, c.title, at)
            })
            .collect()
    };
    conversations.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| b.0.cmp(&a.0)));
    let mut hits = Vec::new();
    for (conversation_id, title, _) in conversations {
        let entries = {
            let v = unlocked(vault)?;
            Entries::load(&v, &conversation_id)?
        };
        let in_entries = entries.ids.iter().zip(&entries.list).find_map(|(id, e)| {
            let text = match &e.body {
                // The screen a message showed is the host's text, not the conversation's (AI-46),
                // and the permission mode and memory notes are Hatoba's (§13.5, AI-53).
                EntryBody::User { text, .. } => {
                    let mut text = Cow::Borrowed(text.as_str());
                    for tag in [PERMISSION_MODE, MEMORY, TERMINAL_STATE] {
                        if let Some(block) = note_block(&text, tag) {
                            text = Cow::Owned(text.replacen(block, "", 1));
                        }
                    }
                    text
                }
                EntryBody::Summary { text } => {
                    let summary = tools::split_permission_note(text).1;
                    match note_block(summary, MEMORY) {
                        Some(block) => Cow::Borrowed(&summary[block.len()..]),
                        None => Cow::Borrowed(summary),
                    }
                }
                EntryBody::Assistant(a) => Cow::Borrowed(a.text.as_str()),
                EntryBody::Tool { .. } => return None,
            };
            snippet(&text, &needle).map(|s| (Some(id.clone()), s))
        });
        if let Some((entry_id, snippet)) =
            in_entries.or_else(|| snippet(&title, &needle).map(|s| (None, s)))
        {
            hits.push(AiSearchHit {
                conversation_id,
                entry_id,
                snippet,
            });
        }
    }
    Ok(hits)
}

/// Text in lowercase, character by character, for matching that ignores case.
fn fold(text: &str) -> Vec<char> {
    text.chars().flat_map(char::to_lowercase).collect()
}

/// About [`SNIPPET_CHARS`] characters of `text` around the first match of `needle` (folded),
/// whitespace collapsed, with `…` where it was cut. `None` without a match.
fn snippet(text: &str, needle: &[char]) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    // The folded text, and for each of its characters the original one it came from.
    let mut folded = Vec::with_capacity(chars.len());
    let mut origin = Vec::with_capacity(chars.len());
    for (i, c) in chars.iter().enumerate() {
        for lower in c.to_lowercase() {
            folded.push(lower);
            origin.push(i);
        }
    }
    let at = folded.windows(needle.len()).position(|w| w == needle)?;
    let (start, end) = (origin[at], origin[at + needle.len() - 1] + 1);
    let mut from = start.saturating_sub(SNIPPET_BEFORE);
    let to = chars.len().min(from + SNIPPET_CHARS.max(end - from));
    // Near the end of the text, the window takes more of what comes before.
    if to - from < SNIPPET_CHARS {
        from = to.saturating_sub(SNIPPET_CHARS);
    }
    let mut out = String::new();
    if from > 0 {
        out.push('…');
    }
    let mut space = false;
    for c in &chars[from..to] {
        if c.is_whitespace() {
            space = true;
        } else {
            if space && !out.is_empty() && !out.ends_with('…') {
                out.push(' ');
            }
            space = false;
            out.push(*c);
        }
    }
    if to < chars.len() {
        out.push('…');
    }
    Some(out)
}
