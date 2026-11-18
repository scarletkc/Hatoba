use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use crate::provider::Protocol;

pub const IMAGE_TOKENS: usize = 4096;
const MAX_BYTES: usize = 4 * 1024 * 1024;

enum Part<'a> {
    Text(&'a str),
    Image {
        name: &'a str,
        mime: &'a str,
        data: &'a str,
    },
}

fn parts(text: &str) -> Vec<Part<'_>> {
    let mut parts = Vec::new();
    let mut rest = text;
    let mut text_start = 0;
    let mut seen = Vec::new();
    let mut attachments = false;
    while let Some((header, body)) = rest
        .strip_prefix('<')
        .and_then(|value| value.split_once(">\n"))
    {
        let tag = header.split(' ').next().unwrap_or_default();
        let note = matches!(
            tag,
            "interrupted"
                | "host_change"
                | "terminal_change"
                | "shell_change"
                | "model_change"
                | "permission_mode"
                | "memory"
                | "file_revert"
                | "terminal_state"
                | "mcp_tools"
                | "mcp_change"
        );
        if !note
            && !matches!(
                tag,
                "terminal_selection"
                    | "connection_diagnostics"
                    | "pasted_text"
                    | "file"
                    | "image"
                    | "host_file"
                    | "host_directory"
                    | "mcp_resource"
                    | "skill"
                    | "mcp_prompt"
            )
        {
            break;
        }
        if (note && (attachments || (seen.contains(&tag) && tag != "file_revert")))
            || (matches!(tag, "terminal_selection" | "connection_diagnostics")
                && seen.contains(&tag))
        {
            break;
        }
        let close = format!("\n</{tag}>");
        let Some(at) = body.match_indices(&close).find_map(|(at, _)| {
            let after = &body[at + close.len()..];
            (after.is_empty() || after.starts_with('\n')).then_some(at)
        }) else {
            break;
        };
        let data = &body[..at];
        let after = &body[at + close.len()..];
        let after = after
            .strip_prefix("\n\n")
            .or_else(|| after.strip_prefix('\n'))
            .unwrap_or(after);
        if tag == "image" {
            let Some((name, mime)) = header
                .strip_prefix("image name=\"")
                .and_then(|value| value.split_once("\" mime=\""))
            else {
                break;
            };
            let Some(mime) = mime.strip_suffix('"') else {
                break;
            };
            if !matches!(
                mime,
                "image/png" | "image/jpeg" | "image/gif" | "image/webp"
            ) || data.len() > MAX_BYTES.div_ceil(3) * 4
                || !STANDARD
                    .decode(data)
                    .is_ok_and(|bytes| bytes.len() <= MAX_BYTES)
            {
                break;
            }
            let offset = text.len() - rest.len();
            if offset > text_start {
                parts.push(Part::Text(&text[text_start..offset]));
            }
            parts.push(Part::Image { name, mime, data });
            text_start = text.len() - after.len();
        }
        seen.push(tag);
        attachments |= !note;
        rest = after;
    }
    if text_start < text.len() {
        parts.push(Part::Text(&text[text_start..]));
    }
    parts
}

pub fn user_content(text: &str, protocol: Protocol) -> Value {
    let parts = parts(text);
    if !parts.iter().any(|part| matches!(part, Part::Image { .. })) {
        return match protocol {
            Protocol::Anthropic => json!([{"type": "text", "text": text}]),
            _ => json!(text),
        };
    }
    Value::Array(
        parts
            .into_iter()
            .map(|part| match part {
                Part::Text(text) => json!({
                    "type": if protocol == Protocol::Responses { "input_text" } else { "text" },
                    "text": text,
                }),
                Part::Image { mime, data, .. } => match protocol {
                    Protocol::Anthropic => json!({
                        "type": "image",
                        "source": {"type": "base64", "media_type": mime, "data": data},
                    }),
                    Protocol::Responses => json!({
                        "type": "input_image", "image_url": format!("data:{mime};base64,{data}"),
                    }),
                    _ => json!({
                        "type": "image_url", "image_url": {"url": format!("data:{mime};base64,{data}")},
                    }),
                },
            })
            .collect(),
    )
}

pub fn estimated_chars(text: &str) -> usize {
    parts(text)
        .iter()
        .map(|part| match part {
            Part::Text(text) => text.encode_utf16().count(),
            Part::Image { .. } => IMAGE_TOKENS * 4,
        })
        .sum()
}

pub fn request_chars(value: &Value) -> usize {
    let (encoded, images) = image_chars(value);
    value.to_string().chars().count().saturating_sub(encoded) + images * IMAGE_TOKENS * 4
}

fn image_chars(value: &Value) -> (usize, usize) {
    match value {
        Value::Object(fields) => {
            if matches!(
                fields.get("type").and_then(Value::as_str),
                Some("image" | "image_url" | "input_image")
            ) {
                return (value.to_string().chars().count(), 1);
            }
            fields
                .values()
                .map(image_chars)
                .fold((0, 0), |total, next| (total.0 + next.0, total.1 + next.1))
        }
        Value::Array(values) => values
            .iter()
            .map(image_chars)
            .fold((0, 0), |total, next| (total.0 + next.0, total.1 + next.1)),
        _ => (0, 0),
    }
}

pub fn searchable_text(text: &str) -> String {
    parts(text)
        .into_iter()
        .map(|part| match part {
            Part::Text(text) => text.to_owned(),
            Part::Image { name, .. } => format!("[image {name}]\n\n"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMAGE: &str =
        "<image name=\"screen.png\" mime=\"image/png\">\naGk=\n</image>\n\nWhat is wrong?";

    #[test]
    fn images_use_each_protocols_native_content() {
        let chat = user_content(IMAGE, Protocol::ChatCompletions);
        assert_eq!(
            chat[0],
            json!({"type": "image_url", "image_url": {"url": "data:image/png;base64,aGk="}})
        );
        assert_eq!(chat[1]["text"], "What is wrong?");
        let responses = user_content(IMAGE, Protocol::Responses);
        assert_eq!(responses[0]["type"], "input_image");
        assert_eq!(responses[1]["type"], "input_text");
        let anthropic = user_content(IMAGE, Protocol::Anthropic);
        assert_eq!(
            anthropic[0]["source"],
            json!({"type": "base64", "media_type": "image/png", "data": "aGk="})
        );
    }

    #[test]
    fn images_after_shell_changes_use_native_content() {
        let prefix = "<shell_change from=\"sh\" to=\"bash\">\nShell changed\n</shell_change>\n\n";
        let content = user_content(&format!("{prefix}{IMAGE}"), Protocol::Responses);
        assert_eq!(content[0]["text"], prefix);
        assert_eq!(content[1]["type"], "input_image");
        assert_eq!(content[2]["text"], "What is wrong?");
    }

    #[test]
    fn plain_text_and_images_inside_text_files_stay_text() {
        let text = format!("<file name=\"example.txt\" lines=\"5\">\n{IMAGE}\n</file>\n\nExplain");
        assert_eq!(user_content(&text, Protocol::Responses), json!(text));
        assert_eq!(
            user_content(&format!("Example:\n\n{IMAGE}"), Protocol::Responses),
            json!(format!("Example:\n\n{IMAGE}"))
        );
    }

    #[test]
    fn history_and_usage_do_not_treat_image_data_as_text() {
        assert_eq!(
            searchable_text(IMAGE),
            "[image screen.png]\n\nWhat is wrong?"
        );
        let plain = json!({"messages": [{"role": "user", "content": "hello"}]});
        assert_eq!(request_chars(&plain), plain.to_string().chars().count());
        let content = user_content(IMAGE, Protocol::Responses);
        assert!(request_chars(&content) >= IMAGE_TOKENS * 4);
    }

    #[test]
    fn notes_text_files_and_multiple_images_keep_their_order() {
        let prefix = "<permission_mode mode=\"manual\">\nManual\n</permission_mode>\n\n<file name=\"a.txt\" lines=\"1\">\nabc\n</file>\n\n";
        let text = format!("{prefix}{}\n\n{IMAGE}", IMAGE.split("\n\n").next().unwrap());
        let content = user_content(&text, Protocol::Responses);
        assert_eq!(content[0]["text"], prefix);
        assert_eq!(content[1]["type"], "input_image");
        assert_eq!(content[2]["type"], "input_image");
        assert_eq!(content[3]["text"], "What is wrong?");
        assert_eq!(
            estimated_chars(&text),
            prefix.encode_utf16().count() + IMAGE_TOKENS * 8 + "What is wrong?".len()
        );
    }
}
