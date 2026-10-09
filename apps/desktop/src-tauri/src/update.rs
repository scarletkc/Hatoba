//! Signed updates (spec §11). The Tauri updater reads `latest.json` from the endpoint in
//! tauri.conf.json, which the newest stable GitHub release carries, and installs the installer it
//! names only when the installer's signature matches the public key there.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use std::time::{Duration, Instant};

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_updater::{Error as UpdaterError, Update, Updater, UpdaterExt};

use crate::dto::{AvailableUpdate, UpdateCheck, UpdateProgress};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::state::AppState;

/// The release page of version X.Y.Z is this followed by X.Y.Z.
const RELEASE_PAGE: &str = "https://github.com/scarletkc/Hatoba/releases/tag/v";
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

/// Checks for a release newer than the running version and keeps it for [`install`].
pub async fn check<R: Runtime>(app: &AppHandle<R>, updates: &Updates) -> AppResult<UpdateCheck> {
    let found = match updater(app)?.check().await {
        Ok(found) => found,
        Err(UpdaterError::ReleaseNotFound) => {
            no_release(app).await?;
            None
        }
        Err(err) => return Err(classify(err)),
    };
    let check = UpdateCheck {
        current_version: app.package_info().version.to_string(),
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
    // The updater exits without RunEvent::Exit, which is where the MCP servers' child processes
    // are otherwise stopped.
    state.mcp.shutdown().await;
    tracing::info!("installing Hatoba {}", update.version);
    tauri::async_runtime::spawn_blocking(move || update.install(bytes))
        .await
        .map_err(|e| AppError::internal(format!("the installer task failed: {e}")))?
        .map_err(classify)?;
    // Windows never gets here: the updater exits once the installer starts. Elsewhere the new
    // version is in place and runs after a restart.
    app.restart()
}

fn updater<R: Runtime>(app: &AppHandle<R>) -> AppResult<Updater> {
    app.updater_builder()
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
async fn no_release<R: Runtime>(app: &AppHandle<R>) -> AppResult<()> {
    let endpoint =
        endpoint(app).ok_or_else(|| AppError::internal("no update endpoint is configured"))?;
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
fn endpoint<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    let updater = app.config().plugins.0.get("updater")?;
    updater
        .get("endpoints")?
        .get(0)?
        .as_str()
        .map(str::to_owned)
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
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
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
    fn one_install_at_a_time() {
        let flag = AtomicBool::new(false);
        let first = Installing::start(&flag).unwrap();
        assert!(Installing::start(&flag).is_err());
        drop(first);
        assert!(Installing::start(&flag).is_ok());
    }
}
