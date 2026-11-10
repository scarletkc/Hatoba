//! `hatoba memory` (HOST-15): other programs read and change the AI assistant's memories of a
//! saved SSH host (AI-53), under the rules its `memory` tool follows
//! ([`hatoba_assistant::ai::memory`]). The global memories stay out: like the custom
//! instructions, they are for Hatoba's assistant, and coding agents keep their own.
//!
//! Memories reach models as context, so their text is never logged (SEC-04).

use hatoba_ai::entry::ToolStatus;
use hatoba_ai::tools::MemoryAction;
use hatoba_assistant::ai::memory::{self, Listed, Place, Scope};
use hatoba_core::vault::Vault;
use serde::Serialize;

use super::find_host;

/// What a listing prints: `text` on stdout, and `notice` for the person when there is nothing to
/// list.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Listing {
    pub(super) text: String,
    pub(super) notice: Option<String>,
}

/// The host a command names, as the place of its memories.
fn place(vault: &Vault, host: &str) -> Result<Place, String> {
    find_host(vault, host).map(|(id, host)| Place {
        scope: Scope::Host(id),
        name: host.name,
    })
}

/// The memories of `host`: as JSON, or each as a line with its name and the day it was last
/// saved, and its text below, indented.
pub(super) fn list(vault: &Vault, host: &str, json: bool) -> Result<Listing, String> {
    if !memory::on(vault) {
        return Err(memory::OFF.to_owned());
    }
    let place = place(vault, host)?;
    let memories = memory::listed(vault, &place.scope);
    if json {
        let entries: Vec<Entry<'_>> = memories.iter().map(Entry::new).collect();
        let mut text = serde_json::to_string_pretty(&entries).unwrap_or_default();
        text.push('\n');
        return Ok(Listing { text, notice: None });
    }
    let notice = memories
        .is_empty()
        .then(|| format!("no memories of {} are saved in Hatoba yet", place.name));
    let text = memories.iter().map(lines).collect::<Vec<_>>().join("\n");
    Ok(Listing { text, notice })
}

/// A memory in `hatoba memory --json`.
#[derive(Serialize)]
struct Entry<'a> {
    name: &'a str,
    /// The local day it was last saved, `YYYY-MM-DD`.
    #[serde(skip_serializing_if = "Option::is_none")]
    updated: Option<&'a str>,
    content: &'a str,
}

impl<'a> Entry<'a> {
    fn new(m: &'a Listed) -> Self {
        Self {
            name: &m.name,
            updated: Some(m.updated.as_str()).filter(|day| !day.is_empty()),
            content: &m.content,
        }
    }
}

/// A memory as text: `name (2026-11-10)`, or the name alone without a day, then its text, each
/// line two spaces in, so no line of it reads as the start of another memory.
fn lines(m: &Listed) -> String {
    let mut out = if m.updated.is_empty() {
        format!("{}\n", m.name)
    } else {
        format!("{} ({})\n", m.name, m.updated)
    };
    for line in m.content.lines() {
        if !line.is_empty() {
            out.push_str("  ");
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// Saves `content` as the memory `name` of `host`, in place of its memory of that name, or
/// deletes it. `Ok` says what was done, `Err` why nothing was.
pub(super) fn change(
    vault: &mut Vault,
    host: &str,
    action: MemoryAction,
    name: String,
    content: Option<String>,
) -> Result<String, String> {
    let place = place(vault, host)?;
    match memory::run_on_host(vault, &place, action, name, content) {
        (ToolStatus::Ok, done) => Ok(done),
        (_, refused) => Err(refused),
    }
}
