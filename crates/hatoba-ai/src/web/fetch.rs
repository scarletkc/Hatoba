//! `fetch_url` (AI-15).
//!
//! GET only, `http`/`https` only, no cookies, no credentials, no proxy, at most 5 redirects
//! followed by hand. Every hop is checked: an IP-literal host directly, a host name through a
//! resolver that refuses names resolving to any non-public address. The connection uses exactly
//! the addresses the resolver checked, so DNS rebinding between the check and the connection is
//! not possible. The system proxy is not used, because a proxy resolves names itself and would
//! defeat the check.
//!
//! Text, HTML, JSON and XML are accepted; HTML becomes Markdown. The download is capped at 5 MB
//! and the whole fetch at 30 s. The result is up to 16,000 characters from `offset`.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use reqwest::StatusCode;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use url::{Host, Url};

use super::text::document_text;
use crate::error::AiError;
use crate::net::{self, BLOCKED_MESSAGE, BlockedAddress, is_blocked};

/// Characters returned per call.
pub const MAX_FETCH_CHARS: usize = 16_000;
/// Bytes downloaded at most; the rest of a longer body is left out.
const MAX_DOWNLOAD_BYTES: usize = 5 * 1024 * 1024;
/// Overall limit of one fetch, redirects included.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REDIRECTS: usize = 5;
/// DOM depth above which HTML is reduced to text instead of Markdown.
const MAX_DOM_DEPTH: usize = 2_000;
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
    /// The media type, e.g. `text/html` (without parameters).
    pub content_type: String,
    /// Character offset of `content`.
    pub offset: u64,
    /// Characters in the whole converted content.
    pub total_length: u64,
    /// Up to 16,000 characters from `offset`.
    pub content: String,
    /// The body was longer than the 5 MB download limit and was cut.
    pub truncated: bool,
}

/// Fetches `url` and returns up to 16,000 characters from `offset` (characters, not bytes) of
/// its content, HTML converted to Markdown (AI-15). Refuses non-public addresses at every hop.
pub async fn fetch_url(
    url: &str,
    offset: u64,
    cancel: &CancellationToken,
) -> Result<FetchResult, AiError> {
    Fetcher::public().fetch(url, offset, cancel).await
}

/// The `fetch_url` result text: a header line with the URL, the content type and the character
/// range, a line on how to read on when more remains, then the content.
#[must_use]
pub fn format_fetch_result(r: &FetchResult) -> String {
    let end = r.offset + r.content.chars().count() as u64;
    let mut out = format!(
        "URL: {} | Content type: {} | Characters {}-{} of {}{}\n",
        r.url,
        r.content_type,
        r.offset.min(r.total_length),
        end,
        r.total_length,
        if r.truncated {
            " (the page was longer than 5 MB and was cut)"
        } else {
            ""
        }
    );
    if end < r.total_length {
        out.push_str(&format!(
            "More content follows: call fetch_url again with offset {end} to read on.\n"
        ));
    } else if r.content.is_empty() {
        out.push_str("No content at this offset.\n");
    }
    out.push('\n');
    out.push_str(&r.content);
    out
}

type Lookup = Arc<dyn Fn(String) -> BoxFuture<'static, io::Result<Vec<IpAddr>>> + Send + Sync>;

/// The system resolver.
fn system_lookup() -> Lookup {
    Arc::new(|host: String| {
        Box::pin(async move {
            Ok(tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .map(|addr| addr.ip())
                .collect())
        })
    })
}

/// A resolver that refuses a name when any of its addresses is not allowed, and otherwise
/// returns exactly the addresses it checked (the connection is pinned to them).
struct GuardedResolver {
    allowed: fn(IpAddr) -> bool,
    lookup: Lookup,
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        let allowed = self.allowed;
        let lookup = Arc::clone(&self.lookup);
        Box::pin(async move {
            let addrs = lookup(host).await?;
            if addrs.is_empty() {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no addresses").into());
            }
            if addrs.iter().any(|ip| !allowed(*ip)) {
                return Err(Box::new(BlockedAddress) as Box<dyn std::error::Error + Send + Sync>);
            }
            let addrs: Addrs = Box::new(addrs.into_iter().map(|ip| SocketAddr::new(ip, 0)));
            Ok(addrs)
        })
    }
}

/// `fetch_url` with an injectable address policy and resolver (tests use them to reach the
/// mock server on 127.0.0.1); [`fetch_url`] always uses [`Fetcher::public`].
pub(crate) struct Fetcher {
    allowed: fn(IpAddr) -> bool,
    lookup: Lookup,
    max_bytes: usize,
    timeout: Duration,
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
        }
    }

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
        }
    }

    fn client(&self) -> Result<reqwest::Client, AiError> {
        net::ensure_crypto_provider();
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .dns_resolver(GuardedResolver {
                allowed: self.allowed,
                lookup: Arc::clone(&self.lookup),
            })
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent(net::USER_AGENT)
            .build()
            .map_err(|_| AiError::Network("could not build the HTTP client".into()))
    }

    /// Refuses an IP-literal host outside the policy (host names go through the resolver).
    fn check_host(&self, url: &Url, redirected: bool) -> Result<(), AiError> {
        let ip: IpAddr = match url.host() {
            Some(Host::Ipv4(ip)) => ip.into(),
            Some(Host::Ipv6(ip)) => ip.into(),
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
        cancel: &CancellationToken,
    ) -> Result<FetchResult, AiError> {
        let url = parse_url(url)?;
        let client = self.client()?;
        let fetched = tokio::time::timeout(self.timeout, self.follow(&client, url, cancel))
            .await
            .map_err(|_| AiError::Network("fetching the page timed out".into()))??;
        let is_html = matches!(
            fetched.media_type.as_str(),
            "text/html" | "application/xhtml+xml"
        );
        let text = decode(&fetched.body, fetched.charset.as_deref(), is_html);
        let content = if is_html {
            tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(AiError::Cancelled),
                markdown = html_to_markdown(text) => markdown,
            }
        } else {
            text
        };
        Ok(page(
            fetched.url.to_string(),
            fetched.media_type,
            &content,
            offset,
            fetched.truncated,
        ))
    }

    async fn follow(
        &self,
        client: &reqwest::Client,
        mut url: Url,
        cancel: &CancellationToken,
    ) -> Result<Fetched, AiError> {
        let mut redirects = 0;
        loop {
            let redirected = redirects > 0;
            self.check_host(&url, redirected)?;
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
                url = parse_url(next.as_str())?;
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
            return Ok(Fetched {
                url,
                media_type,
                charset,
                body,
                truncated,
            });
        }
    }
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

/// Converts HTML to Markdown on a thread with a large stack (the converter recurses once per
/// nesting level). Pages nested deeper than 2,000 levels are reduced to plain text.
async fn html_to_markdown(html: String) -> String {
    let html = Arc::new(html);
    let (tx, rx) = tokio::sync::oneshot::channel();
    let input = Arc::clone(&html);
    let spawned = std::thread::Builder::new()
        .name("hatoba-ai-html".into())
        .stack_size(CONVERT_STACK)
        .spawn(move || {
            let _ = tx.send(convert_html(&input));
        });
    let converted = match spawned {
        Ok(_) => rx.await.ok(),
        Err(_) => None,
    };
    // The thread could not start, or the converter panicked: plain text still serves the model.
    converted.unwrap_or_else(|| document_text(&html))
}

fn convert_html(html: &str) -> String {
    use htmd::element_handler::Handlers;

    let converter = htmd::HtmlToMarkdown::builder()
        .skip_tags(vec![
            "script", "style", "noscript", "template", "svg", "canvas", "iframe", "object",
            "embed", "link", "meta",
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
        .build();
    let Ok(tree) = converter.html_to_tree(html) else {
        return document_text(html);
    };
    if dom_depth(&tree) > MAX_DOM_DEPTH {
        return document_text(html);
    }
    converter.tree_to_markdown(&tree)
}

/// The deepest nesting level of a DOM, counted without recursion.
fn dom_depth(root: &std::rc::Rc<htmd::Node>) -> usize {
    let mut deepest = 0;
    let mut stack = vec![(std::rc::Rc::clone(root), 1usize)];
    while let Some((node, depth)) = stack.pop() {
        deepest = deepest.max(depth);
        for child in node.children.borrow().iter() {
            stack.push((std::rc::Rc::clone(child), depth + 1));
        }
    }
    deepest
}

/// Up to 16,000 characters of `content` from `offset`.
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
    }
}

#[cfg(test)]
mod tests;
