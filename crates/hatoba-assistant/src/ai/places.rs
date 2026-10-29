//! Where a conversation is (AI-09): a saved host, a quick-connect target (HOST-12) or a
//! computer's local terminal (§7.5), how notes name them, where a fork's copies came from (AI-43),
//! and whose host memories a conversation shows (AI-53).

use hatoba_ai::entry::{AiEntry, EntryBody, EntryPlace, EntryTarget};
use hatoba_core::Vault;
use hatoba_core::model::{AiConversation, AiQuickTarget, Item};

use super::blocks::{HOST_CHANGE, note_attr};
use super::memory;
use crate::dto::{AiTurnContext, QuickTarget};

/// How a quick-connect target (HOST-12) is named where a host's display name would be.
pub(super) fn quick_label(target: &QuickTarget) -> String {
    hatoba_ssh::target_label(&target.username, &target.address, target.port)
}

/// The quick-connect target of a tab without a saved host, as the conversation item records it
/// (AI-09).
pub(super) fn tab_target(context: &AiTurnContext) -> Option<AiQuickTarget> {
    context
        .target
        .as_ref()
        .filter(|_| context.host_id.is_none())
        .map(|t| AiQuickTarget::new(&t.username, &t.address, t.port))
}

/// The computer of a local tab (§7.5), or the one a home tab conversation recorded, while the
/// context names neither a host nor a target.
pub(super) fn tab_local(context: &AiTurnContext) -> Option<String> {
    context
        .local
        .as_ref()
        .filter(|_| context.host_id.is_none() && context.target.is_none())
        .map(|l| l.device.clone())
}

/// The shell of a local tab (§7.5) whose terminal tools a turn offers, as the system prompt names
/// it, which a reply records so that the next message on another shell is noted.
pub(super) fn tab_shell(context: &AiTurnContext) -> Option<String> {
    context
        .local
        .as_ref()
        .filter(|_| context.tab && context.host_id.is_none() && context.target.is_none())
        .and_then(|l| l.shell.clone())
}

/// What a local tab's computer (§7.5) is called where a host's display name would be: in notes,
/// the judge's state and the system prompt of a conversation without a terminal.
pub(super) fn local_name(device: &str) -> String {
    format!("{device} {LOCAL_SUFFIX}")
}

/// What follows a computer's name in [`local_name`].
const LOCAL_SUFFIX: &str = "(local terminal)";

/// A saved host's address, port, and user in the form a conversation records a target, which is
/// equal to a target's when HOST-12 would pick the host for it.
pub(super) fn host_target(v: &Vault, host_id: &str) -> Option<AiQuickTarget> {
    v.get(host_id)
        .and_then(Item::as_host)
        .map(|h| AiQuickTarget::new(&h.username, &h.address, h.port))
}

/// How a recorded quick-connect target is named in a `host_change` note: `user@host:port`.
pub(super) fn target_name(target: &AiQuickTarget) -> String {
    hatoba_ssh::target_label(&target.username, &target.address, target.port)
}

/// Whether a `host_change` note's `from` names the server at `here` (AI-09): the target's name,
/// or the display name of a saved host with its address, port, and user.
pub(super) fn names_server(v: &Vault, from: &str, here: &AiQuickTarget) -> bool {
    target_name(here) == from
        || v.hosts().iter().any(|(_, h)| {
            h.name == from && AiQuickTarget::new(&h.username, &h.address, h.port) == *here
        })
}

/// AI-43: where the copies of a fork came from (AI-09), from `later`, the original's entries after
/// the fork point, and the fork's `origin` (the field's doc). The first later user message that
/// records where the entries before it came from tells it. In a conversation from before messages
/// recorded it, the first later `host_change` note does instead, by name ([`place_named`]). With
/// neither, the original has stayed where the copies came from.
pub(super) fn fork_place(
    later: &[AiEntry],
    original: &AiConversation,
    hosts: &[(String, String)],
) -> (Place, Option<String>) {
    for entry in later {
        let EntryBody::User { text, came_from } = &entry.body else {
            continue;
        };
        if let Some(came_from) = came_from {
            let place = Place {
                host_id: came_from.host_id.clone(),
                quick_target: came_from
                    .quick_target
                    .as_ref()
                    .map(|t| AiQuickTarget::new(&t.username, &t.address, t.port)),
                local: came_from.local.clone(),
            };
            return (place, came_from.origin.clone());
        }
        if let Some(from) = note_attr(text, HOST_CHANGE, "from") {
            return place_named(hosts, from);
        }
    }
    let place = Place {
        host_id: original.host_id.clone(),
        quick_target: original.quick_target.clone(),
        local: original.local.clone(),
    };
    (place, original.origin.clone())
}

/// AI-43: the place a `host_change` note's `from` names, among `hosts` (ids and display names):
/// the only saved host with that display name, or else the quick-connect target it writes as
/// `user@host:port`. A display name may look like a target, so the saved host comes first. When it
/// names neither, as for a host renamed or deleted since (whose name the note leaves empty) or a
/// name two hosts share, no place, and `from` as the fork's `origin`.
pub(super) fn place_named(hosts: &[(String, String)], from: String) -> (Place, Option<String>) {
    let mut named = hosts
        .iter()
        .filter(|(_, name)| !from.is_empty() && *name == from);
    if let (Some((host_id, _)), None) = (named.next(), named.next()) {
        let place = Place {
            host_id: Some(host_id.clone()),
            quick_target: None,
            local: None,
        };
        return (place, None);
    }
    if let Some(device) = from
        .strip_suffix(LOCAL_SUFFIX)
        .and_then(|d| d.strip_suffix(' '))
        .filter(|d| !d.is_empty())
    {
        let place = Place {
            host_id: None,
            quick_target: None,
            local: Some(device.to_owned()),
        };
        return (place, None);
    }
    let place = Place {
        host_id: None,
        quick_target: parse_target(&from),
        local: None,
    };
    let origin = place.quick_target.is_none().then_some(from);
    (place, origin)
}

/// Where a conversation is (AI-09, AI-43), as a user message records where the entries before it
/// came from.
pub(super) fn entry_place(conversation: &AiConversation) -> EntryPlace {
    EntryPlace {
        host_id: conversation.host_id.clone(),
        quick_target: conversation.quick_target.as_ref().map(|t| EntryTarget {
            username: t.username.clone(),
            address: t.address.clone(),
            port: t.port,
        }),
        origin: conversation.origin.clone(),
        local: conversation.local.clone(),
    }
}

/// A quick-connect target as [`target_name`] writes it (`user@host:port`, an IPv6 address in
/// brackets), in the form a conversation records it. `None` for anything else, such as a display
/// name.
pub(super) fn parse_target(label: &str) -> Option<AiQuickTarget> {
    if label.chars().any(char::is_whitespace) {
        return None;
    }
    let (username, rest) = label.rsplit_once('@')?;
    let (address, port) = rest.rsplit_once(':')?;
    if !port.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let port = port.parse().ok()?;
    let address = match address.strip_prefix('[') {
        Some(inner) => inner.strip_suffix(']')?,
        None if address.contains(':') => return None,
        None => address,
    };
    if username.is_empty() || address.is_empty() {
        return None;
    }
    Some(AiQuickTarget::new(username, address, port))
}

/// Where a message moves its conversation (AI-09): to a saved host, to the quick-connect target
/// it records in place of one (HOST-12), or to a computer's local terminal (§7.5).
pub(super) struct Place {
    pub(super) host_id: Option<String>,
    pub(super) quick_target: Option<AiQuickTarget>,
    pub(super) local: Option<String>,
}

/// A host's display name, when the host exists.
pub(super) fn host_name(v: &Vault, host_id: &str) -> Option<String> {
    v.get(host_id)
        .and_then(Item::as_host)
        .map(|h| h.name.clone())
}

/// The host or computer whose host memories a conversation on it shows (AI-53), named as notes
/// name it: a saved host that exists, or a computer's local terminal (§7.5). `None` on a
/// quick-connect target (HOST-12), which is no saved host, and nowhere.
pub(super) fn memory_place(
    v: &Vault,
    host_id: Option<&str>,
    quick_target: Option<&AiQuickTarget>,
    local: Option<&str>,
) -> Option<memory::Place> {
    match (host_id, quick_target, local) {
        (Some(id), _, _) => host_name(v, id).map(|name| memory::Place {
            scope: memory::Scope::Host(id.to_owned()),
            name,
        }),
        (None, None, Some(device)) => Some(memory::Place {
            scope: memory::Scope::Local(device.to_owned()),
            name: local_name(device),
        }),
        _ => None,
    }
}

/// [`memory_place`] of where a conversation is.
pub(super) fn conversation_memory_place(
    v: &Vault,
    conversation: &AiConversation,
) -> Option<memory::Place> {
    memory_place(
        v,
        conversation.host_id.as_deref(),
        conversation.quick_target.as_ref(),
        conversation.local.as_deref(),
    )
}

/// Where a conversation that moves to the host `host_id` or the quick-connect `target` (AI-09)
/// was, as a `host_change` note names it: its host's display name, empty when the host no longer
/// exists, or its target (HOST-12), or with neither, a fork's `origin` (AI-43). `None` when it had
/// none of them, as a home tab chat, or when the move is between a target and the saved host with
/// its address, port, and user (as **Save as Host…** makes), which is the same server.
pub(super) fn moved_from(
    v: &Vault,
    conversation: &AiConversation,
    host_id: Option<&str>,
    target: Option<&AiQuickTarget>,
) -> Option<String> {
    match (&conversation.host_id, &conversation.quick_target) {
        (Some(old), _) if target.is_some() && host_target(v, old).as_ref() == target => None,
        (Some(old), _) => Some(host_name(v, old).unwrap_or_default()),
        (None, Some(old)) if host_id.and_then(|id| host_target(v, id)).as_ref() == Some(old) => {
            None
        }
        (None, Some(old)) => Some(target_name(old)),
        (None, None) => conversation
            .local
            .as_deref()
            .map(local_name)
            .or_else(|| conversation.origin.clone()),
    }
}

/// AI-09: whether the next message moves the conversation to the tab's host `host_id`, to the
/// tab's quick-connect `target` (HOST-12) when it has no saved host, or to the computer of a local
/// tab (§7.5). On the home tab, with none of them, it stays where it was.
pub(super) fn moves_to(
    conversation: &AiConversation,
    host_id: Option<&str>,
    target: Option<&AiQuickTarget>,
    local: Option<&str>,
) -> bool {
    match (host_id, target, local) {
        (Some(host_id), _, _) => conversation.host_id.as_deref() != Some(host_id),
        (None, Some(target), _) => {
            conversation.host_id.is_some() || conversation.quick_target.as_ref() != Some(target)
        }
        (None, None, Some(device)) => {
            conversation.host_id.is_some()
                || conversation.quick_target.is_some()
                || conversation.local.as_deref() != Some(device)
        }
        (None, None, None) => false,
    }
}
