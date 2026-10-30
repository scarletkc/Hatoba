//! The blocks a user entry starts with (spec §13.3, "Attachment blocks"): the notes Hatoba
//! puts before a message and the attachments the panel adds, and reading them back.

use hatoba_ai::mcp::{change, deferred};
use hatoba_ai::tools;

/// The tag of the note Hatoba puts before the first message after a reply the user stopped
/// (§13.1).
pub(super) const INTERRUPTED: &str = "interrupted";
/// The tag of the note Hatoba puts before a message that moves the conversation to another
/// host (AI-09).
pub(super) const HOST_CHANGE: &str = "host_change";
/// The tag of the note Hatoba puts before a message whose requests offer the terminal tools when
/// the newest reply's did not, or the other way round (AI-09).
pub(super) const TERMINAL_CHANGE: &str = "terminal_change";
/// The tag of the note Hatoba puts before a message on a local terminal tab of the same computer
/// that runs another shell than the newest reply's (§7.5).
pub(super) const SHELL_CHANGE: &str = "shell_change";
/// The tag of the note Hatoba puts before the first message to another model (AI-05).
pub(super) const MODEL_CHANGE: &str = "model_change";
/// The tag of the note Hatoba puts before a message sent in another permission mode than the
/// model was last told of (AI-16), and before a summary of a conversation in bypass or Auto
/// (AI-47).
pub(super) const PERMISSION_MODE: &str = "permission_mode";
/// The tag of the note Hatoba puts before the first message after the user reverted a change of
/// a file tool call (AI-41), one for each call.
pub(super) const FILE_REVERT: &str = "file_revert";
/// The tag of the note Hatoba puts before a message that comes with another list of deferred MCP
/// tools than the model last saw (AI-45). A conversation with one is deferred.
pub(super) const MCP_TOOLS: &str = deferred::LIST_TAG;
/// The tag of the note Hatoba puts before a message whose requests offer the MCP tools of other
/// servers than the model was last told of, while they are offered directly (AI-30).
pub(super) const MCP_CHANGE: &str = change::TAG;
/// The tag of the note Hatoba puts before a message sent with a connected terminal attached,
/// which shows its screen (AI-46).
pub(super) const TERMINAL_STATE: &str = "terminal_state";
/// The tag of the note Hatoba puts before a conversation's first message and in a summary, which
/// shows the memories (AI-53).
pub(super) const MEMORY: &str = "memory";

/// A kind of block a user entry may start with (spec §13.3, "Attachment blocks").
pub(super) struct AttachmentKind {
    pub(super) tag: &'static str,
    /// A note Hatoba writes, rather than something the user attached.
    pub(super) note: bool,
    /// The title of a conversation whose first message has only this block, from its attributes.
    pub(super) title: fn(&[(&str, &str)]) -> String,
    /// What the judge of Auto mode reads in place of the block (AI-47), from its attributes: its
    /// kind and size. Empty for a note, which the judge does not read.
    pub(super) placeholder: fn(&[(&str, &str)]) -> String,
    /// Whether the opening tag's attributes, in order, are the ones this kind writes.
    attributes: fn(&[(&str, &str)]) -> bool,
}

/// A `lines` attribute: a decimal count.
fn is_count(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

/// The size an attachment's `lines` attribute gives, such as `40 lines`, for a placeholder.
fn lines_of(attrs: &[(&str, &str)]) -> String {
    match attrs.iter().find(|(name, _)| *name == "lines") {
        Some((_, "1")) => "1 line".to_owned(),
        Some((_, count)) => format!("{count} lines"),
        None => "no lines".to_owned(),
    }
}

/// An attribute value as written, with `&amp;`, `&quot;`, `&lt;` and `&gt;` read back.
pub(super) fn attribute_text(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The blocks a user entry may start with. Hatoba writes the notes here, before the blocks the
/// panel writes (`features/ai/attachments.ts`), whose parser follows the same rules.
const ATTACHMENTS: &[AttachmentKind] = &[
    AttachmentKind {
        tag: INTERRUPTED,
        note: true,
        title: |_| String::new(),
        placeholder: |_| String::new(),
        attributes: |attrs| attrs.is_empty(),
    },
    AttachmentKind {
        tag: HOST_CHANGE,
        note: true,
        title: |_| String::new(),
        placeholder: |_| String::new(),
        attributes: |attrs| matches!(attrs, [("from", _), ("to", _)]),
    },
    AttachmentKind {
        tag: TERMINAL_CHANGE,
        note: true,
        title: |_| String::new(),
        placeholder: |_| String::new(),
        attributes: |attrs| matches!(attrs, [("to", "attached" | "detached")]),
    },
    AttachmentKind {
        tag: SHELL_CHANGE,
        note: true,
        title: |_| String::new(),
        placeholder: |_| String::new(),
        attributes: |attrs| matches!(attrs, [("from", _), ("to", _)]),
    },
    AttachmentKind {
        tag: MODEL_CHANGE,
        note: true,
        title: |_| String::new(),
        placeholder: |_| String::new(),
        attributes: |attrs| matches!(attrs, [("from", _), ("to", _)]),
    },
    AttachmentKind {
        tag: PERMISSION_MODE,
        note: true,
        title: |_| String::new(),
        placeholder: |_| String::new(),
        attributes: |attrs| matches!(attrs, [("mode", "manual" | "auto" | "bypass")]),
    },
    AttachmentKind {
        tag: MEMORY,
        note: true,
        title: |_| String::new(),
        placeholder: |_| String::new(),
        attributes: |attrs| match attrs {
            [("entries", entries)] => is_count(entries),
            // Notes stored before the attribute had its own name call it `host`.
            [("entries", entries), (host, _)] => {
                is_count(entries) && (*host == tools::MEMORY_NOTE_HOST || *host == "host")
            }
            _ => false,
        },
    },
    AttachmentKind {
        tag: FILE_REVERT,
        note: true,
        title: |_| String::new(),
        placeholder: |_| String::new(),
        attributes: |attrs| matches!(attrs, [("path", _), ("tool_call_id", _)]),
    },
    AttachmentKind {
        tag: TERMINAL_STATE,
        note: true,
        title: |_| String::new(),
        placeholder: |_| String::new(),
        attributes: |attrs| {
            matches!(attrs, [("host", _), ("alternate_screen", "true" | "false"), ("lines", lines)]
                if is_count(lines))
        },
    },
    AttachmentKind {
        tag: MCP_TOOLS,
        note: true,
        title: |_| String::new(),
        placeholder: |_| String::new(),
        attributes: |attrs| {
            matches!(attrs, [("servers", servers), ("tools", tools)]
                if is_count(servers) && is_count(tools))
        },
    },
    AttachmentKind {
        tag: MCP_CHANGE,
        note: true,
        title: |_| String::new(),
        placeholder: |_| String::new(),
        attributes: |attrs| {
            matches!(attrs, [("available", available), ("unavailable", unavailable)]
                if is_count(available) && is_count(unavailable))
        },
    },
    AttachmentKind {
        tag: "terminal_selection",
        note: false,
        title: |_| "Terminal selection".to_owned(),
        placeholder: |attrs| format!("[terminal selection, {}]", lines_of(attrs)),
        attributes: |attrs| match attrs {
            [("host", _), ("lines", lines)]
            | [("host", _), ("lines", lines), ("truncated", "true")] => is_count(lines),
            _ => false,
        },
    },
    AttachmentKind {
        tag: "connection_diagnostics",
        note: false,
        title: |_| "Connection diagnostics".to_owned(),
        placeholder: |_| "[connection diagnostics]".to_owned(),
        attributes: |attrs| matches!(attrs, [("host", _)]),
    },
    AttachmentKind {
        tag: "pasted_text",
        note: false,
        title: |_| "Pasted text".to_owned(),
        placeholder: |attrs| format!("[pasted text, {}]", lines_of(attrs)),
        attributes: |attrs| matches!(attrs, [("lines", lines)] if is_count(lines)),
    },
    AttachmentKind {
        tag: "file",
        note: false,
        title: |attrs| {
            attrs
                .first()
                .map(|(_, name)| attribute_text(name))
                .unwrap_or_default()
        },
        placeholder: |attrs| {
            let name = attrs.first().map(|(_, name)| attribute_text(name));
            format!("[file {}, {}]", name.unwrap_or_default(), lines_of(attrs))
        },
        attributes: |attrs| matches!(attrs, [("name", _), ("lines", lines)] if is_count(lines)),
    },
    AttachmentKind {
        tag: "host_file",
        note: false,
        title: |attrs| {
            let path = attrs.get(1).map(|(_, path)| attribute_text(path));
            path.as_deref()
                .map(|p| p.rsplit('/').find(|part| !part.is_empty()).unwrap_or(p))
                .unwrap_or_default()
                .to_owned()
        },
        placeholder: |attrs| {
            let path = attrs.get(1).map(|(_, path)| attribute_text(path));
            format!(
                "[file {} on the host, {}]",
                path.unwrap_or_default(),
                lines_of(attrs)
            )
        },
        attributes: |attrs| matches!(attrs, [("host", _), ("path", _), ("lines", lines)] if is_count(lines)),
    },
    AttachmentKind {
        tag: "host_directory",
        note: false,
        title: |attrs| {
            attrs
                .get(1)
                .map(|(_, path)| attribute_text(path))
                .unwrap_or_default()
        },
        placeholder: |attrs| {
            let path = attrs.get(1).map(|(_, path)| attribute_text(path));
            let entries = attrs.get(2).map_or("0", |(_, n)| *n);
            format!(
                "[listing of the directory {} on the host, {entries} entries]",
                path.unwrap_or_default()
            )
        },
        attributes: |attrs| match attrs {
            [("host", _), ("path", _), ("entries", entries)]
            | [
                ("host", _),
                ("path", _),
                ("entries", entries),
                ("truncated", "true"),
            ] => is_count(entries),
            _ => false,
        },
    },
    AttachmentKind {
        tag: "mcp_resource",
        note: false,
        title: |attrs| {
            attrs
                .get(1)
                .map(|(_, uri)| attribute_text(uri))
                .unwrap_or_default()
        },
        placeholder: |attrs| {
            let server = attrs.first().map(|(_, server)| attribute_text(server));
            let uri = attrs.get(1).map(|(_, uri)| attribute_text(uri));
            format!(
                "[MCP resource {} of {}, {}]",
                uri.unwrap_or_default(),
                server.unwrap_or_default(),
                lines_of(attrs)
            )
        },
        attributes: |attrs| matches!(attrs, [("server", _), ("uri", _), ("lines", lines)] if is_count(lines)),
    },
    AttachmentKind {
        tag: "skill",
        note: false,
        title: |attrs| {
            attrs
                .first()
                .map(|(_, name)| format!("/{}", attribute_text(name)))
                .unwrap_or_default()
        },
        placeholder: |attrs| {
            let name = attrs.first().map(|(_, name)| attribute_text(name));
            format!("[skill /{} run]", name.unwrap_or_default())
        },
        attributes: |attrs| matches!(attrs, [("name", _)]),
    },
    AttachmentKind {
        tag: "mcp_prompt",
        note: false,
        title: |attrs| {
            attrs
                .get(1)
                .map(|(_, name)| attribute_text(name))
                .unwrap_or_default()
        },
        placeholder: |attrs| {
            let server = attrs.first().map(|(_, server)| attribute_text(server));
            let name = attrs.get(1).map(|(_, name)| attribute_text(name));
            format!(
                "[MCP prompt {} of {} run]",
                name.unwrap_or_default(),
                server.unwrap_or_default()
            )
        },
        attributes: |attrs| matches!(attrs, [("server", _), ("name", _)]),
    },
];

/// One block a user entry starts with: its kind and its attributes as written.
pub(super) struct Block<'a> {
    pub(super) kind: &'static AttachmentKind,
    pub(super) attrs: Vec<(&'a str, &'a str)>,
}

/// The attachments a message has at most one of, as the panel attaches them (AI-10).
const SINGLE_ATTACHMENTS: [&str; 2] = ["terminal_selection", "connection_diagnostics"];

/// The blocks `text` starts with, in order, and the text after them. Notes come first, at most
/// one of each kind but `file_revert`, which comes once for each reverted call: a note after an
/// attachment, or a second one of another kind, is typed text, and so is a second selection or
/// diagnostics block.
pub(super) fn leading_blocks(text: &str) -> (Vec<Block<'_>>, &str) {
    let mut blocks: Vec<Block<'_>> = Vec::new();
    let mut rest = text;
    while let Some((block, after)) = leading_block(rest) {
        let repeated = blocks.iter().any(|b| b.kind.tag == block.kind.tag);
        let typed = if block.kind.note {
            blocks.iter().any(|b| !b.kind.note) || (repeated && block.kind.tag != FILE_REVERT)
        } else {
            repeated && SINGLE_ATTACHMENTS.contains(&block.kind.tag)
        };
        if typed {
            break;
        }
        blocks.push(block);
        rest = after;
    }
    (blocks, rest)
}

/// An attribute of the note `tag` among the notes a user entry starts with, read back.
pub(super) fn note_attr(text: &str, tag: &str, name: &str) -> Option<String> {
    leading_blocks(text)
        .0
        .iter()
        .take_while(|b| b.kind.note)
        .find(|b| b.kind.tag == tag)
        .and_then(|b| b.attrs.iter().find(|(n, _)| *n == name))
        .map(|(_, value)| attribute_text(value))
}

/// Whether a user entry starts with the note `tag` among its notes.
pub(super) fn has_note(text: &str, tag: &str) -> bool {
    leading_blocks(text)
        .0
        .iter()
        .take_while(|b| b.kind.note)
        .any(|b| b.kind.tag == tag)
}

/// The whole text of the note `tag` among the notes a user entry starts with, its blank line
/// included.
pub(super) fn note_block<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let (blocks, _) = leading_blocks(text);
    let mut rest = text;
    for block in blocks.iter().take_while(|b| b.kind.note) {
        let (_, after) = leading_block(rest)?;
        if block.kind.tag == tag {
            return Some(&rest[..rest.len() - after.len()]);
        }
        rest = after;
    }
    None
}

/// The block `text` starts with (spec §13.3), and the text after it: the opening tag with its
/// attributes (`name="value"`, values without `"`), a line break, the body, a line break and the
/// closing tag, then one blank line, one line break or the end. The body ends at the first
/// closing tag that such a break or the end follows; its writer escapes closing tags inside it.
pub(super) fn leading_block(text: &str) -> Option<(Block<'_>, &str)> {
    let rest = text.strip_prefix('<')?;
    let kind = ATTACHMENTS.iter().find(|k| {
        rest.strip_prefix(k.tag)
            .is_some_and(|after| after.starts_with([' ', '>']))
    })?;
    let mut rest = &rest[kind.tag.len()..];
    let mut attrs = Vec::new();
    while let Some(after) = rest.strip_prefix(' ') {
        let (name, after) = after.split_once("=\"")?;
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
            return None;
        }
        let (value, after) = after.split_once('"')?;
        attrs.push((name, value));
        rest = after;
    }
    if !(kind.attributes)(&attrs) {
        return None;
    }
    let body = rest.strip_prefix(">\n")?;
    let close = format!("\n</{}>", kind.tag);
    let mut from = 0;
    while let Some(at) = body[from..].find(&close) {
        let after = &body[from + at + close.len()..];
        if let Some(typed) = after
            .strip_prefix("\n\n")
            .or_else(|| after.strip_prefix('\n'))
        {
            return Some((Block { kind, attrs }, typed));
        }
        if after.is_empty() {
            return Some((Block { kind, attrs }, after));
        }
        from += at + 1;
    }
    None
}
