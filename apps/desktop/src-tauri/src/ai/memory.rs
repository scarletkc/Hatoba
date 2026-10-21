//! The assistant's memories (AI-53, spec §13.12): `ai_memory` items, oldest first. A global memory
//! shows in every conversation, a host memory only in the conversations on its saved host or its
//! computer's local terminal (§7.5). A conversation starts with them in a `memory` note before its
//! first message and before each summary, and a message on a host whose memories no note of the
//! context showed starts with a note of those, so the system prompt and the context before them
//! stay the same. The `memory` tool and Settings → AI save and delete them under the same rules.
//!
//! Memories reach the model as context, so their text is never logged (SEC-04).

use hatoba_ai::entry::ToolStatus;
use hatoba_ai::tools::{
    self, MAX_MEMORY_BYTES, MAX_MEMORY_NAME_CHARS, MemoryAction, MemoryArgs, MemoryEntry,
    MemoryScope,
};
use hatoba_core::Vault;
use hatoba_core::model::{AiMemory, Item, SETTINGS_ID};

/// Whose memories (AI-53): every conversation's, a saved host's, or a computer's local
/// terminal's. Names are unique, and [`MAX_MEMORY_BYTES`] counts, within each scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    Global,
    /// A saved host, by id.
    Host(String),
    /// A computer, by name.
    Local(String),
}

impl Scope {
    /// The scope of a memory stored with `host_id` and `local`: the host when there is one.
    pub(crate) fn new(host_id: Option<&str>, local: Option<&str>) -> Self {
        match (host_id, local) {
            (Some(id), _) => Self::Host(id.to_owned()),
            (None, Some(device)) => Self::Local(device.to_owned()),
            (None, None) => Self::Global,
        }
    }

    /// Whether `m` is one of its memories.
    fn holds(&self, m: &AiMemory) -> bool {
        match self {
            Self::Global => m.host_id.is_none() && m.local.is_none(),
            Self::Host(id) => m.host_id.as_deref() == Some(id),
            Self::Local(device) => m.host_id.is_none() && m.local.as_deref() == Some(device),
        }
    }
}

/// A saved host or a computer's local terminal that a conversation is on (AI-53): the scope of the
/// host memories it shows, and how notes name it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Place {
    pub(crate) scope: Scope,
    pub(crate) name: String,
}

/// Why a memory was not saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    EmptyName,
    LongName,
    EmptyContent,
    /// Another memory of its scope has the name: a rename or a move in Settings → AI.
    Taken,
    /// With it, the memories of its scope would take `bytes`, over [`MAX_MEMORY_BYTES`].
    Full {
        bytes: usize,
        global: bool,
    },
}

impl Refusal {
    /// What the model reads, and what Settings → AI shows when its own checks let one through.
    pub(crate) fn message(self) -> String {
        match self {
            Self::EmptyName => "The memory has no name.".to_owned(),
            Self::LongName => {
                format!("The name is longer than {MAX_MEMORY_NAME_CHARS} characters.")
            }
            Self::EmptyContent => "The memory is empty. To remove a memory, delete it.".to_owned(),
            Self::Taken => "Another memory has this name.".to_owned(),
            Self::Full { bytes, global } => format!(
                "Memory is full: with this one, the {} memories would take {bytes} bytes, over \
                 {MAX_MEMORY_BYTES}. Merge or delete memories, then save again.",
                if global { "global" } else { "host" }
            ),
        }
    }
}

/// A name as it is stored: one line, control characters as spaces, trimmed.
pub(crate) fn clean_name(name: &str) -> String {
    let line: String = name
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    line.trim().to_owned()
}

/// A text as it is stored: line breaks as `\n`, trimmed.
fn clean_content(content: &str) -> String {
    content
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim()
        .to_owned()
}

/// The memories of `scope`, oldest first.
fn of_scope(v: &Vault, scope: &Scope) -> Vec<(String, AiMemory)> {
    v.ai_memories()
        .into_iter()
        .filter(|(_, m)| scope.holds(m))
        .collect()
}

/// The ids of the memories of `scope` named `name`, as [`clean_name`] stores it, oldest first.
/// Devices that save under one name before they sync each keep their own, so there can be more
/// than one.
fn named(v: &Vault, scope: &Scope, name: &str) -> Vec<String> {
    of_scope(v, scope)
        .into_iter()
        .filter(|(_, m)| m.name == name)
        .map(|(id, _)| id)
        .collect()
}

/// The memories an edit in Settings → AI saves in place of: the memory `id`, and when the edit
/// keeps its name and its scope, the other memories of that name there, which count as one with
/// it. A rename or a move onto another memory's name stays refused.
pub(crate) fn edited(v: &Vault, id: &str, scope: &Scope, name: &str) -> Vec<String> {
    let name = clean_name(name);
    let kept = v
        .get(id)
        .and_then(Item::as_ai_memory)
        .is_some_and(|m| m.name == name && scope.holds(m));
    let mut ids = vec![id.to_owned()];
    if kept {
        ids.extend(
            named(v, scope, &name)
                .into_iter()
                .filter(|other| other != id),
        );
    }
    ids
}

/// Saves a memory of `scope` in place of the memories `ids` (none adds one): the first keeps its
/// place and the others are deleted. Name and text are cleaned first.
///
/// # Errors
/// The [`Refusal`] when they break a rule; the vault's error when the vault cannot store it.
pub(crate) fn save(
    v: &mut Vault,
    ids: &[String],
    scope: &Scope,
    name: &str,
    content: &str,
) -> Result<Result<(String, AiMemory), Refusal>, hatoba_core::Error> {
    let (host_id, local) = match scope {
        Scope::Global => (None, None),
        Scope::Host(id) => (Some(id.clone()), None),
        Scope::Local(device) => (None, Some(device.clone())),
    };
    let memory = AiMemory {
        name: clean_name(name),
        content: clean_content(content),
        host_id,
        local,
        updated_at: 0,
    };
    if memory.name.is_empty() {
        return Ok(Err(Refusal::EmptyName));
    }
    if memory.name.chars().count() > MAX_MEMORY_NAME_CHARS {
        return Ok(Err(Refusal::LongName));
    }
    if memory.content.is_empty() {
        return Ok(Err(Refusal::EmptyContent));
    }
    let others: Vec<AiMemory> = of_scope(v, scope)
        .into_iter()
        .filter(|(other, _)| !ids.contains(other))
        .map(|(_, m)| m)
        .collect();
    if others.iter().any(|m| m.name == memory.name) {
        return Ok(Err(Refusal::Taken));
    }
    let bytes = others.iter().map(AiMemory::size).sum::<usize>() + memory.size();
    if bytes > MAX_MEMORY_BYTES {
        return Ok(Err(Refusal::Full {
            bytes,
            global: *scope == Scope::Global,
        }));
    }
    let id = v.put(ids.first().map(String::as_str), Item::AiMemory(memory))?;
    for other in ids.iter().skip(1) {
        v.delete(other)?;
    }
    let stored = v
        .get(&id)
        .and_then(Item::as_ai_memory)
        .cloned()
        .unwrap_or_default();
    Ok(Ok((id, stored)))
}

/// Whether memory is on (`Settings.ai.memory`), read without copying the settings: on while no
/// settings are stored, as by default.
pub(crate) fn on(v: &Vault) -> bool {
    v.get(SETTINGS_ID)
        .and_then(Item::as_settings)
        .is_none_or(|s| s.ai.memory)
}

/// The memories of `scope` a note shows: oldest first, as many as fit [`MAX_MEMORY_BYTES`]
/// (memories saved on several devices at once can go past it).
fn shown(v: &Vault, scope: &Scope) -> Vec<AiMemory> {
    let mut used = 0;
    of_scope(v, scope)
        .into_iter()
        .map(|(_, m)| m)
        .take_while(|m| {
            used += m.size();
            used <= MAX_MEMORY_BYTES
        })
        .collect()
}

/// The `memory` note a conversation's first message and each summary start with: the global
/// memories, then those of the host or computer the conversation is on, each with the local day
/// it was last saved, as the system prompt gives today. The days do not count toward the limit.
/// Empty while memory is off or there are none.
pub(crate) fn note(v: &Vault, place: Option<&Place>) -> String {
    if !on(v) {
        return String::new();
    }
    let host = place.map(|p| shown(v, &p.scope)).unwrap_or_default();
    block(place, &shown(v, &Scope::Global), &host)
}

/// The `memory` note of a message on a host whose memories no note of the context showed: only
/// those. Empty while memory is off or the host has none.
pub(crate) fn place_note(v: &Vault, place: &Place) -> String {
    if !on(v) {
        return String::new();
    }
    block(Some(place), &[], &shown(v, &place.scope))
}

fn block(place: Option<&Place>, global: &[AiMemory], host: &[AiMemory]) -> String {
    let name = place.map(|p| p.name.as_str());
    let days: Vec<String> = global
        .iter()
        .chain(host)
        .map(|m| day(m.updated_at))
        .collect();
    let entries: Vec<MemoryEntry<'_>> = global
        .iter()
        .map(|m| (m, None))
        .chain(host.iter().map(|m| (m, name)))
        .zip(&days)
        .map(|((m, host), updated)| MemoryEntry {
            name: &m.name,
            updated,
            host,
            content: &m.content,
        })
        .collect();
    tools::memory_block(name, &entries)
}

/// The local day of a Unix ms time as `YYYY-MM-DD`, or empty when it is out of range.
fn day(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|t| t.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// How a `memory` call's result names the memories of a scope: `(kind, of)`, as in "the host
/// memory \"sudo\" of prod-db", and "the host memories of prod-db".
fn words(place: Option<&Place>) -> (&'static str, String) {
    match place {
        None => ("global", String::new()),
        Some(p) => ("host", format!(" of {}", p.name)),
    }
}

/// Runs a `memory` call of a conversation on `place`, whose host memories its `host` scope
/// changes. A call from a response made while memory was on runs nothing once it is off.
pub(crate) fn run(v: &mut Vault, place: Option<&Place>, args: &MemoryArgs) -> (ToolStatus, String) {
    if !on(v) {
        return (
            ToolStatus::Error,
            "Memory is turned off in Hatoba's settings.".to_owned(),
        );
    }
    let (scope, at, other) = match (args.scope, place) {
        (MemoryScope::Global, _) => (Scope::Global, None, place),
        (MemoryScope::Host, Some(p)) => (p.scope.clone(), Some(p), None),
        (MemoryScope::Host, None) => {
            return (
                ToolStatus::Error,
                "This conversation is not on a saved host or a local terminal, so it has no host \
                 memories."
                    .to_owned(),
            );
        }
    };
    let (kind, of) = words(at);
    let name = clean_name(&args.name);
    let found = named(v, &scope, &name);
    let outcome = match args.action {
        MemoryAction::Save => save(
            v,
            &found,
            &scope,
            &name,
            args.content.as_deref().unwrap_or_default(),
        )
        .map(|saved| {
            saved
                .map(|(_, m)| format!("Saved the {kind} memory \"{}\"{of}.", m.name))
                .map_err(Refusal::message)
        }),
        MemoryAction::Delete if !found.is_empty() => found
            .iter()
            .try_for_each(|id| v.delete(id))
            .map(|()| Ok(format!("Deleted the {kind} memory \"{name}\"{of}."))),
        MemoryAction::Delete => {
            let names: Vec<String> = of_scope(v, &scope)
                .into_iter()
                .map(|(_, m)| format!("\"{}\"", m.name))
                .collect();
            let list = if names.is_empty() {
                "none".to_owned()
            } else {
                names.join(", ")
            };
            // The other scope the conversation shows may hold it.
            let elsewhere = match (args.scope, other) {
                (MemoryScope::Global, Some(p)) if !named(v, &p.scope, &name).is_empty() => {
                    format!(" There is a host memory of that name of {}.", p.name)
                }
                (MemoryScope::Host, _) if !named(v, &Scope::Global, &name).is_empty() => {
                    " There is a global memory of that name.".to_owned()
                }
                _ => String::new(),
            };
            Ok(Err(format!(
                "There is no {kind} memory named \"{name}\"{of}. The {kind} memories{of}: \
                 {list}.{elsewhere}"
            )))
        }
    };
    match outcome {
        Ok(Ok(done)) => (ToolStatus::Ok, done),
        Ok(Err(refused)) => (ToolStatus::Error, refused),
        Err(e) => (
            ToolStatus::Error,
            format!("The memory could not be saved: {e}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> Vault {
        let mut v = Vault::open_in_memory().unwrap();
        v.create_with_params("pw", hatoba_core::KdfParams::for_tests())
            .unwrap();
        v
    }

    fn web1() -> Place {
        Place {
            scope: Scope::Host("h1".into()),
            name: "web-1".into(),
        }
    }

    fn call(
        v: &mut Vault,
        place: Option<&Place>,
        action: MemoryAction,
        scope: MemoryScope,
        name: &str,
        content: Option<&str>,
    ) -> String {
        let (status, text) = run(
            v,
            place,
            &MemoryArgs {
                action,
                scope,
                name: name.to_owned(),
                content: content.map(str::to_owned),
            },
        );
        format!("{status:?}: {text}")
    }

    fn names(v: &Vault) -> Vec<String> {
        v.ai_memories().into_iter().map(|(_, m)| m.name).collect()
    }

    fn synced(v: &mut Vault, scope: &Scope, name: &str, content: &str) -> String {
        let mut memory = AiMemory {
            name: name.into(),
            content: content.into(),
            ..AiMemory::default()
        };
        match scope {
            Scope::Global => {}
            Scope::Host(id) => memory.host_id = Some(id.clone()),
            Scope::Local(device) => memory.local = Some(device.clone()),
        }
        v.put(None, Item::AiMemory(memory)).unwrap()
    }

    #[test]
    fn the_tool_saves_replaces_and_deletes_by_name() {
        use {MemoryAction::*, MemoryScope::*};
        let mut v = vault();
        let at = Some(web1());
        let at = at.as_ref();
        assert_eq!(
            call(
                &mut v,
                at,
                Save,
                Host,
                " sudo\nuse ",
                Some("Uses doas.\r\n")
            ),
            "Ok: Saved the host memory \"sudo use\" of web-1."
        );
        assert_eq!(
            call(&mut v, at, Save, Global, "reply language", Some("Chinese.")),
            "Ok: Saved the global memory \"reply language\"."
        );
        // A save under a name that exists replaces that memory, in its place.
        call(
            &mut v,
            at,
            Save,
            Host,
            "sudo use",
            Some("Uses doas, not sudo."),
        );
        assert_eq!(names(&v), ["sudo use", "reply language"]);
        let (_, sudo) = &v.ai_memories()[0];
        assert_eq!(sudo.content, "Uses doas, not sudo.");
        assert_eq!(sudo.host_id.as_deref(), Some("h1"));

        assert_eq!(
            call(&mut v, at, Delete, Global, "nope", None),
            "Error: There is no global memory named \"nope\". The global memories: \"reply \
             language\"."
        );
        // The other scope the conversation shows is named when it holds the name.
        assert_eq!(
            call(&mut v, at, Delete, Global, "sudo use", None),
            "Error: There is no global memory named \"sudo use\". The global memories: \"reply \
             language\". There is a host memory of that name of web-1."
        );
        assert_eq!(
            call(&mut v, at, Delete, Host, "reply language", None),
            "Error: There is no host memory named \"reply language\" of web-1. The host memories \
             of web-1: \"sudo use\". There is a global memory of that name."
        );
        assert_eq!(
            call(&mut v, at, Delete, Host, "sudo use", None),
            "Ok: Deleted the host memory \"sudo use\" of web-1."
        );
        assert_eq!(names(&v), ["reply language"]);

        // A conversation on no saved host or local terminal keeps only global memories.
        assert_eq!(
            call(&mut v, None, Save, Host, "sudo", Some("x")),
            "Error: This conversation is not on a saved host or a local terminal, so it has no \
             host memories."
        );
        assert_eq!(
            call(&mut v, None, Delete, Global, "reply language", None),
            "Ok: Deleted the global memory \"reply language\"."
        );
        assert!(v.ai_memories().is_empty());
    }

    /// Each scope keeps its own names and its own limit, and a call changes only the scope it
    /// names: the global memories, or those of the conversation's host or computer.
    #[test]
    fn scopes_keep_their_own_names_and_limits() {
        use {MemoryAction::*, MemoryScope::*};
        let mut v = vault();
        let web = web1();
        let laptop = Place {
            scope: Scope::Local("LAPTOP".into()),
            name: "LAPTOP (local terminal)".into(),
        };
        for (place, scope) in [(None, Global), (Some(&web), Host), (Some(&laptop), Host)] {
            assert!(call(&mut v, place, Save, scope, "notes", Some("x")).starts_with("Ok"));
        }
        let scopes: Vec<Scope> = v
            .ai_memories()
            .into_iter()
            .map(|(_, m)| Scope::new(m.host_id.as_deref(), m.local.as_deref()))
            .collect();
        assert_eq!(
            scopes,
            [Scope::Global, web.scope.clone(), laptop.scope.clone()]
        );

        // A full scope leaves the others room.
        let big = "x".repeat(MAX_MEMORY_BYTES - "big".len());
        assert!(call(&mut v, Some(&web), Save, Host, "notes", Some(&big)).starts_with("Error"));
        assert!(call(&mut v, Some(&web), Delete, Host, "notes", None).starts_with("Ok"));
        assert!(call(&mut v, Some(&web), Save, Host, "big", Some(&big)).starts_with("Ok"));
        assert_eq!(
            call(&mut v, Some(&web), Save, Host, "more", Some("y")),
            format!(
                "Error: Memory is full: with this one, the host memories would take {} bytes, \
                 over {MAX_MEMORY_BYTES}. Merge or delete memories, then save again.",
                MAX_MEMORY_BYTES + "more".len() + 1
            )
        );
        assert!(call(&mut v, Some(&web), Save, Global, "more", Some("y")).starts_with("Ok"));
        assert!(call(&mut v, Some(&laptop), Save, Host, "more", Some("y")).starts_with("Ok"));
    }

    /// Two devices that save under one name before they sync each keep their memory. The tool
    /// handles them as one: a save keeps the oldest and deletes the others, a delete deletes all.
    #[test]
    fn the_tool_handles_memories_synced_under_one_name_together() {
        use {MemoryAction::*, MemoryScope::*};
        let mut v = vault();
        let global = Scope::Global;
        let first = synced(&mut v, &global, "web-1 sudo", "From the laptop.");
        synced(&mut v, &global, "web-1 sudo", "From the desktop.");
        // The same name in another scope is another memory.
        let host = synced(&mut v, &web1().scope, "web-1 sudo", "On the host.");
        assert_eq!(
            call(&mut v, None, Save, Global, "web-1 sudo", Some("Uses doas.")),
            "Ok: Saved the global memory \"web-1 sudo\"."
        );
        let ids: Vec<String> = v.ai_memories().into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, [first, host.clone()]);
        assert_eq!(v.ai_memories()[0].1.content, "Uses doas.");

        synced(&mut v, &global, "web-1 sudo", "From the desktop again.");
        call(&mut v, None, Delete, Global, "web-1 sudo", None);
        let ids: Vec<String> = v.ai_memories().into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, std::slice::from_ref(&host));
        v.delete(&host).unwrap();

        // Settings → AI: an edit that keeps the name and the scope saves in place of all of them;
        // a rename or a move leaves the others alone, and one onto another memory's name is
        // refused.
        let laptop = synced(&mut v, &global, "web-1 sudo", "From the laptop.");
        let desktop = synced(&mut v, &global, "web-1 sudo", "From the desktop.");
        let other = synced(&mut v, &global, "web-1 sudo", "Other.");
        assert_eq!(
            edited(&v, &desktop, &global, " web-1 sudo "),
            [desktop.clone(), laptop.clone(), other.clone()]
        );
        let ids = edited(&v, &laptop, &global, "web-1 doas");
        assert_eq!(ids, std::slice::from_ref(&laptop));
        save(&mut v, &ids, &global, "web-1 doas", "Uses doas.")
            .unwrap()
            .unwrap();
        assert_eq!(names(&v), ["web-1 doas", "web-1 sudo", "web-1 sudo"]);
        let ids = edited(&v, &desktop, &global, "web-1 doas");
        assert_eq!(
            save(&mut v, &ids, &global, "web-1 doas", "x").unwrap(),
            Err(Refusal::Taken)
        );
        // A move to a host keeps the name and leaves the others of it where they are.
        let web = web1().scope;
        let ids = edited(&v, &other, &web, "web-1 sudo");
        assert_eq!(ids, std::slice::from_ref(&other));
        save(&mut v, &ids, &web, "web-1 sudo", "Other.")
            .unwrap()
            .unwrap();
        assert_eq!(named(&v, &web, "web-1 sudo"), std::slice::from_ref(&other));
        let ids = edited(&v, &desktop, &global, "web-1 sudo");
        assert_eq!(ids, std::slice::from_ref(&desktop));
        save(&mut v, &ids, &global, "web-1 sudo", "Merged.")
            .unwrap()
            .unwrap();
        assert_eq!(names(&v), ["web-1 doas", "web-1 sudo", "web-1 sudo"]);
        // Moving it onto the name the host has is refused.
        let ids = edited(&v, &desktop, &web, "web-1 sudo");
        assert_eq!(
            save(&mut v, &ids, &web, "web-1 sudo", "Merged.").unwrap(),
            Err(Refusal::Taken)
        );
    }

    #[test]
    fn saves_that_break_a_rule_change_nothing() {
        let mut v = vault();
        let global = Scope::Global;
        for (name, content, refusal) in [
            ("  ", "x", Refusal::EmptyName),
            (
                &*"n".repeat(MAX_MEMORY_NAME_CHARS + 1),
                "x",
                Refusal::LongName,
            ),
            ("n", " \n ", Refusal::EmptyContent),
        ] {
            assert_eq!(
                save(&mut v, &[], &global, name, content).unwrap(),
                Err(refusal)
            );
        }
        assert!(v.ai_memories().is_empty());

        // The limit counts names and texts of all memories of the scope together.
        let half = "x".repeat(MAX_MEMORY_BYTES / 2 - 1);
        let (first, _) = save(&mut v, &[], &global, "a", &half).unwrap().unwrap();
        save(&mut v, &[], &global, "b", &half).unwrap().unwrap();
        assert_eq!(
            save(&mut v, &[], &global, "c", "x").unwrap(),
            Err(Refusal::Full {
                bytes: MAX_MEMORY_BYTES + 2,
                global: true
            })
        );
        // Replacing one counts its new size, not both.
        assert!(
            save(&mut v, std::slice::from_ref(&first), &global, "a", "short")
                .unwrap()
                .is_ok()
        );
        // A rename to another memory's name is refused.
        assert_eq!(
            save(&mut v, std::slice::from_ref(&first), &global, "b", "short").unwrap(),
            Err(Refusal::Taken)
        );
    }

    #[test]
    fn the_note_shows_what_fits_while_memory_is_on() {
        let mut v = vault();
        let web = web1();
        assert_eq!(note(&v, Some(&web)), "");
        let global = Scope::Global;
        save(&mut v, &[], &global, "one", "first").unwrap().unwrap();
        save(&mut v, &[], &web.scope, "sudo", "doas")
            .unwrap()
            .unwrap();
        save(&mut v, &[], &global, "two", "second")
            .unwrap()
            .unwrap();
        // Another host's memory shows in none of these notes.
        save(&mut v, &[], &Scope::Host("h2".into()), "other", "x")
            .unwrap()
            .unwrap();
        let days: Vec<String> = v
            .ai_memories()
            .into_iter()
            .map(|(_, m)| day(m.updated_at))
            .collect();
        assert!(days[0].len() == 10 && days[0].starts_with("20"), "{days:?}");
        assert_eq!(
            note(&v, None),
            format!(
                "<memory entries=\"2\">\n<entry name=\"one\" scope=\"global\" updated=\"{}\">\n\
                 first\n</entry>\n\n<entry name=\"two\" scope=\"global\" updated=\"{}\">\n\
                 second\n</entry>\n</memory>\n\n",
                days[0], days[2]
            )
        );
        // On a host, the global memories come first, then the host's.
        assert_eq!(
            note(&v, Some(&web)),
            format!(
                "<memory entries=\"3\" conversation_host=\"web-1\">\n<entry name=\"one\" \
                 scope=\"global\" updated=\"{}\">\nfirst\n</entry>\n\n<entry name=\"two\" \
                 scope=\"global\" updated=\"{}\">\nsecond\n</entry>\n\n<entry name=\"sudo\" \
                 scope=\"host\" host=\"web-1\" updated=\"{}\">\ndoas\n</entry>\n</memory>\n\n",
                days[0], days[2], days[1]
            )
        );
        assert_eq!(
            place_note(&v, &web),
            format!(
                "<memory entries=\"1\" conversation_host=\"web-1\">\n<entry name=\"sudo\" \
                 scope=\"host\" host=\"web-1\" updated=\"{}\">\ndoas\n</entry>\n</memory>\n\n",
                days[1]
            )
        );
        let empty = Place {
            scope: Scope::Local("LAPTOP".into()),
            name: "LAPTOP (local terminal)".into(),
        };
        assert_eq!(place_note(&v, &empty), "");
        // On one with no memories of its own, the note names it and its entries stay global.
        assert!(note(&v, Some(&empty)).starts_with(
            "<memory entries=\"2\" conversation_host=\"LAPTOP (local terminal)\">\n<entry \
             name=\"one\" scope=\"global\""
        ));

        // Memories saved on several devices at once can go past the limit: the note keeps the
        // oldest that fit, of each scope.
        synced(&mut v, &global, "big", &"x".repeat(MAX_MEMORY_BYTES));
        synced(&mut v, &web.scope, "big", &"x".repeat(MAX_MEMORY_BYTES));
        assert!(note(&v, None).starts_with("<memory entries=\"2\">"));
        assert!(
            note(&v, Some(&web)).starts_with("<memory entries=\"3\" conversation_host=\"web-1\">")
        );

        let mut settings = v.settings();
        settings.ai.memory = false;
        v.put(Some(SETTINGS_ID), Item::Settings(settings)).unwrap();
        assert_eq!(note(&v, Some(&web)), "");
        assert_eq!(place_note(&v, &web), "");
        assert_eq!(
            call(
                &mut v,
                None,
                MemoryAction::Save,
                MemoryScope::Global,
                "three",
                Some("third")
            ),
            "Error: Memory is turned off in Hatoba's settings."
        );
        assert_eq!(names(&v), ["one", "sudo", "two", "other", "big", "big"]);
    }
}
