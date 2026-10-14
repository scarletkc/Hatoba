//! The main content of an HTML page (AI-15), in the page's own HTML:
//!
//! 1. Where it is: what Readability chooses, widened to the sections alongside it and to the
//!    `<article>` or `<main>` element holding it, since Readability can take one long section
//!    of a reference page, or start below the page's opening.
//! 2. What goes: blocks that look like navigation (menus, sidebars, footers, asides of links),
//!    and outside Readability's choice, blocks that are mostly links.
//! 3. What stays whatever it looks like: what Readability chose, and code blocks.
//!
//! Readability's own cleanup inside the content is not used: it judges blocks by their class
//! names, and utility classes such as `scroll-mt-2` make code blocks and headings look like
//! clutter to it.

use std::collections::HashSet;

use dom_query::{Document, NodeId, NodeRef};
use dom_smoothie::{Config, Readability};

/// Elements above which a page is not searched for its main content.
pub(super) const MAX_ELEMENTS: usize = 50_000;
/// Marks each element with its position, to find Readability's choice in the page.
const MARK: &str = "data-hatoba-node";
/// Class and id parts of navigation-like blocks (Readability's own list, less the words that
/// also name comments, replies and page headers, which are content here).
const NAVIGATION_NAMES: &[&str] = &[
    "-ad-",
    "ad-break",
    "agegate",
    "ai2html",
    "banner",
    "breadcrumbs",
    "combx",
    "cover-wrap",
    "disqus",
    "footer",
    "gdpr",
    "legends",
    "menu",
    "pager",
    "pagination",
    "popup",
    "related",
    "rss",
    "shoutbox",
    "sidebar",
    "skyscraper",
    "social",
    "sponsor",
    "yom-remote",
];
/// Class and id parts that keep a block whatever else its names say.
const CONTENT_NAMES: &[&str] = &["article", "body", "column", "content", "main", "shadow"];

/// The HTML of the main content of `html`, the page at `url`; `None` when the page does not read
/// as one text (listings, search results, client-rendered shells) or Readability finds none.
pub(super) fn main_content(html: &str, url: &str) -> Option<String> {
    let page = Document::from(html);
    let chosen = readability_choice(html, url, &page)?;
    let keep: HashSet<NodeId> = with_ancestors(chosen.iter().copied())
        .union(&with_ancestors(page.select("pre").nodes().iter().copied()))
        .copied()
        .collect();
    let chosen: HashSet<NodeId> = chosen.iter().map(|n| n.id).collect();
    let roots = outermost(&page, &chosen);
    let widened = match common_start(&roots) {
        Some(start) => vec![with_sections(start)],
        None => roots.clone(),
    };
    let region = marked_content(&widened).map_or(widened, |marked| vec![marked]);
    let roots: HashSet<NodeId> = roots.iter().map(|n| n.id).collect();
    let mut out = String::new();
    for top in region {
        leave_out_navigation(top, &roots, &keep);
        out.push_str(&top.html());
    }
    (!out.is_empty()).then_some(out)
}

/// The elements of `page` that Readability chooses as the content of `html`, or `None` when
/// the page does not read as one text.
fn readability_choice<'a>(html: &str, url: &str, page: &'a Document) -> Option<Vec<NodeRef<'a>>> {
    // Readability changes the document it reads, so it reads a copy whose elements carry their
    // position in `page`.
    let copy = Document::from(html);
    for (i, n) in copy.select("*").nodes().iter().enumerate() {
        n.set_attr(MARK, &i.to_string());
    }
    let config = Config {
        max_elements_to_parse: MAX_ELEMENTS,
        ..Config::default()
    };
    let mut readability = Readability::with_document(copy, Some(url), Some(config)).ok()?;
    if !readability.is_probably_readable() {
        return None;
    }
    let article = readability.parse().ok()?;
    let elements = page.select("*").nodes().to_vec();
    Some(
        Document::from(article.content.as_ref())
            .select(&format!("[{MARK}]"))
            .nodes()
            .iter()
            .filter_map(|n| n.attr(MARK)?.parse::<usize>().ok())
            .filter_map(|i| elements.get(i).copied())
            .collect(),
    )
}

/// Removes the navigation in `top` that holds nothing of `keep`, and outside Readability's
/// choice (`roots`), blocks that are mostly links too: language lists, toolbars, file listings.
fn leave_out_navigation(top: NodeRef, roots: &HashSet<NodeId>, keep: &HashSet<NodeId>) {
    let inside = roots.contains(&top.id);
    let mut stack: Vec<_> = top
        .element_children()
        .into_iter()
        .map(|n| (n, inside))
        .collect();
    while let Some((n, inside)) = stack.pop() {
        if !keep.contains(&n.id) && (is_navigation(&n) || (!inside && mostly_links(&n))) {
            n.remove_from_parent();
        } else if !n.has_name("pre") {
            let inside = inside || roots.contains(&n.id);
            stack.extend(n.element_children().into_iter().map(|c| (c, inside)));
        }
    }
}

/// The nodes and all their ancestors.
fn with_ancestors<'a>(nodes: impl Iterator<Item = NodeRef<'a>>) -> HashSet<NodeId> {
    let mut found = HashSet::new();
    for node in nodes {
        let mut at = Some(node);
        while let Some(n) = at {
            if !found.insert(n.id) {
                break;
            }
            at = n.parent();
        }
    }
    found
}

/// The elements of `chosen` not inside another one, in document order.
fn outermost<'a>(page: &'a Document, chosen: &HashSet<NodeId>) -> Vec<NodeRef<'a>> {
    let mut roots = Vec::new();
    let mut stack = vec![page.root()];
    while let Some(n) = stack.pop() {
        if chosen.contains(&n.id) {
            roots.push(n);
            continue;
        }
        let mut children = n.element_children();
        children.reverse();
        stack.extend(children);
    }
    roots
}

/// The one element Readability's choice starts from: the only root, or the parent the roots
/// share. `None` when they share none below `<body>`.
fn common_start<'a>(roots: &[NodeRef<'a>]) -> Option<NodeRef<'a>> {
    let start = match roots {
        [] => return None,
        [only] => *only,
        [first, rest @ ..] => {
            let parent = first.parent()?;
            if !rest
                .iter()
                .all(|r| r.parent().is_some_and(|p| p.id == parent.id))
            {
                return None;
            }
            parent
        }
    };
    (!is_page_level(&start)).then_some(start)
}

/// `<body>`, `<html>` or the document.
fn is_page_level(n: &NodeRef) -> bool {
    !n.is_element() || n.has_name("body") || n.has_name("html")
}

/// `root`'s parent, and so on up, while it has siblings of its own kind (tag and class): the
/// other sections of a reference page, which Readability leaves out when one section is much
/// longer than the rest.
fn with_sections(root: NodeRef) -> NodeRef {
    let kind = |n: &NodeRef| (n.node_name(), n.attr("class"));
    let mut root = root;
    while let Some(parent) = root.parent() {
        if is_page_level(&parent) {
            break;
        }
        let alike = parent
            .element_children()
            .iter()
            .any(|c| c.id != root.id && kind(c) == kind(&root));
        if !alike {
            break;
        }
        root = parent;
    }
    root
}

/// The innermost `<article>`, `<main>` or `role="main"` element holding all of `roots`: the part
/// the page itself marks as its content, which Readability's choice can start partway into.
fn marked_content<'a>(roots: &[NodeRef<'a>]) -> Option<NodeRef<'a>> {
    let mut at = roots.first().copied();
    while let Some(n) = at {
        let marked = n.has_name("article")
            || n.has_name("main")
            || n.attr("role").is_some_and(|r| r.as_ref() == "main");
        if marked && roots.iter().all(|r| is_within(r, &n)) {
            return Some(n);
        }
        at = n.parent();
    }
    None
}

/// `n` is `ancestor` or inside it.
fn is_within(n: &NodeRef, ancestor: &NodeRef) -> bool {
    let mut at = Some(*n);
    while let Some(n) = at {
        if n.id == ancestor.id {
            return true;
        }
        at = n.parent();
    }
    false
}

/// Navigation, site headers and footers, forms' controls and dialogs, and asides that are
/// mostly links.
fn is_navigation(n: &NodeRef) -> bool {
    let Some(tag) = n.node_name() else {
        return false;
    };
    if matches!(
        tag.as_ref(),
        "nav" | "footer" | "button" | "select" | "input" | "textarea" | "dialog"
    ) {
        return true;
    }
    let role = n.attr("role");
    let role = role.as_deref().unwrap_or("");
    if matches!(
        role,
        "navigation" | "banner" | "contentinfo" | "search" | "dialog" | "menu" | "menubar"
    ) {
        return true;
    }
    if (tag.as_ref() == "aside" || role == "complementary") && mostly_links(n) {
        return true;
    }
    let names = format!(
        "{} {}",
        n.attr("class").unwrap_or_default(),
        n.attr("id").unwrap_or_default()
    )
    .to_ascii_lowercase();
    NAVIGATION_NAMES.iter().any(|w| names.contains(w))
        && !CONTENT_NAMES.iter().any(|w| names.contains(w))
}

/// More than half of the element's text is link text.
fn mostly_links(n: &NodeRef) -> bool {
    let count = |s: &str| s.chars().filter(|c| !c.is_whitespace()).count();
    let total = count(&n.text());
    let links: usize = n
        .descendants_it()
        .filter(|d| d.has_name("a"))
        .map(|a| count(&a.text()))
        .sum();
    total > 0 && links * 2 > total
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://docs.example.test/install/";

    fn paragraphs(topic: &str) -> String {
        (1..=6)
            .map(|i| {
                format!(
                    "<p>Paragraph {i} about {topic}, with enough words in it, commas, clauses, \
                     and details that it reads as the text of an article rather than a menu, \
                     which is what the scoring looks for.</p>"
                )
            })
            .collect()
    }

    #[test]
    fn navigation_is_left_out_and_code_blocks_and_tips_stay() {
        let html = format!(
            r##"<html><head><title>Install</title></head><body>
            <header class="site-header"><a href="/">Home</a></header>
            <div class="row">
              <aside class="td-sidebar"><ul><li><a href="/a">A</a></li><li><a href="/b">B</a></li></ul></aside>
              <main><nav class="breadcrumbs"><a href="/">Docs</a> / <a href="/install/">Install</a></nav>
                <div class="mw-heading"><h2 id="setup">Set up</h2><a href="#edit">edit</a></div>
                {}
                <div class="scroll-mt-2 group"><button>Copy</button><div class="highlight"><pre><code>$ sudo apt install docker-ce</code></pre></div></div>
                <aside class="tip">Restart the shell after installing, so the group change applies.</aside>
                {}
                <div role="navigation"><a href="/prev">Previous</a> <a href="/next">Next</a></div>
              </main>
            </div>
            <footer>© Example</footer></body></html>"##,
            paragraphs("installing"),
            paragraphs("configuring")
        );
        let main = main_content(&html, URL).unwrap();
        for kept in [
            "sudo apt install docker-ce",
            "Set up",
            "Restart the shell",
            "Paragraph 6 about configuring",
        ] {
            assert!(main.contains(kept), "missing {kept:?} in:\n{main}");
        }
        for left_out in ["td-sidebar", "breadcrumbs", "Previous", "Copy", "©", MARK] {
            assert!(
                !main.contains(left_out),
                "{left_out:?} must not appear in:\n{main}"
            );
        }
    }

    #[test]
    fn the_other_sections_and_the_opening_come_back() {
        // A reference page: one long section among short ones, after an opening outside them.
        let html = format!(
            r#"<html><body><nav><a href="/">Home</a></nav>
            <main>
              <div class="intro"><h1>CREATE TABLE</h1><p>Defines a new table.</p></div>
              <ul class="langs"><li><a href="/de/">Deutsch</a></li><li><a href="/fr/">Français</a></li></ul>
              <div class="refentry">
                <div class="refsect1"><h2>Description</h2><p>Creates an empty table.</p></div>
                <div class="refsect1"><h2>Parameters</h2>{}</div>
                <div class="refsect1"><h2>Examples</h2><pre>CREATE TABLE films (code char(5));</pre></div>
                <div class="refsect1"><h2>See Also</h2><p><a href="/alter">ALTER TABLE</a>, <a href="/drop">DROP TABLE</a></p></div>
              </div>
            </main><footer>Footer</footer></body></html>"#,
            paragraphs("parameters")
        );
        let main = main_content(&html, URL).unwrap();
        for kept in [
            "Defines a new table.",
            "Creates an empty table.",
            "Paragraph 6 about parameters",
            "CREATE TABLE films",
        ] {
            assert!(main.contains(kept), "missing {kept:?} in:\n{main}");
        }
        for left_out in ["Home", "Deutsch", "Footer"] {
            assert!(
                !main.contains(left_out),
                "{left_out:?} must not appear in:\n{main}"
            );
        }
    }

    #[test]
    fn pages_that_are_not_one_text_have_none() {
        let listing = format!(
            "<html><body><ul>{}</ul></body></html>",
            (0..50)
                .map(|i| format!("<li><a href=\"/item/{i}\">Item {i}</a></li>"))
                .collect::<String>()
        );
        assert_eq!(main_content(&listing, URL), None);
        assert_eq!(
            main_content("<html><body><div id=\"app\"></div></body></html>", URL),
            None
        );
    }

    #[test]
    fn too_many_elements_have_none() {
        let html = format!(
            "<html><body><article>{}{}</article></body></html>",
            paragraphs("size"),
            "<span>x</span>".repeat(MAX_ELEMENTS)
        );
        assert_eq!(main_content(&html, URL), None);
    }
}
