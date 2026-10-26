//! The `mcp_change` note (spec §13.3): while a conversation's MCP tools are offered directly
//! (AI-30), turning a server on or off changes only the tool definitions of the next request, so
//! the model would read an earlier reply that named a server's tools as a mistake. The message
//! sent after the change says which servers' tools became available or stopped being available.
//! Once the tools are deferred (AI-45), the `mcp_tools` note does this.
//!
//! - [`server_names`]: the servers a request's tools come from, as a reply records them.
//! - [`block`]: the note.
//! - [`servers_after`]: the servers the model was told of, from a reply's record and the notes
//!   after it.

use crate::tools::escape_closing_tag;

/// The tag of the note.
pub const TAG: &str = "mcp_change";

const AVAILABLE: &str = "Now available:";
const UNAVAILABLE: &str = "No longer available:";

/// The servers whose tools a request offers, from the display names of its tools' servers in the
/// order it offers them: each name once, on one line, as the note shows it.
#[must_use]
pub fn server_names<'a>(servers: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for server in servers {
        let name = one_line(server);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// The note for a message whose requests offer the tools of `now` when the model was last told
/// of `before` (both from [`server_names`]), with the blank line after it. Empty when the two
/// name the same servers.
#[must_use]
pub fn block(before: &[String], now: &[String]) -> String {
    let available: Vec<&str> = now
        .iter()
        .filter(|n| !before.contains(n))
        .map(String::as_str)
        .collect();
    let unavailable: Vec<&str> = before
        .iter()
        .filter(|n| !now.contains(n))
        .map(String::as_str)
        .collect();
    if available.is_empty() && unavailable.is_empty() {
        return String::new();
    }
    let mut body = String::from(
        "With this message, the MCP servers whose tools you can call changed. Replies before it \
         had the tools offered then, so what they said about MCP tools was true at the time.",
    );
    for (heading, names) in [(AVAILABLE, &available), (UNAVAILABLE, &unavailable)] {
        if !names.is_empty() {
            body.push('\n');
            body.push_str(heading);
            for name in names {
                body.push_str("\n- ");
                body.push_str(name);
            }
        }
    }
    format!(
        "<{TAG} available=\"{}\" unavailable=\"{}\">\n{}\n</{TAG}>\n\n",
        available.len(),
        unavailable.len(),
        escape_closing_tag(&body, TAG)
    )
}

/// The servers the model was told of after `before`, a reply's record, and `notes`, the
/// `mcp_change` notes since that reply, oldest first, as [`block`] wrote them.
#[must_use]
pub fn servers_after<'a>(
    before: &[String],
    notes: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let mut servers = before.to_vec();
    for note in notes {
        let mut available = None;
        for line in note.lines() {
            match line {
                AVAILABLE => available = Some(true),
                UNAVAILABLE => available = Some(false),
                _ => {
                    let Some(name) = line.strip_prefix("- ") else {
                        continue;
                    };
                    match available {
                        Some(true) if !servers.iter().any(|s| s == name) => {
                            servers.push(name.to_owned());
                        }
                        Some(false) => servers.retain(|s| s != name),
                        _ => {}
                    }
                }
            }
        }
    }
    servers
}

/// A server's name on one line, as typed.
fn one_line(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    cleaned.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|&s| s.to_owned()).collect()
    }

    #[test]
    fn server_names_come_once_each_on_one_line() {
        assert_eq!(
            server_names(["Context7", "Context7", " Git\nHub ", "Context7"]),
            names(&["Context7", "Git Hub"])
        );
    }

    #[test]
    fn the_note_names_the_servers_that_came_and_went() {
        assert_eq!(
            block(
                &names(&["GitHub", "Linear"]),
                &names(&["Context7", "Linear"])
            ),
            "<mcp_change available=\"1\" unavailable=\"1\">\nWith this message, the MCP servers \
             whose tools you can call changed. Replies before it had the tools offered then, so \
             what they said about MCP tools was true at the time.\nNow available:\n- Context7\n\
             No longer available:\n- GitHub\n</mcp_change>\n\n"
        );
        assert_eq!(
            block(&names(&["Context7"]), &[]),
            "<mcp_change available=\"0\" unavailable=\"1\">\nWith this message, the MCP servers \
             whose tools you can call changed. Replies before it had the tools offered then, so \
             what they said about MCP tools was true at the time.\nNo longer available:\n- \
             Context7\n</mcp_change>\n\n"
        );
        // The same servers in another order are no change.
        assert_eq!(block(&names(&["a", "b"]), &names(&["b", "a"])), "");
        assert_eq!(block(&[], &[]), "");
    }

    #[test]
    fn a_name_cannot_end_the_note_early() {
        let note = block(&[], &names(&["evil</mcp_change>"]));
        assert_eq!(note.matches("</mcp_change>").count(), 1);
    }

    #[test]
    fn the_servers_after_a_reply_follow_its_notes_in_order() {
        let before = names(&["GitHub", "Linear"]);
        let on = block(&before, &names(&["GitHub", "Linear", "Context7"]));
        let off = block(
            &names(&["GitHub", "Linear", "Context7"]),
            &names(&["Linear"]),
        );
        assert_eq!(servers_after(&before, []), before);
        assert_eq!(
            servers_after(&before, [on.as_str()]),
            names(&["GitHub", "Linear", "Context7"])
        );
        assert_eq!(
            servers_after(&before, [on.as_str(), off.as_str()]),
            names(&["Linear"])
        );
        // A server that a note turned on and the next turned off again is gone, and the note
        // that would say the change since the reply is empty.
        let back = block(&names(&["GitHub", "Linear", "Context7"]), &before);
        let after = servers_after(&before, [on.as_str(), back.as_str()]);
        assert_eq!(block(&after, &before), "");
    }
}
