//! Plain text from HTML without a DOM: search snippets, and the fallback for pages whose nesting
//! is too deep to convert to Markdown safely. Linear time, no recursion.

/// Elements whose content is never text.
const SKIPPED: &[&str] = &["script", "style", "noscript", "template", "svg", "head"];
/// Elements that start a new line.
const BLOCKS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "br",
    "dd",
    "div",
    "dl",
    "dt",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "td",
    "th",
    "tr",
    "ul",
];

/// Decodes the common named entities and every numeric one.
pub(crate) fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest[1..]
            .find(';')
            .filter(|&end| end <= 10)
            .and_then(|end| {
                let name = &rest[1..=end];
                let c = match name {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some(' '),
                    _ => name
                        .strip_prefix("#x")
                        .or_else(|| name.strip_prefix("#X"))
                        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                        .or_else(|| name.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                        .and_then(char::from_u32),
                }?;
                Some((c, end + 2))
            });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The tag name at the start of `tag` (after `<` or `</`), lowercase.
fn tag_name(tag: &str) -> String {
    tag.chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Text of an HTML fragment or document, with block elements on their own lines.
pub(crate) fn document_text(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut raw = String::with_capacity(html.len() / 2);
    let mut i = 0;
    while i < html.len() {
        let Some(lt) = html[i..].find('<').map(|p| i + p) else {
            raw.push_str(&html[i..]);
            break;
        };
        raw.push_str(&html[i..lt]);
        let after = &html[lt + 1..];
        if after.starts_with("!--") {
            i = lower[lt..]
                .find("-->")
                .map_or(html.len(), |end| lt + end + 3);
            continue;
        }
        let closing = after.starts_with('/');
        let name = tag_name(if closing { &after[1..] } else { after });
        if name.is_empty() && !after.starts_with('!') && !after.starts_with('?') {
            // A lone `<` in text.
            raw.push('<');
            i = lt + 1;
            continue;
        }
        let tag_end = html[lt..].find('>').map_or(html.len(), |end| lt + end + 1);
        if !closing && SKIPPED.contains(&name.as_str()) {
            let close = format!("</{name}");
            i = lower[tag_end..]
                .find(&close)
                .and_then(|p| html[tag_end + p..].find('>').map(|e| tag_end + p + e + 1))
                .unwrap_or(html.len());
            continue;
        }
        if BLOCKS.contains(&name.as_str()) {
            raw.push('\n');
        } else {
            raw.push(' ');
        }
        i = tag_end;
    }
    let decoded = decode_entities(&raw);
    let mut out = String::with_capacity(decoded.len());
    for line in decoded.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&line);
    }
    out
}

/// Single-line text of an HTML snippet (search results).
pub(crate) fn plain_text(html: &str) -> String {
    document_text(html)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_decode() {
        assert_eq!(
            decode_entities("a &amp; b &lt;c&gt; &#39;d&#x27; &quot;&nbsp;&bogus; &"),
            "a & b <c> 'd' \" &bogus; &"
        );
        assert_eq!(decode_entities("&#128512;"), "😀");
        assert_eq!(decode_entities("&#xD800;"), "&#xD800;");
    }

    #[test]
    fn documents_become_text() {
        let html = "<html><head><title>T</title><style>p{}</style></head><body>\
            <h1>Title</h1><p>One <b>bold</b>&amp;more</p><!-- hidden -->\
            <script>alert('<p>x</p>')</script><SCRIPT>bad()</SCRIPT>\
            <ul><li>a</li><li>b</li></ul>1 < 2</body></html>";
        assert_eq!(document_text(html), "Title\nOne bold &more\na\nb\n1 < 2");
        assert_eq!(plain_text("Use <strong>x</strong>\n now"), "Use x now");
        assert_eq!(document_text("<div><script>never closed"), "");
    }
}
