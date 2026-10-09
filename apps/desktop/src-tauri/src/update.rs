//! Signed updates (spec §11). Every GitHub release carries a `latest.json`, from which the Tauri
//! updater installs the installer it names, and only when the installer's signature matches the
//! public key in tauri.conf.json. A stable version reads `latest.json` from the endpoint there,
//! which is the latest stable release; a prerelease version reads it from the newest release,
//! prereleases included.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use std::time::{Duration, Instant};

use hatoba_core::version::Version;
use serde::Deserialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, Runtime, Url};
use tauri_plugin_updater::{Error as UpdaterError, Update, Updater, UpdaterExt};

use crate::dto::{AvailableUpdate, UpdateCheck, UpdateProgress};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::state::AppState;

/// The release page of version X.Y.Z is this followed by X.Y.Z.
const RELEASE_PAGE: &str = "https://github.com/scarletkc/Hatoba/releases/tag/v";
/// Where a prerelease version finds the newest release.
const GITHUB: Releases<'static> = Releases {
    api: "https://api.github.com/repos/scarletkc/Hatoba/releases?per_page=20",
    downloads: "https://github.com/scarletkc/Hatoba/releases/download/",
};
/// A page of releases with their notes stays far below this.
const MAX_RELEASES_BYTES: usize = 4 * 1024 * 1024;
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The installer download has no overall time limit; it fails when no data arrives for this long.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Download progress reaches the WebView at most this often.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// The update the last check found, and whether it is being installed.
#[derive(Default)]
pub struct Updates {
    found: Mutex<Option<Update>>,
    installing: AtomicBool,
}

impl Updates {
    fn found(&self) -> MutexGuard<'_, Option<Update>> {
        self.found.lock().unwrap_or_else(|p| p.into_inner())
    }
}

struct Releases<'a> {
    /// GitHub's list of releases. Drafts are not listed without a token.
    api: &'a str,
    /// Release assets download from here; a `latest.json` anywhere else is ignored.
    downloads: &'a str,
}

/// Checks for a release newer than the running version and keeps it for [`install`].
pub async fn check<R: Runtime>(app: &AppHandle<R>, updates: &Updates) -> AppResult<UpdateCheck> {
    check_with(app, updates, &GITHUB, before_exit(app)).await
}

/// Runs when the installer is about to start and Hatoba to exit: on Windows, after the download
/// passed its signature check and was unpacked, so a failed install leaves the MCP servers
/// running. The exit skips RunEvent::Exit, which otherwise stops their child processes.
fn before_exit<R: Runtime>(app: &AppHandle<R>) -> impl Fn() + Send + Sync + 'static {
    let app = app.clone();
    move || {
        if let Some(state) = app.try_state::<AppState>() {
            // Each stop is reported, in case the installer then fails to start.
            tauri::async_runtime::block_on(state.mcp.stop_all(&state.vault));
        }
        app.cleanup_before_exit();
    }
}

async fn check_with<R: Runtime>(
    app: &AppHandle<R>,
    updates: &Updates,
    releases: &Releases<'_>,
    before_exit: impl Fn() + Send + Sync + 'static,
) -> AppResult<UpdateCheck> {
    let current = &app.package_info().version;
    let endpoint = if current.pre.is_empty() {
        Some(configured_endpoint(app)?)
    } else {
        newest_manifest(releases).await?
    };
    let found = match endpoint {
        None => None,
        Some(endpoint) => match updater(app, endpoint.clone(), before_exit)?.check().await {
            Ok(found) => found,
            Err(UpdaterError::ReleaseNotFound) => {
                no_release(endpoint).await?;
                None
            }
            Err(err) => return Err(classify(err)),
        },
    };
    let check = UpdateCheck {
        current_version: current.to_string(),
        update: found.as_ref().map(describe),
    };
    *updates.found() = found;
    Ok(check)
}

/// Downloads the update the last check found, checks its signature, and runs its installer,
/// which closes Hatoba and starts the new version. Returns only when that fails.
pub async fn install(app: &AppHandle, progress: &Channel<UpdateProgress>) -> AppResult<()> {
    let state = app.state::<AppState>();
    let update = state
        .updates
        .found()
        .clone()
        .ok_or_else(|| AppError::not_found("update"))?;
    let _installing = Installing::start(&state.updates.installing)?;
    let bytes = download(&update, |p| {
        let _ = progress.send(p);
    })
    .await?;
    let _ = progress.send(UpdateProgress::Installing);
    tracing::info!("installing Hatoba {}", update.version);
    tauri::async_runtime::spawn_blocking(move || update.install(bytes))
        .await
        .map_err(|e| AppError::internal(format!("the installer task failed: {e}")))?
        .map_err(classify)?;
    // Windows never gets here: the updater exits once the installer starts. Elsewhere the new
    // version is in place and runs after a restart.
    app.restart()
}

fn updater<R: Runtime>(
    app: &AppHandle<R>,
    endpoint: Url,
    before_exit: impl Fn() + Send + Sync + 'static,
) -> AppResult<Updater> {
    app.updater_builder()
        .endpoints(vec![endpoint])
        .map_err(classify)?
        .on_before_exit(before_exit)
        .timeout(CHECK_TIMEOUT)
        .configure_client(|client| {
            client
                .connect_timeout(CONNECT_TIMEOUT)
                .read_timeout(READ_TIMEOUT)
        })
        .build()
        .map_err(classify)
}

fn describe(update: &Update) -> AvailableUpdate {
    AvailableUpdate {
        version: update.version.clone(),
        notes: update.body.clone().filter(|notes| !notes.trim().is_empty()),
        published_at: update
            .date
            .map(|date| (date.unix_timestamp_nanos() / 1_000_000) as i64),
        release_url: format!("{RELEASE_PAGE}{}", update.version),
    }
}

/// Downloads the installer and checks its signature, reporting progress at most every
/// [`PROGRESS_INTERVAL`] and always once the last byte arrived.
async fn download(update: &Update, report: impl Fn(UpdateProgress)) -> AppResult<Vec<u8>> {
    let mut downloaded = 0u64;
    let mut reported: Option<Instant> = None;
    update
        .download(
            |chunk, total| {
                downloaded += chunk as u64;
                let now = Instant::now();
                let due = reported.is_none_or(|at| now - at >= PROGRESS_INTERVAL);
                if due || total == Some(downloaded) {
                    reported = Some(now);
                    report(UpdateProgress::Downloading { downloaded, total });
                }
            },
            || {},
        )
        .await
        .map_err(classify)
}

/// Holds [`Updates::installing`] for one install, so a second request does not start another.
struct Installing<'a>(&'a AtomicBool);

impl<'a> Installing<'a> {
    fn start(flag: &'a AtomicBool) -> AppResult<Self> {
        if flag.swap(true, Ordering::AcqRel) {
            return Err(AppError::internal("an update is already being installed"));
        }
        Ok(Self(flag))
    }
}

impl Drop for Installing<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// The updater reports every answer without a release the same way, so this asks the endpoint
/// again for its status. GitHub answers 404 until a stable release with `latest.json` exists,
/// which means there is nothing newer; any other status is an error.
async fn no_release(endpoint: Url) -> AppResult<()> {
    let resp = client()?
        .get(endpoint)
        .send()
        .await
        .map_err(|e| unreachable(&e))?;
    match resp.status().as_u16() {
        404 => Ok(()),
        status => Err(AppError::internal(format!(
            "the update endpoint answered HTTP {status}"
        ))),
    }
}

/// The first endpoint in the updater's configuration.
fn configured_endpoint<R: Runtime>(app: &AppHandle<R>) -> AppResult<Url> {
    let endpoint = app
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|updater| updater.get("endpoints")?.get(0)?.as_str())
        .ok_or_else(|| AppError::internal("no update endpoint is configured"))?;
    endpoint
        .parse()
        .map_err(|_| AppError::internal(format!("'{endpoint}' is not an update endpoint")))
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// The `latest.json` of the newest release, prereleases included, or `None` when no release
/// carries one.
async fn newest_manifest(releases: &Releases<'_>) -> AppResult<Option<Url>> {
    let mut resp = client()?
        .get(releases.api)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| unreachable(&e))?;
    let status = resp.status().as_u16();
    if status != 200 {
        return Err(AppError::internal(format!("GitHub answered HTTP {status}")));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| unreachable(&e))? {
        if body.len() + chunk.len() > MAX_RELEASES_BYTES {
            return Err(AppError::internal("GitHub's list of releases is too large"));
        }
        body.extend_from_slice(&chunk);
    }
    let list: Vec<Release> = serde_json::from_slice(&body)
        .map_err(|_| AppError::internal("GitHub's list of releases could not be read"))?;
    Ok(pick_manifest(&list, releases.downloads))
}

fn pick_manifest(list: &[Release], downloads: &str) -> Option<Url> {
    list.iter()
        .filter(|release| !release.draft)
        .filter_map(|release| {
            let tag = &release.tag_name;
            let version = Version::parse(tag.strip_prefix('v').unwrap_or(tag))?;
            let manifest = release.assets.iter().find(|asset| {
                asset.name == "latest.json" && asset.browser_download_url.starts_with(downloads)
            })?;
            Some((version, &manifest.browser_download_url))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .and_then(|(_, url)| url.parse().ok())
}

fn client() -> AppResult<reqwest::Client> {
    // reqwest is built without a bundled crypto provider; this installs the ring provider that
    // hatoba-core's sync transport and the updater also use. Whichever installs it first wins,
    // and all agree.
    static INSTALL_PROVIDER: Once = Once::new();
    INSTALL_PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    reqwest::Client::builder()
        .timeout(CHECK_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(concat!("Hatoba/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| AppError::internal("could not build the HTTP client"))
}

/// No HTTP answer at all (DNS, connection, TLS, timeout, a dropped download): the "offline"
/// code that the sync transport also reports.
fn unreachable(err: &reqwest::Error) -> AppError {
    tracing::info!("update: GitHub unreachable: {err}");
    let detail = if err.is_timeout() {
        "GitHub did not answer in time"
    } else {
        "GitHub could not be reached"
    };
    AppError::new(ErrorCode::SyncOffline, detail)
}

fn classify(err: UpdaterError) -> AppError {
    match err {
        UpdaterError::Reqwest(e) if !e.is_decode() => unreachable(&e),
        UpdaterError::Minisign(_)
        | UpdaterError::Base64(_)
        | UpdaterError::SignatureUtf8(_)
        | UpdaterError::SignedVersionMismatch { .. }
        | UpdaterError::MissingSignedVersion => {
            tracing::warn!("update refused: {err}");
            AppError::new(ErrorCode::UpdateSignature, err.to_string())
        }
        UpdaterError::Io(e) => AppError::io(e),
        other => AppError::internal(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::{Value, json};
    use tauri::test::MockRuntime;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    // A throwaway minisign key pair, made with `tauri signer generate`; its private half was
    // deleted after signing INSTALLER for version 0.2.0.
    const PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDZBOTg3OEJBMDI2RjkxODEKUldTQmtXOEN1bmlZYWxXbEhiTXYrT0VCVmR2U2RCUHpMWEltV3RkOEJxRlZScDBOTytPc2xkcGMK";
    const SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTQmtXOEN1bmlZYXZDMTNSbGdqcU5sWjRFSUVZY0twQTI2NThPRmYxR2FuaGhRRDJOemVET0Y0TDR3ejlEQ0ZqTlpBLzFzcGRWSjEwT2s2WWVhMHI4Mi9oTnIySTNZTFFNPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkxNTM2Nzc5CWZpbGU6aW5zdGFsbGVyLmV4ZQl2ZXJzaW9uOjAuMi4wCm1IdlhLdVlYMFFHMzIwSFdpU0VIVDMvUzZkNUtEdHg5QkRveitqbFFtWGtuUkl1VGM5QnZYejVtU25hUThsTXI5alFnU1ROdTg4RXJRbFgxZ0g0REJ3PT0K";
    const INSTALLER: &[u8] = b"Hatoba 0.2.0 installer stand-in\n";

    /// An app at version 0.1.0 whose updater reads `endpoint`, configured like tauri.conf.json
    /// apart from the test key and the plain-HTTP endpoint.
    fn app(endpoint: &str) -> tauri::App<MockRuntime> {
        app_at("0.1.0", endpoint)
    }

    fn app_at(version: &str, endpoint: &str) -> tauri::App<MockRuntime> {
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context.package_info_mut().version = version.parse().unwrap();
        context.config_mut().plugins.0.insert(
            "updater".into(),
            json!({
                "pubkey": PUBKEY,
                "endpoints": [endpoint],
                "requireSignedVersion": true,
                "dangerousInsecureTransportProtocol": true,
                "windows": { "installMode": "passive" },
            }),
        );
        tauri::test::mock_builder()
            .plugin(tauri_plugin_updater::Builder::new().build())
            .build(context)
            .expect("build the mock app")
    }

    /// A manifest like the one `release.mjs dist` writes, with an entry for every platform the
    /// tests may run on.
    fn manifest(server: &MockServer, version: &str) -> Value {
        let platform =
            json!({ "signature": SIGNATURE, "url": format!("{}/installer.exe", server.uri()) });
        let platforms: serde_json::Map<String, Value> = ["windows", "linux", "darwin"]
            .iter()
            .flat_map(|os| {
                ["x86_64", "aarch64"].map(|arch| (format!("{os}-{arch}"), platform.clone()))
            })
            .collect();
        json!({
            "version": version,
            "notes": "## Faster sync\n\nSync sends only what changed.",
            "pub_date": "2026-10-01T12:00:00Z",
            "platforms": platforms,
        })
    }

    async fn serve(server: &MockServer, latest: ResponseTemplate, installer: &[u8]) {
        Mock::given(method("GET"))
            .and(path("/latest.json"))
            .respond_with(latest)
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/installer.exe"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(installer.to_vec()))
            .mount(server)
            .await;
    }

    async fn check_at(server: &MockServer, updates: &Updates) -> AppResult<UpdateCheck> {
        let app = app(&format!("{}/latest.json", server.uri()));
        check(app.handle(), updates).await
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_download_that_cannot_be_unpacked_leaves_the_app_running() {
        use std::sync::Arc;

        let server = MockServer::start().await;
        let latest = manifest(&server, "0.2.0");
        serve(
            &server,
            ResponseTemplate::new(200).set_body_json(latest),
            INSTALLER,
        )
        .await;
        let app = app(&format!("{}/latest.json", server.uri()));
        let exited = Arc::new(AtomicBool::new(false));
        let hook = Arc::clone(&exited);
        let updates = Updates::default();
        check_with(app.handle(), &updates, &GITHUB, move || {
            hook.store(true, Ordering::SeqCst)
        })
        .await
        .unwrap();

        // Not an installer, so unpacking fails before the hook, and before the updater would
        // start the installer and exit this process.
        let update = updates.found().clone().unwrap();
        let err = update.install(INSTALLER).map_err(classify).unwrap_err();
        assert_eq!(err.code, ErrorCode::Internal);
        assert!(!exited.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_newer_signed_release_is_offered_and_downloads() {
        let server = MockServer::start().await;
        let latest = manifest(&server, "0.2.0");
        serve(
            &server,
            ResponseTemplate::new(200).set_body_json(latest),
            INSTALLER,
        )
        .await;
        let updates = Updates::default();

        let found = check_at(&server, &updates).await.unwrap();
        assert_eq!(
            found,
            UpdateCheck {
                current_version: "0.1.0".into(),
                update: Some(AvailableUpdate {
                    version: "0.2.0".into(),
                    notes: Some("## Faster sync\n\nSync sends only what changed.".into()),
                    published_at: Some(1_790_856_000_000),
                    release_url: "https://github.com/scarletkc/Hatoba/releases/tag/v0.2.0".into(),
                }),
            }
        );

        let update = updates.found().clone().expect("the check keeps the update");
        let progress = Mutex::new(Vec::new());
        let bytes = download(&update, |p| progress.lock().unwrap().push(p))
            .await
            .unwrap();
        assert_eq!(bytes, INSTALLER);
        let size = INSTALLER.len() as u64;
        assert_eq!(
            progress.into_inner().unwrap().last(),
            Some(&UpdateProgress::Downloading {
                downloaded: size,
                total: Some(size)
            })
        );
    }

    #[tokio::test]
    async fn an_installer_that_does_not_match_its_signature_is_refused() {
        let server = MockServer::start().await;
        let latest = manifest(&server, "0.2.0");
        serve(
            &server,
            ResponseTemplate::new(200).set_body_json(latest),
            b"tampered installer",
        )
        .await;
        let updates = Updates::default();
        check_at(&server, &updates).await.unwrap();

        let update = updates.found().clone().unwrap();
        let err = download(&update, |_| {}).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::UpdateSignature);
    }

    #[tokio::test]
    async fn a_manifest_announcing_another_version_than_the_signed_one_is_refused() {
        // A tampered manifest pairs a higher version with an older release's installer.
        let server = MockServer::start().await;
        let latest = manifest(&server, "0.3.0");
        serve(
            &server,
            ResponseTemplate::new(200).set_body_json(latest),
            INSTALLER,
        )
        .await;
        let updates = Updates::default();
        check_at(&server, &updates).await.unwrap();

        let update = updates.found().clone().unwrap();
        let err = download(&update, |_| {}).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::UpdateSignature);
        assert!(err.detail.contains("0.3.0"), "{}", err.detail);
    }

    #[tokio::test]
    async fn the_same_or_an_older_release_is_not_an_update() {
        for version in ["0.1.0", "0.1.0-rc.1", "0.0.9"] {
            let server = MockServer::start().await;
            let latest = manifest(&server, version);
            serve(
                &server,
                ResponseTemplate::new(200).set_body_json(latest),
                INSTALLER,
            )
            .await;
            let updates = Updates::default();
            let found = check_at(&server, &updates).await.unwrap();
            assert_eq!(found.update, None, "{version}");
            assert!(updates.found().is_none());
        }
    }

    #[tokio::test]
    async fn no_release_yet_is_up_to_date_and_forgets_an_earlier_update() {
        let server = MockServer::start().await;
        let latest = manifest(&server, "0.2.0");
        serve(
            &server,
            ResponseTemplate::new(200).set_body_json(latest),
            INSTALLER,
        )
        .await;
        let updates = Updates::default();
        check_at(&server, &updates).await.unwrap();
        assert!(updates.found().is_some());

        server.reset().await;
        serve(&server, ResponseTemplate::new(404), INSTALLER).await;
        let found = check_at(&server, &updates).await.unwrap();
        assert_eq!(
            found,
            UpdateCheck {
                current_version: "0.1.0".into(),
                update: None
            }
        );
        assert!(updates.found().is_none());
    }

    #[tokio::test]
    async fn unexpected_answers_are_errors() {
        for status in [403, 500, 502] {
            let server = MockServer::start().await;
            serve(&server, ResponseTemplate::new(status), INSTALLER).await;
            let err = check_at(&server, &Updates::default()).await.unwrap_err();
            assert_eq!(err.code, ErrorCode::Internal, "{status}");
            assert!(err.detail.contains(&status.to_string()), "{}", err.detail);
        }
        for body in ["<html>", r#"{"version":"0.2.0"}"#, ""] {
            let server = MockServer::start().await;
            serve(
                &server,
                ResponseTemplate::new(200).set_body_string(body),
                INSTALLER,
            )
            .await;
            let err = check_at(&server, &Updates::default()).await.unwrap_err();
            assert_eq!(err.code, ErrorCode::Internal, "{body}");
        }
    }

    #[tokio::test]
    async fn an_unreachable_endpoint_is_offline() {
        // A port that nothing listens on (wiremock keeps a dropped server listening for reuse).
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let endpoint = format!("http://127.0.0.1:{port}/latest.json");
        let err = check(app(&endpoint).handle(), &Updates::default())
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SyncOffline);
    }

    #[test]
    fn the_exit_hook_can_wait_for_async_work() {
        // `install` runs the updater on a blocking thread of Tauri's runtime, and the hook blocks
        // there until the MCP servers have stopped.
        let stopped = tauri::async_runtime::block_on(async {
            tauri::async_runtime::spawn_blocking(|| tauri::async_runtime::block_on(async { true }))
                .await
                .unwrap()
        });
        assert!(stopped);
    }

    #[test]
    fn one_install_at_a_time() {
        let flag = AtomicBool::new(false);
        let first = Installing::start(&flag).unwrap();
        assert!(Installing::start(&flag).is_err());
        drop(first);
        assert!(Installing::start(&flag).is_ok());
    }

    /// GitHub's list of releases as `api` on `server`, each release with the given assets.
    async fn list_releases(server: &MockServer, releases: Value) {
        Mock::given(method("GET"))
            .and(path("/releases"))
            .respond_with(ResponseTemplate::new(200).set_body_json(releases))
            .mount(server)
            .await;
    }

    fn release(server: &MockServer, tag: &str, manifest: Option<&str>) -> Value {
        let assets: Vec<Value> = manifest
            .map(|name| json!({ "name": "latest.json", "browser_download_url": format!("{}/{name}", server.uri()) }))
            .into_iter()
            .collect();
        json!({ "tag_name": tag, "draft": false, "prerelease": tag.contains('-'), "assets": assets })
    }

    async fn serve_manifest(server: &MockServer, name: &str, version: &str) {
        Mock::given(method("GET"))
            .and(path(format!("/{name}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(manifest(server, version)))
            .mount(server)
            .await;
    }

    async fn check_prerelease(server: &MockServer, version: &str) -> AppResult<UpdateCheck> {
        // The configured endpoint answers 404, so a result can only come from the list.
        let app = app_at(version, &format!("{}/configured/latest.json", server.uri()));
        let api = format!("{}/releases", server.uri());
        let downloads = format!("{}/", server.uri());
        let releases = Releases {
            api: &api,
            downloads: &downloads,
        };
        check_with(app.handle(), &Updates::default(), &releases, || {}).await
    }

    #[tokio::test]
    async fn a_prerelease_follows_the_newest_release_with_an_update_manifest() {
        let server = MockServer::start().await;
        let mut draft = release(&server, "v0.4.0", Some("draft.json"));
        draft["draft"] = json!(true);
        list_releases(
            &server,
            json!([
                draft,
                release(&server, "v0.3.0-alpha.1", None),
                release(&server, "v0.1.0", Some("stable.json")),
                release(&server, "v0.2.0-beta.1", Some("beta.json")),
                release(&server, "nightly", Some("nightly.json")),
            ]),
        )
        .await;
        serve_manifest(&server, "beta.json", "0.2.0-beta.1").await;

        let found = check_prerelease(&server, "0.2.0-alpha.1").await.unwrap();
        assert_eq!(found.current_version, "0.2.0-alpha.1");
        assert_eq!(found.update.unwrap().version, "0.2.0-beta.1");
    }

    #[tokio::test]
    async fn a_prerelease_moves_to_the_final_release() {
        let server = MockServer::start().await;
        list_releases(
            &server,
            json!([
                release(&server, "v0.2.0-rc.2", Some("rc.json")),
                release(&server, "v0.2.0", Some("final.json")),
            ]),
        )
        .await;
        serve_manifest(&server, "final.json", "0.2.0").await;

        let found = check_prerelease(&server, "0.2.0-rc.1").await.unwrap();
        assert_eq!(found.update.unwrap().version, "0.2.0");
    }

    #[tokio::test]
    async fn a_prerelease_is_up_to_date_without_a_release_manifest_from_the_repository() {
        let server = MockServer::start().await;
        let mut elsewhere = release(&server, "v0.3.0", None);
        elsewhere["assets"] = json!([
            { "name": "latest.json", "browser_download_url": "https://example.com/latest.json" }
        ]);
        list_releases(
            &server,
            json!([release(&server, "v0.1.0-alpha.1", None), elsewhere]),
        )
        .await;

        let found = check_prerelease(&server, "0.1.0-alpha.1").await.unwrap();
        assert_eq!(found.update, None);
    }

    #[tokio::test]
    async fn a_stable_version_never_asks_for_prereleases() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/releases"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/configured/latest.json"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let found = check_prerelease(&server, "0.1.0").await.unwrap();
        assert_eq!(found.update, None);
    }

    #[tokio::test]
    async fn a_failed_list_of_releases_is_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/releases"))
            .respond_with(ResponseTemplate::new(403).set_body_string("rate limited"))
            .mount(&server)
            .await;
        let err = check_prerelease(&server, "0.2.0-alpha.1")
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Internal);
        assert!(err.detail.contains("403"), "{}", err.detail);

        server.reset().await;
        list_releases(&server, json!({ "message": "Not Found" })).await;
        let err = check_prerelease(&server, "0.2.0-alpha.1")
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Internal);
    }
}
