//! `hatoba memory` (HOST-15): other programs read and change the AI assistant's memories (AI-53),
//! the global ones and those of a saved SSH host, under the rules its `memory` tool follows
//! ([`hatoba_assistant::ai::memory`]). A listing shows what a conversation on the host starts
//! with.
//!
//! Memories reach models as context, so their text is never logged (SEC-04).

use hatoba_ai::entry::ToolStatus;
use hatoba_ai::tools::{MemoryAction, MemoryArgs, MemoryScope};
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

/// The host a command names, as the place of its host memories; `None` without one.
fn place(vault: &Vault, host: Option<&str>) -> Result<Option<Place>, String> {
    host.map(|wanted| {
        find_host(vault, wanted).map(|(id, host)| Place {
            scope: Scope::Host(id),
            name: host.name,
        })
    })
    .transpose()
}

/// The global memories, then with `host` that host's: as JSON, or each as a line with its name,
/// its scope and the day it was last saved, and its text below, indented.
pub(super) fn list(vault: &Vault, host: Option<&str>, json: bool) -> Result<Listing, String> {
    if !memory::on(vault) {
        return Err(memory::OFF.to_owned());
    }
    let place = place(vault, host)?;
    let memories = memory::listed(vault, place.as_ref());
    if json {
        let entries: Vec<Entry<'_>> = memories.iter().map(Entry::new).collect();
        let mut text = serde_json::to_string_pretty(&entries).unwrap_or_default();
        text.push('\n');
        return Ok(Listing { text, notice: None });
    }
    let notice = memories.is_empty().then(|| match &place {
        Some(place) => format!(
            "no global memories and no memories of {} are saved in Hatoba yet",
            place.name
        ),
        None => "no global memories are saved in Hatoba yet".to_owned(),
    });
    let text = memories.iter().map(lines).collect::<Vec<_>>().join("\n");
    Ok(Listing { text, notice })
}

/// A memory in `hatoba memory --json`.
#[derive(Serialize)]
struct Entry<'a> {
    name: &'a str,
    /// `global` or `host`.
    scope: &'static str,
    /// The host's name, of a host memory.
    #[serde(skip_serializing_if = "Option::is_none")]
    host: Option<&'a str>,
    /// The local day it was last saved, `YYYY-MM-DD`.
    #[serde(skip_serializing_if = "Option::is_none")]
    updated: Option<&'a str>,
    content: &'a str,
}

impl<'a> Entry<'a> {
    fn new(m: &'a Listed) -> Self {
        Self {
            name: &m.name,
            scope: if m.host.is_some() { "host" } else { "global" },
            host: m.host.as_deref(),
            updated: Some(m.updated.as_str()).filter(|day| !day.is_empty()),
            content: &m.content,
        }
    }
}

/// A memory as text: `name (global, 2026-11-10)` or `name (host db, 2026-11-10)`, then its text,
/// each line two spaces in, so no line of it reads as the start of another memory.
fn lines(m: &Listed) -> String {
    let mut of = match &m.host {
        Some(host) => format!("host {host}"),
        None => "global".to_owned(),
    };
    if !m.updated.is_empty() {
        of.push_str(", ");
        of.push_str(&m.updated);
    }
    let mut out = format!("{} ({of})\n", m.name);
    for line in m.content.lines() {
        if !line.is_empty() {
            out.push_str("  ");
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// Saves `content` as the memory `name`, in place of the one of that name, or deletes it, among
/// the global memories or with `host` among that host's. `Ok` says what was done, `Err` why
/// nothing was.
pub(super) fn change(
    vault: &mut Vault,
    host: Option<&str>,
    action: MemoryAction,
    name: String,
    content: Option<String>,
) -> Result<String, String> {
    let place = place(vault, host)?;
    let args = MemoryArgs {
        action,
        scope: if place.is_some() {
            MemoryScope::Host
        } else {
            MemoryScope::Global
        },
        name,
        content,
    };
    match memory::run(vault, place.as_ref(), &args) {
        (ToolStatus::Ok, done) => Ok(done),
        (_, refused) => Err(refused),
    }
}
