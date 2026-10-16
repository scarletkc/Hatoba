//! The assistant's memories (AI-53, spec §13.12): `ai_memory` items, oldest first, which a
//! conversation starts with in a `memory` note before its first message and before each summary,
//! so the system prompt and the context before them stay the same. The `memory` tool and
//! Settings → AI save and delete them under the same rules.
//!
//! Memories reach the model as context, so their text is never logged (SEC-04).

use hatoba_ai::entry::ToolStatus;
use hatoba_ai::tools::{self, MAX_MEMORY_BYTES, MAX_MEMORY_NAME_CHARS, MemoryAction, MemoryArgs};
use hatoba_core::Vault;
use hatoba_core::model::{AiMemory, Item, SETTINGS_ID};

/// Why a memory was not saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    EmptyName,
    LongName,
    EmptyContent,
    /// Another memory has the name: a rename in Settings → AI.
    Taken,
    /// With it, all memories would take this many bytes, over [`MAX_MEMORY_BYTES`].
    Full(usize),
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
            Self::Full(bytes) => format!(
                "Memory is full: with this one, all memories would take {bytes} bytes, over \
                 {MAX_MEMORY_BYTES}. Merge or delete memories, then save again."
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

/// The ids of the memories named `name`, as [`clean_name`] stores it, oldest first. Devices that
/// save under one name before they sync each keep their own, so there can be more than one.
fn named(v: &Vault, name: &str) -> Vec<String> {
    v.ai_memories()
        .into_iter()
        .filter(|(_, m)| m.name == name)
        .map(|(id, _)| id)
        .collect()
}

/// The memories an edit in Settings → AI saves in place of: the memory `id`, and when the edit
/// keeps its name, the other memories of that name, which count as one with it. A rename into
/// another memory's name stays refused.
pub(crate) fn edited(v: &Vault, id: &str, name: &str) -> Vec<String> {
    let name = clean_name(name);
    let kept = v
        .get(id)
        .and_then(Item::as_ai_memory)
        .is_some_and(|m| m.name == name);
    let mut ids = vec![id.to_owned()];
    if kept {
        ids.extend(named(v, &name).into_iter().filter(|other| other != id));
    }
    ids
}

/// Saves a memory in place of the memories `ids` (none adds one): the first keeps its place and
/// the others are deleted. Name and text are cleaned first.
///
/// # Errors
/// The [`Refusal`] when they break a rule; the vault's error when the vault cannot store it.
pub(crate) fn save(
    v: &mut Vault,
    ids: &[String],
    name: &str,
    content: &str,
) -> Result<Result<(String, AiMemory), Refusal>, hatoba_core::Error> {
    let memory = AiMemory {
        name: clean_name(name),
        content: clean_content(content),
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
    let others: Vec<AiMemory> = v
        .ai_memories()
        .into_iter()
        .filter(|(other, _)| !ids.contains(other))
        .map(|(_, m)| m)
        .collect();
    if others.iter().any(|m| m.name == memory.name) {
        return Ok(Err(Refusal::Taken));
    }
    let bytes = others.iter().map(AiMemory::size).sum::<usize>() + memory.size();
    if bytes > MAX_MEMORY_BYTES {
        return Ok(Err(Refusal::Full(bytes)));
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

/// The `memory` note a conversation's first message and each summary start with: the memories,
/// oldest first, as many as fit [`MAX_MEMORY_BYTES`] (memories saved on several devices at once
/// can go past it), each with the local day it was last saved, as the system prompt gives today.
/// The days do not count toward the limit. Empty while memory is off or there are none.
pub(crate) fn note(v: &Vault) -> String {
    if !on(v) {
        return String::new();
    }
    let memories = v.ai_memories();
    let mut used = 0;
    let shown: Vec<(&str, String, &str)> = memories
        .iter()
        .map(|(_, m)| m)
        .take_while(|m| {
            used += m.size();
            used <= MAX_MEMORY_BYTES
        })
        .map(|m| (m.name.as_str(), day(m.updated_at), m.content.as_str()))
        .collect();
    let shown: Vec<(&str, &str, &str)> = shown
        .iter()
        .map(|(name, updated, content)| (*name, updated.as_str(), *content))
        .collect();
    tools::memory_block(&shown)
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

/// Runs a `memory` call. A call from a response made while memory was on runs nothing once it is
/// off.
pub(crate) fn run(v: &mut Vault, args: &MemoryArgs) -> (ToolStatus, String) {
    if !on(v) {
        return (
            ToolStatus::Error,
            "Memory is turned off in Hatoba's settings.".to_owned(),
        );
    }
    let name = clean_name(&args.name);
    let found = named(v, &name);
    let outcome = match args.action {
        MemoryAction::Save => save(
            v,
            &found,
            &name,
            args.content.as_deref().unwrap_or_default(),
        )
        .map(|saved| {
            saved
                .map(|(_, m)| format!("Saved the memory \"{}\".", m.name))
                .map_err(Refusal::message)
        }),
        MemoryAction::Delete if !found.is_empty() => found
            .iter()
            .try_for_each(|id| v.delete(id))
            .map(|()| Ok(format!("Deleted the memory \"{name}\"."))),
        MemoryAction::Delete => {
            let names: Vec<String> = v
                .ai_memories()
                .into_iter()
                .map(|(_, m)| format!("\"{}\"", m.name))
                .collect();
            let list = if names.is_empty() {
                "none".to_owned()
            } else {
                names.join(", ")
            };
            Ok(Err(format!(
                "There is no memory named \"{name}\". Memories: {list}."
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

    fn call(v: &mut Vault, action: MemoryAction, name: &str, content: Option<&str>) -> String {
        let (status, text) = run(
            v,
            &MemoryArgs {
                action,
                name: name.to_owned(),
                content: content.map(str::to_owned),
            },
        );
        format!("{status:?}: {text}")
    }

    fn names(v: &Vault) -> Vec<String> {
        v.ai_memories().into_iter().map(|(_, m)| m.name).collect()
    }

    #[test]
    fn the_tool_saves_replaces_and_deletes_by_name() {
        let mut v = vault();
        assert_eq!(
            call(
                &mut v,
                MemoryAction::Save,
                " web-1\nsudo ",
                Some("Uses doas.\r\n")
            ),
            "Ok: Saved the memory \"web-1 sudo\"."
        );
        call(
            &mut v,
            MemoryAction::Save,
            "reply language",
            Some("Chinese."),
        );
        // A save under a name that exists replaces that memory, in its place.
        call(
            &mut v,
            MemoryAction::Save,
            "web-1 sudo",
            Some("Uses doas, not sudo."),
        );
        assert_eq!(names(&v), ["web-1 sudo", "reply language"]);
        assert_eq!(v.ai_memories()[0].1.content, "Uses doas, not sudo.");

        assert_eq!(
            call(&mut v, MemoryAction::Delete, "nope", None),
            "Error: There is no memory named \"nope\". Memories: \"web-1 sudo\", \"reply \
             language\"."
        );
        assert_eq!(
            call(&mut v, MemoryAction::Delete, "web-1 sudo", None),
            "Ok: Deleted the memory \"web-1 sudo\"."
        );
        assert_eq!(names(&v), ["reply language"]);
    }

    /// Two devices that save under one name before they sync each keep their memory. The tool
    /// handles them as one: a save keeps the oldest and deletes the others, a delete deletes all.
    #[test]
    fn the_tool_handles_memories_synced_under_one_name_together() {
        let mut v = vault();
        let synced = |v: &mut Vault, content: &str| {
            v.put(
                None,
                Item::AiMemory(AiMemory {
                    name: "web-1 sudo".into(),
                    content: content.into(),
                    updated_at: 0,
                }),
            )
            .unwrap()
        };
        let first = synced(&mut v, "From the laptop.");
        synced(&mut v, "From the desktop.");
        assert_eq!(
            call(&mut v, MemoryAction::Save, "web-1 sudo", Some("Uses doas.")),
            "Ok: Saved the memory \"web-1 sudo\"."
        );
        assert_eq!(
            v.ai_memories(),
            [(
                first,
                AiMemory {
                    name: "web-1 sudo".into(),
                    content: "Uses doas.".into(),
                    updated_at: v.ai_memories()[0].1.updated_at,
                }
            )]
        );

        synced(&mut v, "From the desktop again.");
        call(&mut v, MemoryAction::Delete, "web-1 sudo", None);
        assert!(v.ai_memories().is_empty());

        // Settings → AI: an edit that keeps the name saves in place of all of them; a rename
        // leaves the others alone, and a rename into another memory's name is refused.
        let laptop = synced(&mut v, "From the laptop.");
        let desktop = synced(&mut v, "From the desktop.");
        let other = synced(&mut v, "Other.");
        assert_eq!(
            edited(&v, &desktop, " web-1 sudo "),
            [desktop.clone(), laptop.clone(), other.clone()]
        );
        let ids = edited(&v, &laptop, "web-1 doas");
        assert_eq!(ids, std::slice::from_ref(&laptop));
        save(&mut v, &ids, "web-1 doas", "Uses doas.")
            .unwrap()
            .unwrap();
        assert_eq!(names(&v), ["web-1 doas", "web-1 sudo", "web-1 sudo"]);
        let ids = edited(&v, &desktop, "web-1 doas");
        assert_eq!(
            save(&mut v, &ids, "web-1 doas", "x").unwrap(),
            Err(Refusal::Taken)
        );
        let ids = edited(&v, &desktop, "web-1 sudo");
        save(&mut v, &ids, "web-1 sudo", "Merged.")
            .unwrap()
            .unwrap();
        assert_eq!(names(&v), ["web-1 doas", "web-1 sudo"]);
        assert_eq!(v.ai_memories()[1].0, desktop);
    }

    #[test]
    fn saves_that_break_a_rule_change_nothing() {
        let mut v = vault();
        for (name, content, refusal) in [
            ("  ", "x", Refusal::EmptyName),
            (
                &*"n".repeat(MAX_MEMORY_NAME_CHARS + 1),
                "x",
                Refusal::LongName,
            ),
            ("n", " \n ", Refusal::EmptyContent),
        ] {
            assert_eq!(save(&mut v, &[], name, content).unwrap(), Err(refusal));
        }
        assert!(v.ai_memories().is_empty());

        // The limit counts names and texts of all memories together.
        let half = "x".repeat(MAX_MEMORY_BYTES / 2 - 1);
        let (first, _) = save(&mut v, &[], "a", &half).unwrap().unwrap();
        save(&mut v, &[], "b", &half).unwrap().unwrap();
        assert_eq!(
            save(&mut v, &[], "c", "x").unwrap(),
            Err(Refusal::Full(MAX_MEMORY_BYTES + 2))
        );
        // Replacing one counts its new size, not both.
        assert!(
            save(&mut v, std::slice::from_ref(&first), "a", "short")
                .unwrap()
                .is_ok()
        );
        // A rename to another memory's name is refused.
        assert_eq!(
            save(&mut v, std::slice::from_ref(&first), "b", "short").unwrap(),
            Err(Refusal::Taken)
        );
    }

    #[test]
    fn the_note_shows_what_fits_while_memory_is_on() {
        let mut v = vault();
        assert_eq!(note(&v), "");
        save(&mut v, &[], "one", "first").unwrap().unwrap();
        save(&mut v, &[], "two", "second").unwrap().unwrap();
        let days: Vec<String> = v
            .ai_memories()
            .into_iter()
            .map(|(_, m)| day(m.updated_at))
            .collect();
        assert!(days[0].len() == 10 && days[0].starts_with("20"), "{days:?}");
        assert_eq!(
            note(&v),
            format!(
                "<memory entries=\"2\">\n<entry name=\"one\" updated=\"{}\">\nfirst\n</entry>\n\n\
                 <entry name=\"two\" updated=\"{}\">\nsecond\n</entry>\n</memory>\n\n",
                days[0], days[1]
            )
        );

        // Memories saved on several devices at once can go past the limit: the note keeps the
        // oldest that fit.
        let big = "x".repeat(MAX_MEMORY_BYTES);
        v.put(
            None,
            Item::AiMemory(AiMemory {
                name: "big".into(),
                content: big,
                updated_at: 0,
            }),
        )
        .unwrap();
        assert!(note(&v).starts_with("<memory entries=\"2\">"));

        let mut settings = v.settings();
        settings.ai.memory = false;
        v.put(Some(SETTINGS_ID), Item::Settings(settings)).unwrap();
        assert_eq!(note(&v), "");
        assert_eq!(
            call(&mut v, MemoryAction::Save, "three", Some("third")),
            "Error: Memory is turned off in Hatoba's settings."
        );
        assert_eq!(names(&v), ["one", "two", "big"]);
    }
}
