//! `fetch_url` against a mock server on 127.0.0.1.
//!
//! The tests use a [`Fetcher`] whose policy is the public one plus an exception for 127.0.0.1
//! (the mock server), and a resolver with fixed answers: `public.test` stands for a public host
//! that is really the mock server, `rebind.test` resolves to a private address, `mixed.test` to
//! one allowed and one private address. [`fetch_url`] itself is tested to refuse the mock server.

use std::io;
use std::net::Ipv4Addr;

use reqwest::header::CONTENT_TYPE;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

const MOCK_IP: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

fn test_lookup() -> Lookup {
    Arc::new(|host: String| {
        Box::pin(async move {
            match host.as_str() {
                "public.test" => Ok(vec![MOCK_IP]),
                "rebind.test" => Ok(vec!["10.0.0.7".parse().unwrap()]),
                "mixed.test" => Ok(vec![MOCK_IP, "192.168.1.1".parse().unwrap()]),
                _ => Err(io::Error::new(io::ErrorKind::NotFound, "unknown test host")),
            }
        })
    })
}

fn fetcher_with(max_bytes: usize, timeout: Duration) -> Fetcher {
    Fetcher::for_tests(
        |ip| ip == MOCK_IP || !is_blocked(ip),
        test_lookup(),
        max_bytes,
        timeout,
    )
}

fn fetcher() -> Fetcher {
    fetcher_with(MAX_DOWNLOAD_BYTES, FETCH_TIMEOUT)
}

/// `http://public.test:<port><path>`: reaches the mock server through the guarded resolver.
fn public_url(server: &MockServer, path: &str) -> String {
    format!("http://public.test:{}{path}", server.address().port())
}

fn body(content_type: &str, body: impl Into<Vec<u8>>) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header(CONTENT_TYPE.as_str(), content_type)
        .set_body_bytes(body.into())
}

async fn mount(server: &MockServer, at: &str, response: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path(at))
        .respond_with(response)
        .mount(server)
        .await;
}

fn redirect(status: u16, location: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).insert_header("location", location)
}

const PAGE: &str = r#"<!doctype html><html><head><title>Nginx reload</title>
<style>body { color: red }</style><script>alert("steal")</script></head>
<body><h1>Reloading nginx</h1><p>Run <code>nginx -s reload</code>, see
<a href="https://nginx.org/en/docs/">the docs</a>.</p>
<img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk" alt="diagram">
<img src="https://example.com/logo.png" alt="logo">
<ul><li>one</li><li>two</li></ul></body></html>"#;

#[tokio::test]
async fn html_becomes_markdown() {
    let server = MockServer::start().await;
    mount(&server, "/page", body("text/html; charset=utf-8", PAGE)).await;
    let cancel = CancellationToken::new();
    let result = fetcher()
        .fetch(&public_url(&server, "/page#section"), 0, false, &cancel)
        .await
        .unwrap();
    assert_eq!(result.url, public_url(&server, "/page"));
    assert_eq!(result.content_type, "text/html");
    assert_eq!(result.offset, 0);
    assert!(!result.truncated);
    let md = &result.content;
    for expected in [
        "# Reloading nginx",
        "`nginx -s reload`",
        "[the docs](https://nginx.org/en/docs/)",
        "![logo](https://example.com/logo.png)",
        "diagram",
        "one",
    ] {
        assert!(md.contains(expected), "missing {expected:?} in:\n{md}");
    }
    for absent in ["alert", "steal", "color: red", "base64", "iVBOR"] {
        assert!(!md.contains(absent), "{absent:?} must not appear in:\n{md}");
    }
    assert_eq!(result.total_length, md.chars().count() as u64);

    // The same page through its IP literal.
    let direct = fetcher()
        .fetch(&format!("{}/page", server.uri()), 0, false, &cancel)
        .await
        .unwrap();
    assert_eq!(direct.content, result.content);
}

#[tokio::test]
async fn text_json_and_xml_pass_through() {
    let server = MockServer::start().await;
    mount(&server, "/a.json", body("application/json", r#"{"a": 1}"#)).await;
    mount(&server, "/feed", body("application/atom+xml", "<feed/>")).await;
    mount(&server, "/x.yaml", body("text/yaml", "a: 1\n")).await;
    mount(
        &server,
        "/problem",
        body("application/problem+json; charset=utf-8", "{}"),
    )
    .await;
    let cancel = CancellationToken::new();
    for (at, ct, content) in [
        ("/a.json", "application/json", r#"{"a": 1}"#),
        ("/feed", "application/atom+xml", "<feed/>"),
        ("/x.yaml", "text/yaml", "a: 1\n"),
        ("/problem", "application/problem+json", "{}"),
    ] {
        let result = fetcher()
            .fetch(&public_url(&server, at), 0, false, &cancel)
            .await
            .unwrap();
        assert_eq!(
            (result.content_type.as_str(), result.content.as_str()),
            (ct, content)
        );
    }
}

#[tokio::test]
async fn other_content_types_are_refused() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/img",
        body("image/png", vec![0x89, b'P', b'N', b'G']),
    )
    .await;
    mount(&server, "/zip", body("application/octet-stream", "PK")).await;
    mount(
        &server,
        "/none",
        ResponseTemplate::new(200).set_body_bytes(b"???".to_vec()),
    )
    .await;
    let cancel = CancellationToken::new();
    for at in ["/img", "/zip", "/none"] {
        let err = fetcher()
            .fetch(&public_url(&server, at), 0, false, &cancel)
            .await
            .unwrap_err();
        assert!(
            matches!(err, AiError::UnsupportedContentType(_)),
            "{at}: {err:?}"
        );
    }
}

#[tokio::test]
async fn long_content_pages_by_characters() {
    let server = MockServer::start().await;
    // 40,000 characters, most of them three bytes long.
    let text: String = "日本語ab".repeat(8_000);
    mount(
        &server,
        "/long.txt",
        body("text/plain; charset=utf-8", text.clone()),
    )
    .await;
    let cancel = CancellationToken::new();
    let url = public_url(&server, "/long.txt");
    let fetch = |offset| {
        let url = url.clone();
        let cancel = cancel.clone();
        async move { fetcher().fetch(&url, offset, false, &cancel).await.unwrap() }
    };
    let first = fetch(0).await;
    assert_eq!(first.total_length, 40_000);
    assert_eq!(first.content.chars().count(), MAX_FETCH_CHARS);
    assert!(text.starts_with(&first.content));
    let second = fetch(30_000).await;
    assert_eq!(second.offset, 30_000);
    assert_eq!(format!("{}{}", first.content, second.content), text);
    let tail = fetch(39_990).await;
    assert_eq!(tail.content, "日本語ab日本語ab");
    let past = fetch(50_000).await;
    assert_eq!((past.offset, past.content.as_str()), (40_000, ""));

    let header = format_fetch_result(&first);
    assert!(header.starts_with(&format!(
        "URL: {url} | Content type: text/plain | Characters 0-30000 of 40000\n\
         More content follows: call fetch_url again with offset 30000 to read on.\n\n"
    )));
    assert!(format_fetch_result(&tail).contains("Characters 39990-40000 of 40000\n\n"));
    assert!(format_fetch_result(&past).contains("No content at this offset."));
}

#[tokio::test]
async fn private_addresses_are_refused_including_after_a_redirect() {
    let server = MockServer::start().await;
    let port = server.address().port();
    mount(&server, "/to-ip", redirect(302, "http://10.0.0.1/admin")).await;
    mount(
        &server,
        "/to-v6",
        redirect(307, &format!("http://[::1]:{port}/page")),
    )
    .await;
    mount(
        &server,
        "/to-name",
        redirect(301, &format!("http://rebind.test:{port}/page")),
    )
    .await;
    mount(
        &server,
        "/to-mixed",
        redirect(308, &format!("http://mixed.test:{port}/page")),
    )
    .await;
    mount(
        &server,
        "/to-metadata",
        redirect(302, "http://169.254.169.254/latest/meta-data/"),
    )
    .await;
    mount(&server, "/page", body("text/plain", "inside")).await;
    let cancel = CancellationToken::new();
    for at in ["/to-ip", "/to-v6", "/to-metadata"] {
        let err = fetcher()
            .fetch(&public_url(&server, at), 0, false, &cancel)
            .await
            .unwrap_err();
        assert_eq!(err, AiError::Blocked(REDIRECT_BLOCKED.into()), "{at}");
    }
    // A name on another host is not followed, so it is not resolved either: a fetch of it is a
    // call of its own, refused below.
    for (at, to) in [("/to-name", "rebind.test"), ("/to-mixed", "mixed.test")] {
        let result = fetcher()
            .fetch(&public_url(&server, at), 0, false, &cancel)
            .await
            .unwrap();
        assert_eq!(
            result.redirect,
            Some(format!("http://{to}:{port}/page")),
            "{at}"
        );
    }
    // Directly, without a redirect.
    for url in [
        format!("http://rebind.test:{port}/page"),
        format!("http://mixed.test:{port}/page"),
        "http://192.168.1.1/".to_owned(),
        "http://[fd00::1]/".to_owned(),
    ] {
        let err = fetcher().fetch(&url, 0, false, &cancel).await.unwrap_err();
        assert_eq!(err, AiError::Blocked(BLOCKED_MESSAGE.into()), "{url}");
    }
    // Only the page itself was ever requested from the mock server's own routes.
    let paths: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| r.url.path().to_owned())
        .collect();
    assert!(!paths.contains(&"/page".to_owned()), "{paths:?}");
}

#[tokio::test]
async fn the_public_fetch_refuses_loopback() {
    let server = MockServer::start().await;
    mount(&server, "/page", body("text/plain", "inside")).await;
    let port = server.address().port();
    let cancel = CancellationToken::new();
    for url in [
        format!("{}/page", server.uri()),
        format!("http://localhost:{port}/page"),
        format!("http://[::1]:{port}/page"),
        format!("http://2130706433:{port}/page"),
        format!("http://0x7f.1:{port}/page"),
        format!("http://[::ffff:127.0.0.1]:{port}/page"),
        format!("http://0.0.0.0:{port}/page"),
    ] {
        let err = fetch_url(&url, 0, false, &cancel).await.unwrap_err();
        assert!(matches!(err, AiError::Blocked(_)), "{url}: {err:?}");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn behind_a_local_proxy_the_targets_are_still_checked() {
    // A proxy on loopback, as desktop proxy tools run one: the mock server, which answers the
    // proxied request itself. Its own address is no target, so the public policy lets it through.
    let proxy = MockServer::start().await;
    mount(&proxy, "/page", body("text/plain", "through the proxy")).await;
    let fetcher = Fetcher {
        proxy: Proxy::At(format!("http://localhost:{}", proxy.address().port())),
        ..Fetcher::public()
    };
    let cancel = CancellationToken::new();
    let page = fetcher
        .fetch("http://docs.example.test/page", 0, false, &cancel)
        .await
        .unwrap();
    assert_eq!(page.content, "through the proxy");
    // The proxy resolves the targets' names, so what is refused here is refused by the host as
    // written: an address, or localhost by its name.
    for url in [
        "http://localhost/page",
        "http://api.LOCALHOST./page",
        "http://127.0.0.1/page",
        "http://10.1.2.3/page",
        "http://[::1]/page",
    ] {
        let err = fetcher.fetch(url, 0, false, &cancel).await.unwrap_err();
        assert!(matches!(err, AiError::Blocked(_)), "{url}: {err:?}");
    }
    assert_eq!(proxy.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn redirects_to_another_host_are_returned_not_followed() {
    let server = MockServer::start().await;
    let other = MockServer::start().await;
    let port = server.address().port();
    let other_port = other.address().port();
    mount(
        &server,
        "/away",
        redirect(302, "https://elsewhere.test/page?q=1"),
    )
    .await;
    // The same host on another port, and in other letter case, is followed.
    mount(
        &server,
        "/port",
        redirect(301, &format!("http://public.test:{other_port}/page")),
    )
    .await;
    mount(
        &server,
        "/case",
        redirect(302, &format!("http://PUBLIC.test:{port}/page")),
    )
    .await;
    mount(&server, "/page", body("text/plain", "here")).await;
    mount(&other, "/page", body("text/plain", "other port")).await;
    let cancel = CancellationToken::new();

    let away = fetcher()
        .fetch(&public_url(&server, "/away"), 0, false, &cancel)
        .await
        .unwrap();
    assert_eq!(
        away,
        FetchResult {
            url: public_url(&server, "/away"),
            redirect: Some("https://elsewhere.test/page?q=1".into()),
            ..FetchResult::default()
        }
    );
    assert_eq!(
        format_fetch_result(&away),
        format!(
            "URL: {} | Redirects to another host: https://elsewhere.test/page?q=1\n\
             fetch_url follows redirects only on the host it was given. To read the page, call \
             fetch_url with that URL.\n",
            public_url(&server, "/away")
        )
    );

    let ported = fetcher()
        .fetch(&public_url(&server, "/port"), 0, false, &cancel)
        .await
        .unwrap();
    assert_eq!(
        (ported.redirect, ported.content.as_str()),
        (None, "other port")
    );
    let cased = fetcher()
        .fetch(&public_url(&server, "/case"), 0, false, &cancel)
        .await
        .unwrap();
    assert_eq!((cased.redirect, cased.content.as_str()), (None, "here"));
}

#[tokio::test]
async fn redirects_are_followed_up_to_five() {
    let server = MockServer::start().await;
    mount(&server, "/r1", redirect(301, "/r2")).await;
    mount(&server, "/r2", redirect(302, "r3")).await;
    mount(&server, "/r3", redirect(303, "/page")).await;
    mount(&server, "/page", body("text/plain", "landed")).await;
    mount(&server, "/loop", redirect(302, "/loop")).await;
    mount(&server, "/nowhere", ResponseTemplate::new(302)).await;
    mount(&server, "/gone", ResponseTemplate::new(404)).await;
    let cancel = CancellationToken::new();
    let result = fetcher()
        .fetch(&public_url(&server, "/r1"), 0, false, &cancel)
        .await
        .unwrap();
    assert_eq!(result.url, public_url(&server, "/page"));
    assert_eq!(result.content, "landed");

    let err = fetcher()
        .fetch(&public_url(&server, "/loop"), 0, false, &cancel)
        .await
        .unwrap_err();
    assert_eq!(
        err,
        AiError::Network("too many redirects (more than 5)".into())
    );
    let loops = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/loop")
        .count();
    assert_eq!(loops, 6, "the first request and five redirects");

    assert!(matches!(
        fetcher()
            .fetch(&public_url(&server, "/nowhere"), 0, false, &cancel)
            .await,
        Err(AiError::Protocol(_))
    ));
    assert_eq!(
        fetcher()
            .fetch(&public_url(&server, "/gone"), 0, false, &cancel)
            .await
            .unwrap_err(),
        AiError::Http {
            status: 404,
            message: "Not Found".into()
        }
    );
}

#[tokio::test]
async fn bad_urls_are_refused_before_any_request() {
    let cancel = CancellationToken::new();
    for url in [
        "ftp://example.com/file",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "not a url",
        "http://user:pass@public.test/",
        "https://token@public.test/",
    ] {
        assert!(
            matches!(
                fetcher().fetch(url, 0, false, &cancel).await,
                Err(AiError::InvalidUrl(_))
            ),
            "{url}"
        );
    }
}

#[tokio::test]
async fn downloads_are_capped_and_charsets_decoded() {
    let server = MockServer::start().await;
    mount(&server, "/big", body("text/plain", "x".repeat(5_000))).await;
    mount(
        &server,
        "/latin1",
        body(
            "text/plain; charset=ISO-8859-1",
            vec![b'c', b'a', b'f', 0xE9],
        ),
    )
    .await;
    let mut meta = b"<html><head><meta charset=\"windows-1252\"></head><body><p>na".to_vec();
    meta.push(0xEF);
    meta.extend_from_slice(b"ve</p></body></html>");
    mount(&server, "/meta", body("text/html", meta)).await;
    let cancel = CancellationToken::new();
    let capped = fetcher_with(1_000, FETCH_TIMEOUT)
        .fetch(&public_url(&server, "/big"), 0, false, &cancel)
        .await
        .unwrap();
    assert!(capped.truncated);
    assert_eq!(capped.total_length, 1_000);
    assert!(format_fetch_result(&capped).contains("longer than 5 MB"));

    let latin1 = fetcher()
        .fetch(&public_url(&server, "/latin1"), 0, false, &cancel)
        .await
        .unwrap();
    assert_eq!(latin1.content, "café");
    let meta = fetcher()
        .fetch(&public_url(&server, "/meta"), 0, false, &cancel)
        .await
        .unwrap();
    assert_eq!(meta.content, "naïve");
}

#[tokio::test]
async fn deeply_nested_html_falls_back_to_text() {
    let server = MockServer::start().await;
    let depth = 20_000;
    let html = format!(
        "<html><body>{}deep text{}</body></html>",
        "<div>".repeat(depth),
        "</div>".repeat(depth)
    );
    mount(&server, "/deep", body("text/html", html)).await;
    let result = fetcher()
        .fetch(
            &public_url(&server, "/deep"),
            0,
            false,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(result.content, "deep text");
}

#[tokio::test]
async fn cancellation_and_the_time_limit() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/slow",
        body("text/plain", "late").set_delay(Duration::from_secs(30)),
    )
    .await;
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        trigger.cancel();
    });
    let started = std::time::Instant::now();
    let err = fetcher()
        .fetch(&public_url(&server, "/slow"), 0, false, &cancel)
        .await
        .unwrap_err();
    assert_eq!(err, AiError::Cancelled);
    let err = fetcher_with(MAX_DOWNLOAD_BYTES, Duration::from_millis(300))
        .fetch(
            &public_url(&server, "/slow"),
            0,
            false,
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(err, AiError::Network("fetching the page timed out".into()));
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn content_types_and_charsets_parse() {
    assert_eq!(
        parse_content_type("Text/HTML; Charset=\"UTF-8\""),
        ("text/html".to_owned(), Some("utf-8".to_owned()))
    );
    assert_eq!(parse_content_type(""), (String::new(), None));
    for ok in [
        "text/plain",
        "text/csv",
        "application/json",
        "application/xml",
        "application/ld+json",
        "image/svg+xml",
    ] {
        assert!(is_text_type(ok), "{ok}");
    }
    for refused in [
        "image/png",
        "application/pdf",
        "application/octet-stream",
        "video/mp4",
        "application/zip",
    ] {
        assert!(!is_text_type(refused), "{refused}");
    }
    assert_eq!(
        sniff_meta_charset(
            b"<meta http-equiv=\"Content-Type\" content=\"text/html; charset=gbk\">"
        )
        .as_deref(),
        Some("gbk")
    );
    assert_eq!(sniff_meta_charset(b"<p>plain</p>"), None);
    // GBK for 中文.
    assert_eq!(
        decode(&[0xD6, 0xD0, 0xCE, 0xC4], Some("gbk"), false),
        "中文"
    );
    assert_eq!(
        decode("ok".as_bytes(), Some("no-such-charset"), false),
        "ok"
    );
}

#[test]
fn dom_size_is_measured_without_recursion() {
    let tree = htmd::HtmlToMarkdown::new()
        .html_to_tree("<html><body><div><p>x</p></div></body></html>")
        .unwrap();
    // document > html > body > div > p > text, and html, head, body, div and p.
    assert_eq!(dom_size(&tree), (6, 5));
}

#[test]
fn markdown_keeps_structure() {
    let url = Url::parse("https://docs.example.test/nginx").unwrap();
    let converted = convert_html(PAGE, &url, false);
    assert_eq!(
        converted.markdown,
        "# Reloading nginx\n\nRun `nginx -s reload`, see \
         [the docs](https://nginx.org/en/docs/).\n\ndiagram ![logo](https://example.com/logo.png)\
         \n\n*   one\n*   two"
    );
    // Too short to have main content.
    assert_eq!(converted.part, PagePart::Whole);
    assert_eq!(converted.title.as_deref(), Some("Nginx reload"));
}

/// An article of `steps` paragraphs with a site header, a sidebar, breadcrumbs, a footer, a code
/// block in a wrapper whose class reads as clutter to Readability, and relative links.
fn article(title: &str, steps: usize) -> String {
    let paragraphs: String = (1..=steps)
        .map(|i| {
            format!(
                "<p>Step {i} of setting up the server, with enough words in it, commas, \
                 clauses, and details that it reads as the text of an article, as the \
                 <a href=\"../reference/#step-{i}\">reference</a> explains.</p>"
            )
        })
        .collect();
    format!(
        r#"<!doctype html><html><head><title>  {title}
        </title></head><body>
        <header class="site-header"><a href="/">Example Docs</a></header>
        <div class="layout">
          <aside class="sidebar"><ul><li><a href="/a">Sidebar link A</a></li>
            <li><a href="/b">Sidebar link B</a></li></ul></aside>
          <main>
            <nav class="breadcrumbs"><a href="/">Docs</a> / <a href="./">Guide</a></nav>
            <h1>{title}</h1>
            {paragraphs}
            <div class="scroll-mt-2 group"><button>Copy</button><div class="highlight">
              <pre><code>$ sudo systemctl restart sshd</code></pre></div></div>
            <p><img src="img/diagram.png" alt="diagram"> <a href="setup">Next: setup</a></p>
          </main>
        </div>
        <footer>Copyright Example</footer></body></html>"#
    )
}

#[tokio::test]
async fn articles_come_back_as_their_main_content() {
    let server = MockServer::start().await;
    mount(&server, "/old", redirect(301, "/docs/guide/")).await;
    mount(
        &server,
        "/docs/guide/",
        body("text/html", article("Restarting sshd", 6)),
    )
    .await;
    let cancel = CancellationToken::new();
    let at = |path: &str| public_url(&server, path);

    // After a redirect, so links resolve against the page's final URL.
    let main = fetcher()
        .fetch(&at("/old"), 0, false, &cancel)
        .await
        .unwrap();
    assert_eq!(main.url, at("/docs/guide/"));
    assert_eq!(main.part, Some(PagePart::Main));
    assert_eq!(main.title.as_deref(), Some("Restarting sshd"));
    for kept in [
        "# Restarting sshd",
        "Step 6 of setting up the server",
        "$ sudo systemctl restart sshd",
        &format!("[reference]({}#step-1)", at("/docs/reference/")),
        &format!("![diagram]({})", at("/docs/guide/img/diagram.png")),
        &format!("[Next: setup]({})", at("/docs/guide/setup")),
    ] {
        assert!(
            main.content.contains(kept),
            "missing {kept:?} in:\n{}",
            main.content
        );
    }
    for left_out in [
        "Sidebar link",
        "Example Docs",
        "Guide](",
        "Copyright",
        "Copy\n",
    ] {
        assert!(
            !main.content.contains(left_out),
            "{left_out:?} must not appear in:\n{}",
            main.content
        );
    }
    let text = format_fetch_result(&main);
    assert!(text.starts_with(&format!(
        "URL: {} | Content type: text/html | Main content | Characters 0-{len} of {len}\n\
         Title: Restarting sshd\n\
         Navigation, sidebars and footers are left out. For the whole page, call fetch_url with \
         full_page true.\n\n",
        at("/docs/guide/"),
        len = main.total_length
    )));

    // The whole page on request, its links absolute too.
    let whole = fetcher()
        .fetch(&at("/docs/guide/"), 0, true, &cancel)
        .await
        .unwrap();
    assert_eq!(whole.part, Some(PagePart::Whole));
    assert!(whole.total_length > main.total_length);
    for kept in [
        "Sidebar link A",
        "Copyright Example",
        "$ sudo systemctl restart sshd",
        &format!("[Sidebar link A]({})", at("/a")),
    ] {
        assert!(
            whole.content.contains(kept),
            "missing {kept:?} in:\n{}",
            whole.content
        );
    }
    assert!(format_fetch_result(&whole).contains("| Whole page |"));
    assert!(!format_fetch_result(&whole).contains("are left out"));
}

#[tokio::test]
async fn pages_without_main_content_come_back_whole() {
    let server = MockServer::start().await;
    let listing = format!(
        "<html><head><title>Releases</title></head><body><ul>{}</ul></body></html>",
        (0..40)
            .map(|i| format!("<li><a href=\"/v{i}\">Version {i}</a></li>"))
            .collect::<String>()
    );
    mount(&server, "/releases", body("text/html", listing)).await;
    mount(
        &server,
        "/app",
        body(
            "text/html",
            "<html><head><title>App</title></head><body><div id=\"root\"></div>\
             <script src=\"/app.js\"></script></body></html>",
        ),
    )
    .await;
    let cancel = CancellationToken::new();
    let releases = fetcher()
        .fetch(&public_url(&server, "/releases"), 0, false, &cancel)
        .await
        .unwrap();
    assert_eq!(releases.part, Some(PagePart::Whole));
    assert!(releases.content.contains("Version 39"));
    let app = fetcher()
        .fetch(&public_url(&server, "/app"), 0, false, &cancel)
        .await
        .unwrap();
    assert_eq!(
        (app.part, app.title.as_deref(), app.content.as_str()),
        (Some(PagePart::Whole), Some("App"), "")
    );
}

#[tokio::test]
async fn reading_on_keeps_the_part() {
    let server = MockServer::start().await;
    let long: String = (0..2_000)
        .map(|i| format!("<p>Paragraph {i}, a line of the long page.</p>"))
        .collect();
    mount(
        &server,
        "/long",
        body("text/html", format!("<html><body>{long}</body></html>")),
    )
    .await;
    mount(&server, "/article", body("text/html", article("Long", 300))).await;
    let cancel = CancellationToken::new();
    let whole = fetcher()
        .fetch(&public_url(&server, "/long"), 0, false, &cancel)
        .await
        .unwrap();
    assert_eq!(whole.part, Some(PagePart::Whole));
    assert!(format_fetch_result(&whole).contains(
        "More content follows: call fetch_url again with offset 30000 and full_page true to \
         read on.\n"
    ));
    let main = fetcher()
        .fetch(&public_url(&server, "/article"), 0, false, &cancel)
        .await
        .unwrap();
    assert_eq!(main.part, Some(PagePart::Main));
    assert!(
        format_fetch_result(&main)
            .contains("More content follows: call fetch_url again with offset 30000 to read on.\n")
    );
}

#[test]
fn code_blocks_keep_their_lines() {
    let url = Url::parse("https://docs.example.test/").unwrap();
    let code = |html: &str| convert_html(html, &url, true).markdown;
    // A highlighter's per-line spans, each ending in its line break, and a copy button.
    assert_eq!(
        code(
            "<pre><code class=\"language-bash\"><span class=\"line\"><span class=\"cl\">sudo apt \
             update\n</span></span><span class=\"line\"><span class=\"cl\">sudo apt install \
             <span class=\"m\">curl</span>\n</span></span></code><button>Copy</button></pre>"
        ),
        "```bash\nsudo apt update\nsudo apt install curl\n```"
    );
    assert_eq!(code("<pre>a<br>b &lt;c&gt;</pre>"), "```\na\nb <c>\n```");
    // A fence longer than any backtick run inside.
    assert_eq!(
        code("<pre class=\"lang-md\">```\nx\n```</pre>"),
        "````md\n```\nx\n```\n````"
    );
}

#[test]
fn links_resolve_against_the_base_element() {
    let url = Url::parse("https://example.test/a/page").unwrap();
    let html = "<html><head><base href=\"/static/\"></head><body>\
                <a href=\"guide.html\">guide</a> <a href=\"mailto:x@example.test\">mail</a> \
                <a href=\"https://other.test/\">other</a></body></html>";
    let converted = convert_html(html, &url, true);
    assert_eq!(
        converted.markdown,
        "[guide](https://example.test/static/guide.html) [mail](mailto:x@example.test) \
         [other](https://other.test/)"
    );
    assert_eq!(converted.title, None);
}
