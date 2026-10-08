//! `fetch_url` against a mock server on 127.0.0.1.
//!
//! The tests use a [`Fetcher`] whose policy is the public one plus an exception for 127.0.0.1
//! (the mock server), and a resolver with fixed answers: `public.test` stands for a public host
//! that is really the mock server, `rebind.test` resolves to a private address, `mixed.test` to
//! one allowed and one private address. [`fetch_url`] itself is tested to refuse the mock server.

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
        .fetch(&public_url(&server, "/page#section"), 0, &cancel)
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
        .fetch(&format!("{}/page", server.uri()), 0, &cancel)
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
            .fetch(&public_url(&server, at), 0, &cancel)
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
            .fetch(&public_url(&server, at), 0, &cancel)
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
        async move { fetcher().fetch(&url, offset, &cancel).await.unwrap() }
    };
    let first = fetch(0).await;
    assert_eq!(first.total_length, 40_000);
    assert_eq!(first.content.chars().count(), MAX_FETCH_CHARS);
    assert!(text.starts_with(&first.content));
    let second = fetch(16_000).await;
    assert_eq!(second.offset, 16_000);
    assert_eq!(
        format!("{}{}", first.content, second.content),
        text.chars().take(32_000).collect::<String>()
    );
    let tail = fetch(39_990).await;
    assert_eq!(tail.content, "日本語ab日本語ab");
    let past = fetch(50_000).await;
    assert_eq!((past.offset, past.content.as_str()), (40_000, ""));

    let header = format_fetch_result(&first);
    assert!(header.starts_with(&format!(
        "URL: {url} | Content type: text/plain | Characters 0-16000 of 40000\n\
         More content follows: call fetch_url again with offset 16000 to read on.\n\n"
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
    for at in ["/to-ip", "/to-v6", "/to-name", "/to-mixed", "/to-metadata"] {
        let err = fetcher()
            .fetch(&public_url(&server, at), 0, &cancel)
            .await
            .unwrap_err();
        assert_eq!(err, AiError::Blocked(REDIRECT_BLOCKED.into()), "{at}");
    }
    // Directly, without a redirect.
    for url in [
        format!("http://rebind.test:{port}/page"),
        format!("http://mixed.test:{port}/page"),
        "http://192.168.1.1/".to_owned(),
        "http://[fd00::1]/".to_owned(),
    ] {
        let err = fetcher().fetch(&url, 0, &cancel).await.unwrap_err();
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
        let err = fetch_url(&url, 0, &cancel).await.unwrap_err();
        assert!(matches!(err, AiError::Blocked(_)), "{url}: {err:?}");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
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
        .fetch(&public_url(&server, "/r1"), 0, &cancel)
        .await
        .unwrap();
    assert_eq!(result.url, public_url(&server, "/page"));
    assert_eq!(result.content, "landed");

    let err = fetcher()
        .fetch(&public_url(&server, "/loop"), 0, &cancel)
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
            .fetch(&public_url(&server, "/nowhere"), 0, &cancel)
            .await,
        Err(AiError::Protocol(_))
    ));
    assert_eq!(
        fetcher()
            .fetch(&public_url(&server, "/gone"), 0, &cancel)
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
                fetcher().fetch(url, 0, &cancel).await,
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
        .fetch(&public_url(&server, "/big"), 0, &cancel)
        .await
        .unwrap();
    assert!(capped.truncated);
    assert_eq!(capped.total_length, 1_000);
    assert!(format_fetch_result(&capped).contains("longer than 5 MB"));

    let latin1 = fetcher()
        .fetch(&public_url(&server, "/latin1"), 0, &cancel)
        .await
        .unwrap();
    assert_eq!(latin1.content, "café");
    let meta = fetcher()
        .fetch(&public_url(&server, "/meta"), 0, &cancel)
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
        .fetch(&public_url(&server, "/deep"), 0, &CancellationToken::new())
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
        .fetch(&public_url(&server, "/slow"), 0, &cancel)
        .await
        .unwrap_err();
    assert_eq!(err, AiError::Cancelled);
    let err = fetcher_with(MAX_DOWNLOAD_BYTES, Duration::from_millis(300))
        .fetch(&public_url(&server, "/slow"), 0, &CancellationToken::new())
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
fn dom_depth_is_measured_without_recursion() {
    let tree = htmd::HtmlToMarkdown::new()
        .html_to_tree("<html><body><div><p>x</p></div></body></html>")
        .unwrap();
    // document > html > body > div > p > text
    assert_eq!(dom_depth(&tree), 6);
}

#[test]
fn markdown_keeps_structure() {
    assert_eq!(
        convert_html(PAGE),
        "Nginx reload\n\n# Reloading nginx\n\nRun `nginx -s reload`, see \
         [the docs](https://nginx.org/en/docs/).\n\ndiagram ![logo](https://example.com/logo.png)\
         \n\n*   one\n*   two"
    );
}
