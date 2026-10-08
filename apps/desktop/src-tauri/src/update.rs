//! The update check (spec §11): asks GitHub Releases for the newest stable release and compares
//! it with the running version. Installing is up to the user, who downloads from the release page.

use std::sync::Once;
use std::time::Duration;

use hatoba_core::version::Version;
use serde::Deserialize;

use crate::dto::UpdateCheck;
use crate::error::{AppError, AppResult, ErrorCode};

/// Answers with the newest release that is neither a draft nor a prerelease. Prereleases are
/// published with `--latest=false`, so they never show up here.
const LATEST_RELEASE_API: &str = "https://api.github.com/repos/scarletkc/Hatoba/releases/latest";
/// Every release page starts with this; anything else in `html_url` is refused.
const RELEASE_PAGE_PREFIX: &str = "https://github.com/scarletkc/Hatoba/releases/";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// A release answer is a few kilobytes; a larger one is refused.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// Checks GitHub for a release newer than `current`.
pub async fn check(current: &str) -> AppResult<UpdateCheck> {
    let (status, body) = fetch_latest().await?;
    evaluate(current, status, &body)
}

async fn fetch_latest() -> AppResult<(u16, Vec<u8>)> {
    let client = client()?;
    let mut resp = client
        .get(LATEST_RELEASE_API)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| unreachable(&e))?;
    let status = resp.status().as_u16();
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| unreachable(&e))? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(AppError::internal("the release answer is too large"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok((status, body))
}

fn client() -> AppResult<reqwest::Client> {
    // reqwest is built without a bundled crypto provider; this installs the ring provider that
    // hatoba-core's sync transport also uses. Whichever installs it first wins, and both agree.
    static INSTALL_PROVIDER: Once = Once::new();
    INSTALL_PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        // GitHub's API rejects requests without a User-Agent.
        .user_agent(concat!("Hatoba/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| AppError::internal("could not build the HTTP client"))
}

/// No HTTP answer at all (DNS, connection, TLS, timeout): the "offline" code that the sync
/// transport also reports.
fn unreachable(err: &reqwest::Error) -> AppError {
    tracing::info!("update check: GitHub unreachable: {err}");
    let detail = if err.is_timeout() {
        "GitHub did not answer in time"
    } else {
        "GitHub could not be reached"
    };
    AppError::new(ErrorCode::SyncOffline, detail)
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
}

/// Turns GitHub's answer into the result the UI shows.
fn evaluate(current: &str, status: u16, body: &[u8]) -> AppResult<UpdateCheck> {
    let running = Version::parse(current)
        .ok_or_else(|| AppError::internal(format!("'{current}' is not a release version")))?;
    match status {
        200 => {}
        // No stable release has been published yet, so there is nothing newer.
        404 => {
            return Ok(UpdateCheck {
                current_version: running.to_string(),
                latest_version: None,
                release_url: None,
                update_available: false,
            });
        }
        _ => return Err(AppError::internal(format!("GitHub answered HTTP {status}"))),
    }
    let release: Release = serde_json::from_slice(body)
        .map_err(|_| AppError::internal("GitHub's release answer could not be read"))?;
    let tag = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);
    let latest = Version::parse(tag).ok_or_else(|| {
        AppError::internal(format!(
            "the latest release tag '{}' is not a release version",
            release.tag_name
        ))
    })?;
    if !release.html_url.starts_with(RELEASE_PAGE_PREFIX) {
        return Err(AppError::internal(
            "the latest release links outside the Hatoba repository",
        ));
    }
    Ok(UpdateCheck {
        current_version: running.to_string(),
        latest_version: Some(latest.to_string()),
        release_url: Some(release.html_url),
        update_available: latest > running,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str) -> Vec<u8> {
        serde_json::json!({
            "tag_name": tag,
            "html_url": format!("https://github.com/scarletkc/Hatoba/releases/tag/{tag}"),
            "name": format!("Hatoba {tag}"),
            "prerelease": false,
            "assets": [],
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn a_newer_release_is_an_update() {
        let check = evaluate("0.1.0", 200, &release("v0.2.0")).unwrap();
        assert_eq!(
            check,
            UpdateCheck {
                current_version: "0.1.0".into(),
                latest_version: Some("0.2.0".into()),
                release_url: Some("https://github.com/scarletkc/Hatoba/releases/tag/v0.2.0".into()),
                update_available: true,
            }
        );
        // A prerelease build moves to its final release.
        assert!(
            evaluate("0.2.0-rc.1", 200, &release("v0.2.0"))
                .unwrap()
                .update_available
        );
    }

    #[test]
    fn the_same_or_an_older_release_is_not_an_update() {
        for (current, tag) in [
            ("0.2.0", "v0.2.0"),
            ("0.3.0", "v0.2.0"),
            ("0.3.0-beta.1", "v0.2.9"),
        ] {
            let check = evaluate(current, 200, &release(tag)).unwrap();
            assert!(!check.update_available, "{current} vs {tag}");
            assert_eq!(check.latest_version.as_deref(), tag.strip_prefix('v'));
        }
    }

    #[test]
    fn no_release_yet_is_up_to_date() {
        let body = br#"{"message":"Not Found","status":"404"}"#;
        let check = evaluate("0.1.0", 404, body).unwrap();
        assert_eq!(
            check,
            UpdateCheck {
                current_version: "0.1.0".into(),
                latest_version: None,
                release_url: None,
                update_available: false,
            }
        );
    }

    #[test]
    fn a_tag_without_the_v_prefix_is_accepted() {
        assert!(
            evaluate("0.1.0", 200, &release("0.2.0"))
                .unwrap()
                .update_available
        );
    }

    #[test]
    fn unexpected_answers_are_errors() {
        let rate_limited = br#"{"message":"API rate limit exceeded"}"#;
        for status in [403, 429, 500, 502] {
            let err = evaluate("0.1.0", status, rate_limited).unwrap_err();
            assert_eq!(err.code, ErrorCode::Internal);
            assert!(err.detail.contains(&status.to_string()));
        }
        for body in [&b"<html>"[..], br#"{"tag_name":"v0.2.0"}"#, b""] {
            assert_eq!(
                evaluate("0.1.0", 200, body).unwrap_err().code,
                ErrorCode::Internal
            );
        }
        let bad_tag = evaluate("0.1.0", 200, &release("nightly")).unwrap_err();
        assert!(bad_tag.detail.contains("nightly"));
    }

    #[test]
    fn a_release_page_outside_the_repository_is_refused() {
        for url in [
            "https://example.com/scarletkc/Hatoba/releases/tag/v0.2.0",
            "https://github.com/someone/Hatoba/releases/tag/v0.2.0",
            "https://github.com/scarletkc/Hatoba.evil/releases/tag/v0.2.0",
            "http://github.com/scarletkc/Hatoba/releases/tag/v0.2.0",
        ] {
            let body = serde_json::json!({ "tag_name": "v0.2.0", "html_url": url }).to_string();
            let err = evaluate("0.1.0", 200, body.as_bytes()).unwrap_err();
            assert_eq!(err.code, ErrorCode::Internal, "{url}");
        }
    }

    #[test]
    fn the_running_version_must_be_a_release_version() {
        let err = evaluate("0.1.0-dev", 404, b"").unwrap_err();
        assert_eq!(err.code, ErrorCode::Internal);
    }
}
