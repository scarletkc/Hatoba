//! `fetch_url` (AI-15).
//!
//! GET only, `http`/`https` only, no cookies, no credentials, at most 5 redirects followed by
//! hand, through the system proxy when one is set. Redirects are followed only on the host the
//! call named: a redirect to another host ends the fetch with that URL, so a fetch of it goes
//! through approval like any other (AI-19), and an allowed domain cannot pass the call on to one
//! that is not. Every hop is checked: an IP-literal host and
//! `localhost` directly, and a host name the client resolves itself through a resolver that
//! refuses names resolving to any non-public address. The connection uses exactly the addresses
//! the resolver checked, so DNS rebinding between the check and the connection is not possible.
//! A proxy resolves the names it is sent itself, so behind one the check of a name's addresses is
//! the proxy's to make; the names the resolver sees that no hop requests are the proxy's own,
//! which may well be local, and resolve unchecked.
//!
//! Text, HTML, JSON and XML are accepted. HTML becomes Markdown: the page's main content, or the
//! whole page when the call asks for it or no main content is found ([`extract`]), with links
//! made absolute. The download is capped at 5 MB and the whole fetch at 90 s. The result is up to
//! 30,000 characters from `offset`.

mod extract;

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use markup5ever_rcdom::{Node, NodeData};
use reqwest::StatusCode;
use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use url::{Host, Url};

use super::text::document_text;
use crate::error::AiError;
use crate::net::{
    self, BLOCKED_MESSAGE, GuardedResolver, Lookup, is_blocked, refuse_blocked, system_lookup,
};

/// Characters returned per call.
pub const MAX_FETCH_CHARS: usize = 30_000;
/// Bytes downloaded at most; the rest of a longer body is left out.
const MAX_DOWNLOAD_BYTES: usize = 5 * 1024 * 1024;
/// Overall limit of one fetch, redirects included.
const FETCH_TIMEOUT: Duration = Duration::from_secs(90);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REDIRECTS: usize = 5;
/// DOM depth above which HTML is reduced to text instead of Markdown.
const MAX_DOM_DEPTH: usize = 2_000;
/// Main content shorter than this, in characters of Markdown, gives way to the whole page.
const MIN_MAIN_CHARS: usize = 500;
/// Characters of a page title kept.
const MAX_TITLE_CHARS: usize = 300;
/// Stack of the thread that converts HTML (the converter recurses once per nesting level).
const CONVERT_STACK: usize = 64 * 1024 * 1024;
const ACCEPT_VALUE: &str = "text/html, application/xhtml+xml, text/markdown, text/plain, \
     application/json, application/xml;q=0.9, text/*;q=0.8";
const REDIRECT_BLOCKED: &str = "the URL redirected to an address on a loopback, private, \
     link-local or otherwise non-public network, which fetch_url does not fetch";

/// A fetched page, or one page of it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchResult {
    /// The final URL, after redirects.
    pub url: String,
    /// Where `url` redirected to, on another host, which is not followed. The result then has no
    /// content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect: Option<String>,
    /// The media type, e.g. `text/html` (without parameters).
    pub content_type: String,
    /// The `<title>` of an HTML page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Which part of an HTML page `content` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PagePart>,
    /// Character offset of `content`.
    pub offset: u64,
    /// Characters in the whole converted content.
    pub total_length: u64,
    /// Up to 30,000 characters from `offset`.
    pub content: String,
    /// The body was longer than the 5 MB download limit and was cut.
    pub truncated: bool,
}

/// Which part of an HTML page a [`FetchResult`] holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PagePart {
    /// The main content, without navigation, sidebars and footers.
    Main,
    /// The whole page: the call asked for it, or no main content was found.
    Whole,
}

/// Fetches `url` and returns up to 30,000 characters from `offset` (characters, not bytes) of
/// its content, HTML converted to Markdown: the main content, or with `full_page` the whole page
/// (AI-15). Refuses non-public addresses at every hop.
pub async fn fetch_url(
    url: &str,
    offset: u64,
    full_page: bool,
    cancel: &CancellationToken,
) -> Result<FetchResult, AiError> {
    Fetcher::public()
        .fetch(url, offset, full_page, cancel)
        .await
}

/// The `fetch_url` result text: a header line with the URL, the content type, the part of an
/// HTML page and the character range, the page title, a line on how to read on when more remains,
/// then the content.
#[must_use]
pub fn format_fetch_result(r: &FetchResult) -> String {
    if let Some(to) = &r.redirect {
        return format!(
            "URL: {} | Redirects to another host: {to}\n\
             fetch_url follows redirects only on the host it was given. To read the page, call \
             fetch_url with that URL.\n",
            r.url
        );
    }
    let end = r.offset + r.content.chars().count() as u64;
    let mut out = format!(
        "URL: {} | Content type: {}{} | Characters {}-{} of {}{}\n",
        r.url,
        r.content_type,
        match r.part {
            Some(PagePart::Main) => " | Main content",
            Some(PagePart::Whole) => " | Whole page",
            None => "",
        },
        r.offset.min(r.total_length),
        end,
        r.total_length,
        if r.truncated {
            " (the page was longer than 5 MB and was cut)"
        } else {
            ""
        }
    );
    if let Some(title) = &r.title {
        out.push_str(&format!("Title: {title}\n"));
    }
    if r.part == Some(PagePart::Main) {
        out.push_str(
            "Navigation, sidebars and footers are left out. For the whole page, call fetch_url \
             with full_page true.\n",
        );
    }
    if end < r.total_length {
        // The whole page reads the same with full_page, which skips looking for main content.
        let whole = if r.part == Some(PagePart::Whole) {
            " and full_page true"
        } else {
            ""
        };
        out.push_str(&format!(
            "More content follows: call fetch_url again with offset {end}{whole} to read on.\n"
        ));
    } else if r.content.is_empty() {
        out.push_str("No content at this offset.\n");
    }
    out.push('\n');
    out.push_str(&r.content);
    out
}

/// `fetch_url` with an injectable address policy, resolver and proxy (tests use them to reach the
/// mock server on 127.0.0.1); [`fetch_url`] always uses [`Fetcher::public`].
pub(crate) struct Fetcher {
    allowed: fn(IpAddr) -> bool,
    lookup: Lookup,
    max_bytes: usize,
    timeout: Duration,
    proxy: Proxy,
}

/// Where a [`Fetcher`] sends its requests.
pub(crate) enum Proxy {
    /// Through the system proxy when one is set, else straight to the host.
    System,
    /// Straight to the host, whatever proxy the environment sets.
    #[cfg(test)]
    None,
    /// Through the proxy at this URL.
    #[cfg(test)]
    At(String),
}

/// How a fetch ended.
enum Hop {
    /// The page.
    Page(Fetched),
    /// `at` redirected to `to`, on another host.
    Elsewhere { at: Url, to: Url },
}

/// What one successful hop returned.
struct Fetched {
    url: Url,
    media_type: String,
    charset: Option<String>,
    body: Vec<u8>,
    truncated: bool,
}

impl Fetcher {
    pub(crate) fn public() -> Self {
        Self {
            allowed: |ip| !is_blocked(ip),
            lookup: system_lookup(),
            max_bytes: MAX_DOWNLOAD_BYTES,
            timeout: FETCH_TIMEOUT,
            proxy: Proxy::System,
        }
    }

    /// A fetcher that connects straight to the host, so the mock server is reached whatever
    /// proxy the environment sets.
    #[cfg(test)]
    pub(crate) fn for_tests(
        allowed: fn(IpAddr) -> bool,
        lookup: Lookup,
        max_bytes: usize,
        timeout: Duration,
    ) -> Self {
        Self {
            allowed,
            lookup,
            max_bytes,
            timeout,
            proxy: Proxy::None,
        }
    }

    /// The client of one fetch. `targets` are the host names its hops request, which the resolver
    /// holds to the policy.
    fn client(&self, targets: &Arc<Mutex<HashSet<String>>>) -> Result<reqwest::Client, AiError> {
        net::ensure_crypto_provider();
        let builder = match &self.proxy {
            Proxy::System => reqwest::Client::builder(),
            #[cfg(test)]
            Proxy::None => reqwest::Client::builder().no_proxy(),
            #[cfg(test)]
            Proxy::At(url) => reqwest::Client::builder()
                .proxy(reqwest::Proxy::all(url).map_err(|e| AiError::Network(e.to_string()))?),
        };
        builder
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .dns_resolver(GuardedResolver {
                allowed: self.allowed,
                lookup: Arc::clone(&self.lookup),
                refusal: refuse_blocked,
                targets: Arc::clone(targets),
            })
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent(net::USER_AGENT)
            .build()
            .map_err(|_| AiError::Network("could not build the HTTP client".into()))
    }

    /// Refuses an IP-literal host outside the policy, and `localhost` when the policy refuses
    /// loopback, by its name, since behind a proxy the resolver never sees it. Other host names go
    /// through the resolver.
    fn check_host(&self, url: &Url, redirected: bool) -> Result<(), AiError> {
        let ip: IpAddr = match url.host() {
            Some(Host::Ipv4(ip)) => ip.into(),
            Some(Host::Ipv6(ip)) => ip.into(),
            Some(Host::Domain(name)) if is_localhost(name) => Ipv4Addr::LOCALHOST.into(),
            Some(Host::Domain(_)) => return Ok(()),
            None => return Err(AiError::InvalidUrl("the URL has no host".into())),
        };
        if (self.allowed)(ip) {
            Ok(())
        } else {
            Err(blocked(redirected))
        }
    }

    pub(crate) async fn fetch(
        &self,
        url: &str,
        offset: u64,
        full_page: bool,
        cancel: &CancellationToken,
    ) -> Result<FetchResult, AiError> {
        let url = parse_url(url)?;
        let targets = Arc::new(Mutex::new(HashSet::new()));
        let client = self.client(&targets)?;
        let hop = tokio::time::timeout(self.timeout, self.follow(&client, &targets, url, cancel))
            .await
            .map_err(|_| AiError::Network("fetching the page timed out".into()))??;
        let fetched = match hop {
            Hop::Page(fetched) => fetched,
            Hop::Elsewhere { at, to } => {
                return Ok(FetchResult {
                    url: at.to_string(),
                    redirect: Some(to.to_string()),
                    ..FetchResult::default()
                });
            }
        };
        let is_html = matches!(
            fetched.media_type.as_str(),
            "text/html" | "application/xhtml+xml"
        );
        let text = decode(&fetched.body, fetched.charset.as_deref(), is_html);
        let (content, title, part) = if is_html {
            let converted = tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(AiError::Cancelled),
                converted = html_to_markdown(text, fetched.url.clone(), full_page) => converted,
            };
            (converted.markdown, converted.title, Some(converted.part))
        } else {
            (text, None, None)
        };
        Ok(FetchResult {
            title,
            part,
            ..page(
                fetched.url.to_string(),
                fetched.media_type,
                &content,
                offset,
                fetched.truncated,
            )
        })
    }

    async fn follow(
        &self,
        client: &reqwest::Client,
        targets: &Mutex<HashSet<String>>,
        mut url: Url,
        cancel: &CancellationToken,
    ) -> Result<Hop, AiError> {
        let host = url.host().map(|h| h.to_owned());
        let mut redirects = 0;
        loop {
            let redirected = redirects > 0;
            self.check_host(&url, redirected)?;
            if let Some(Host::Domain(name)) = url.host() {
                targets
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(name.to_ascii_lowercase());
            }
            let request = client.get(url.clone()).header(ACCEPT, ACCEPT_VALUE);
            let response = match net::send(request, cancel).await {
                Err(AiError::Blocked(_)) => return Err(blocked(redirected)),
                other => other?,
            };
            let status = response.status();
            if matches!(
                status,
                StatusCode::MOVED_PERMANENTLY
                    | StatusCode::FOUND
                    | StatusCode::SEE_OTHER
                    | StatusCode::TEMPORARY_REDIRECT
                    | StatusCode::PERMANENT_REDIRECT
            ) {
                if redirects == MAX_REDIRECTS {
                    return Err(AiError::Network("too many redirects (more than 5)".into()));
                }
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| {
                        AiError::Protocol("a redirect without a usable Location header".into())
                    })?;
                let next = url
                    .join(location)
                    .map_err(|_| AiError::InvalidUrl("the redirect target is not a URL".into()))?;
                let next = parse_url(next.as_str())?;
                if next.host().map(|h| h.to_owned()) != host {
                    // Refused addresses still fail here, without a request.
                    self.check_host(&next, true)?;
                    return Ok(Hop::Elsewhere { at: url, to: next });
                }
                url = next;
                redirects += 1;
                continue;
            }
            if !status.is_success() {
                return Err(AiError::Http {
                    status: status.as_u16(),
                    message: status
                        .canonical_reason()
                        .unwrap_or("the server refused the request")
                        .to_owned(),
                });
            }
            let header = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let (media_type, charset) = parse_content_type(header);
            if media_type.is_empty() {
                return Err(AiError::UnsupportedContentType(
                    "the server sent no content type".into(),
                ));
            }
            if !is_text_type(&media_type) {
                return Err(AiError::UnsupportedContentType(format!(
                    "{} (only text, HTML, JSON and XML are fetched)",
                    net::clean_message(&media_type)
                )));
            }
            let (body, truncated) = net::read_capped(response, self.max_bytes, cancel).await?;
            return Ok(Hop::Page(Fetched {
                url,
                media_type,
                charset,
                body,
                truncated,
            }));
        }
    }
}

/// `localhost` and its subdomains, which resolve to loopback (RFC 6761), with or without the
/// final dot.
fn is_localhost(name: &str) -> bool {
    let name = name.strip_suffix('.').unwrap_or(name).to_ascii_lowercase();
    name == "localhost" || name.ends_with(".localhost")
}

fn blocked(redirected: bool) -> AiError {
    AiError::Blocked(
        if redirected {
            REDIRECT_BLOCKED
        } else {
            BLOCKED_MESSAGE
        }
        .into(),
    )
}

/// Parses and checks a URL to fetch: `http`/`https`, a host, no user name or password.
fn parse_url(url: &str) -> Result<Url, AiError> {
    let mut url =
        Url::parse(url.trim()).map_err(|_| AiError::InvalidUrl("not a valid URL".into()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AiError::InvalidUrl(
            "only http and https URLs are fetched".into(),
        ));
    }
    if url.host().is_none() {
        return Err(AiError::InvalidUrl("the URL has no host".into()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AiError::InvalidUrl(
            "URLs with a user name or password are not fetched".into(),
        ));
    }
    url.set_fragment(None);
    Ok(url)
}

/// `(media type, charset)` from a `Content-Type` header, lowercase.
fn parse_content_type(header: &str) -> (String, Option<String>) {
    let mut parts = header.split(';');
    let media_type = parts.next().unwrap_or("").trim().to_ascii_lowercase();
    let charset = parts.find_map(|p| {
        let (key, value) = p.split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| value.trim().trim_matches('"').to_ascii_lowercase())
    });
    (media_type, charset)
}

/// Text, HTML, JSON and XML, including `+json` and `+xml` types.
fn is_text_type(media_type: &str) -> bool {
    media_type.starts_with("text/")
        || matches!(
            media_type,
            "application/json" | "application/xml" | "application/xhtml+xml"
        )
        || media_type.ends_with("+json")
        || media_type.ends_with("+xml")
}

/// A `charset=` label in the first bytes of an HTML document (`<meta charset>` or the
/// `http-equiv` form).
fn sniff_meta_charset(body: &[u8]) -> Option<String> {
    let head = String::from_utf8_lossy(&body[..body.len().min(4096)]).to_ascii_lowercase();
    let at = head.find("charset=")? + "charset=".len();
    let label: String = head[at..]
        .trim_start_matches(['"', '\'', ' '])
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
        .collect();
    (!label.is_empty()).then_some(label)
}

/// Decodes a body: a BOM wins, then the header's charset, then (HTML) a `<meta>` charset, then
/// UTF-8. Invalid sequences become U+FFFD.
fn decode(body: &[u8], charset: Option<&str>, html: bool) -> String {
    let label = charset
        .map(str::to_owned)
        .or_else(|| html.then(|| sniff_meta_charset(body)).flatten());
    let encoding = label
        .and_then(|l| encoding_rs::Encoding::for_label(l.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    let (text, _, _) = encoding.decode(body);
    text.into_owned()
}

/// An HTML page as Markdown.
struct Converted {
    markdown: String,
    title: Option<String>,
    part: PagePart,
}

/// Converts HTML to Markdown on a thread with a large stack (the converter recurses once per
/// nesting level). Pages nested deeper than 2,000 levels are reduced to plain text.
async fn html_to_markdown(html: String, url: Url, full_page: bool) -> Converted {
    let html = Arc::new(html);
    let (tx, rx) = tokio::sync::oneshot::channel();
    let input = Arc::clone(&html);
    let spawned = std::thread::Builder::new()
        .name("hatoba-ai-html".into())
        .stack_size(CONVERT_STACK)
        .spawn(move || {
            let _ = tx.send(convert_html(&input, &url, full_page));
        });
    let converted = match spawned {
        Ok(_) => rx.await.ok(),
        Err(_) => None,
    };
    // The thread could not start, or the converter panicked: plain text still serves the model.
    converted.unwrap_or_else(|| Converted {
        markdown: document_text(&html),
        title: None,
        part: PagePart::Whole,
    })
}

/// The main content of the page at `url` as Markdown, or the whole page with `full_page`, when
/// it has too many elements to search, or when its main content is missing or short.
fn convert_html(html: &str, url: &Url, full_page: bool) -> Converted {
    let converter = markdown_converter();
    let Ok(tree) = converter.html_to_tree(html) else {
        return Converted {
            markdown: document_text(html),
            title: None,
            part: PagePart::Whole,
        };
    };
    let title = page_title(&tree);
    let (depth, elements) = dom_size(&tree);
    if depth > MAX_DOM_DEPTH {
        return Converted {
            markdown: document_text(html),
            title,
            part: PagePart::Whole,
        };
    }
    let base = base_url(&tree, url);
    let main = (!full_page && elements <= extract::MAX_ELEMENTS)
        .then(|| extract::main_content(html, url.as_str()))
        .flatten()
        .and_then(|main| converter.html_to_tree(&main).ok())
        .map(|main| {
            absolute_links(&main, &base);
            converter.tree_to_markdown(&main)
        })
        .filter(|markdown| markdown.chars().count() >= MIN_MAIN_CHARS);
    let (markdown, part) = match main {
        Some(markdown) => (markdown, PagePart::Main),
        None => {
            absolute_links(&tree, &base);
            (converter.tree_to_markdown(&tree), PagePart::Whole)
        }
    };
    Converted {
        markdown,
        title,
        part,
    }
}

fn markdown_converter() -> htmd::HtmlToMarkdown {
    use htmd::element_handler::Handlers;

    htmd::HtmlToMarkdown::builder()
        // The title is in the result's header.
        .skip_tags(vec![
            "title", "script", "style", "noscript", "template", "svg", "canvas", "iframe",
            "object", "embed", "link", "meta",
        ])
        // Inline `data:` images would flood the page with base64; keep their alt text only.
        .add_handler(
            vec!["img"],
            |handlers: &dyn Handlers, element: htmd::Element| {
                let attr = |name: &str| {
                    element
                        .attrs
                        .iter()
                        .find(|a| &*a.name.local == name)
                        .map(|a| a.value.to_string())
                };
                if attr("src").is_some_and(|src| src.trim_start().starts_with("data:")) {
                    Some(attr("alt").unwrap_or_default().into())
                } else {
                    handlers.fallback(element)
                }
            },
        )
        // htmd drops the line breaks that end the per-line `<span>`s highlighters wrap code in.
        .add_handler(vec!["pre"], |_: &dyn Handlers, element: htmd::Element| {
            Some(code_block(element.node).into())
        })
        .build()
}

/// A `<pre>` as a fenced code block: its text as written, without buttons, in the language its
/// own or its `<code>`'s class names (`language-x` or `lang-x`).
fn code_block(pre: &Rc<Node>) -> String {
    let mut text = String::new();
    let mut language = None;
    let mut stack = vec![Rc::clone(pre)];
    while let Some(node) = stack.pop() {
        match &node.data {
            NodeData::Text { contents } => text.push_str(&contents.borrow()),
            NodeData::Element { name, attrs, .. } => {
                match &*name.local {
                    "button" | "script" | "style" | "template" => continue,
                    "br" => text.push('\n'),
                    _ => {}
                }
                if language.is_none() {
                    language = attrs
                        .borrow()
                        .iter()
                        .find(|a| &*a.name.local == "class")
                        .and_then(|a| class_language(&a.value));
                }
                stack.extend(node.children.borrow().iter().rev().cloned());
            }
            _ => {}
        }
    }
    let code = text.strip_suffix('\n').unwrap_or(&text);
    let longest = code.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!(
        "\n\n{fence}{}\n{code}\n{fence}\n\n",
        language.unwrap_or_default()
    )
}

/// The language a class attribute names with `language-x` or `lang-x`.
fn class_language(class: &str) -> Option<String> {
    class
        .split_whitespace()
        .find_map(|c| {
            c.strip_prefix("language-")
                .or_else(|| c.strip_prefix("lang-"))
        })
        .filter(|l| {
            !l.is_empty()
                && l.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '_' | '#' | '.'))
        })
        .map(str::to_owned)
}

/// The deepest nesting level of a DOM and its number of elements, counted without recursion.
fn dom_size(root: &Rc<Node>) -> (usize, usize) {
    let (mut deepest, mut elements) = (0, 0);
    let mut stack = vec![(Rc::clone(root), 1usize)];
    while let Some((node, depth)) = stack.pop() {
        deepest = deepest.max(depth);
        if matches!(node.data, NodeData::Element { .. }) {
            elements += 1;
        }
        for child in node.children.borrow().iter() {
            stack.push((Rc::clone(child), depth + 1));
        }
    }
    (deepest, elements)
}

/// The first child element of `node` named `name`.
fn child_element(node: &Node, name: &str) -> Option<Rc<Node>> {
    node.children
        .borrow()
        .iter()
        .find(|c| matches!(&c.data, NodeData::Element { name: n, .. } if &*n.local == name))
        .cloned()
}

fn attribute(node: &Node, name: &str) -> Option<String> {
    let NodeData::Element { attrs, .. } = &node.data else {
        return None;
    };
    attrs
        .borrow()
        .iter()
        .find(|a| &*a.name.local == name)
        .map(|a| a.value.to_string())
}

/// The page's `<head>`.
fn head(root: &Node) -> Option<Rc<Node>> {
    let html = child_element(root, "html")?;
    child_element(&html, "head")
}

/// The text of the page's `<title>`, on one line, at most 300 characters.
fn page_title(root: &Node) -> Option<String> {
    let head = head(root)?;
    let title = child_element(&head, "title")?;
    let mut text = String::new();
    for child in title.children.borrow().iter() {
        if let NodeData::Text { contents } = &child.data {
            text.push_str(&contents.borrow());
        }
    }
    let title: String = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect();
    (!title.is_empty()).then_some(title)
}

/// What the page's relative links resolve against: its `<base href>` resolved against `url`, or
/// `url`.
fn base_url(root: &Node, url: &Url) -> Url {
    head(root)
        .and_then(|head| child_element(&head, "base"))
        .and_then(|base| attribute(&base, "href"))
        .and_then(|href| url.join(href.trim()).ok())
        .filter(|base| matches!(base.scheme(), "http" | "https"))
        .unwrap_or_else(|| url.clone())
}

/// Makes the link targets and image sources in a DOM absolute.
fn absolute_links(root: &Rc<Node>, base: &Url) {
    let mut stack = vec![Rc::clone(root)];
    while let Some(node) = stack.pop() {
        if let NodeData::Element { name, attrs, .. } = &node.data {
            let key = match &*name.local {
                "a" => "href",
                "img" => "src",
                _ => "",
            };
            for attr in attrs
                .borrow_mut()
                .iter_mut()
                .filter(|a| !key.is_empty() && &*a.name.local == key)
            {
                if let Ok(absolute) = base.join(attr.value.trim()) {
                    attr.value = absolute.as_str().into();
                }
            }
        }
        stack.extend(node.children.borrow().iter().cloned());
    }
}

/// Up to 30,000 characters of `content` from `offset`.
fn page(
    url: String,
    content_type: String,
    content: &str,
    offset: u64,
    truncated: bool,
) -> FetchResult {
    let total = content.chars().count();
    let start = usize::try_from(offset).unwrap_or(usize::MAX).min(total);
    let byte_start = content
        .char_indices()
        .nth(start)
        .map_or(content.len(), |(i, _)| i);
    let rest = &content[byte_start..];
    let byte_end = rest
        .char_indices()
        .nth(MAX_FETCH_CHARS)
        .map_or(rest.len(), |(i, _)| i);
    FetchResult {
        url,
        content_type,
        offset: start as u64,
        total_length: total as u64,
        content: rest[..byte_end].to_owned(),
        truncated,
        ..FetchResult::default()
    }
}

#[cfg(test)]
mod tests;
